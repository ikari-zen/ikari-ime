//! 設定アプリの Model のうち、画面の状態を持たない部分（検証、整形、既定値、変更の組み立て）。
//! 今の `SettingsValues`・`KeymapPresetTable`・`SettingsModel` の `set*`・`add*`・`remove*`・`reset*`・キー割り当ての判定。
//! `@Observable` の状態（読み込み中、エラーの文言、非同期の送受信）は Swift に残る。画面の文言も Swift（ここは種類だけ返す）。
//!
//! 使い方: 画面は「今効いている値」（`effective_entries` の形）から `SettingsValues` を作り、操作を `SettingsIntent` にして
//! `SettingsValues::plan` に渡す。返った `SettingsChanges` を殻に送る（`UpdateSettings`）。`None` は何もしない。

use std::collections::{BTreeMap, HashMap};

use super::chord::KeyChord;
use super::defaults::*;
use super::parse::{parse_bundle_id_list, switch_value};
use super::text::{split_lines, trim_blank};
use super::{SettingsChanges, SettingsEntry};
use crate::types::{InputMethodKind, KeymapPreset};

/// `engine.idle_unload_secs` の範囲（1 日まで）。`0` は解放しない
pub const IDLE_UNLOAD_RANGE: std::ops::RangeInclusive<i64> = 0..=86_400;
/// `engine.lm_deadline_ms` の範囲（暫定。エンジンも同じ範囲に丸める）
pub const LM_DEADLINE_RANGE: std::ops::RangeInclusive<i64> = 50..=2000;
/// `engine.convert_wait_ms` の範囲（暫定。0 = 待たない）
pub const CONVERT_WAIT_RANGE: std::ops::RangeInclusive<i64> = 0..=200;

/// 操作ごとのキー割り当て。`Preset` は上書きなし
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeymapBinding {
    Preset,
    /// 割り当てを外す（`none`）
    None,
    /// `ctrl+j` の表記
    Chord(String),
}

/// 画面に出す値の一式（`settings.tsv` のキーごと）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsValues {
    pub input_method: InputMethodKind,
    pub keymap_preset: KeymapPreset,
    /// 上書きがある操作だけ（操作の名前 → 割り当て）
    pub keymap_bindings: BTreeMap<String, KeymapBinding>,
    pub paging_wrap: bool,
    pub page_size: u32,
    pub lm_enabled: bool,
    pub idle_unload_secs: u32,
    pub lm_deadline_ms: u32,
    pub convert_wait_ms: u32,
    pub emoji_shortcode: bool,
    pub emoji_shortcode_exclude: Vec<String>,
    pub emoji_small_letters: bool,
    pub postal_japanese: bool,
    pub postal_english: bool,
    pub postal_te_prefix: bool,
    pub unicode_notation: bool,
    pub date_weekday_dates: bool,
    pub candidate_hide_when_confident: bool,
}

impl Default for SettingsValues {
    fn default() -> Self {
        let d = default_snapshot();
        Self {
            input_method: d.input_method,
            keymap_preset: d.keymap_preset,
            keymap_bindings: BTreeMap::new(),
            paging_wrap: d.paging_wrap,
            page_size: d.page_size,
            lm_enabled: d.engine_lm,
            idle_unload_secs: d.engine_idle_unload_secs,
            lm_deadline_ms: d.engine_lm_deadline_ms,
            convert_wait_ms: d.engine_convert_wait_ms,
            emoji_shortcode: d.emoji_shortcode,
            emoji_shortcode_exclude: d.emoji_shortcode_exclude,
            emoji_small_letters: d.emoji_small_letters,
            postal_japanese: d.postal_japanese,
            postal_english: d.postal_english,
            postal_te_prefix: d.postal_te_prefix,
            unicode_notation: d.unicode_notation,
            date_weekday_dates: d.date_weekday_dates,
            candidate_hide_when_confident: d.candidate_hide_when_confident,
        }
    }
}

impl SettingsValues {
    /// 「今効いている値」（キーと書き方そのままの組）から作る。読めない値と未知のキーは既定のままにする（画面を壊さない）。
    /// 範囲は殻の `validate` より狭い（`paging.size` 以外の数は画面で選べる範囲）。範囲の外は既定のまま
    pub fn from_entries<'a>(entries: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let snapshot: HashMap<&str, &str> = entries.into_iter().collect();
        let get = |key: &str| snapshot.get(key).copied();
        let mut values = Self::default();

        if let Some(method) = get(KEY_INPUT_METHOD).and_then(input_method_from_name) {
            values.input_method = method;
        }
        if let Some(preset) = get(KEY_KEYMAP_PRESET).and_then(preset_from_name) {
            values.keymap_preset = preset;
        }
        for action in KEYMAP_ACTIONS {
            let Some(value) =
                get(&format!("{KEY_KEYMAP_PREFIX}{action}")).filter(|v| !v.is_empty())
            else {
                continue;
            };
            if value == VALUE_NONE {
                values
                    .keymap_bindings
                    .insert(action.to_string(), KeymapBinding::None);
            } else if KeyChord::parse(value).is_some() {
                values
                    .keymap_bindings
                    .insert(action.to_string(), KeymapBinding::Chord(value.to_string()));
            }
        }
        match get(KEY_PAGING_WRAP) {
            Some(VALUE_TRUE) => values.paging_wrap = true,
            Some(VALUE_FALSE) => values.paging_wrap = false,
            _ => {}
        }
        if let Some(size) = get(KEY_PAGING_SIZE)
            .and_then(|t| t.parse::<u32>().ok())
            .filter(|n| (PAGE_SIZE_MIN..=PAGE_SIZE_MAX).contains(n))
        {
            values.page_size = size;
        }
        let ranged = |key: &str, range: &std::ops::RangeInclusive<i64>| {
            get(key)
                .and_then(|t| t.parse::<i64>().ok())
                .filter(|n| range.contains(n))
                .and_then(|n| u32::try_from(n).ok())
        };
        let toggle = |key: &str, current: bool| match get(key) {
            Some(VALUE_ON) => true,
            Some(VALUE_OFF) => false,
            _ => current,
        };
        values.lm_enabled = toggle(KEY_ENGINE_LM, values.lm_enabled);
        if let Some(n) = ranged(KEY_ENGINE_IDLE_UNLOAD_SECS, &IDLE_UNLOAD_RANGE) {
            values.idle_unload_secs = n;
        }
        if let Some(n) = ranged(KEY_ENGINE_LM_DEADLINE_MS, &LM_DEADLINE_RANGE) {
            values.lm_deadline_ms = n;
        }
        if let Some(n) = ranged(KEY_ENGINE_CONVERT_WAIT_MS, &CONVERT_WAIT_RANGE) {
            values.convert_wait_ms = n;
        }
        values.emoji_shortcode = toggle(KEY_EMOJI_SHORTCODE, values.emoji_shortcode);
        if let Some(list) = get(KEY_EMOJI_SHORTCODE_EXCLUDE).and_then(parse_bundle_id_list) {
            values.emoji_shortcode_exclude = list;
        }
        values.postal_japanese = toggle(KEY_POSTAL_JAPANESE, values.postal_japanese);
        values.postal_english = toggle(KEY_POSTAL_ENGLISH, values.postal_english);
        values.postal_te_prefix = toggle(KEY_POSTAL_TE_PREFIX, values.postal_te_prefix);
        values.emoji_small_letters = toggle(KEY_EMOJI_SMALL_LETTERS, values.emoji_small_letters);
        values.unicode_notation = toggle(KEY_UNICODE_NOTATION, values.unicode_notation);
        values.date_weekday_dates = toggle(KEY_DATE_WEEKDAY_DATES, values.date_weekday_dates);
        values.candidate_hide_when_confident = toggle(
            KEY_CANDIDATE_HIDE_WHEN_CONFIDENT,
            values.candidate_hide_when_confident,
        );
        values
    }
}

// ---------------------------------------------------------------- プリセットの表

/// プリセットの TSV から、操作ごとの割り当てを引く。上書きが「プリセットと同じ」かの判定に使う。
#[derive(Debug, Clone, Default)]
pub struct KeymapPresetTable {
    chords: HashMap<KeymapPreset, HashMap<String, Vec<KeyChord>>>,
}

impl KeymapPresetTable {
    /// `read` は `macos` / `ms-ime` / `atok` の TSV（`keymap/<プリセット>.tsv`）を返す。
    /// 読めないプリセット（ない、行が壊れている）は空のまま（画面に既定のキーを出さないだけ）
    pub fn new(read: impl Fn(KeymapPreset) -> Option<String>) -> Self {
        let mut chords = HashMap::new();
        for preset in KEYMAP_PRESETS {
            let Some(bindings) = read(preset).and_then(|text| parse_preset(&text)) else {
                continue;
            };
            let mut by_action: HashMap<String, Vec<KeyChord>> = HashMap::new();
            for (chord, action) in bindings {
                by_action.entry(action).or_default().push(chord);
            }
            for list in by_action.values_mut() {
                list.sort_by(|a, b| (a.modifier_bits(), &a.key).cmp(&(b.modifier_bits(), &b.key)));
            }
            chords.insert(preset, by_action);
        }
        Self { chords }
    }

    /// F キーが先、次に修飾キーの少ない順、最後にキー名の順
    pub fn chords(&self, action: &str, preset: KeymapPreset) -> &[KeyChord] {
        self.chords
            .get(&preset)
            .and_then(|by_action| by_action.get(action))
            .map_or(&[], Vec::as_slice)
    }

    /// 文字キーの既定の割り当て（`ctrl+j`）。F キーは常に使えるので除く
    pub fn primary_chord(&self, action: &str, preset: KeymapPreset) -> Option<&KeyChord> {
        self.chords(action, preset)
            .iter()
            .find(|chord| !chord.is_function_key())
    }
}

/// 「キー<TAB>操作」の行。壊れた行が 1 つでもあれば `None`（今の `Keymap.parsePreset` が投げるのと同じ）
fn parse_preset(tsv: &str) -> Option<HashMap<KeyChord, String>> {
    let mut table: HashMap<KeyChord, String> = HashMap::new();
    for raw in split_lines(tsv) {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').collect();
        let [key, action] = columns.as_slice() else {
            return None;
        };
        let chord = KeyChord::parse(key)?;
        let action = trim_blank(action);
        if !is_keymap_action(action) {
            return None;
        }
        table.insert(chord, action.to_string());
    }
    Some(table)
}

// ---------------------------------------------------------------- 画面の行

/// キー割り当ての 1 行（文言は Swift が付ける）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapRowData {
    pub action: String,
    /// 今の割り当て。プリセットと同じ値は `Preset` にそろえる
    pub binding: KeymapBinding,
    /// 割り当てのキー（`⌃J`）。ないときは `None`（画面は「なし」と出す）
    pub key_text: Option<String>,
    /// プリセットの割り当て（上書き中に「戻す」と何になるか）。ないときは `None`
    pub preset_text: Option<String>,
}

impl SettingsValues {
    /// 上書きがプリセットの割り当てと同じなら `Preset` とみなす
    pub fn resolved_binding(&self, action: &str, presets: &KeymapPresetTable) -> KeymapBinding {
        let Some(binding) = self.keymap_bindings.get(action) else {
            return KeymapBinding::Preset;
        };
        if let KeymapBinding::Chord(text) = binding
            && let Some(chord) = KeyChord::parse(text)
            && presets.chords(action, self.keymap_preset).contains(&chord)
        {
            return KeymapBinding::Preset;
        }
        binding.clone()
    }

    pub fn keymap_rows(&self, presets: &KeymapPresetTable) -> Vec<KeymapRowData> {
        KEYMAP_ACTIONS
            .iter()
            .map(|action| {
                let binding = self.resolved_binding(action, presets);
                let preset_text = presets
                    .primary_chord(action, self.keymap_preset)
                    .map(KeyChord::display);
                let key_text = match &binding {
                    KeymapBinding::Preset => preset_text.clone(),
                    KeymapBinding::None => None,
                    KeymapBinding::Chord(text) => Some(KeyChord::display_text(text)),
                };
                KeymapRowData {
                    action: (*action).to_string(),
                    binding,
                    key_text,
                    preset_text,
                }
            })
            .collect()
    }

    /// 今効いている、その操作のキー（F キーを含む）
    pub fn effective_chords(&self, action: &str, presets: &KeymapPresetTable) -> Vec<KeyChord> {
        let preset = presets.chords(action, self.keymap_preset);
        match self.resolved_binding(action, presets) {
            KeymapBinding::Preset => preset.to_vec(),
            KeymapBinding::None => Vec::new(),
            // 上書きは文字キーを置き換え、F キーは残す
            KeymapBinding::Chord(text) => KeyChord::parse(&text)
                .into_iter()
                .chain(preset.iter().filter(|c| c.is_function_key()).cloned())
                .collect(),
        }
    }

    /// そのキーを、ほかの操作がすでに使っていれば、その操作
    pub fn conflicting_action(
        &self,
        chord: &KeyChord,
        excluding: &str,
        presets: &KeymapPresetTable,
    ) -> Option<&'static str> {
        KEYMAP_ACTIONS.iter().copied().find(|other| {
            *other != excluding && self.effective_chords(other, presets).contains(chord)
        })
    }
}

// ---------------------------------------------------------------- 変更の組み立て

/// 画面の操作
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsIntent {
    InputMethod(InputMethodKind),
    KeymapPreset(KeymapPreset),
    PagingWrap(bool),
    /// 範囲の外は範囲に丸める
    PageSize(i64),
    LmEnabled(bool),
    IdleUnloadSecs(i64),
    LmDeadlineMs(i64),
    ConvertWaitMs(i64),
    PostalJapanese(bool),
    PostalEnglish(bool),
    PostalTePrefix(bool),
    CandidateHideWhenConfident(bool),
    UnicodeNotation(bool),
    DateWeekdayDates(bool),
    EmojiSmallLetters(bool),
    EmojiShortcode(bool),
    /// 入力欄の文字列（空白は削る。使えない文字があれば `InvalidBundleId`）
    AddEmojiShortcodeExclude(String),
    RemoveEmojiShortcodeExclude(String),
    ResetEmojiShortcodeExclude,
    Keymap {
        action: String,
        binding: KeymapBinding,
    },
}

/// 組み立てられなかった理由（文言は画面が決める）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// Bundle ID に使えない文字がある、または 1 つでない
    InvalidBundleId,
    /// 知らない操作の名前
    UnknownAction,
    /// そのキーはほかの操作が使っている
    KeyConflict {
        /// 画面の表記（`⌃J`）
        chord_display: String,
        /// 使っている操作の名前
        other_action: String,
    },
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBundleId => f.write_str("invalid bundle id"),
            Self::UnknownAction => f.write_str("unknown keymap action"),
            Self::KeyConflict { other_action, .. } => {
                write!(f, "key is already used by {other_action}")
            }
        }
    }
}

impl std::error::Error for PlanError {}

type Plan = Result<Option<SettingsChanges>, PlanError>;

fn set_one(key: &str, value: impl Into<String>) -> Plan {
    Ok(Some(SettingsChanges {
        set: vec![SettingsEntry {
            key: key.to_string(),
            value: value.into(),
        }],
        reset: Vec::new(),
    }))
}

fn reset_one(key: &str) -> Plan {
    Ok(Some(SettingsChanges {
        set: Vec::new(),
        reset: vec![key.to_string()],
    }))
}

fn switch_changed(current: bool, wanted: bool, key: &str) -> Plan {
    if current == wanted {
        return Ok(None);
    }
    set_one(key, switch_value(wanted))
}

fn clamped_changed(
    current: u32,
    wanted: i64,
    range: &std::ops::RangeInclusive<i64>,
    key: &str,
) -> Plan {
    let clamped = wanted.clamp(*range.start(), *range.end());
    if i64::from(current) == clamped {
        return Ok(None);
    }
    set_one(key, clamped.to_string())
}

/// 入力欄の Bundle ID を確かめる。空は無視（`Ok(None)`）。使えない文字、または複数なら `InvalidBundleId`
fn valid_bundle_id(raw: &str) -> Result<Option<String>, PlanError> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match parse_bundle_id_list(text).as_deref() {
        Some([id]) => Ok(Some(id.clone())),
        _ => Err(PlanError::InvalidBundleId),
    }
}

fn join_list(list: &[String]) -> String {
    list.join(&LIST_SEPARATOR.to_string())
}

fn add_to_list(current: &[String], raw: &str, key: &str) -> Plan {
    let Some(id) = valid_bundle_id(raw)? else {
        return Ok(None);
    };
    if current.contains(&id) {
        return Ok(None);
    }
    let mut next = current.to_vec();
    next.push(id);
    set_one(key, join_list(&next))
}

fn remove_from_list(current: &[String], id: &str, key: &str) -> Plan {
    if !current.iter().any(|existing| existing == id) {
        return Ok(None);
    }
    let next: Vec<String> = current.iter().filter(|e| *e != id).cloned().collect();
    // 空の一覧は空の値（除外なし）として書く
    set_one(key, join_list(&next))
}

fn reset_list(current: &[String], default: &[&str], key: &str) -> Plan {
    if current
        .iter()
        .map(String::as_str)
        .eq(default.iter().copied())
    {
        return Ok(None);
    }
    reset_one(key)
}

impl SettingsValues {
    /// 画面の操作を、殻に送る変更にする。何も変わらないときは `Ok(None)`（送らない）。
    /// 殻が `engine.*` の変更を見て、エンジンを起こし直す（ADR 0005 §7.3）
    pub fn plan(&self, intent: &SettingsIntent, presets: &KeymapPresetTable) -> Plan {
        use SettingsIntent as I;
        match intent {
            I::InputMethod(method) => {
                if *method == self.input_method {
                    return Ok(None);
                }
                set_one(KEY_INPUT_METHOD, input_method_name(*method))
            }
            I::KeymapPreset(preset) => {
                if *preset == self.keymap_preset {
                    return Ok(None);
                }
                set_one(KEY_KEYMAP_PRESET, preset_name(*preset))
            }
            I::PagingWrap(wrap) => {
                if *wrap == self.paging_wrap {
                    return Ok(None);
                }
                set_one(
                    KEY_PAGING_WRAP,
                    if *wrap { VALUE_TRUE } else { VALUE_FALSE },
                )
            }
            I::PageSize(size) => clamped_changed(
                self.page_size,
                *size,
                &(i64::from(PAGE_SIZE_MIN)..=i64::from(PAGE_SIZE_MAX)),
                KEY_PAGING_SIZE,
            ),
            I::LmEnabled(on) => switch_changed(self.lm_enabled, *on, KEY_ENGINE_LM),
            I::IdleUnloadSecs(n) => clamped_changed(
                self.idle_unload_secs,
                *n,
                &IDLE_UNLOAD_RANGE,
                KEY_ENGINE_IDLE_UNLOAD_SECS,
            ),
            I::LmDeadlineMs(n) => clamped_changed(
                self.lm_deadline_ms,
                *n,
                &LM_DEADLINE_RANGE,
                KEY_ENGINE_LM_DEADLINE_MS,
            ),
            I::ConvertWaitMs(n) => clamped_changed(
                self.convert_wait_ms,
                *n,
                &CONVERT_WAIT_RANGE,
                KEY_ENGINE_CONVERT_WAIT_MS,
            ),
            I::PostalJapanese(on) => switch_changed(self.postal_japanese, *on, KEY_POSTAL_JAPANESE),
            I::PostalEnglish(on) => switch_changed(self.postal_english, *on, KEY_POSTAL_ENGLISH),
            I::PostalTePrefix(on) => {
                switch_changed(self.postal_te_prefix, *on, KEY_POSTAL_TE_PREFIX)
            }
            I::CandidateHideWhenConfident(on) => switch_changed(
                self.candidate_hide_when_confident,
                *on,
                KEY_CANDIDATE_HIDE_WHEN_CONFIDENT,
            ),
            I::UnicodeNotation(on) => {
                switch_changed(self.unicode_notation, *on, KEY_UNICODE_NOTATION)
            }
            I::DateWeekdayDates(on) => {
                switch_changed(self.date_weekday_dates, *on, KEY_DATE_WEEKDAY_DATES)
            }
            I::EmojiSmallLetters(on) => {
                switch_changed(self.emoji_small_letters, *on, KEY_EMOJI_SMALL_LETTERS)
            }
            I::EmojiShortcode(on) => switch_changed(self.emoji_shortcode, *on, KEY_EMOJI_SHORTCODE),
            I::AddEmojiShortcodeExclude(raw) => add_to_list(
                &self.emoji_shortcode_exclude,
                raw,
                KEY_EMOJI_SHORTCODE_EXCLUDE,
            ),
            I::RemoveEmojiShortcodeExclude(id) => remove_from_list(
                &self.emoji_shortcode_exclude,
                id,
                KEY_EMOJI_SHORTCODE_EXCLUDE,
            ),
            I::ResetEmojiShortcodeExclude => reset_list(
                &self.emoji_shortcode_exclude,
                &DEFAULT_EMOJI_SHORTCODE_EXCLUDE,
                KEY_EMOJI_SHORTCODE_EXCLUDE,
            ),
            I::Keymap { action, binding } => self.plan_keymap(action, binding, presets),
        }
    }

    /// 操作ごとの割り当てを変える。`Preset` は上書きの行を消す（`reset`）
    fn plan_keymap(
        &self,
        action: &str,
        binding: &KeymapBinding,
        presets: &KeymapPresetTable,
    ) -> Plan {
        if !is_keymap_action(action) {
            return Err(PlanError::UnknownAction);
        }
        let key = format!("{KEY_KEYMAP_PREFIX}{action}");
        match binding {
            KeymapBinding::Preset => {
                if !self.keymap_bindings.contains_key(action) {
                    return Ok(None);
                }
                reset_one(&key)
            }
            KeymapBinding::None => set_one(&key, VALUE_NONE),
            KeymapBinding::Chord(text) => {
                // 読めない表記は何もしない（画面の入力は `KeyChordText.record` が読み戻せるものだけを渡す）
                let Some(chord) = KeyChord::parse(text) else {
                    return Ok(None);
                };
                if let Some(other) = self.conflicting_action(&chord, action, presets) {
                    return Err(PlanError::KeyConflict {
                        chord_display: chord.display(),
                        other_action: other.to_string(),
                    });
                }
                set_one(&key, chord.to_text())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MACOS: &str = include_str!("../../../macos/Resources/keymap/macos.tsv");
    const MS_IME: &str = include_str!("../../../macos/Resources/keymap/ms-ime.tsv");
    const ATOK: &str = include_str!("../../../macos/Resources/keymap/atok.tsv");

    fn presets() -> KeymapPresetTable {
        KeymapPresetTable::new(|preset| {
            Some(
                match preset {
                    KeymapPreset::Macos => MACOS,
                    KeymapPreset::MsIme => MS_IME,
                    KeymapPreset::Atok => ATOK,
                }
                .to_string(),
            )
        })
    }

    fn plan(values: &SettingsValues, intent: SettingsIntent) -> Plan {
        values.plan(&intent, &presets())
    }

    fn set_of(plan: Plan) -> (String, String) {
        let changes = plan.unwrap().unwrap();
        assert!(changes.reset.is_empty());
        assert_eq!(changes.set.len(), 1);
        (changes.set[0].key.clone(), changes.set[0].value.clone())
    }

    fn reset_of(plan: Plan) -> String {
        let changes = plan.unwrap().unwrap();
        assert!(changes.set.is_empty());
        assert_eq!(changes.reset.len(), 1);
        changes.reset[0].clone()
    }

    fn pair(key: &str, value: &str) -> (String, String) {
        (key.to_string(), value.to_string())
    }

    // ---- SettingsValues(snapshot:)

    #[test]
    fn empty_entries_give_defaults() {
        assert_eq!(SettingsValues::from_entries([]), SettingsValues::default());
    }

    #[test]
    fn effective_entries_round_trip_into_values() {
        let mut snapshot = default_snapshot();
        snapshot.input_method = InputMethodKind::Kana;
        snapshot.keymap_preset = KeymapPreset::Atok;
        snapshot.page_size = 7;
        snapshot.engine_lm = false;
        snapshot.engine_idle_unload_secs = 60;
        snapshot.emoji_shortcode_exclude = vec!["com.a.b".into()];
        snapshot
            .keymap_overrides
            .insert("convert.hiragana".into(), "none".into());
        snapshot
            .keymap_overrides
            .insert("reconvert".into(), "ctrl+r".into());
        let entries = super::super::effective_entries(&snapshot);
        let values =
            SettingsValues::from_entries(entries.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        assert_eq!(values.input_method, InputMethodKind::Kana);
        assert_eq!(values.keymap_preset, KeymapPreset::Atok);
        assert_eq!(values.page_size, 7);
        assert!(!values.lm_enabled);
        assert_eq!(values.idle_unload_secs, 60);
        assert_eq!(values.emoji_shortcode_exclude, vec!["com.a.b"]);
        assert_eq!(
            values.keymap_bindings.get("convert.hiragana"),
            Some(&KeymapBinding::None)
        );
        assert_eq!(
            values.keymap_bindings.get("reconvert"),
            Some(&KeymapBinding::Chord("ctrl+r".into()))
        );
    }

    #[test]
    fn unreadable_or_out_of_range_entries_keep_the_default() {
        let values = SettingsValues::from_entries([
            ("input.method", "dvorak"),
            ("paging.size", "10"),
            ("engine.lm_deadline_ms", "10"),
            ("engine.convert_wait_ms", "201"),
            ("engine.idle_unload_secs", "86401"),
            ("engine.lm", "maybe"),
            ("keymap.reconvert", "ctrl+ctrl+r"),
            ("keymap.unknown", "ctrl+r"),
            ("emoji.shortcode_exclude", "a b"),
        ]);
        assert_eq!(values, SettingsValues::default());
        let edge = SettingsValues::from_entries([
            ("engine.lm_deadline_ms", "50"),
            ("engine.convert_wait_ms", "0"),
            ("engine.idle_unload_secs", "86400"),
        ]);
        assert_eq!(
            (
                edge.lm_deadline_ms,
                edge.convert_wait_ms,
                edge.idle_unload_secs
            ),
            (50, 0, 86_400)
        );
    }

    // ---- 変更の組み立て（SettingsModelTests の期待）

    #[test]
    fn plain_settings_write_their_key_and_skip_when_unchanged() {
        let v = SettingsValues::default();
        assert_eq!(
            set_of(plan(&v, SettingsIntent::InputMethod(InputMethodKind::Kana))),
            pair("input.method", "kana")
        );
        assert_eq!(
            set_of(plan(&v, SettingsIntent::KeymapPreset(KeymapPreset::MsIme))),
            pair("keymap.preset", "ms-ime")
        );
        assert_eq!(
            set_of(plan(&v, SettingsIntent::PagingWrap(false))),
            pair("paging.wrap", "false")
        );
        assert_eq!(
            set_of(plan(&v, SettingsIntent::LmEnabled(false))),
            pair("engine.lm", "off")
        );
        assert_eq!(
            set_of(plan(&v, SettingsIntent::PostalEnglish(true))),
            pair("postal.english", "on")
        );
        assert_eq!(
            set_of(plan(&v, SettingsIntent::CandidateHideWhenConfident(false))),
            pair("candidate.hide_when_confident", "off")
        );
        for unchanged in [
            SettingsIntent::InputMethod(InputMethodKind::Romaji),
            SettingsIntent::KeymapPreset(KeymapPreset::Macos),
            SettingsIntent::PagingWrap(true),
            SettingsIntent::PageSize(3),
            SettingsIntent::LmEnabled(true),
            SettingsIntent::PostalJapanese(true),
            SettingsIntent::EmojiShortcode(false),
        ] {
            assert_eq!(plan(&v, unchanged), Ok(None));
        }
    }

    #[test]
    fn numbers_are_clamped_to_their_range() {
        let v = SettingsValues::default();
        assert_eq!(
            set_of(plan(&v, SettingsIntent::PageSize(0))),
            pair("paging.size", "1")
        );
        assert_eq!(
            set_of(plan(&v, SettingsIntent::PageSize(99))),
            pair("paging.size", "9")
        );
        assert_eq!(
            set_of(plan(&v, SettingsIntent::PageSize(5))),
            pair("paging.size", "5")
        );
        assert_eq!(
            set_of(plan(&v, SettingsIntent::IdleUnloadSecs(-5))),
            pair("engine.idle_unload_secs", "0")
        );
        assert_eq!(
            set_of(plan(&v, SettingsIntent::IdleUnloadSecs(1_000_000))),
            pair("engine.idle_unload_secs", "86400")
        );
        assert_eq!(
            set_of(plan(&v, SettingsIntent::LmDeadlineMs(1))),
            pair("engine.lm_deadline_ms", "50")
        );
        assert_eq!(
            set_of(plan(&v, SettingsIntent::LmDeadlineMs(9999))),
            pair("engine.lm_deadline_ms", "2000")
        );
        assert_eq!(
            set_of(plan(&v, SettingsIntent::ConvertWaitMs(500))),
            pair("engine.convert_wait_ms", "200")
        );
        // 丸めた結果が今と同じなら送らない
        assert_eq!(plan(&v, SettingsIntent::LmDeadlineMs(300)), Ok(None));
        let at_max = SettingsValues {
            page_size: 9,
            ..Default::default()
        };
        assert_eq!(plan(&at_max, SettingsIntent::PageSize(100)), Ok(None));
    }

    #[test]
    fn emoji_shortcode_exclude_add_remove_reset() {
        let v = SettingsValues::default();
        let (key, value) = set_of(plan(
            &v,
            SettingsIntent::AddEmojiShortcodeExclude("com.x.y".into()),
        ));
        assert_eq!(key, "emoji.shortcode_exclude");
        assert!(value.ends_with(",com.x.y"));
        assert_eq!(
            set_of(plan(
                &v,
                SettingsIntent::RemoveEmojiShortcodeExclude("com.apple.Terminal".into())
            ))
            .1,
            "com.googlecode.iterm2,com.github.wez.wezterm,com.mitchellh.ghostty"
        );
        assert_eq!(
            plan(&v, SettingsIntent::ResetEmojiShortcodeExclude),
            Ok(None)
        );
        let mut changed = SettingsValues::default();
        changed.emoji_shortcode_exclude.clear();
        assert_eq!(
            reset_of(plan(&changed, SettingsIntent::ResetEmojiShortcodeExclude)),
            "emoji.shortcode_exclude"
        );
    }

    // ---- キー割り当て

    #[test]
    fn preset_table_reads_the_bundled_presets() {
        let p = presets();
        let macos = p.chords("convert.hiragana", KeymapPreset::Macos);
        assert_eq!(
            macos.iter().map(KeyChord::to_text).collect::<Vec<_>>(),
            vec!["f6", "ctrl+j"]
        );
        assert_eq!(
            p.primary_chord("convert.hiragana", KeymapPreset::Macos)
                .map(KeyChord::display),
            Some("⌃J".to_string())
        );
        assert_eq!(
            p.primary_chord("convert.hiragana", KeymapPreset::MsIme)
                .map(KeyChord::display),
            Some("⌃U".to_string())
        );
        // F キーだけの操作には文字キーの既定がない
        assert!(
            p.primary_chord("convert.halfwidth_alnum", KeymapPreset::Macos)
                .is_none()
        );
        assert!(p.chords("mode.toggle", KeymapPreset::Macos).is_empty());
        // 今の Swift の presetTableListsFunctionKeysFirstAndPrimaryChordIsACharacterKey の残りを写した
        assert_eq!(
            p.primary_chord("convert.halfwidth_alnum", KeymapPreset::Atok)
                .map(KeyChord::to_text),
            Some("ctrl+@".to_string())
        );
        assert_eq!(
            p.primary_chord("reconvert", KeymapPreset::Atok)
                .map(KeyChord::to_text),
            Some("ctrl+shift+y".to_string())
        );
    }

    #[test]
    fn broken_or_missing_presets_stay_empty() {
        let p = KeymapPresetTable::new(|preset| match preset {
            KeymapPreset::Macos => Some("f6\tconvert.hiragana\nnot a line\n".to_string()),
            KeymapPreset::MsIme => None,
            KeymapPreset::Atok => Some("# c\n\nf6\tconvert.hiragana\r\n".to_string()),
        });
        assert!(p.chords("convert.hiragana", KeymapPreset::Macos).is_empty());
        assert!(p.chords("convert.hiragana", KeymapPreset::MsIme).is_empty());
        assert_eq!(p.chords("convert.hiragana", KeymapPreset::Atok).len(), 1);
    }

    #[test]
    fn keymap_changes_set_none_reset_and_chord() {
        let v = SettingsValues::default();
        let keymap = |binding| SettingsIntent::Keymap {
            action: "convert.hiragana".into(),
            binding,
        };
        assert_eq!(
            set_of(plan(&v, keymap(KeymapBinding::None))),
            pair("keymap.convert.hiragana", "none")
        );
        // 表記は整えて書く
        assert_eq!(
            set_of(plan(
                &v,
                keymap(KeymapBinding::Chord("Shift+Ctrl+H".into()))
            )),
            pair("keymap.convert.hiragana", "ctrl+shift+h")
        );
        // 上書きがなければ戻すものがない
        assert_eq!(plan(&v, keymap(KeymapBinding::Preset)), Ok(None));
        let mut overridden = SettingsValues::default();
        overridden
            .keymap_bindings
            .insert("convert.hiragana".into(), KeymapBinding::None);
        assert_eq!(
            reset_of(plan(&overridden, keymap(KeymapBinding::Preset))),
            "keymap.convert.hiragana"
        );
        // 読めない表記と知らない操作
        assert_eq!(
            plan(&v, keymap(KeymapBinding::Chord("ctrl+ctrl+x".into()))),
            Ok(None)
        );
        assert_eq!(
            plan(
                &v,
                SettingsIntent::Keymap {
                    action: "nope".into(),
                    binding: KeymapBinding::None
                }
            ),
            Err(PlanError::UnknownAction)
        );
    }

    #[test]
    fn a_key_used_by_another_action_is_refused() {
        let v = SettingsValues::default();
        // ctrl+k は macOS プリセットで全角カタカナ
        let result = plan(
            &v,
            SettingsIntent::Keymap {
                action: "convert.hiragana".into(),
                binding: KeymapBinding::Chord("ctrl+k".into()),
            },
        );
        assert_eq!(
            result,
            Err(PlanError::KeyConflict {
                chord_display: "⌃K".into(),
                other_action: "convert.fullwidth_katakana".into()
            })
        );
        // 自分自身のキーは衝突ではない
        assert!(
            plan(
                &v,
                SettingsIntent::Keymap {
                    action: "convert.hiragana".into(),
                    binding: KeymapBinding::Chord("ctrl+j".into()),
                },
            )
            .unwrap()
            .is_some()
        );
    }

    #[test]
    fn an_override_frees_the_preset_key_and_keeps_function_keys() {
        let mut v = SettingsValues::default();
        v.keymap_bindings.insert(
            "convert.hiragana".into(),
            KeymapBinding::Chord("ctrl+h".into()),
        );
        let p = presets();
        let effective: Vec<String> = v
            .effective_chords("convert.hiragana", &p)
            .iter()
            .map(KeyChord::to_text)
            .collect();
        assert_eq!(effective, vec!["ctrl+h", "f6"]);
        // 外れた ctrl+j は別の操作に割り当てられる
        let ctrl_j = KeyChord::parse("ctrl+j").unwrap();
        assert_eq!(v.conflicting_action(&ctrl_j, "reconvert", &p), None);
        // none は F キーも外す
        v.keymap_bindings
            .insert("convert.hiragana".into(), KeymapBinding::None);
        assert!(v.effective_chords("convert.hiragana", &p).is_empty());
        let f6 = KeyChord::parse("f6").unwrap();
        assert_eq!(v.conflicting_action(&f6, "reconvert", &p), None);
    }

    #[test]
    fn an_override_equal_to_the_preset_resolves_to_preset() {
        let mut v = SettingsValues::default();
        v.keymap_bindings.insert(
            "convert.hiragana".into(),
            KeymapBinding::Chord("ctrl+j".into()),
        );
        assert_eq!(
            v.resolved_binding("convert.hiragana", &presets()),
            KeymapBinding::Preset
        );
        v.keymap_bindings.insert(
            "convert.hiragana".into(),
            KeymapBinding::Chord("ctrl+h".into()),
        );
        assert_eq!(
            v.resolved_binding("convert.hiragana", &presets()),
            KeymapBinding::Chord("ctrl+h".into())
        );
    }

    #[test]
    fn keymap_rows_show_override_preset_and_unset_texts() {
        let mut v = SettingsValues::default();
        v.keymap_bindings.insert(
            "convert.fullwidth_katakana".into(),
            KeymapBinding::Chord("ctrl+g".into()),
        );
        v.keymap_bindings
            .insert("reconvert".into(), KeymapBinding::None);
        let rows = v.keymap_rows(&presets());
        assert_eq!(rows.len(), KEYMAP_ACTIONS.len());
        let row = |action: &str| rows.iter().find(|r| r.action == action).unwrap();
        assert_eq!(row("convert.hiragana").key_text.as_deref(), Some("⌃J"));
        assert_eq!(row("convert.hiragana").binding, KeymapBinding::Preset);
        assert_eq!(
            row("convert.fullwidth_katakana").key_text.as_deref(),
            Some("⌃G")
        );
        assert_eq!(
            row("convert.fullwidth_katakana").preset_text.as_deref(),
            Some("⌃K")
        );
        assert_eq!(row("reconvert").key_text, None);
        assert_eq!(row("reconvert").preset_text.as_deref(), Some("⌃⇧R"));
        assert_eq!(row("mode.toggle").key_text, None);
        assert_eq!(row("segment.shrink").key_text.as_deref(), Some("⇧←"));
    }

    // ---- キー割り当てのプリセットの内容（Swift の KeymapTests.presetFilesFollowTheContractTable の写し）

    fn bindings_of(text: &str) -> HashMap<KeyChord, String> {
        parse_preset(text).unwrap()
    }

    fn chord(text: &str) -> KeyChord {
        KeyChord::parse(text).unwrap()
    }

    #[test]
    fn preset_files_follow_the_contract_table() {
        let macos = bindings_of(MACOS);
        let ms_ime = bindings_of(MS_IME);
        let expectations = [
            ("convert.hiragana", "f6", Some("ctrl+j"), Some("ctrl+u")),
            (
                "convert.fullwidth_katakana",
                "f7",
                Some("ctrl+k"),
                Some("ctrl+i"),
            ),
            (
                "convert.halfwidth_katakana",
                "f8",
                Some("ctrl+;"),
                Some("ctrl+o"),
            ),
            (
                "convert.fullwidth_alnum",
                "f9",
                Some("ctrl+l"),
                Some("ctrl+p"),
            ),
            ("convert.halfwidth_alnum", "f10", None, Some("ctrl+t")),
        ];
        for (action, function, macos_key, ms_key) in expectations {
            assert_eq!(
                macos.get(&chord(function)).map(String::as_str),
                Some(action)
            );
            assert_eq!(
                ms_ime.get(&chord(function)).map(String::as_str),
                Some(action)
            );
            if let Some(key) = macos_key {
                assert_eq!(macos.get(&chord(key)).map(String::as_str), Some(action));
            }
            if let Some(key) = ms_key {
                assert_eq!(ms_ime.get(&chord(key)).map(String::as_str), Some(action));
            }
        }
        for (action, key) in [
            ("segment.focus_left", "left"),
            ("segment.focus_right", "right"),
            ("segment.shrink", "shift+left"),
            ("segment.expand", "shift+right"),
            ("reconvert", "ctrl+shift+r"),
            ("candidate.next", "down"),
            ("candidate.previous", "up"),
        ] {
            assert_eq!(macos.get(&chord(key)).map(String::as_str), Some(action));
            assert_eq!(ms_ime.get(&chord(key)).map(String::as_str), Some(action));
        }
        assert_eq!(macos.len(), 16);
        assert_eq!(ms_ime.len(), 17);
        assert!(!macos.contains_key(&chord("ctrl+u")));
        assert!(!ms_ime.contains_key(&chord("ctrl+j")));
    }

    /// ATOK 風は MS-IME 風を土台に、半角英数（ctrl+@）と再変換（ctrl+shift+y）だけ違う（#62）
    #[test]
    fn atok_preset_differs_from_ms_ime_only_in_halfwidth_alnum_and_reconvert() {
        let atok = bindings_of(ATOK);
        let mut expected = bindings_of(MS_IME);
        expected.remove(&chord("ctrl+t"));
        expected.remove(&chord("ctrl+shift+r"));
        expected.insert(chord("ctrl+@"), "convert.halfwidth_alnum".into());
        // US 配列では @ が Shift+2 なので、ctrl+shift+@ も同じ操作にする
        expected.insert(chord("ctrl+shift+@"), "convert.halfwidth_alnum".into());
        expected.insert(chord("ctrl+shift+y"), "reconvert".into());
        assert_eq!(atok, expected);
    }

    #[test]
    fn preset_lines_that_break_the_rules_are_rejected() {
        for tsv in [
            "f6",
            "f6\tnope",
            "ctrl+ctrl+j\tconvert.hiragana",
            "\nf6\tconvert.hiragana\textra",
            "f6 convert.hiragana",
        ] {
            assert!(parse_preset(tsv).is_none(), "{tsv:?}");
        }
        assert_eq!(
            parse_preset("# c\n\nf6\tconvert.hiragana\r\n  \n")
                .unwrap()
                .len(),
            1
        );
    }
}
