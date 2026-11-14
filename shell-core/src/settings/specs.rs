//! 画面に出す設定の一覧（ADR 0040 §2.5、W4-s1）。キー、種類、既定値、画面の区分、文言の鍵、反映の扱い。
//!
//! **判断の置き場所: Rust**。macOS と Windows の設定画面は、この一覧に従って画面を作る（何を出すか、どの範囲か、
//! どこに置くか）。文言そのものは OS の画面が持つ（鍵は [`SettingSpec::label_key`]・[`SettingSpec::description_key`]。
//! 多言語化は後）。片方の OS の画面だけに設定を足さない。
//!
//! # 二重の定義を作らない
//! 検証と既定値の本体は `defaults.rs`（定数）、`parse.rs`（検査）、`values.rs`（範囲）のまま。一覧はそれらから作る:
//! 既定値は `effective_entries(default_snapshot())` から引き、範囲と選択肢は同じ定数から引く。
//! 一覧と本体のずれはテストで止める。
//!
//! 値の書き込みは、画面が `SettingsIntent`（`values.rs`）にして `SettingsValues::plan` へ渡す。一覧は「何があるか」を
//! 言い、「どう書くか」は言わない（`on`/`off` と `true`/`false` の違いなどは本体が持つ）。

use std::fmt::Write as _;
use std::ops::RangeInclusive;

use crate::types::InputMethodKind;

use super::defaults::*;
use super::parse::effective_entries;
use super::values::{
    CLAUSE_MIN_CHARS_RANGE, CONVERT_WAIT_RANGE, IDLE_UNLOAD_RANGE, LM_DEADLINE_RANGE,
};

/// 画面。`settings.tsv` のキーを持つ画面だけ（ユーザー辞書、学習、このアプリについては、キーがないので一覧の外）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsScreen {
    /// 入力（macOS の `SettingsTab.input`）
    Input,
    /// 変換（macOS の `SettingsTab.conversion`）
    Conversion,
}

impl SettingsScreen {
    /// 書き出しに使う名前（macOS の `SettingsTab` の rawValue と同じ。変えない）
    pub fn id(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Conversion => "conversion",
        }
    }
}

/// 画面の中の節（画面での並びの順）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsSection {
    InputMethod,
    KeymapPreset,
    KeymapActions,
    Candidate,
    ClauseDelete,
    LanguageModel,
    EmojiShortcode,
    Postal,
    UnicodeNotation,
    WeekdayDates,
    /// 開発者向け（ADR 0061）。変換の画面のいちばん下
    Developer,
}

impl SettingsSection {
    /// 節を置く画面
    pub fn screen(self) -> SettingsScreen {
        match self {
            Self::InputMethod | Self::KeymapPreset | Self::KeymapActions => SettingsScreen::Input,
            Self::Candidate
            | Self::ClauseDelete
            | Self::LanguageModel
            | Self::EmojiShortcode
            | Self::Postal
            | Self::UnicodeNotation
            | Self::WeekdayDates
            | Self::Developer => SettingsScreen::Conversion,
        }
    }

    /// 文言の鍵や一覧の書き出しに使う名前（変えない）
    pub fn id(self) -> &'static str {
        match self {
            Self::InputMethod => "input_method",
            Self::KeymapPreset => "keymap_preset",
            Self::KeymapActions => "keymap_actions",
            Self::Candidate => "candidate",
            Self::ClauseDelete => "clause_delete",
            Self::LanguageModel => "language_model",
            Self::EmojiShortcode => "emoji_shortcode",
            Self::Postal => "postal",
            Self::UnicodeNotation => "unicode_notation",
            Self::WeekdayDates => "weekday_dates",
            Self::Developer => "developer",
        }
    }

    /// 節の題の文言の鍵。文言は OS の画面が持つ
    pub fn title_key(self) -> String {
        format!("settings.section.{}.title", self.id())
    }
}

/// 値の種類。画面は種類から部品を選ぶ（スイッチ、数の選択、選択肢、リスト、キー入力）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingKind {
    /// オン・オフ
    Bool,
    /// 整数（両端を含む。範囲の外は `SettingsValues::plan` が範囲に丸める）
    Int { min: i64, max: i64 },
    /// 決まった選択肢のどれか（`settings.tsv` の書き方）
    Enum { choices: Vec<&'static str> },
    /// Bundle ID の並び（アプリの除外リスト。追加・削除・既定に戻す）
    BundleIdList,
    /// キー割り当て（`ctrl+j` の表記、`none` で外す、行なしでプリセットのまま）
    KeyChord,
}

/// 変更が効くまでの扱い。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reflect {
    /// 次のキー入力から効く
    Immediate,
    /// エンジンを起こし直す（`engine_settings_differ` と同じ。読み込み中の数秒は辞書だけで変換する）
    EngineRestart,
}

/// 画面に出す設定 1 件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingSpec {
    /// `settings.tsv` のキー（`keymap.<操作>` を含む）
    pub key: String,
    pub kind: SettingKind,
    /// 既定値（`settings.tsv` の書き方）。`None` は「行がない」が既定（キー割り当て: プリセットのまま）
    pub default: Option<String>,
    pub section: SettingsSection,
    pub reflect: Reflect,
}

impl SettingSpec {
    pub fn screen(&self) -> SettingsScreen {
        self.section.screen()
    }

    /// 表示名の文言の鍵。文言は OS の画面が持つ
    pub fn label_key(&self) -> String {
        format!("settings.{}.label", self.key)
    }

    /// 説明の文言の鍵。文言は OS の画面が持つ（説明のない設定は画面が何も出さなくてよい）
    pub fn description_key(&self) -> String {
        format!("settings.{}.description", self.key)
    }
}

/// 画面に出す設定の全部。画面での並びの順（画面ごと、節ごと、節の中）。
pub fn all_specs() -> Vec<SettingSpec> {
    use Reflect::{EngineRestart, Immediate};
    use SettingsSection as Sec;

    let defaults: std::collections::HashMap<String, String> =
        effective_entries(&default_snapshot()).into_iter().collect();
    let spec = |key: &str, kind: SettingKind, section: Sec, reflect: Reflect| SettingSpec {
        key: key.to_string(),
        kind,
        default: defaults.get(key).cloned(),
        section,
        reflect,
    };
    let int = |range: &RangeInclusive<i64>| SettingKind::Int {
        min: *range.start(),
        max: *range.end(),
    };
    let page_size = i64::from(PAGE_SIZE_MIN)..=i64::from(PAGE_SIZE_MAX);

    let mut specs = vec![
        spec(
            KEY_INPUT_METHOD,
            SettingKind::Enum {
                choices: vec![
                    input_method_name(InputMethodKind::Romaji),
                    input_method_name(InputMethodKind::Kana),
                ],
            },
            Sec::InputMethod,
            Immediate,
        ),
        spec(
            KEY_KEYMAP_PRESET,
            SettingKind::Enum {
                choices: KEYMAP_PRESETS.map(preset_name).to_vec(),
            },
            Sec::KeymapPreset,
            Immediate,
        ),
    ];
    specs.extend(KEYMAP_ACTIONS.iter().map(|action| {
        spec(
            &format!("{KEY_KEYMAP_PREFIX}{action}"),
            SettingKind::KeyChord,
            Sec::KeymapActions,
            Immediate,
        )
    }));
    specs.extend([
        spec(
            KEY_PAGING_WRAP,
            SettingKind::Bool,
            Sec::Candidate,
            Immediate,
        ),
        spec(KEY_PAGING_SIZE, int(&page_size), Sec::Candidate, Immediate),
        spec(
            KEY_CANDIDATE_HIDE_WHEN_CONFIDENT,
            SettingKind::Bool,
            Sec::Candidate,
            Immediate,
        ),
        spec(
            KEY_CLAUSE_MIN_CHARS,
            int(&CLAUSE_MIN_CHARS_RANGE),
            Sec::ClauseDelete,
            Immediate,
        ),
        spec(
            KEY_ENGINE_LM,
            SettingKind::Bool,
            Sec::LanguageModel,
            EngineRestart,
        ),
        spec(
            KEY_ENGINE_IDLE_UNLOAD_SECS,
            int(&IDLE_UNLOAD_RANGE),
            Sec::LanguageModel,
            EngineRestart,
        ),
        spec(
            KEY_ENGINE_LM_DEADLINE_MS,
            int(&LM_DEADLINE_RANGE),
            Sec::LanguageModel,
            EngineRestart,
        ),
        spec(
            KEY_ENGINE_CONVERT_WAIT_MS,
            int(&CONVERT_WAIT_RANGE),
            Sec::LanguageModel,
            EngineRestart,
        ),
        spec(
            KEY_EMOJI_SHORTCODE,
            SettingKind::Bool,
            Sec::EmojiShortcode,
            Immediate,
        ),
        spec(
            KEY_EMOJI_SMALL_LETTERS,
            SettingKind::Bool,
            Sec::EmojiShortcode,
            Immediate,
        ),
        spec(
            KEY_EMOJI_SHORTCODE_EXCLUDE,
            SettingKind::BundleIdList,
            Sec::EmojiShortcode,
            Immediate,
        ),
        spec(
            KEY_POSTAL_JAPANESE,
            SettingKind::Bool,
            Sec::Postal,
            EngineRestart,
        ),
        spec(
            KEY_POSTAL_TE_PREFIX,
            SettingKind::Bool,
            Sec::Postal,
            EngineRestart,
        ),
        spec(
            KEY_POSTAL_ENGLISH,
            SettingKind::Bool,
            Sec::Postal,
            Immediate,
        ),
        spec(
            KEY_UNICODE_NOTATION,
            SettingKind::Bool,
            Sec::UnicodeNotation,
            EngineRestart,
        ),
        spec(
            KEY_DATE_WEEKDAY_DATES,
            SettingKind::Bool,
            Sec::WeekdayDates,
            EngineRestart,
        ),
        spec(
            KEY_DEVELOPER_LM_STATUS,
            SettingKind::Bool,
            Sec::Developer,
            Immediate,
        ),
    ]);
    specs
}

/// `screen` に出す設定だけ（並びは `all_specs` のまま）。
pub fn specs_for(screen: SettingsScreen) -> Vec<SettingSpec> {
    all_specs()
        .into_iter()
        .filter(|spec| spec.screen() == screen)
        .collect()
}

/// 一覧の書き出し（1 行 1 件、タブ区切り）。両 OS の画面のテストが読む。
/// 列: キー、種類、節、画面、既定値（なしは空）、反映。
pub fn specs_tsv() -> String {
    let mut out = String::new();
    for spec in all_specs() {
        let kind = match &spec.kind {
            SettingKind::Bool => "bool".to_string(),
            SettingKind::Int { min, max } => format!("int:{min}..{max}"),
            SettingKind::Enum { choices } => format!("enum:{}", choices.join("|")),
            SettingKind::BundleIdList => "bundle_id_list".to_string(),
            SettingKind::KeyChord => "key_chord".to_string(),
        };
        let reflect = match spec.reflect {
            Reflect::Immediate => "immediate",
            Reflect::EngineRestart => "engine_restart",
        };
        let _ = writeln!(
            out,
            "{}\t{kind}\t{}\t{}\t{}\t{reflect}",
            spec.key,
            spec.section.id(),
            spec.screen().id(),
            spec.default.as_deref().unwrap_or("")
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::super::parse::{engine_settings_differ, is_acceptable, is_known_key, parse_tsv};
    use super::*;
    use crate::types::KeymapPreset;

    /// macOS の画面のテストが読む一覧の書き出し（`specs_tsv` と同じ中身。ずれたらこのテストが落ちる）
    const GOLDEN_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../macos/Resources/settings-specs.tsv"
    );
    /// 設定すると、書き出しを作り直す
    const UPDATE_ENV: &str = "IKARI_UPDATE_SPECS_GOLDEN";

    fn spec_of(key: &str) -> SettingSpec {
        all_specs()
            .into_iter()
            .find(|spec| spec.key == key)
            .unwrap_or_else(|| panic!("spec for {key}"))
    }

    fn snapshot_with(key: &str, value: &str) -> crate::types::SettingsSnapshot {
        parse_tsv(&format!("{key}\t{value}")).snapshot
    }

    /// 真偽の書き方（`paging.wrap` だけ `true`/`false`、ほかは `on`/`off`）
    fn bool_words(key: &str) -> (&'static str, &'static str) {
        if key == KEY_PAGING_WRAP {
            (VALUE_TRUE, VALUE_FALSE)
        } else {
            (VALUE_ON, VALUE_OFF)
        }
    }

    #[test]
    fn keys_are_unique_and_known() {
        let specs = all_specs();
        let keys: BTreeSet<&str> = specs.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys.len(), specs.len(), "duplicate key");
        for spec in &specs {
            assert!(is_known_key(&spec.key), "{} is not a known key", spec.key);
        }
    }

    /// 一覧と `effective_entries`（`SettingsSnapshot` の全項目）のキーが同じ。どちらかに足し忘れると落ちる
    #[test]
    fn keys_match_every_setting_in_the_snapshot() {
        let mut snapshot = default_snapshot();
        for action in KEYMAP_ACTIONS {
            snapshot
                .keymap_overrides
                .insert(action.to_string(), "f1".to_string());
        }
        let from_values: BTreeSet<String> = effective_entries(&snapshot)
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        let from_specs: BTreeSet<String> = all_specs().into_iter().map(|s| s.key).collect();
        assert_eq!(from_specs, from_values);
    }

    #[test]
    fn defaults_follow_the_default_snapshot() {
        let entries: std::collections::HashMap<String, String> =
            effective_entries(&default_snapshot()).into_iter().collect();
        for spec in all_specs() {
            assert_eq!(
                spec.default,
                entries.get(&spec.key).cloned(),
                "{}",
                spec.key
            );
            assert_eq!(
                spec.default.is_none(),
                spec.kind == SettingKind::KeyChord,
                "only key bindings have no default line: {}",
                spec.key
            );
            if let Some(default) = &spec.default {
                assert!(is_acceptable(&spec.key, default), "{}", spec.key);
            }
        }
    }

    #[test]
    fn every_key_binding_is_listed_once() {
        let bindings = all_specs()
            .into_iter()
            .filter(|s| s.kind == SettingKind::KeyChord)
            .count();
        assert_eq!(bindings, KEYMAP_ACTIONS.len());
        assert_eq!(
            spec_of("keymap.clause.undo").section,
            SettingsSection::KeymapActions
        );
    }

    #[test]
    fn kinds_agree_with_validation() {
        for spec in all_specs() {
            let key = spec.key.as_str();
            match &spec.kind {
                SettingKind::Bool => {
                    let (on, off) = bool_words(key);
                    assert!(is_acceptable(key, on), "{key}");
                    assert!(is_acceptable(key, off), "{key}");
                    assert!(!is_acceptable(key, "maybe"), "{key}");
                }
                SettingKind::Int { min, max } => {
                    let default: i64 = spec.default.as_deref().unwrap().parse().unwrap();
                    assert!((*min..=*max).contains(&default), "{key}");
                    assert!(is_acceptable(key, &min.to_string()), "{key}");
                    assert!(is_acceptable(key, &max.to_string()), "{key}");
                    assert!(!is_acceptable(key, "many"), "{key}");
                }
                SettingKind::Enum { choices } => {
                    assert!(choices.len() >= 2, "{key}");
                    for choice in choices {
                        assert!(is_acceptable(key, choice), "{key} {choice}");
                    }
                    assert!(!is_acceptable(key, "no-such-choice"), "{key}");
                    assert!(choices.contains(&spec.default.as_deref().unwrap()));
                }
                SettingKind::BundleIdList => {
                    assert!(is_acceptable(key, "com.example.app"), "{key}");
                }
                SettingKind::KeyChord => {
                    assert!(is_acceptable(key, "ctrl+j"), "{key}");
                    assert!(is_acceptable(key, VALUE_NONE), "{key}");
                }
            }
        }
    }

    #[test]
    fn key_preset_choices_are_all_presets() {
        let SettingKind::Enum { choices } = spec_of(KEY_KEYMAP_PRESET).kind else {
            panic!("enum");
        };
        let presets = [KeymapPreset::Macos, KeymapPreset::MsIme, KeymapPreset::Atok];
        assert_eq!(choices, presets.map(preset_name).to_vec());
    }

    /// `Reflect::EngineRestart` は `engine_settings_differ` と同じ（ADR 0005 §7.3）
    #[test]
    fn reflect_matches_engine_settings_differ() {
        let base = default_snapshot();
        for spec in all_specs() {
            let default = spec.default.as_deref().unwrap_or("");
            let alt = match &spec.kind {
                SettingKind::Bool => {
                    let (on, off) = bool_words(&spec.key);
                    if default == on { off } else { on }.to_string()
                }
                SettingKind::Int { min, max } => {
                    if default == min.to_string() { max } else { min }.to_string()
                }
                // 選択肢、リスト、キー割り当てはエンジンに関わらない
                _ => {
                    assert_eq!(spec.reflect, Reflect::Immediate, "{}", spec.key);
                    continue;
                }
            };
            let changed = snapshot_with(&spec.key, &alt);
            assert_ne!(changed, base, "{}: alt value had no effect", spec.key);
            assert_eq!(
                engine_settings_differ(&base, &changed),
                spec.reflect == Reflect::EngineRestart,
                "{}",
                spec.key
            );
        }
    }

    /// 同じ節の設定は続いて並び、節は画面ごとにまとまる（画面は節をこの順に並べる）
    #[test]
    fn specs_are_grouped_by_section_then_screen() {
        let mut sections: Vec<SettingsSection> = Vec::new();
        for spec in all_specs() {
            if sections.last() != Some(&spec.section) {
                assert!(
                    !sections.contains(&spec.section),
                    "{:?} is split",
                    spec.section
                );
                sections.push(spec.section);
            }
        }
        let mut screens: Vec<SettingsScreen> = sections.iter().map(|s| s.screen()).collect();
        screens.dedup();
        assert_eq!(
            screens,
            vec![SettingsScreen::Input, SettingsScreen::Conversion]
        );
        let per_screen =
            specs_for(SettingsScreen::Input).len() + specs_for(SettingsScreen::Conversion).len();
        assert_eq!(per_screen, all_specs().len());
    }

    #[test]
    fn label_and_description_keys_are_derived_from_the_key() {
        let spec = spec_of(KEY_PAGING_SIZE);
        assert_eq!(spec.label_key(), "settings.paging.size.label");
        assert_eq!(spec.description_key(), "settings.paging.size.description");
        assert_eq!(
            SettingsSection::Postal.title_key(),
            "settings.section.postal.title"
        );
    }

    /// macOS の画面のテストが読むファイルが、今の一覧と同じ（`IKARI_UPDATE_SPECS_GOLDEN=1` で作り直す）
    #[test]
    fn golden_file_for_the_screens_is_current() {
        let expected = specs_tsv();
        if std::env::var_os(UPDATE_ENV).is_some() {
            std::fs::write(GOLDEN_PATH, &expected).unwrap();
        }
        // Windows では git が改行を CRLF で取り出すことがある（autocrlf）。LF と CRLF のどちらでも同じに読む
        let actual = std::fs::read_to_string(GOLDEN_PATH)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert_eq!(
            actual, expected,
            "macos/Resources/settings-specs.tsv is stale. Run with {UPDATE_ENV}=1"
        );
    }
}
