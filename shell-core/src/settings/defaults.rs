//! 設定のキーの名前と既定値（今の `Settings` の定数。Settings.swift）。

use std::collections::BTreeMap;

use crate::types::{InputMethodKind, KeymapPreset, SettingsSnapshot};

// ---- キー（docs/phase2-contract.md §3）
pub const KEY_INPUT_METHOD: &str = "input.method";
pub const KEY_KEYMAP_PRESET: &str = "keymap.preset";
pub const KEY_KEYMAP_PREFIX: &str = "keymap.";
pub const KEY_PAGING_WRAP: &str = "paging.wrap";
pub const KEY_PAGING_SIZE: &str = "paging.size";
pub const KEY_ENGINE_PREFIX: &str = "engine.";
pub const KEY_ENGINE_LM: &str = "engine.lm";
pub const KEY_ENGINE_IDLE_UNLOAD_SECS: &str = "engine.idle_unload_secs";
pub const KEY_ENGINE_LM_DEADLINE_MS: &str = "engine.lm_deadline_ms";
pub const KEY_ENGINE_CONVERT_WAIT_MS: &str = "engine.convert_wait_ms";
pub const KEY_EMOJI_SHORTCODE: &str = "emoji.shortcode";
pub const KEY_EMOJI_SHORTCODE_EXCLUDE: &str = "emoji.shortcode_exclude";
pub const KEY_EMOJI_SMALL_LETTERS: &str = "emoji.small_letters";
pub const KEY_POSTAL_JAPANESE: &str = "postal.japanese";
pub const KEY_POSTAL_ENGLISH: &str = "postal.english";
pub const KEY_POSTAL_TE_PREFIX: &str = "postal.te_prefix";
pub const KEY_UNICODE_NOTATION: &str = "unicode.notation";
pub const KEY_DATE_WEEKDAY_DATES: &str = "date.weekday_dates";
pub const KEY_CANDIDATE_HIDE_WHEN_CONFIDENT: &str = "candidate.hide_when_confident";
pub const KEY_CLAUSE_MIN_CHARS: &str = "clause.min_chars";

// ---- 値の書き方
pub const VALUE_ON: &str = "on";
pub const VALUE_OFF: &str = "off";
pub const VALUE_TRUE: &str = "true";
pub const VALUE_FALSE: &str = "false";
/// `keymap.<操作>` で割り当てを外すときの値
pub const VALUE_NONE: &str = "none";
/// `emoji.shortcode_exclude` などの値の区切り
pub const LIST_SEPARATOR: char = ',';

// ---- 既定値と範囲
pub const PAGE_SIZE_MIN: u32 = 1;
pub const PAGE_SIZE_MAX: u32 = 9;
/// `paging.size` の既定。最初に見せる候補の数（ADR 0014 §9）
pub const DEFAULT_PAGE_SIZE: u32 = 3;
/// エンジンに頼む 1 ページの件数（いつも上限。ADR 0014 §2.3 (c)）
pub const ENGINE_PAGE_SIZE: u32 = PAGE_SIZE_MAX;
pub const DEFAULT_ENGINE_IDLE_UNLOAD_SECS: u32 = 300;
/// エンジンの `DEFAULT_LM_DEADLINE_MS` と同じ
pub const DEFAULT_ENGINE_LM_DEADLINE_MS: u32 = 300;
/// エンジンの `DEFAULT_CONVERT_WAIT_MS` と同じ
pub const DEFAULT_ENGINE_CONVERT_WAIT_MS: u32 = 30;
pub const DEFAULT_EMOJI_SHORTCODE: bool = false;
pub const DEFAULT_EMOJI_SMALL_LETTERS: bool = false;
pub const DEFAULT_POSTAL_JAPANESE: bool = true;
pub const DEFAULT_POSTAL_ENGLISH: bool = false;
pub const DEFAULT_POSTAL_TE_PREFIX: bool = false;
pub const DEFAULT_UNICODE_NOTATION: bool = false;
pub const DEFAULT_DATE_WEEKDAY_DATES: bool = false;
/// 2026-10-28 ユーザー決定（#69）
pub const DEFAULT_CANDIDATE_HIDE_WHEN_CONFIDENT: bool = true;

/// `clause.min_chars` の範囲。1 = 下限なし（ADR 0050 §5.1。2026-11-05 ユーザー決定）
pub const CLAUSE_MIN_CHARS_MIN: u32 = 1;
pub const CLAUSE_MIN_CHARS_MAX: u32 = 10;
/// `clause.min_chars` の既定。文節ごとの削除で、消す文字がこの数に満たなければ前の文節まで広げる（ADR 0050。ユーザー決定）
pub const DEFAULT_CLAUSE_MIN_CHARS: u32 = 3;

/// `emoji.shortcode_exclude` の既定（ADR 0027 §8.1 Q17）。ターミナル: Terminal、iTerm2、WezTerm、Ghostty
pub const DEFAULT_EMOJI_SHORTCODE_EXCLUDE: [&str; 4] = [
    "com.apple.Terminal",
    "com.googlecode.iterm2",
    "com.github.wez.wezterm",
    "com.mitchellh.ghostty",
];

/// 割り当てられる操作の名前（`keymap.<操作>` と keymap の TSV の操作）。並びは `effective_entries` と画面の順
pub const KEYMAP_ACTIONS: [&str; 18] = [
    "convert.hiragana",
    "convert.fullwidth_katakana",
    "convert.halfwidth_katakana",
    "convert.fullwidth_alnum",
    "convert.halfwidth_alnum",
    "segment.focus_left",
    "segment.focus_right",
    "segment.shrink",
    "segment.expand",
    "candidate.next",
    "candidate.previous",
    "reconvert",
    "mode.toggle",
    "mode.english",
    "mode.japanese",
    "clause.delete",
    "clause.delete_preview",
    "clause.undo",
];

pub fn is_keymap_action(name: &str) -> bool {
    KEYMAP_ACTIONS.contains(&name)
}

pub(crate) fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

/// 既定値の設定の一式（`SettingsSnapshot::default()` と同じ）。
impl Default for SettingsSnapshot {
    fn default() -> Self {
        default_snapshot()
    }
}

/// 既定値の設定の一式（ファイルがない、または読めないときに使う。Swift の `Settings.default`）
pub fn default_snapshot() -> SettingsSnapshot {
    SettingsSnapshot {
        input_method: InputMethodKind::Romaji,
        keymap_preset: KeymapPreset::Macos,
        keymap_overrides: BTreeMap::new(),
        paging_wrap: true,
        page_size: DEFAULT_PAGE_SIZE,
        engine_lm: true,
        engine_idle_unload_secs: DEFAULT_ENGINE_IDLE_UNLOAD_SECS,
        engine_lm_deadline_ms: DEFAULT_ENGINE_LM_DEADLINE_MS,
        engine_convert_wait_ms: DEFAULT_ENGINE_CONVERT_WAIT_MS,
        emoji_shortcode: DEFAULT_EMOJI_SHORTCODE,
        emoji_shortcode_exclude: strings(&DEFAULT_EMOJI_SHORTCODE_EXCLUDE),
        emoji_small_letters: DEFAULT_EMOJI_SMALL_LETTERS,
        postal_japanese: DEFAULT_POSTAL_JAPANESE,
        postal_english: DEFAULT_POSTAL_ENGLISH,
        postal_te_prefix: DEFAULT_POSTAL_TE_PREFIX,
        unicode_notation: DEFAULT_UNICODE_NOTATION,
        date_weekday_dates: DEFAULT_DATE_WEEKDAY_DATES,
        candidate_hide_when_confident: DEFAULT_CANDIDATE_HIDE_WHEN_CONFIDENT,
        clause_min_chars: DEFAULT_CLAUSE_MIN_CHARS,
    }
}

pub(crate) fn input_method_name(method: InputMethodKind) -> &'static str {
    match method {
        InputMethodKind::Romaji => "romaji",
        InputMethodKind::Kana => "kana",
    }
}

pub(crate) fn input_method_from_name(name: &str) -> Option<InputMethodKind> {
    match name {
        "romaji" => Some(InputMethodKind::Romaji),
        "kana" => Some(InputMethodKind::Kana),
        _ => None,
    }
}

pub fn preset_name(preset: KeymapPreset) -> &'static str {
    match preset {
        KeymapPreset::Macos => "macos",
        KeymapPreset::MsIme => "ms-ime",
        KeymapPreset::Atok => "atok",
    }
}

pub fn preset_from_name(name: &str) -> Option<KeymapPreset> {
    [KeymapPreset::Macos, KeymapPreset::MsIme, KeymapPreset::Atok]
        .into_iter()
        .find(|preset| preset_name(*preset) == name)
}

pub const KEYMAP_PRESETS: [KeymapPreset; 3] =
    [KeymapPreset::Macos, KeymapPreset::MsIme, KeymapPreset::Atok];
