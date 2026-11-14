//! 設定アプリの Model 用の口（ヘッダの `ikari_settings_app`。ADR 0041 §2.3。L-C 後半で追加）。
//! 入出力は `ikari.settings.v1` の `AppRequest` / `AppReply`。`shell-core::settings` の純関数を呼ぶだけで、
//! 状態を持たない（どのスレッドからでも呼べる）。要求の欄が足りない・範囲外なら `IKARI_SHELL_ERR_DECODE`。

use ikari_proto::settings::v1 as app;
use ikari_proto::shell::v1 as pb;
use ikari_proto::v1 as engine;
use ikari_shell_core::settings::{
    self, CLAUSE_MIN_CHARS_MAX, CLAUSE_MIN_CHARS_MIN, CONVERT_WAIT_RANGE, DictionaryDraft,
    DraftProblem, IDLE_UNLOAD_RANGE, ImportSize, KeyChord, KeymapBinding, KeymapPresetTable,
    KeymapRowData, LM_DEADLINE_RANGE, MAX_IMPORT_BYTES, PAGE_SIZE_MAX, PAGE_SIZE_MIN, PlanError,
    SettingsIntent, SettingsValues,
};
use ikari_shell_core::{KeymapPreset, SettingsChanges};
use prost::Message;

use crate::convert::{
    key_from_pb, method_from_pb, method_to_pb, modifiers_from_pb, modifiers_to_pb, preset_from_pb,
    preset_to_pb,
};
use crate::panel::run;
use crate::shell::IKARI_SHELL_ERR_DECODE;
use crate::{IkariShellBuf, IkariStr};

type Reply = Result<app::app_reply::Body, i32>;

/// # Safety
/// `request` は借用の文字列（読める `len` バイト、または NULL）。`out` は NULL か書き込めること。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_app(request: IkariStr, out: *mut IkariShellBuf) -> i32 {
    run(request, out, |raw| {
        let request = app::AppRequest::decode(raw).map_err(|_| IKARI_SHELL_ERR_DECODE)?;
        let body = handle(request.body.ok_or(IKARI_SHELL_ERR_DECODE)?)?;
        Ok(app::AppReply { body: Some(body) }.encode_to_vec())
    })
}

/// 要求 1 つを処理する（FFI を通さないテストからも呼ぶ）。
pub fn handle(body: app::app_request::Body) -> Reply {
    use app::app_reply::Body as Out;
    use app::app_request::Body as In;
    Ok(match body {
        In::Describe(request) => {
            let values = values_from(&request.entries);
            let presets = presets_from(&request.presets)?;
            Out::Described(app::Described {
                values: Some(values_to_pb(&values)),
                rows: values
                    .keymap_rows(&presets)
                    .into_iter()
                    .map(row_to_pb)
                    .collect(),
            })
        }
        In::Plan(request) => {
            let values = values_from(&request.entries);
            let presets = presets_from(&request.presets)?;
            let intent = intent_from_pb(request.intent.and_then(|i| i.kind))?;
            Out::Planned(planned_to_pb(values.plan(&intent, &presets)))
        }
        In::Chord(request) => Out::Chord(match request.form.ok_or(IKARI_SHELL_ERR_DECODE)? {
            app::chord_request::Form::Record(key) => {
                let input = key_from_pb(key).map_err(|_| IKARI_SHELL_ERR_DECODE)?;
                record_chord(&input)
            }
            form => chord_info(form),
        }),
        In::DictionaryEntry(request) => {
            let draft = request.draft.unwrap_or_default();
            let draft = DictionaryDraft {
                reading: draft.reading,
                surface: draft.surface,
                pos_name: draft.pos_name,
                conj_type: draft.conj_type,
                comment: draft.comment,
            };
            use app::dictionary_entry_result::Result as R;
            let result = match settings::dictionary_entry_from_draft(&draft, &request.options) {
                Ok(entry) => R::Entry(entry),
                Err(problem) => R::Problem(draft_problem_to_pb(problem).into()),
            };
            Out::DictionaryEntry(app::DictionaryEntryResult {
                result: Some(result),
            })
        }
        In::DictionaryList(request) => Out::DictionaryList(app::DictionaryList {
            indices: settings::dictionary_display_order(&request.entries)
                .into_iter()
                .filter(|&i| {
                    settings::dictionary_entry_matches(&request.entries[i], &request.query)
                })
                .map(|i| u32::try_from(i).map_err(|_| IKARI_SHELL_ERR_DECODE))
                .collect::<Result<_, _>>()?,
        }),
        In::PosDisplay(request) => Out::Text(settings::pos_display(&request.pos)),
        In::ProfileSet(request) => {
            let rows = settings::profile_rows(&engine::ProfileReply {
                domains: request.rows,
            });
            let weight = settings::profile_weight_from_proto(request.weight);
            use app::profile_set_result::Result as R;
            let result = match settings::profile_set_request(&rows, &request.id, weight) {
                Some(set) => R::Request(set),
                None => R::Unchanged(pb::Empty {}),
            };
            Out::ProfileSet(app::ProfileSetResult {
                result: Some(result),
            })
        }
        In::ImportSize(request) => Out::ImportSize(
            match settings::import_size(request.size) {
                ImportSize::Ok => app::ImportSizeVerdict::Ok,
                ImportSize::TooLarge => app::ImportSizeVerdict::TooLarge,
                ImportSize::Empty => app::ImportSizeVerdict::Empty,
            }
            .into(),
        ),
        In::ExportFileName(request) => Out::Text(settings::export_file_name(
            request.year,
            request.month,
            request.day,
        )),
        In::Constants(_) => Out::Constants(constants()),
    })
}

fn values_from(entries: &std::collections::HashMap<String, String>) -> SettingsValues {
    SettingsValues::from_entries(entries.iter().map(|(k, v)| (k.as_str(), v.as_str())))
}

/// 渡されたプリセットだけを読む（渡されない・読めないプリセットは空）
fn presets_from(texts: &[app::PresetText]) -> Result<KeymapPresetTable, i32> {
    let mut by_preset: Vec<(KeymapPreset, &str)> = Vec::with_capacity(texts.len());
    for text in texts {
        let preset =
            preset_from_pb(text.preset, "PresetText.preset").map_err(|_| IKARI_SHELL_ERR_DECODE)?;
        by_preset.push((preset, text.tsv.as_str()));
    }
    Ok(KeymapPresetTable::new(|preset| {
        by_preset
            .iter()
            .find(|(p, _)| *p == preset)
            .map(|(_, tsv)| (*tsv).to_string())
    }))
}

fn values_to_pb(values: &SettingsValues) -> pb::SettingsSnapshot {
    pb::SettingsSnapshot {
        developer_lm_status: values.developer_lm_status,
        input_method: method_to_pb(values.input_method).into(),
        keymap_preset: preset_to_pb(values.keymap_preset).into(),
        keymap_overrides: values
            .keymap_bindings
            .iter()
            .filter_map(|(action, binding)| {
                let value = match binding {
                    KeymapBinding::Preset => return None,
                    KeymapBinding::None => settings::VALUE_NONE.to_string(),
                    KeymapBinding::Chord(text) => text.clone(),
                };
                Some((action.clone(), value))
            })
            .collect(),
        paging_wrap: values.paging_wrap,
        page_size: values.page_size,
        engine_lm: values.lm_enabled,
        engine_idle_unload_secs: values.idle_unload_secs,
        engine_lm_deadline_ms: values.lm_deadline_ms,
        engine_convert_wait_ms: values.convert_wait_ms,
        emoji_shortcode: values.emoji_shortcode,
        emoji_shortcode_exclude: values.emoji_shortcode_exclude.clone(),
        emoji_small_letters: values.emoji_small_letters,
        postal_japanese: values.postal_japanese,
        postal_english: values.postal_english,
        postal_te_prefix: values.postal_te_prefix,
        unicode_notation: values.unicode_notation,
        date_weekday_dates: values.date_weekday_dates,
        candidate_hide_when_confident: values.candidate_hide_when_confident,
        clause_min_chars: values.clause_min_chars,
    }
}

fn binding_to_pb(binding: &KeymapBinding) -> app::Binding {
    use app::binding::Kind;
    app::Binding {
        kind: Some(match binding {
            KeymapBinding::Preset => Kind::Preset(pb::Empty {}),
            KeymapBinding::None => Kind::None(pb::Empty {}),
            KeymapBinding::Chord(text) => Kind::Chord(text.clone()),
        }),
    }
}

/// 種類のない Binding は preset（proto3 の既定）
fn binding_from_pb(binding: Option<app::Binding>) -> KeymapBinding {
    use app::binding::Kind;
    match binding.and_then(|b| b.kind) {
        None | Some(Kind::Preset(_)) => KeymapBinding::Preset,
        Some(Kind::None(_)) => KeymapBinding::None,
        Some(Kind::Chord(text)) => KeymapBinding::Chord(text),
    }
}

fn row_to_pb(row: KeymapRowData) -> app::KeymapRow {
    app::KeymapRow {
        binding: Some(binding_to_pb(&row.binding)),
        action: row.action,
        key_text: row.key_text,
        preset_text: row.preset_text,
    }
}

fn intent_from_pb(kind: Option<app::intent::Kind>) -> Result<SettingsIntent, i32> {
    use SettingsIntent as I;
    use app::intent::Kind as K;
    Ok(match kind.ok_or(IKARI_SHELL_ERR_DECODE)? {
        K::InputMethod(v) => I::InputMethod(
            method_from_pb(v, "Intent.input_method").map_err(|_| IKARI_SHELL_ERR_DECODE)?,
        ),
        K::KeymapPreset(v) => I::KeymapPreset(
            preset_from_pb(v, "Intent.keymap_preset").map_err(|_| IKARI_SHELL_ERR_DECODE)?,
        ),
        K::PagingWrap(v) => I::PagingWrap(v),
        K::PageSize(v) => I::PageSize(v),
        K::LmEnabled(v) => I::LmEnabled(v),
        K::IdleUnloadSecs(v) => I::IdleUnloadSecs(v),
        K::LmDeadlineMs(v) => I::LmDeadlineMs(v),
        K::ConvertWaitMs(v) => I::ConvertWaitMs(v),
        K::ClauseMinChars(v) => I::ClauseMinChars(v),
        K::PostalJapanese(v) => I::PostalJapanese(v),
        K::PostalEnglish(v) => I::PostalEnglish(v),
        K::PostalTePrefix(v) => I::PostalTePrefix(v),
        K::CandidateHideWhenConfident(v) => I::CandidateHideWhenConfident(v),
        K::UnicodeNotation(v) => I::UnicodeNotation(v),
        K::DateWeekdayDates(v) => I::DateWeekdayDates(v),
        K::EmojiSmallLetters(v) => I::EmojiSmallLetters(v),
        K::EmojiShortcode(v) => I::EmojiShortcode(v),
        K::AddEmojiShortcodeExclude(v) => I::AddEmojiShortcodeExclude(v),
        K::RemoveEmojiShortcodeExclude(v) => I::RemoveEmojiShortcodeExclude(v),
        K::ResetEmojiShortcodeExclude(_) => I::ResetEmojiShortcodeExclude,
        K::Keymap(keymap) => I::Keymap {
            action: keymap.action,
            binding: binding_from_pb(keymap.binding),
        },
    })
}

fn changes_to_pb(changes: SettingsChanges) -> pb::SettingsChanges {
    pb::SettingsChanges {
        set: changes
            .set
            .into_iter()
            .map(|entry| pb::SettingsEntry {
                key: entry.key,
                value: entry.value,
            })
            .collect(),
        reset: changes.reset,
    }
}

fn planned_to_pb(plan: Result<Option<SettingsChanges>, PlanError>) -> app::Planned {
    use app::plan_error::Kind as E;
    use app::planned::Result as R;
    let result = match plan {
        Ok(Some(changes)) => R::Changes(changes_to_pb(changes)),
        Ok(None) => R::Unchanged(pb::Empty {}),
        Err(error) => R::Error(app::PlanError {
            kind: Some(match error {
                PlanError::InvalidBundleId => E::InvalidBundleId(pb::Empty {}),
                PlanError::UnknownAction => E::UnknownAction(pb::Empty {}),
                PlanError::KeyConflict {
                    chord_display,
                    other_action,
                } => E::KeyConflict(app::KeyConflict {
                    chord_display,
                    other_action,
                }),
            }),
        }),
    };
    app::Planned {
        result: Some(result),
    }
}

/// 表記を読む、または押したキーを表記に直して読み戻せるか確かめる（今の `KeyChordText`）
fn chord_info(form: app::chord_request::Form) -> app::ChordInfo {
    use app::chord_request::Form;
    let text = match form {
        // record は handle が先に受ける（`record_chord`）。ここへは来ないが、来ても落とさない
        Form::Record(_) => return app::ChordInfo::default(),
        Form::Text(text) => text,
        Form::Parts(parts) => {
            // 押したキーは、表記に直して読み戻せるときだけ valid。表記と表示は読み戻せなくても返す
            let chord = KeyChord::new(parts.key, modifiers_from_pb(parts.modifiers));
            let readable = KeyChord::parse(&chord.to_text()).as_ref() == Some(&chord);
            return app::ChordInfo {
                valid: readable,
                text: chord.to_text(),
                display: chord.display(),
                function_key: chord.is_function_key(),
                ..Default::default()
            };
        }
    };
    match KeyChord::parse(&text) {
        Some(chord) => app::ChordInfo {
            valid: true,
            text: chord.to_text(),
            display: chord.display(),
            function_key: chord.is_function_key(),
            ..Default::default()
        },
        None => app::ChordInfo {
            valid: false,
            text: String::new(),
            display: text,
            function_key: false,
            ..Default::default()
        },
    }
}

/// 押されたキーを割り当てとして受け入れるか（shell-core の `KeyChord::record`）
fn record_chord(input: &ikari_shell_core::KeyInput) -> app::ChordInfo {
    use app::ChordOutcome;
    use ikari_shell_core::keymap::{ChordRecording, ChordRefusal};
    let (outcome, chord) = match KeyChord::record(input) {
        ChordRecording::Chord(chord) => (ChordOutcome::Accepted, Some(chord)),
        ChordRecording::Cancel => (ChordOutcome::Cancel, None),
        ChordRecording::Refused(ChordRefusal::NeedsModifier) => (ChordOutcome::NeedsModifier, None),
        ChordRecording::Refused(ChordRefusal::Unsupported) => (ChordOutcome::UnsupportedKey, None),
    };
    app::ChordInfo {
        valid: chord.is_some(),
        text: chord.as_ref().map(KeyChord::to_text).unwrap_or_default(),
        display: chord.as_ref().map(KeyChord::display).unwrap_or_default(),
        function_key: chord.as_ref().is_some_and(KeyChord::is_function_key),
        outcome: outcome as i32,
        accepted: chord.as_ref().map(|chord| app::ChordParts {
            key: chord.key.clone(),
            modifiers: modifiers_to_pb(&chord.modifiers),
        }),
    }
}

fn draft_problem_to_pb(problem: DraftProblem) -> app::DraftProblem {
    match problem {
        DraftProblem::ReadingEmpty => app::DraftProblem::ReadingEmpty,
        DraftProblem::SurfaceEmpty => app::DraftProblem::SurfaceEmpty,
        DraftProblem::PosNotChosen => app::DraftProblem::PosNotChosen,
        DraftProblem::ConjTypeNeeded => app::DraftProblem::ConjTypeNeeded,
        DraftProblem::ControlCharacters => app::DraftProblem::ControlCharacters,
    }
}

fn range_to_pb(range: &std::ops::RangeInclusive<i64>) -> app::IntRange {
    app::IntRange {
        min: *range.start(),
        max: *range.end(),
    }
}

fn constants() -> app::Constants {
    app::Constants {
        defaults: Some(values_to_pb(&SettingsValues::default())),
        page_size: Some(range_to_pb(
            &(i64::from(PAGE_SIZE_MIN)..=i64::from(PAGE_SIZE_MAX)),
        )),
        idle_unload_secs: Some(range_to_pb(&IDLE_UNLOAD_RANGE)),
        lm_deadline_ms: Some(range_to_pb(&LM_DEADLINE_RANGE)),
        convert_wait_ms: Some(range_to_pb(&CONVERT_WAIT_RANGE)),
        clause_min_chars: Some(range_to_pb(
            &(i64::from(CLAUSE_MIN_CHARS_MIN)..=i64::from(CLAUSE_MIN_CHARS_MAX)),
        )),
        max_import_bytes: MAX_IMPORT_BYTES,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app::app_reply::Body as Out;
    use app::app_request::Body as In;

    fn presets() -> Vec<app::PresetText> {
        let root = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../shell-core/resources/keymap"
        );
        [
            (pb::KeymapPreset::Macos, "macos"),
            (pb::KeymapPreset::MsIme, "ms-ime"),
            (pb::KeymapPreset::Atok, "atok"),
        ]
        .into_iter()
        .map(|(preset, name)| app::PresetText {
            preset: preset.into(),
            tsv: std::fs::read_to_string(format!("{root}/{name}.tsv")).unwrap_or_default(),
        })
        .collect()
    }

    fn call(body: In) -> Out {
        // FFI を通して往復する
        let bytes = app::AppRequest { body: Some(body) }.encode_to_vec();
        let mut out = IkariShellBuf::EMPTY;
        let status = unsafe {
            ikari_settings_app(
                IkariStr {
                    ptr: bytes.as_ptr(),
                    len: bytes.len(),
                },
                &mut out,
            )
        };
        assert_eq!(status, crate::shell::IKARI_SHELL_OK);
        let raw = unsafe { crate::bytes(out.ptr, out.len) }.to_vec();
        unsafe { crate::ikari_shell_buf_free(out) };
        app::AppReply::decode(raw.as_slice())
            .ok()
            .and_then(|r| r.body)
            .expect("reply")
    }

    #[test]
    fn describe_of_nothing_is_the_defaults_and_lists_every_action() {
        let Out::Described(described) = call(In::Describe(app::Describe {
            entries: Default::default(),
            presets: presets(),
        })) else {
            panic!("expected described");
        };
        let Out::Constants(constants) = call(In::Constants(pb::Empty {})) else {
            panic!("expected constants");
        };
        assert_eq!(described.values, constants.defaults);
        assert_eq!(described.rows.len(), settings::KEYMAP_ACTIONS.len());
        assert_eq!(constants.page_size, Some(app::IntRange { min: 1, max: 9 }));
        assert_eq!(constants.max_import_bytes, 64 << 20);
    }

    #[test]
    fn plan_reports_unchanged_changes_and_conflicts() {
        let plan = |entries: &[(&str, &str)], kind: app::intent::Kind| {
            let Out::Planned(planned) = call(In::Plan(app::Plan {
                entries: entries
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                    .collect(),
                presets: presets(),
                intent: Some(app::Intent { kind: Some(kind) }),
            })) else {
                panic!("expected planned");
            };
            planned.result.expect("result")
        };
        use app::intent::Kind as K;
        use app::planned::Result as R;
        assert!(matches!(plan(&[], K::PagingWrap(true)), R::Unchanged(_)));
        let R::Changes(changes) = plan(&[], K::PageSize(42)) else {
            panic!("expected changes");
        };
        assert_eq!(changes.set[0].key, "paging.size");
        assert_eq!(changes.set[0].value, "9");
        assert!(matches!(
            plan(&[], K::AddEmojiShortcodeExclude("a b".into())),
            R::Error(app::PlanError {
                kind: Some(app::plan_error::Kind::InvalidBundleId(_))
            })
        ));
    }

    #[test]
    fn chords_are_read_written_and_displayed() {
        let info = |form| {
            let Out::Chord(info) = call(In::Chord(app::ChordRequest { form: Some(form) })) else {
                panic!("expected chord");
            };
            info
        };
        use app::chord_request::Form;
        let parsed = info(Form::Text("ctrl+;".into()));
        assert!(parsed.valid);
        assert_eq!(parsed.display, "⌃;");
        let unreadable = info(Form::Text("???".into()));
        assert!(!unreadable.valid);
        assert_eq!(unreadable.display, "???");
        let recorded = info(Form::Parts(app::ChordParts {
            key: "j".into(),
            modifiers: Some(pb::Modifiers {
                control: true,
                shift: true,
                ..Default::default()
            }),
        }));
        assert_eq!(recorded.text, "ctrl+shift+j");
        assert_eq!(recorded.display, "⌃⇧J");
        assert!(info(Form::Text("f6".into())).function_key);
        assert_eq!(parsed.outcome, app::ChordOutcome::Unspecified as i32);
    }

    #[test]
    fn pressed_keys_are_judged_by_shell_core() {
        use app::ChordOutcome as O;
        let judge = |kind: pb::KeyKind, text: &str, function: u32, modifiers: pb::Modifiers| {
            let form = app::chord_request::Form::Record(pb::KeyInput {
                kind: kind as i32,
                text: text.into(),
                function,
                modifiers: Some(modifiers),
                ..Default::default()
            });
            let Out::Chord(info) = call(In::Chord(app::ChordRequest { form: Some(form) })) else {
                panic!("expected chord");
            };
            info
        };
        let ctrl = pb::Modifiers {
            control: true,
            ..Default::default()
        };
        let none = pb::Modifiers::default();
        let accepted = judge(pb::KeyKind::Character, "j", 0, ctrl);
        assert_eq!(accepted.outcome, O::Accepted as i32);
        assert!(accepted.valid);
        let parts = accepted.accepted.clone().expect("accepted parts");
        assert_eq!(parts.key, "j");
        assert!(parts.modifiers.is_some_and(|m| m.control && !m.shift));
        assert_eq!(
            (accepted.text.as_str(), accepted.display.as_str()),
            ("ctrl+j", "⌃J")
        );
        let f6 = judge(pb::KeyKind::Function, "", 6, none);
        assert_eq!(f6.outcome, O::Accepted as i32);
        assert!(f6.function_key);
        assert_eq!(
            judge(pb::KeyKind::Escape, "", 0, none).outcome,
            O::Cancel as i32
        );
        let plain = judge(pb::KeyKind::Character, "j", 0, none);
        assert_eq!(plain.outcome, O::NeedsModifier as i32);
        assert!(!plain.valid && plain.text.is_empty());
        assert_eq!(
            judge(pb::KeyKind::Space, "", 0, ctrl).outcome,
            O::UnsupportedKey as i32
        );
        // 読み戻せない番号は、使えないキー
        assert_eq!(
            judge(pb::KeyKind::Function, "", 21, none).outcome,
            O::UnsupportedKey as i32
        );
    }

    #[test]
    fn missing_bodies_are_decode_errors() {
        assert_eq!(
            handle(In::Chord(app::ChordRequest { form: None })),
            Err(IKARI_SHELL_ERR_DECODE)
        );
        assert_eq!(
            handle(In::Plan(app::Plan::default())),
            Err(IKARI_SHELL_ERR_DECODE)
        );
    }
}
