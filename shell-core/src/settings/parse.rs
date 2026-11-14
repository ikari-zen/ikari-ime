//! `settings.tsv` の解釈（今の `Settings.parse(tsv:)` と周辺。Settings.swift）。
//!
//! 未知のキーと不正な値は無視して既定値を使い、警告（`SettingsIssue`）に積む（読み込みで入力を止めない）。
//! 警告は行番号とキー名と種類だけで、値の文字列は含めない（ログにユーザーの文字列を残さない）。

use super::SettingsValidation;
use super::chord::KeyChord;
use super::defaults::*;
use super::text::{has_forbidden, split_lines, trim_blank};
use crate::types::{SettingsIssue, SettingsIssueKind, SettingsSnapshot};

/// TSV を読む。`engine.lm`、`engine.idle_unload_secs`、`engine.lm_deadline_ms`、`engine.convert_wait_ms` は殻も読む
/// （メニューとエンジンの再起動の判定のため）。値が不正なら警告。それ以外の `engine.*` は警告なしで無視する。
pub(super) fn parse_tsv(tsv: &str) -> SettingsValidation {
    let mut settings = default_snapshot();
    let mut issues: Vec<SettingsIssue> = Vec::new();

    // 先頭の BOM（Windows のメモ帳など）は、最初のキーに混ざらないように外す（ADR 0040 §2.9）
    let tsv = ikari_textfile::strip_bom(tsv);
    for (offset, raw_line) in split_lines(tsv).into_iter().enumerate() {
        let number = u32::try_from(offset + 1).unwrap_or(u32::MAX);
        let line = raw_line.trim_matches('\r');
        if trim_blank(line).is_empty() || line.starts_with('#') {
            continue;
        }
        let mut columns = line.splitn(2, '\t');
        let (Some(raw_key), Some(raw_value)) = (columns.next(), columns.next()) else {
            issues.push(issue(number, "", SettingsIssueKind::Malformed));
            continue;
        };
        let key = trim_blank(raw_key);
        let value = trim_blank(raw_value);
        if let Some(kind) = apply_line(&mut settings, key, value) {
            issues.push(issue(number, key, kind));
        }
    }
    SettingsValidation {
        snapshot: settings,
        issues,
    }
}

/// 1 行を設定に当てる。不正なら警告の種類を返す（設定は変えない）
fn apply_line(
    settings: &mut SettingsSnapshot,
    key: &str,
    value: &str,
) -> Option<SettingsIssueKind> {
    use SettingsIssueKind::{InvalidValue, UnknownKey};
    let ok = |applied: bool| if applied { None } else { Some(InvalidValue) };

    if key.starts_with(KEY_ENGINE_PREFIX) {
        return match key {
            KEY_ENGINE_LM => set_switch(value, &mut settings.engine_lm),
            KEY_ENGINE_IDLE_UNLOAD_SECS => set_count(value, &mut settings.engine_idle_unload_secs),
            KEY_ENGINE_LM_DEADLINE_MS => set_count(value, &mut settings.engine_lm_deadline_ms),
            KEY_ENGINE_CONVERT_WAIT_MS => set_count(value, &mut settings.engine_convert_wait_ms),
            // それ以外の `engine.*` は無視する（警告も出さない）
            _ => None,
        };
    }
    if let Some(action) = key
        .strip_prefix(KEY_KEYMAP_PREFIX)
        .filter(|_| key != KEY_KEYMAP_PRESET)
    {
        if !is_keymap_action(action) {
            return Some(UnknownKey);
        }
        if value == VALUE_NONE || KeyChord::parse(value).is_some() {
            settings
                .keymap_overrides
                .insert(action.to_string(), value.to_string());
            return None;
        }
        return Some(InvalidValue);
    }

    match key {
        KEY_EMOJI_SHORTCODE_EXCLUDE => ok(parse_bundle_id_list(value)
            .map(|list| settings.emoji_shortcode_exclude = list)
            .is_some()),
        KEY_EMOJI_SHORTCODE => set_switch(value, &mut settings.emoji_shortcode),
        KEY_EMOJI_SMALL_LETTERS => set_switch(value, &mut settings.emoji_small_letters),
        KEY_POSTAL_JAPANESE => set_switch(value, &mut settings.postal_japanese),
        KEY_POSTAL_ENGLISH => set_switch(value, &mut settings.postal_english),
        KEY_POSTAL_TE_PREFIX => set_switch(value, &mut settings.postal_te_prefix),
        KEY_UNICODE_NOTATION => set_switch(value, &mut settings.unicode_notation),
        KEY_DATE_WEEKDAY_DATES => set_switch(value, &mut settings.date_weekday_dates),
        KEY_CANDIDATE_HIDE_WHEN_CONFIDENT => {
            set_switch(value, &mut settings.candidate_hide_when_confident)
        }
        KEY_DEVELOPER_LM_STATUS => set_switch(value, &mut settings.developer_lm_status),
        KEY_INPUT_METHOD => ok(input_method_from_name(value)
            .map(|method| settings.input_method = method)
            .is_some()),
        KEY_KEYMAP_PRESET => ok(preset_from_name(value)
            .map(|preset| settings.keymap_preset = preset)
            .is_some()),
        KEY_PAGING_WRAP => match value {
            VALUE_TRUE => {
                settings.paging_wrap = true;
                None
            }
            VALUE_FALSE => {
                settings.paging_wrap = false;
                None
            }
            _ => Some(InvalidValue),
        },
        KEY_CLAUSE_MIN_CHARS => match value.parse::<u32>() {
            Ok(n) if (CLAUSE_MIN_CHARS_MIN..=CLAUSE_MIN_CHARS_MAX).contains(&n) => {
                settings.clause_min_chars = n;
                None
            }
            _ => Some(InvalidValue),
        },
        KEY_PAGING_SIZE => match value.parse::<u32>() {
            Ok(size) if (PAGE_SIZE_MIN..=PAGE_SIZE_MAX).contains(&size) => {
                settings.page_size = size;
                None
            }
            _ => Some(InvalidValue),
        },
        _ => Some(UnknownKey),
    }
}

fn set_switch(value: &str, target: &mut bool) -> Option<SettingsIssueKind> {
    match parse_switch(value) {
        Some(on) => {
            *target = on;
            None
        }
        None => Some(SettingsIssueKind::InvalidValue),
    }
}

fn set_count(value: &str, target: &mut u32) -> Option<SettingsIssueKind> {
    match parse_count(value) {
        Some(n) => {
            *target = n;
            None
        }
        None => Some(SettingsIssueKind::InvalidValue),
    }
}

fn issue(line: u32, key: &str, kind: SettingsIssueKind) -> SettingsIssue {
    SettingsIssue {
        line,
        key: key.to_string(),
        kind,
    }
}

/// `on` / `off`
fn parse_switch(value: &str) -> Option<bool> {
    match value {
        VALUE_ON => Some(true),
        VALUE_OFF => Some(false),
        _ => None,
    }
}

/// 0 以上の整数。Swift は `Int`（64 ビット）、ここは `u32`（`SettingsSnapshot` の型）。
/// 範囲を超える値は不正として扱う（Swift では通るが、実用の値ではない）
fn parse_count(value: &str) -> Option<u32> {
    value.parse::<u32>().ok()
}

pub(super) fn switch_value(on: bool) -> &'static str {
    if on { VALUE_ON } else { VALUE_OFF }
}

/// Bundle ID の一覧（`,` 区切り）。空の値は空の一覧。空白は削り、空の要素は捨てて、重複は最初だけ残す。
/// 使えない文字（英数字、`.`、`-`、`_` 以外）があれば `None`
pub fn parse_bundle_id_list(value: &str) -> Option<Vec<String>> {
    let mut result: Vec<String> = Vec::new();
    for part in value.split(LIST_SEPARATOR) {
        let id = trim_blank(part);
        if id.is_empty() {
            continue;
        }
        if !id
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'))
        {
            return None;
        }
        if !result.iter().any(|existing| existing == id) {
            result.push(id.to_string());
        }
    }
    Some(result)
}

/// 殻の設定アプリ・メニューが書き換えてよい（既知の）キーか。`keymap.<操作>` は操作の名前が既知のものだけ
pub fn is_known_key(key: &str) -> bool {
    match key {
        KEY_INPUT_METHOD
        | KEY_KEYMAP_PRESET
        | KEY_PAGING_WRAP
        | KEY_PAGING_SIZE
        | KEY_ENGINE_LM
        | KEY_ENGINE_IDLE_UNLOAD_SECS
        | KEY_ENGINE_LM_DEADLINE_MS
        | KEY_ENGINE_CONVERT_WAIT_MS
        | KEY_EMOJI_SHORTCODE
        | KEY_EMOJI_SHORTCODE_EXCLUDE
        | KEY_EMOJI_SMALL_LETTERS
        | KEY_POSTAL_JAPANESE
        | KEY_POSTAL_ENGLISH
        | KEY_POSTAL_TE_PREFIX
        | KEY_UNICODE_NOTATION
        | KEY_DATE_WEEKDAY_DATES
        | KEY_CANDIDATE_HIDE_WHEN_CONFIDENT
        | KEY_DEVELOPER_LM_STATUS
        | KEY_CLAUSE_MIN_CHARS => true,
        _ => key
            .strip_prefix(KEY_KEYMAP_PREFIX)
            .is_some_and(is_keymap_action),
    }
}

/// 書いてよい（既知の）キーと値か。値の検査は `parse_tsv` と同じ規則。
/// 行の区切りや制御文字を含むものは偽（ファイルに別の行を混ぜられないように）
pub fn is_acceptable(key: &str, value: &str) -> bool {
    if !is_known_key(key) || has_forbidden(value) {
        return false;
    }
    parse_tsv(&format!("{key}\t{value}")).issues.is_empty()
}

/// `engine.*` と、エンジンが起動時に読む `postal.japanese`、`postal.te_prefix`、`unicode.notation`、
/// `date.weekday_dates` のどれかが違う。違えば、殻はエンジンへ `Shutdown{SETTINGS_CHANGED}` を送って起こし直す
/// （ADR 0005 §7.3）
pub fn engine_settings_differ(a: &SettingsSnapshot, b: &SettingsSnapshot) -> bool {
    a.engine_lm != b.engine_lm
        || a.engine_idle_unload_secs != b.engine_idle_unload_secs
        || a.engine_lm_deadline_ms != b.engine_lm_deadline_ms
        || a.engine_convert_wait_ms != b.engine_convert_wait_ms
        || a.postal_japanese != b.postal_japanese
        || a.postal_te_prefix != b.postal_te_prefix
        || a.unicode_notation != b.unicode_notation
        || a.date_weekday_dates != b.date_weekday_dates
}

/// 設定アプリに返す「今効いている値」。キーは `settings.tsv` のキー、値は書き方そのまま。
/// `keymap.<操作>` は上書きがあるものだけ（なければプリセットのまま）。順は固定。
pub fn effective_entries(settings: &SettingsSnapshot) -> Vec<(String, String)> {
    let join = |list: &[String]| list.join(&LIST_SEPARATOR.to_string());
    let mut entries: Vec<(String, String)> = vec![
        (
            KEY_INPUT_METHOD.into(),
            input_method_name(settings.input_method).into(),
        ),
        (
            KEY_KEYMAP_PRESET.into(),
            preset_name(settings.keymap_preset).into(),
        ),
    ];
    for action in KEYMAP_ACTIONS {
        if let Some(value) = settings.keymap_overrides.get(action) {
            entries.push((format!("{KEY_KEYMAP_PREFIX}{action}"), value.clone()));
        }
    }
    let paging_wrap = if settings.paging_wrap {
        VALUE_TRUE
    } else {
        VALUE_FALSE
    };
    entries.extend(
        [
            (KEY_PAGING_WRAP, paging_wrap.to_string()),
            (KEY_PAGING_SIZE, settings.page_size.to_string()),
            (KEY_ENGINE_LM, switch_value(settings.engine_lm).into()),
            (
                KEY_ENGINE_IDLE_UNLOAD_SECS,
                settings.engine_idle_unload_secs.to_string(),
            ),
            (
                KEY_ENGINE_LM_DEADLINE_MS,
                settings.engine_lm_deadline_ms.to_string(),
            ),
            (
                KEY_ENGINE_CONVERT_WAIT_MS,
                settings.engine_convert_wait_ms.to_string(),
            ),
            (
                KEY_EMOJI_SHORTCODE,
                switch_value(settings.emoji_shortcode).into(),
            ),
            (
                KEY_EMOJI_SHORTCODE_EXCLUDE,
                join(&settings.emoji_shortcode_exclude),
            ),
            (
                KEY_EMOJI_SMALL_LETTERS,
                switch_value(settings.emoji_small_letters).into(),
            ),
            (
                KEY_POSTAL_JAPANESE,
                switch_value(settings.postal_japanese).into(),
            ),
            (
                KEY_POSTAL_ENGLISH,
                switch_value(settings.postal_english).into(),
            ),
            (
                KEY_POSTAL_TE_PREFIX,
                switch_value(settings.postal_te_prefix).into(),
            ),
            (
                KEY_UNICODE_NOTATION,
                switch_value(settings.unicode_notation).into(),
            ),
            (
                KEY_DATE_WEEKDAY_DATES,
                switch_value(settings.date_weekday_dates).into(),
            ),
            (
                KEY_CANDIDATE_HIDE_WHEN_CONFIDENT,
                switch_value(settings.candidate_hide_when_confident).into(),
            ),
            (KEY_CLAUSE_MIN_CHARS, settings.clause_min_chars.to_string()),
            (
                KEY_DEVELOPER_LM_STATUS,
                switch_value(settings.developer_lm_status).into(),
            ),
        ]
        .map(|(key, value)| (key.to_string(), value)),
    );
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{InputMethodKind, KeymapPreset};

    fn parse(tsv: &str) -> SettingsValidation {
        parse_tsv(tsv)
    }

    #[test]
    fn lf_crlf_and_bom_files_parse_the_same() {
        let text = "# notes\ninput.method\tkana\nkeymap.preset\tms-ime\npaging.size\t5\n";
        let expected = parse(text);
        assert!(expected.issues.is_empty());
        assert_eq!(expected.snapshot.page_size, 5);
        for (v, variant_text) in ikari_textfile::testing::read_variants(text) {
            let got = parse(&variant_text);
            assert_eq!(got.snapshot, expected.snapshot, "{v:?}");
            assert!(got.issues.is_empty(), "{v:?}: {:?}", got.issues);
        }
    }

    fn kinds(v: &SettingsValidation) -> Vec<(u32, &str, SettingsIssueKind)> {
        v.issues
            .iter()
            .map(|i| (i.line, i.key.as_str(), i.kind))
            .collect()
    }

    // ---- SettingsParsingTests

    #[test]
    fn empty_text_gives_defaults() {
        let result = parse("");
        assert_eq!(result.snapshot, default_snapshot());
        assert!(result.issues.is_empty());
        let d = default_snapshot();
        assert_eq!(d.keymap_preset, KeymapPreset::Macos);
        assert!(d.paging_wrap);
        assert_eq!(d.page_size, 3);
        assert_eq!(d.input_method, InputMethodKind::Romaji);
    }

    #[test]
    fn reads_every_key() {
        let tsv = "# コメント\ninput.method\tkana\nkeymap.preset\tms-ime\nkeymap.convert.hiragana\tctrl+h\n\
                   keymap.convert.halfwidth_alnum\tnone\npaging.wrap\tfalse\npaging.size\t5\nengine.lm\toff\n\
                   engine.idle_unload_secs\t0";
        let result = parse(tsv);
        assert!(result.issues.is_empty());
        let mut expected = default_snapshot();
        expected.input_method = InputMethodKind::Kana;
        expected.keymap_preset = KeymapPreset::MsIme;
        expected
            .keymap_overrides
            .insert("convert.hiragana".into(), "ctrl+h".into());
        expected
            .keymap_overrides
            .insert("convert.halfwidth_alnum".into(), "none".into());
        expected.paging_wrap = false;
        expected.page_size = 5;
        expected.engine_lm = false;
        expected.engine_idle_unload_secs = 0;
        assert_eq!(result.snapshot, expected);
    }

    #[test]
    fn invalid_values_fall_back_to_defaults_with_issues_that_carry_no_value() {
        let tsv = "input.method\tdvorak\nkeymap.preset\tvim\nkeymap.convert.hiragana\tctrl+ctrl+j\n\
                   paging.wrap\tmaybe\npaging.size\t10";
        let result = parse(tsv);
        assert_eq!(result.snapshot, default_snapshot());
        assert_eq!(
            kinds(&result),
            vec![
                (1, "input.method", SettingsIssueKind::InvalidValue),
                (2, "keymap.preset", SettingsIssueKind::InvalidValue),
                (
                    3,
                    "keymap.convert.hiragana",
                    SettingsIssueKind::InvalidValue
                ),
                (4, "paging.wrap", SettingsIssueKind::InvalidValue),
                (5, "paging.size", SettingsIssueKind::InvalidValue),
            ]
        );
    }

    #[test]
    fn page_size_out_of_range_is_rejected() {
        for value in ["0", "-1", "abc", "", "9.5", "１", "10"] {
            let result = parse(&format!("paging.size\t{value}"));
            assert_eq!(result.snapshot.page_size, 3, "{value:?}");
            assert_eq!(result.issues.len(), 1, "{value:?}");
        }
        for (value, size) in [("1", 1), ("9", 9)] {
            assert_eq!(
                parse(&format!("paging.size\t{value}")).snapshot.page_size,
                size
            );
        }
    }

    #[test]
    fn unknown_keys_and_malformed_lines_are_reported() {
        let result =
            parse("no.such\t1\nkeymap.convert.bogus\tf6\njust-a-key\nkeymap.preset\tmacos");
        assert_eq!(result.snapshot, default_snapshot());
        assert_eq!(
            kinds(&result),
            vec![
                (1, "no.such", SettingsIssueKind::UnknownKey),
                (2, "keymap.convert.bogus", SettingsIssueKind::UnknownKey),
                (3, "", SettingsIssueKind::Malformed),
            ]
        );
    }

    #[test]
    fn handles_windows_line_endings_and_surrounding_spaces() {
        let result = parse("paging.wrap\t false \r\npaging.size\t3\r\n");
        assert!(!result.snapshot.paging_wrap);
        assert_eq!(result.snapshot.page_size, 3);
        assert!(result.issues.is_empty());
    }

    #[test]
    fn later_lines_win() {
        assert_eq!(
            parse("paging.size\t3\npaging.size\t4").snapshot.page_size,
            4
        );
    }

    #[test]
    fn line_numbers_count_blank_and_comment_lines() {
        let result = parse("# c\n\npaging.size\tx\n");
        assert_eq!(result.issues[0].line, 3);
    }

    // ---- SettingsEngineKeysTests

    #[test]
    fn engine_keys_default_to_lm_on_and_five_minutes() {
        let parsed = parse("");
        assert!(parsed.snapshot.engine_lm);
        assert_eq!(parsed.snapshot.engine_idle_unload_secs, 300);
    }

    #[test]
    fn parses_engine_keys() {
        let parsed = parse("engine.lm\toff\nengine.idle_unload_secs\t0\n");
        assert!(!parsed.snapshot.engine_lm);
        assert_eq!(parsed.snapshot.engine_idle_unload_secs, 0);
        assert!(parsed.issues.is_empty());
    }

    #[test]
    fn invalid_engine_values_warn_and_keep_the_default() {
        for line in [
            "engine.lm\ttrue",
            "engine.lm\t",
            "engine.idle_unload_secs\t-1",
            "engine.idle_unload_secs\tabc",
            "engine.lm_deadline_ms\t-1",
            "engine.lm_deadline_ms\tabc",
            "engine.convert_wait_ms\t-1",
            "engine.convert_wait_ms\tabc",
        ] {
            let parsed = parse(line);
            assert_eq!(parsed.snapshot, default_snapshot(), "{line:?}");
            assert_eq!(parsed.issues.len(), 1, "{line:?}");
            assert_eq!(parsed.issues[0].kind, SettingsIssueKind::InvalidValue);
        }
    }

    #[test]
    fn other_engine_keys_are_ignored_without_warnings() {
        let parsed = parse("engine.future_key\tx\n");
        assert_eq!(parsed.snapshot, default_snapshot());
        assert!(parsed.issues.is_empty());
    }

    #[test]
    fn engine_settings_differ_only_for_engine_keys() {
        let base = default_snapshot();
        assert!(!engine_settings_differ(&base, &base));
        let mut other = base.clone();
        other.page_size = 5;
        other.keymap_preset = KeymapPreset::MsIme;
        other.input_method = InputMethodKind::Kana;
        assert!(!engine_settings_differ(&base, &other));
        let mutations: [fn(&mut SettingsSnapshot); 8] = [
            |s| s.engine_lm = false,
            |s| s.engine_idle_unload_secs = 60,
            |s| s.engine_lm_deadline_ms = 500,
            |s| s.engine_convert_wait_ms = 0,
            |s| s.postal_japanese = false,
            |s| s.postal_te_prefix = true,
            |s| s.unicode_notation = true,
            |s| s.date_weekday_dates = true,
        ];
        for mutate in mutations {
            let mut changed = base.clone();
            mutate(&mut changed);
            assert!(engine_settings_differ(&base, &changed));
        }
        // 殻だけが読むキー
        let mut shell_only = base.clone();
        shell_only.postal_english = true;
        shell_only.emoji_shortcode = true;
        shell_only.developer_lm_status = true;
        shell_only.candidate_hide_when_confident = false;
        assert!(!engine_settings_differ(&base, &shell_only));
    }

    #[test]
    fn timeout_keys_default_and_parse() {
        let none = parse("").snapshot;
        assert_eq!(none.engine_lm_deadline_ms, 300);
        assert_eq!(none.engine_convert_wait_ms, 30);
        let parsed = parse("engine.lm_deadline_ms\t800\nengine.convert_wait_ms\t0\n");
        assert_eq!(parsed.snapshot.engine_lm_deadline_ms, 800);
        assert_eq!(parsed.snapshot.engine_convert_wait_ms, 0);
        assert!(parsed.issues.is_empty());
        assert!(is_acceptable("engine.lm_deadline_ms", "500"));
        assert!(!is_acceptable("engine.convert_wait_ms", "-1"));
    }

    #[test]
    fn effective_entries_list_every_known_key_in_a_fixed_order() {
        let mut settings = default_snapshot();
        settings.input_method = InputMethodKind::Kana;
        settings.keymap_preset = KeymapPreset::MsIme;
        settings.paging_wrap = false;
        settings.page_size = 5;
        settings.engine_lm = false;
        // BTreeMap の並び（名前順）でなく、操作の並びで出す
        settings
            .keymap_overrides
            .insert("convert.halfwidth_alnum".into(), "none".into());
        settings
            .keymap_overrides
            .insert("convert.hiragana".into(), "ctrl+h".into());
        let entries: Vec<String> = effective_entries(&settings)
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let expected = vec![
            "input.method=kana".to_string(),
            "keymap.preset=ms-ime".into(),
            "keymap.convert.hiragana=ctrl+h".into(),
            "keymap.convert.halfwidth_alnum=none".into(),
            "paging.wrap=false".into(),
            "paging.size=5".into(),
            "engine.lm=off".into(),
            "engine.idle_unload_secs=300".into(),
            "engine.lm_deadline_ms=300".into(),
            "engine.convert_wait_ms=30".into(),
            "emoji.shortcode=off".into(),
            format!(
                "emoji.shortcode_exclude={}",
                DEFAULT_EMOJI_SHORTCODE_EXCLUDE.join(",")
            ),
            "emoji.small_letters=off".into(),
            "postal.japanese=on".into(),
            "postal.english=off".into(),
            "postal.te_prefix=off".into(),
            "unicode.notation=off".into(),
            "date.weekday_dates=off".into(),
            "candidate.hide_when_confident=on".into(),
            "clause.min_chars=3".into(),
            "developer.lm_status=off".into(),
        ];
        assert_eq!(entries, expected);
    }

    #[test]
    fn effective_entries_round_trip_through_parse() {
        let mut settings = default_snapshot();
        settings.input_method = InputMethodKind::Kana;
        settings.keymap_preset = KeymapPreset::MsIme;
        settings.paging_wrap = false;
        settings.page_size = 2;
        settings.engine_lm = false;
        settings.engine_idle_unload_secs = 0;
        settings
            .keymap_overrides
            .insert("convert.fullwidth_katakana".into(), "ctrl+q".into());
        let tsv = effective_entries(&settings)
            .iter()
            .map(|(k, v)| format!("{k}\t{v}"))
            .collect::<Vec<_>>()
            .join("\n");
        let parsed = parse(&tsv);
        assert_eq!(parsed.snapshot, settings);
        assert!(parsed.issues.is_empty());
    }

    #[test]
    fn acceptable_keys_and_values() {
        assert!(is_acceptable("keymap.preset", "ms-ime"));
        assert!(is_acceptable("keymap.preset", "atok"));
        assert!(is_acceptable("keymap.convert.hiragana", "ctrl+j"));
        assert!(is_acceptable("keymap.convert.hiragana", "none"));
        assert!(is_acceptable("engine.lm", "off"));
        assert!(is_acceptable("paging.size", "9"));
        assert!(!is_acceptable("paging.size", "10"));
        assert!(!is_acceptable("keymap.preset", "emacs"));
        assert!(!is_acceptable("keymap.convert.unknown", "ctrl+j"));
        assert!(!is_acceptable("engine.future_key", "x"));
        assert!(!is_acceptable("unknown", "x"));
        assert!(!is_acceptable("", "x"));
    }

    #[test]
    fn known_keys_are_the_documented_ones() {
        for key in [
            "input.method",
            "keymap.preset",
            "paging.wrap",
            "paging.size",
            "engine.lm",
            "engine.idle_unload_secs",
            "engine.lm_deadline_ms",
            "engine.convert_wait_ms",
        ] {
            assert!(is_known_key(key), "{key}");
        }
        for action in KEYMAP_ACTIONS {
            assert!(is_known_key(&format!("keymap.{action}")));
        }
        for key in [
            "keymap.",
            "keymap.nope",
            "engine.",
            "engine.other",
            "paging",
            "",
        ] {
            assert!(!is_known_key(key), "{key:?}");
        }
    }

    #[test]
    fn rejects_keys_and_values_that_could_inject_another_line() {
        assert!(!is_acceptable("paging.wrap", "true\nengine.lm\toff"));
        assert!(!is_acceptable("paging.wrap", "true\r"));
        assert!(!is_acceptable("paging.wrap\tx", "true"));
        assert!(!is_acceptable("paging.wrap\n", "true"));
    }

    // ---- 廃止した `context.shared_exclude`（ADR 0046）

    #[test]
    fn a_leftover_shared_exclude_line_is_an_unknown_key_warning() {
        // 読み込みは止まらず、ほかの行は効く。警告は行番号とキー名と種類だけ（値は含めない）
        let result = parse("context.shared_exclude\tcom.a.b\npaging.size\t5\n");
        assert_eq!(result.snapshot.page_size, 5);
        assert_eq!(result.issues.len(), 1);
        assert_eq!(result.issues[0].line, 1);
        assert_eq!(result.issues[0].key, "context.shared_exclude");
        assert_eq!(result.issues[0].kind, SettingsIssueKind::UnknownKey);
        assert!(!is_known_key("context.shared_exclude"));
    }

    // ---- 郵便番号、Unicode、曜日、候補の隠し方、絵文字

    #[test]
    fn on_off_switches_default_parse_and_reject() {
        let d = default_snapshot();
        assert!(d.postal_japanese && !d.postal_english && !d.postal_te_prefix);
        assert!(!d.unicode_notation && !d.date_weekday_dates && !d.emoji_small_letters);
        assert!(d.candidate_hide_when_confident && !d.emoji_shortcode);
        let parsed = parse(
            "postal.japanese\toff\npostal.english\ton\npostal.te_prefix\ton\nunicode.notation\ton\n\
             date.weekday_dates\ton\nemoji.small_letters\ton\nemoji.shortcode\ton\n\
             candidate.hide_when_confident\toff",
        );
        assert!(parsed.issues.is_empty());
        let s = parsed.snapshot;
        assert!(!s.postal_japanese && s.postal_english && s.postal_te_prefix);
        assert!(s.unicode_notation && s.date_weekday_dates && s.emoji_small_letters);
        assert!(s.emoji_shortcode && !s.candidate_hide_when_confident);
        for key in [
            "postal.japanese",
            "postal.english",
            "postal.te_prefix",
            "unicode.notation",
            "date.weekday_dates",
            "emoji.small_letters",
            "emoji.shortcode",
            "candidate.hide_when_confident",
        ] {
            let bad = parse(&format!("{key}\ttrue"));
            assert_eq!(bad.snapshot, default_snapshot(), "{key}");
            assert_eq!(bad.issues.len(), 1, "{key}");
        }
    }

    #[test]
    fn clause_min_chars_defaults_to_3_and_accepts_1_to_10() {
        assert_eq!(parse("").snapshot.clause_min_chars, 3);
        for (value, expected) in [("1", 1), ("10", 10)] {
            let parsed = parse(&format!("clause.min_chars\t{value}"));
            assert_eq!(parsed.snapshot.clause_min_chars, expected);
            assert!(parsed.issues.is_empty());
        }
        for value in ["0", "11", "-1", "x", ""] {
            let parsed = parse(&format!("clause.min_chars\t{value}"));
            assert_eq!(parsed.snapshot.clause_min_chars, 3, "{value}");
            assert_eq!(parsed.issues.len(), 1, "{value}");
        }
    }

    #[test]
    fn emoji_shortcode_exclude_parses_like_the_shared_list() {
        let parsed = parse("emoji.shortcode_exclude\tcom.x.y,com.z.w\n");
        assert_eq!(
            parsed.snapshot.emoji_shortcode_exclude,
            vec!["com.x.y", "com.z.w"]
        );
        assert_eq!(parse("emoji.shortcode_exclude\ta b").issues.len(), 1);
    }

    #[test]
    fn bundle_id_list_rules() {
        assert_eq!(parse_bundle_id_list(""), Some(vec![]));
        assert_eq!(parse_bundle_id_list(" , ,"), Some(vec![]));
        assert_eq!(
            parse_bundle_id_list("a.b,a.b,c_d"),
            Some(vec!["a.b".into(), "c_d".into()])
        );
        assert_eq!(parse_bundle_id_list("a b"), None);
        assert_eq!(parse_bundle_id_list("a/b"), None);
    }
}
