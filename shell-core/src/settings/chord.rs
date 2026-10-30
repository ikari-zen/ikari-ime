//! キーの表記（`ctrl+j`、`f6`、`shift+space`。docs/phase2-contract.md §4）。設定の検証と、設定アプリの表示に使う。
//!
//! 今の `KeyChord(parsing:)`（Keymap.swift）と `KeyChordText`（設定アプリ）の純関数の部分。
//! `shell-core::keymap`（L-A1）が同じ型を持つなら、統合のときにそちらへ寄せる（ここは settings だけで閉じる）。
//! `KeyChordText.record`（macOS の keyCode から記録する部分）は OS 側なので移さない。

use super::text::trim_blank;

/// 修飾キーの集合（Swift の `KeyModifiers` の rawValue と同じ並び）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct ChordModifiers(pub u8);

impl ChordModifiers {
    pub const SHIFT: Self = Self(1 << 0);
    pub const CONTROL: Self = Self(1 << 1);
    pub const OPTION: Self = Self(1 << 2);
    pub const COMMAND: Self = Self(1 << 3);

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// キーの組み合わせ。`key` は小文字の名前（1 文字のキー、`space`、`left`、`f6` など）
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub key: String,
    pub modifiers: ChordModifiers,
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
/// 表記に出す順（`to_text`）
const MODIFIER_NAMES: [(&str, ChordModifiers); 4] = [
    ("ctrl", ChordModifiers::CONTROL),
    ("shift", ChordModifiers::SHIFT),
    ("option", ChordModifiers::OPTION),
    ("command", ChordModifiers::COMMAND),
];

impl KeyChord {
    /// `ctrl+j` のような表記から作る。読めなければ `None`。
    /// キーは小文字に直す。未知の修飾キーと、修飾キーの重複は読めない扱い。
    pub fn parse(text: &str) -> Option<Self> {
        let trimmed = trim_blank(text).to_lowercase();
        if trimmed.is_empty() {
            return None;
        }
        let plus = SEPARATOR.to_string();
        let double = format!("{SEPARATOR}{SEPARATOR}");

        // キー自体が `+` の場合（`+`、`ctrl++`）を先に切り分ける
        let (key_part, modifier_part): (&str, &str) = if trimmed == plus {
            (&plus, "")
        } else if let Some(rest) = trimmed.strip_suffix(&double) {
            if rest.is_empty() {
                return None;
            }
            (&plus, rest)
        } else if let Some(index) = trimmed.rfind(SEPARATOR) {
            let modifiers = &trimmed[..index];
            if modifiers.is_empty() {
                return None;
            }
            (&trimmed[index + 1..], modifiers)
        } else {
            (&trimmed, "")
        };

        let mut modifiers = ChordModifiers::default();
        if !modifier_part.is_empty() {
            for name in modifier_part.split(SEPARATOR) {
                let (_, modifier) = MODIFIER_NAMES.iter().find(|(n, _)| *n == name)?;
                if modifiers.contains(*modifier) {
                    return None;
                }
                modifiers.0 |= modifier.0;
            }
        }
        if !is_valid_key_name(key_part) {
            return None;
        }
        Some(Self {
            key: key_part.to_string(),
            modifiers,
        })
    }

    /// 設定ファイルの表記（`ctrl+shift+j`、`f6`）。`parse` で読み戻せる
    pub fn to_text(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for (name, modifier) in MODIFIER_NAMES {
            if self.modifiers.contains(modifier) {
                parts.push(name);
            }
        }
        parts.push(&self.key);
        parts.join("+")
    }

    /// 画面に出す表記（`⌃J`、`F6`、`⇧←`）。修飾キーは Apple の順（⌃⌥⇧⌘）。矢印は記号
    pub fn display(&self) -> String {
        let mut text = String::new();
        for (symbol, modifier) in [
            ('⌃', ChordModifiers::CONTROL),
            ('⌥', ChordModifiers::OPTION),
            ('⇧', ChordModifiers::SHIFT),
            ('⌘', ChordModifiers::COMMAND),
        ] {
            if self.modifiers.contains(modifier) {
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

    /// 設定ファイルの表記から画面の表記へ。読めなければ元の文字列
    pub fn display_text(text: &str) -> String {
        Self::parse(text).map_or_else(|| text.to_string(), |chord| chord.display())
    }

    /// `f` + 数字（F キーは常に使える）
    pub fn is_function_key(&self) -> bool {
        self.key.chars().count() > 1
            && self
                .key
                .strip_prefix('f')
                .is_some_and(|n| n.parse::<i64>().is_ok())
    }
}

/// `f1`〜`f20`、名前つきのキー、空白でない 1 文字のどれか
fn is_valid_key_name(name: &str) -> bool {
    if NAMED_KEYS.contains(&name) {
        return true;
    }
    let mut chars = name.chars();
    if let (Some(first), None) = (chars.next(), chars.next()) {
        return !first.is_whitespace();
    }
    if let Some(number) = name.strip_prefix('f') {
        return number
            .parse::<u32>()
            .is_ok_and(|n| FUNCTION_KEY_RANGE.contains(&n) && n.to_string() == number);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(text: &str) -> KeyChord {
        KeyChord::parse(text).unwrap()
    }

    #[test]
    fn parses_modifiers_and_keys() {
        let c = chord("Ctrl+Shift+J");
        assert_eq!(c.key, "j");
        assert_eq!(
            c.modifiers.0,
            ChordModifiers::CONTROL.0 | ChordModifiers::SHIFT.0
        );
        assert_eq!(chord("f6").modifiers, ChordModifiers::default());
        assert_eq!(chord("ctrl+;").key, ";");
        assert_eq!(chord("f").key, "f");
    }

    #[test]
    fn plus_can_be_the_key() {
        assert_eq!(chord("+").key, "+");
        let c = chord("ctrl++");
        assert_eq!(c.key, "+");
        assert!(c.modifiers.contains(ChordModifiers::CONTROL));
    }

    #[test]
    fn rejects_unreadable_chords() {
        for text in [
            "",
            "  ",
            "ctrl+",
            "+j",
            "alt+j",
            "ctrl+ctrl+j",
            "f0",
            "f21",
            "f06",
            "ab",
            "ctrl+ab",
            "+ +",
        ] {
            assert!(KeyChord::parse(text).is_none(), "{text:?}");
        }
    }

    #[test]
    fn text_round_trips() {
        for text in [
            "ctrl+shift+j",
            "f6",
            "shift+left",
            "ctrl+;",
            "ctrl++",
            "option+command+x",
        ] {
            assert_eq!(chord(text).to_text(), text);
        }
        // 修飾キーは ctrl, shift, option, command の順に整う
        assert_eq!(chord("shift+ctrl+j").to_text(), "ctrl+shift+j");
    }

    #[test]
    fn display_uses_apple_order_and_arrow_symbols() {
        assert_eq!(chord("ctrl+j").display(), "⌃J");
        assert_eq!(chord("f6").display(), "F6");
        assert_eq!(chord("shift+left").display(), "⇧←");
        assert_eq!(chord("right").display(), "→");
        assert_eq!(chord("ctrl+option+shift+command+x").display(), "⌃⌥⇧⌘X");
        assert_eq!(KeyChord::display_text("ctrl+j"), "⌃J");
        assert_eq!(KeyChord::display_text("bogus"), "bogus");
    }

    #[test]
    fn function_key_detection() {
        assert!(chord("f6").is_function_key());
        assert!(!chord("left").is_function_key());
        assert!(!chord("f").is_function_key());
        assert!(!chord("ctrl+j").is_function_key());
    }
}
