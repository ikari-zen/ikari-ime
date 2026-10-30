//! キーの表記（`ctrl+j`、`f6`、`shift+space`。docs/phase2-contract.md §4）。設定の検証と、設定アプリの表示に使う。
//!
//! 今の `KeyChord(parsing:)`（Keymap.swift）と `KeyChordText`（設定アプリ）の純関数の部分。
//! 型は `keymap::KeyChord` 1 つにまとめた（L-A2。修飾キーは契約の `Modifiers`）。ここは設定の側の名前とテストだけを持つ。
//! `KeyChordText.record`（macOS の keyCode から記録する部分）は OS 側なので移さない。

pub use crate::keymap::KeyChord;

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
        assert!(c.modifiers.control && c.modifiers.shift && !c.modifiers.option);
        assert!(chord("f6").modifiers.is_empty());
        assert_eq!(chord("ctrl+;").key, ";");
        assert_eq!(chord("f").key, "f");
    }

    #[test]
    fn plus_can_be_the_key() {
        assert_eq!(chord("+").key, "+");
        let c = chord("ctrl++");
        assert_eq!(c.key, "+");
        assert!(c.modifiers.control);
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
