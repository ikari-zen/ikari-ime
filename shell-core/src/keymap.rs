//! キー割り当て（今の `Keymap.swift`。macOS の keyCode の表 `KeyCodes.swift` は OS 側に残す。レーン L-A1）。
//!
//! 修飾キーは契約の `Modifiers` を使う。プリセットの TSV（`macos/Resources/keymap/*.tsv`）は呼び出し側が読んで渡す。

use std::collections::{BTreeMap, HashMap};

use unicode_segmentation::UnicodeSegmentation;

use crate::types::{KeyInput, KeyKind, Modifiers};

/// `settings.tsv` の `keymap.<操作>` のキーの頭。
const SETTINGS_KEY_PREFIX: &str = "keymap.";
/// `keymap.<操作>` で割り当てを外すときの値。
pub const NONE_VALUE: &str = "none";

/// 割り当てられる操作。名前（`name()`）は keymap の TSV と `settings.tsv` の `keymap.<操作>` のキーに使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KeymapAction {
    ToHiragana,
    ToFullwidthKatakana,
    ToHalfwidthKatakana,
    ToFullwidthAlnum,
    ToHalfwidthAlnum,
    /// フォーカスを左の文節へ（変換中）。既定は `left`
    SegmentFocusLeft,
    /// フォーカスを右の文節へ（変換中）。既定は `right`
    SegmentFocusRight,
    /// フォーカス中の文節を 1 文字縮める（変換中）。既定は `shift+left`
    SegmentShrink,
    /// フォーカス中の文節を 1 文字伸ばす（変換中）。既定は `shift+right`
    SegmentExpand,
    /// 次の候補（変換中。Space と同じ）。既定は `down`
    CandidateNext,
    /// 前の候補（変換中。Shift+Space と同じ）。既定は `up`
    CandidatePrevious,
    /// 選択した確定済みの文字列の再変換（idle）。既定は `ctrl+shift+r`
    Reconvert,
    /// 日本語 ⇄ 英語を行き来する（既定の割り当ては空。ADR 0026 §5.3）
    ModeToggle,
    /// 英語のモードにする
    ModeEnglish,
    /// 日本語のモードにする
    ModeJapanese,
    /// 直前の文節をすぐ消す（ADR 0050）。既定は `option+backspace`
    ClauseDelete,
    /// 削除の対象をハイライトするモードに入る（ADR 0050）。既定は `shift+option+backspace`
    ClauseDeletePreview,
}

impl KeymapAction {
    /// 設定アプリの画面と `settings.tsv` の並び（Swift の `allCases` と同じ順）。
    pub const ALL: [Self; 17] = [
        Self::ToHiragana,
        Self::ToFullwidthKatakana,
        Self::ToHalfwidthKatakana,
        Self::ToFullwidthAlnum,
        Self::ToHalfwidthAlnum,
        Self::SegmentFocusLeft,
        Self::SegmentFocusRight,
        Self::SegmentShrink,
        Self::SegmentExpand,
        Self::CandidateNext,
        Self::CandidatePrevious,
        Self::Reconvert,
        Self::ModeToggle,
        Self::ModeEnglish,
        Self::ModeJapanese,
        Self::ClauseDelete,
        Self::ClauseDeletePreview,
    ];

    /// TSV と設定のキーに使う名前（Swift の `rawValue`）。
    pub fn name(self) -> &'static str {
        match self {
            Self::ToHiragana => "convert.hiragana",
            Self::ToFullwidthKatakana => "convert.fullwidth_katakana",
            Self::ToHalfwidthKatakana => "convert.halfwidth_katakana",
            Self::ToFullwidthAlnum => "convert.fullwidth_alnum",
            Self::ToHalfwidthAlnum => "convert.halfwidth_alnum",
            Self::SegmentFocusLeft => "segment.focus_left",
            Self::SegmentFocusRight => "segment.focus_right",
            Self::SegmentShrink => "segment.shrink",
            Self::SegmentExpand => "segment.expand",
            Self::CandidateNext => "candidate.next",
            Self::CandidatePrevious => "candidate.previous",
            Self::Reconvert => "reconvert",
            Self::ModeToggle => "mode.toggle",
            Self::ModeEnglish => "mode.english",
            Self::ModeJapanese => "mode.japanese",
            Self::ClauseDelete => "clause.delete",
            Self::ClauseDeletePreview => "clause.delete_preview",
        }
    }

    /// 名前から。知らない名前は `None`。
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.name() == name)
    }

    /// F6〜F10 とプリセットの字種変換キー。
    pub fn is_script_conversion(self) -> bool {
        matches!(
            self,
            Self::ToHiragana
                | Self::ToFullwidthKatakana
                | Self::ToHalfwidthKatakana
                | Self::ToFullwidthAlnum
                | Self::ToHalfwidthAlnum
        )
    }

    /// 入力モードの切り替え（`mode.*`）。
    pub fn is_mode_switch(self) -> bool {
        matches!(
            self,
            Self::ModeToggle | Self::ModeEnglish | Self::ModeJapanese
        )
    }
}

/// キーの組み合わせ（`ctrl+j`、`f6`、`shift+space` の表記）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyChord {
    /// 小文字の名前。1 文字のキー（`j`、`;`）、`space`、`enter`、`escape`、`backspace`、`left`、`right`、`up`、`down`、`f1`〜`f20`
    pub key: String,
    pub modifiers: Modifiers,
}

const NAMED_KEYS: [&str; 8] = [
    "space",
    "enter",
    "escape",
    "backspace",
    "left",
    "right",
    "up",
    "down",
];
const FUNCTION_KEY_RANGE: std::ops::RangeInclusive<u32> = 1..=20;
const SEPARATOR: char = '+';

/// 押されたキーを、割り当てとして引けない理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordRefusal {
    /// 修飾キーのない文字キー（打鍵そのものを奪ってしまう）
    NeedsModifier,
    /// 状態機械が割り当てとして引かないキー（Space、Enter、Esc、Backspace、複数文字など）
    Unsupported,
}

/// 設定アプリでキーを押して記録した結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordRecording {
    Chord(KeyChord),
    /// 修飾キーなしの Esc。記録をやめる
    Cancel,
    Refused(ChordRefusal),
}

impl KeyChord {
    /// 状態機械が割り当てとして引くキー。F キーと、←/→/↑/↓（修飾キーの有無を問わない）と、
    /// Ctrl / Option / Command を伴う 1 文字だけ。`Machine::bound_action` と設定アプリの記録が同じ規則を使う。
    pub fn for_lookup(input: &KeyInput) -> Result<Self, ChordRefusal> {
        let named = |name: &str| Self::new(name, input.modifiers);
        match &input.key {
            KeyKind::Function { number } => Ok(named(&format!("f{number}"))),
            KeyKind::Character { text } => {
                if text.graphemes(true).count() != 1 {
                    return Err(ChordRefusal::Unsupported);
                }
                let m = input.modifiers;
                if !(m.control || m.option || m.command) {
                    return Err(ChordRefusal::NeedsModifier);
                }
                Ok(named(&text.to_lowercase()))
            }
            KeyKind::Left => Ok(named("left")),
            KeyKind::Right => Ok(named("right")),
            KeyKind::Up => Ok(named("up")),
            KeyKind::Down => Ok(named("down")),
            _ => Err(ChordRefusal::Unsupported),
        }
    }

    /// 設定アプリで押されたキーを割り当てとして受け入れるか。JIS の位置にあるキーは、配列が出す文字（`layout_text`）で見る。
    /// 表記に直して読み戻せないものは断る。
    pub fn record(input: &KeyInput) -> ChordRecording {
        if input.key == KeyKind::Escape && input.modifiers.is_empty() {
            return ChordRecording::Cancel;
        }
        let looked_up = match &input.key {
            KeyKind::Jis { .. } => {
                let mut as_character = input.clone();
                as_character.key = KeyKind::Character {
                    text: input.layout_text.clone().unwrap_or_default(),
                };
                Self::for_lookup(&as_character)
            }
            _ => Self::for_lookup(input),
        };
        match looked_up {
            Ok(chord) if Self::parse(&chord.to_text()).as_ref() == Some(&chord) => {
                ChordRecording::Chord(chord)
            }
            Ok(_) => ChordRecording::Refused(ChordRefusal::Unsupported),
            Err(refusal) => ChordRecording::Refused(refusal),
        }
    }

    pub fn new(key: impl Into<String>, modifiers: Modifiers) -> Self {
        Self {
            key: key.into(),
            modifiers,
        }
    }

    /// 修飾キーなしのキー。
    pub fn plain(key: impl Into<String>) -> Self {
        Self::new(key, Modifiers::NONE)
    }

    /// `ctrl+j` のような表記から作る。読めなければ `None`。
    /// キーは小文字に直す。未知の修飾キーと、修飾キーの重複は読めない扱いにする。
    pub fn parse(text: &str) -> Option<Self> {
        let trimmed = text.trim_matches(is_inline_space).to_lowercase();
        if trimmed.is_empty() {
            return None;
        }
        let plus = SEPARATOR.to_string();
        let double = format!("{SEPARATOR}{SEPARATOR}");

        // キー自体が `+` の場合（`+`、`ctrl++`）を先に切り分ける
        let (key_part, modifier_part): (&str, &str) = if trimmed == plus {
            (&plus, "")
        } else if let Some(head) = trimmed.strip_suffix(double.as_str()) {
            if head.is_empty() {
                return None;
            }
            (&plus, head)
        } else if let Some(index) = trimmed.rfind(SEPARATOR) {
            let head = &trimmed[..index];
            if head.is_empty() {
                return None;
            }
            (&trimmed[index + SEPARATOR.len_utf8()..], head)
        } else {
            (trimmed.as_str(), "")
        };

        let mut modifiers = Modifiers::NONE;
        if !modifier_part.is_empty() {
            for name in modifier_part.split(SEPARATOR) {
                let flag = match name {
                    "ctrl" => &mut modifiers.control,
                    "shift" => &mut modifiers.shift,
                    "option" => &mut modifiers.option,
                    "command" => &mut modifiers.command,
                    _ => return None,
                };
                if *flag {
                    return None;
                }
                *flag = true;
            }
        }
        if !Self::is_valid_key_name(key_part) {
            return None;
        }
        Some(Self::new(key_part, modifiers))
    }

    /// `f1`〜`f20`、名前つきのキー、空白でない 1 文字のどれか。
    fn is_valid_key_name(name: &str) -> bool {
        if NAMED_KEYS.contains(&name) {
            return true;
        }
        let mut chars = name.chars();
        if let (Some(first), None) = (chars.next(), chars.next()) {
            return !first.is_whitespace();
        }
        name.strip_prefix('f')
            .and_then(|rest| rest.parse::<u32>().ok().filter(|n| n.to_string() == rest))
            .is_some_and(|number| FUNCTION_KEY_RANGE.contains(&number))
    }

    /// `f` + 数字（F キーは常に使える）。
    pub fn is_function_key(&self) -> bool {
        self.key.chars().count() > 1
            && self
                .key
                .strip_prefix('f')
                .is_some_and(|rest| rest.parse::<i64>().is_ok())
    }

    /// 設定ファイルの表記（`ctrl+shift+j`、`f6`）。`parse` で読み戻せる。修飾キーは ctrl, shift, option, command の順。
    pub fn to_text(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for (name, on) in [
            ("ctrl", self.modifiers.control),
            ("shift", self.modifiers.shift),
            ("option", self.modifiers.option),
            ("command", self.modifiers.command),
        ] {
            if on {
                parts.push(name);
            }
        }
        parts.push(&self.key);
        parts.join("+")
    }

    /// 画面に出す表記（`⌃J`、`F6`、`⇧←`）。修飾キーは Apple の順（⌃⌥⇧⌘）。左右の矢印は記号。
    pub fn display(&self) -> String {
        let mut text = String::new();
        for (symbol, on) in [
            ('⌃', self.modifiers.control),
            ('⌥', self.modifiers.option),
            ('⇧', self.modifiers.shift),
            ('⌘', self.modifiers.command),
        ] {
            if on {
                text.push(symbol);
            }
        }
        match self.key.as_str() {
            "left" => text.push('←'),
            "right" => text.push('→'),
            other => text.push_str(&other.to_uppercase()),
        }
        text
    }

    /// 設定ファイルの表記から画面の表記へ。読めなければ元の文字列。
    pub fn display_text(text: &str) -> String {
        Self::parse(text).map_or_else(|| text.to_string(), |chord| chord.display())
    }

    /// 並べるときの修飾キーの重み（Swift の `KeyModifiers.rawValue` と同じ: shift 1、control 2、option 4、command 8）。
    pub fn modifier_bits(&self) -> u8 {
        u8::from(self.modifiers.shift)
            | u8::from(self.modifiers.control) << 1
            | u8::from(self.modifiers.option) << 2
            | u8::from(self.modifiers.command) << 3
    }
}

/// 改行ではない空白（Swift の `CharacterSet.whitespaces` 相当）。
fn is_inline_space(c: char) -> bool {
    c.is_whitespace() && !is_newline(c)
}

fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{0b}' | '\u{0c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeymapError {
    /// プリセットの TSV の行が読めない（行番号は 1 始まり。内容は含めない）
    InvalidLine { number: usize },
}

impl std::fmt::Display for KeymapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLine { number } => write!(f, "keymap preset: invalid line {number}"),
        }
    }
}

impl std::error::Error for KeymapError {}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Keymap {
    pub bindings: HashMap<KeyChord, KeymapAction>,
}

impl Keymap {
    pub fn new(bindings: HashMap<KeyChord, KeymapAction>) -> Self {
        Self { bindings }
    }

    /// プリセットの TSV と、操作ごとの上書きから作る。
    /// 上書きされた操作は、プリセットの文字キー（`ctrl+j` など）の割り当てを置き換える。
    /// F キーの割り当ては残す（F6〜F10 は常に使える。ADR 0004 §15.1）。
    /// プリセットの行が読めなければ `KeymapError`。
    pub fn from_preset(
        preset_tsv: &str,
        overrides: &BTreeMap<KeymapAction, KeyChord>,
    ) -> Result<Self, KeymapError> {
        let mut table = parse_preset(preset_tsv)?;
        for (action, chord) in overrides {
            remove_bindings(*action, &mut table, true);
            table.insert(chord.clone(), *action);
        }
        Ok(Self { bindings: table })
    }

    /// `settings.tsv` の上書き（`SettingsSnapshot::keymap_overrides`）から作る。
    /// キーは操作の名前（`convert.hiragana`。`keymap.` が付いていてもよい）、値はキーの表記または `none`。
    /// `none` はその操作の割り当てを F キーも含めて外す。
    /// 読めない上書きと知らない操作は無視する（入力を止めない。`settings::validate` が先に警告を出している）。
    pub fn from_settings(
        preset_tsv: &str,
        overrides: &BTreeMap<String, String>,
    ) -> Result<Self, KeymapError> {
        let mut table = parse_preset(preset_tsv)?;
        for (name, text) in overrides {
            let name = name.strip_prefix(SETTINGS_KEY_PREFIX).unwrap_or(name);
            let Some(action) = KeymapAction::from_name(name) else {
                continue;
            };
            if text == NONE_VALUE {
                remove_bindings(action, &mut table, false);
            } else if let Some(chord) = KeyChord::parse(text) {
                remove_bindings(action, &mut table, true);
                table.insert(chord, action);
            }
        }
        Ok(Self { bindings: table })
    }

    pub fn action(&self, chord: &KeyChord) -> Option<KeymapAction> {
        self.bindings.get(chord).copied()
    }
}

fn parse_preset(tsv: &str) -> Result<HashMap<KeyChord, KeymapAction>, KeymapError> {
    let mut table = HashMap::new();
    let normalized = tsv.replace("\r\n", "\n");
    for (offset, raw_line) in normalized.split(is_newline).enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').collect();
        let parsed = match columns.as_slice() {
            [key, action] => KeyChord::parse(key).zip(KeymapAction::from_name(
                action.trim_matches(is_inline_space),
            )),
            _ => None,
        };
        let Some((chord, action)) = parsed else {
            return Err(KeymapError::InvalidLine { number: offset + 1 });
        };
        table.insert(chord, action);
    }
    Ok(table)
}

fn remove_bindings(
    action: KeymapAction,
    table: &mut HashMap<KeyChord, KeymapAction>,
    keep_function_keys: bool,
) {
    table
        .retain(|chord, bound| *bound != action || (keep_function_keys && chord.is_function_key()));
}

#[cfg(test)]
mod tests {
    use super::*;

    const MACOS: &str = include_str!("../../macos/Resources/keymap/macos.tsv");
    const MS_IME: &str = include_str!("../../macos/Resources/keymap/ms-ime.tsv");
    const ATOK: &str = include_str!("../../macos/Resources/keymap/atok.tsv");

    fn chord(text: &str) -> KeyChord {
        KeyChord::parse(text).unwrap()
    }

    fn key(kind: KeyKind, modifiers: Modifiers) -> KeyInput {
        KeyInput {
            modifiers,
            ..KeyInput::new(kind)
        }
    }

    fn character(text: &str, modifiers: Modifiers) -> KeyInput {
        key(KeyKind::Character { text: text.into() }, modifiers)
    }

    const CTRL: Modifiers = Modifiers {
        control: true,
        ..Modifiers::NONE
    };
    const SHIFT: Modifiers = Modifiers {
        shift: true,
        ..Modifiers::NONE
    };
    const CTRL_SHIFT: Modifiers = Modifiers {
        control: true,
        shift: true,
        ..Modifiers::NONE
    };

    #[test]
    fn records_a_character_with_a_command_modifier() {
        assert_eq!(
            KeyChord::record(&character("j", CTRL)),
            ChordRecording::Chord(chord("ctrl+j"))
        );
        // Shift が付いたときの大文字は小文字にそろえる
        assert_eq!(
            KeyChord::record(&character("J", CTRL_SHIFT)),
            ChordRecording::Chord(chord("ctrl+shift+j"))
        );
        assert_eq!(
            KeyChord::record(&character(";", CTRL)),
            ChordRecording::Chord(chord("ctrl+;"))
        );
    }

    #[test]
    fn records_jis_keys_by_the_layout_text() {
        let mut input = key(KeyKind::Jis { code: 38 }, CTRL);
        input.layout_text = Some("j".into());
        assert_eq!(
            KeyChord::record(&input),
            ChordRecording::Chord(chord("ctrl+j"))
        );
        input.layout_text = None;
        assert_eq!(
            KeyChord::record(&input),
            ChordRecording::Refused(ChordRefusal::Unsupported)
        );
    }

    #[test]
    fn records_function_and_arrow_keys_with_or_without_modifiers() {
        assert_eq!(
            KeyChord::record(&key(KeyKind::Function { number: 6 }, Modifiers::NONE)),
            ChordRecording::Chord(chord("f6"))
        );
        assert_eq!(
            KeyChord::record(&key(KeyKind::Function { number: 10 }, SHIFT)),
            ChordRecording::Chord(chord("shift+f10"))
        );
        assert_eq!(
            KeyChord::record(&key(KeyKind::Left, SHIFT)),
            ChordRecording::Chord(chord("shift+left"))
        );
        assert_eq!(
            KeyChord::record(&key(KeyKind::Down, Modifiers::NONE)),
            ChordRecording::Chord(chord("down"))
        );
        // 読み戻せない F キーは断る
        assert_eq!(
            KeyChord::record(&key(KeyKind::Function { number: 21 }, Modifiers::NONE)),
            ChordRecording::Refused(ChordRefusal::Unsupported)
        );
    }

    #[test]
    fn plain_or_shift_only_characters_need_a_modifier() {
        assert_eq!(
            KeyChord::record(&character("j", Modifiers::NONE)),
            ChordRecording::Refused(ChordRefusal::NeedsModifier)
        );
        assert_eq!(
            KeyChord::record(&character("J", SHIFT)),
            ChordRecording::Refused(ChordRefusal::NeedsModifier)
        );
    }

    #[test]
    fn keys_the_machine_does_not_look_up_are_refused() {
        for kind in [
            KeyKind::Space,
            KeyKind::Enter,
            KeyKind::Backspace,
            KeyKind::Other,
        ] {
            assert_eq!(
                KeyChord::record(&key(kind.clone(), CTRL)),
                ChordRecording::Refused(ChordRefusal::Unsupported),
                "{kind:?}"
            );
        }
        // 複数文字は、修飾キーがなくても「使えないキー」
        assert_eq!(
            KeyChord::record(&character("ab", Modifiers::NONE)),
            ChordRecording::Refused(ChordRefusal::Unsupported)
        );
        assert_eq!(
            KeyChord::record(&key(KeyKind::Escape, CTRL)),
            ChordRecording::Refused(ChordRefusal::Unsupported)
        );
    }

    #[test]
    fn plain_escape_cancels() {
        assert_eq!(
            KeyChord::record(&key(KeyKind::Escape, Modifiers::NONE)),
            ChordRecording::Cancel
        );
    }

    fn mods(control: bool, shift: bool, option: bool, command: bool) -> Modifiers {
        Modifiers {
            shift,
            control,
            option,
            command,
        }
    }

    #[test]
    fn parses_valid_notations() {
        let ctrl = mods(true, false, false, false);
        let shift = mods(false, true, false, false);
        let cases: Vec<(&str, &str, Modifiers)> = vec![
            ("ctrl+j", "j", ctrl),
            ("f6", "f6", Modifiers::NONE),
            ("shift+space", "space", shift),
            ("ctrl+;", ";", ctrl),
            ("CTRL+J", "j", ctrl),
            ("ctrl+shift+f10", "f10", mods(true, true, false, false)),
            (
                "command+option+enter",
                "enter",
                mods(false, false, true, true),
            ),
            ("escape", "escape", Modifiers::NONE),
            ("backspace", "backspace", Modifiers::NONE),
            ("f20", "f20", Modifiers::NONE),
            ("left", "left", Modifiers::NONE),
            ("shift+right", "right", shift),
            ("ctrl+shift+r", "r", mods(true, true, false, false)),
            ("ctrl++", "+", ctrl),
            ("+", "+", Modifiers::NONE),
            (" ctrl+k ", "k", ctrl),
        ];
        for (text, key, modifiers) in cases {
            assert_eq!(
                KeyChord::parse(text),
                Some(KeyChord::new(key, modifiers)),
                "{text}"
            );
        }
    }

    #[test]
    fn rejects_invalid_notations() {
        for text in [
            "",
            " ",
            "ctrl+",
            "ctrl",
            "+j",
            "meta+j",
            "ctrl+ctrl+j",
            "ctrl+jj",
            "f0",
            "f21",
            "f06",
            "ff",
            "ctrl+f1x",
            "ctrl+ ",
            "ctrl + j",
            "tab",
        ] {
            assert_eq!(KeyChord::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn preset_files_follow_the_contract_table() {
        let macos = Keymap::from_preset(MACOS, &BTreeMap::new()).unwrap();
        let ms_ime = Keymap::from_preset(MS_IME, &BTreeMap::new()).unwrap();
        type Row = (
            KeymapAction,
            &'static str,
            Option<&'static str>,
            Option<&'static str>,
        );
        let expectations: [Row; 5] = [
            (
                KeymapAction::ToHiragana,
                "f6",
                Some("ctrl+j"),
                Some("ctrl+u"),
            ),
            (
                KeymapAction::ToFullwidthKatakana,
                "f7",
                Some("ctrl+k"),
                Some("ctrl+i"),
            ),
            (
                KeymapAction::ToHalfwidthKatakana,
                "f8",
                Some("ctrl+;"),
                Some("ctrl+o"),
            ),
            (
                KeymapAction::ToFullwidthAlnum,
                "f9",
                Some("ctrl+l"),
                Some("ctrl+p"),
            ),
            (KeymapAction::ToHalfwidthAlnum, "f10", None, Some("ctrl+t")),
        ];
        for (action, function, macos_key, ms_key) in expectations {
            assert_eq!(macos.action(&chord(function)), Some(action));
            assert_eq!(ms_ime.action(&chord(function)), Some(action));
            if let Some(key) = macos_key {
                assert_eq!(macos.action(&chord(key)), Some(action));
            }
            if let Some(key) = ms_key {
                assert_eq!(ms_ime.action(&chord(key)), Some(action));
            }
        }
        // 文節の操作と再変換（ADR 0006 §8.1）は両プリセットで同じ。↑↓ は候補の軸（ADR 0014 §2.4）
        let segment: [(KeymapAction, &str); 7] = [
            (KeymapAction::SegmentFocusLeft, "left"),
            (KeymapAction::SegmentFocusRight, "right"),
            (KeymapAction::SegmentShrink, "shift+left"),
            (KeymapAction::SegmentExpand, "shift+right"),
            (KeymapAction::Reconvert, "ctrl+shift+r"),
            (KeymapAction::CandidateNext, "down"),
            (KeymapAction::CandidatePrevious, "up"),
        ];
        for (action, key) in segment {
            assert_eq!(macos.action(&chord(key)), Some(action));
            assert_eq!(ms_ime.action(&chord(key)), Some(action));
            assert!(!action.is_script_conversion());
        }
        assert_eq!(macos.bindings.len(), 16);
        assert_eq!(ms_ime.bindings.len(), 17);
        // 別のプリセットのキーは効かない
        assert_eq!(macos.action(&chord("ctrl+u")), None);
        assert_eq!(ms_ime.action(&chord("ctrl+j")), None);
    }

    /// ATOK 風は MS-IME 風を土台に、半角英数（ctrl+@）と再変換（ctrl+shift+y）だけ違う（#62）
    #[test]
    fn atok_preset_differs_from_ms_ime_only_in_halfwidth_alnum_and_reconvert() {
        let atok = Keymap::from_preset(ATOK, &BTreeMap::new()).unwrap();
        let ms_ime = Keymap::from_preset(MS_IME, &BTreeMap::new()).unwrap();
        let mut expected = ms_ime.bindings.clone();
        expected.remove(&chord("ctrl+t"));
        expected.remove(&chord("ctrl+shift+r"));
        expected.insert(chord("ctrl+@"), KeymapAction::ToHalfwidthAlnum);
        // US 配列では @ が Shift+2 なので、ctrl+shift+@ も同じ操作にする
        expected.insert(chord("ctrl+shift+@"), KeymapAction::ToHalfwidthAlnum);
        expected.insert(chord("ctrl+shift+y"), KeymapAction::Reconvert);
        assert_eq!(atok.bindings, expected);
        assert_eq!(
            atok.action(&chord("f10")),
            Some(KeymapAction::ToHalfwidthAlnum)
        );
        assert_eq!(atok.action(&chord("ctrl+shift+r")), None);
        assert_eq!(atok.action(&chord("ctrl+t")), None);
    }

    #[test]
    fn overrides_replace_character_keys_but_function_keys_remain() {
        let overrides = BTreeMap::from([(KeymapAction::ToHiragana, chord("ctrl+h"))]);
        let keymap = Keymap::from_preset(MACOS, &overrides).unwrap();
        assert_eq!(
            keymap.action(&chord("ctrl+h")),
            Some(KeymapAction::ToHiragana)
        );
        assert_eq!(keymap.action(&chord("ctrl+j")), None);
        assert_eq!(keymap.action(&chord("f6")), Some(KeymapAction::ToHiragana));
    }

    fn settings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn settings_none_removes_every_binding_of_the_action() {
        let overrides = settings(&[
            ("convert.hiragana", "none"),
            ("convert.fullwidth_katakana", "ctrl+g"),
        ]);
        let keymap = Keymap::from_settings(MACOS, &overrides).unwrap();
        assert_eq!(keymap.action(&chord("ctrl+j")), None);
        assert_eq!(keymap.action(&chord("f6")), None);
        assert_eq!(
            keymap.action(&chord("ctrl+g")),
            Some(KeymapAction::ToFullwidthKatakana)
        );
        assert_eq!(
            keymap.action(&chord("f7")),
            Some(KeymapAction::ToFullwidthKatakana)
        );
        assert_eq!(keymap.action(&chord("ctrl+k")), None);
    }

    #[test]
    fn settings_keys_may_carry_the_keymap_prefix() {
        let overrides = settings(&[("keymap.convert.hiragana", "none")]);
        let keymap = Keymap::from_settings(MACOS, &overrides).unwrap();
        assert_eq!(keymap.action(&chord("f6")), None);
    }

    #[test]
    fn settings_with_unreadable_override_or_unknown_action_are_ignored() {
        let overrides = settings(&[("convert.hiragana", "ctrl+ctrl+x"), ("convert.bogus", "f6")]);
        let keymap = Keymap::from_settings(MACOS, &overrides).unwrap();
        assert_eq!(
            keymap.action(&chord("ctrl+j")),
            Some(KeymapAction::ToHiragana)
        );
        assert_eq!(keymap.bindings.len(), 16);
    }

    /// Swift の `parsedSettingsDriveTheKeymap` の keymap 側
    #[test]
    fn ms_ime_with_hiragana_none_drives_the_keymap() {
        let overrides = settings(&[("convert.hiragana", "none")]);
        let keymap = Keymap::from_settings(MS_IME, &overrides).unwrap();
        let ctrl = mods(true, false, false, false);
        assert_eq!(keymap.action(&KeyChord::new("u", ctrl)), None);
        assert_eq!(
            keymap.action(&KeyChord::new("i", ctrl)),
            Some(KeymapAction::ToFullwidthKatakana)
        );
    }

    #[test]
    fn comments_and_blank_lines_in_presets_are_skipped() {
        let keymap =
            Keymap::from_preset("# c\n\nf6\tconvert.hiragana\r\n  \n", &BTreeMap::new()).unwrap();
        assert_eq!(keymap.bindings.len(), 1);
    }

    #[test]
    fn broken_preset_lines_report_the_line_number() {
        let cases = [
            ("f6", 1),
            ("f6\tnope", 1),
            ("ctrl+ctrl+j\tconvert.hiragana", 1),
            ("\nf6\tconvert.hiragana\textra", 2),
            ("f6 convert.hiragana", 1),
        ];
        for (tsv, line) in cases {
            assert_eq!(
                Keymap::from_preset(tsv, &BTreeMap::new()),
                Err(KeymapError::InvalidLine { number: line }),
                "{tsv:?}"
            );
        }
    }

    #[test]
    fn action_names_round_trip() {
        for action in KeymapAction::ALL {
            assert_eq!(KeymapAction::from_name(action.name()), Some(action));
        }
        assert!(KeymapAction::ModeToggle.is_mode_switch());
        assert!(!KeymapAction::Reconvert.is_mode_switch());
    }
}
