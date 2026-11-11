//! Windows の仮想キー（VK）と修飾、OS が返した文字 → `ShellEvent`（純粋）。
//!
//! 文字は VK の位置ではなく、OS が現在の配列で返した文字（`ToUnicode` 相当）から作る。JIS、US、ほかのどの配列でも同じ
//! （ユーザー決定: どのキーボード配列でも動かす）。VK で読むのは、配列によらず意味が決まるキー（Space、Enter、矢印、F1〜F20 など）だけ。
//!
//! かな入力用の物理キー（`KeyKind::Jis`）は、スキャンコードが要るのでこの表では作らない（かなの打鍵は W4 以降で扱う）。

use ikari_shell_core::{KeyInput, KeyKind, Modifiers, ShellEvent};

/// 仮想キーコード（`VK_*`）。`windows` crate に依存しないよう、数値を自分で持つ（winuser.h の値）。
pub mod vk {
    pub const BACK: u16 = 0x08;
    pub const TAB: u16 = 0x09;
    pub const RETURN: u16 = 0x0D;
    pub const SHIFT: u16 = 0x10;
    pub const CONTROL: u16 = 0x11;
    pub const MENU: u16 = 0x12;
    pub const CAPITAL: u16 = 0x14;
    /// `VK_KANA`（`VK_HANGUL` と同じ値）
    pub const KANA: u16 = 0x15;
    pub const ESCAPE: u16 = 0x1B;
    pub const SPACE: u16 = 0x20;
    pub const LEFT: u16 = 0x25;
    pub const UP: u16 = 0x26;
    pub const RIGHT: u16 = 0x27;
    pub const DOWN: u16 = 0x28;
    pub const LWIN: u16 = 0x5B;
    pub const RWIN: u16 = 0x5C;
    pub const F1: u16 = 0x70;
    pub const F20: u16 = 0x83;
    pub const F21: u16 = 0x84;
    pub const F24: u16 = 0x87;
    pub const LSHIFT: u16 = 0xA0;
    pub const RSHIFT: u16 = 0xA1;
    pub const LCONTROL: u16 = 0xA2;
    pub const RCONTROL: u16 = 0xA3;
    pub const LMENU: u16 = 0xA4;
    pub const RMENU: u16 = 0xA5;
    /// JIS の英数キー（`VK_OEM_ATTN`）
    pub const OEM_ATTN: u16 = 0xF0;
    /// JIS のひらがなキー（`VK_OEM_COPY`）
    pub const OEM_COPY: u16 = 0xF2;
}

/// 押されている修飾キー。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WinModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub win: bool,
}

/// OS から受け取ったキーの押下 1 回。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawKey<'a> {
    /// 仮想キーコード（`VK_*`）
    pub vk: u16,
    pub modifiers: WinModifiers,
    /// OS が現在の配列で返した文字（`ToUnicode` 相当）。Shift だけを適用し、Ctrl と Alt は外したもの。出ないキーは空。
    /// AltGr（Ctrl+Alt）で出る文字は、呼ぶ側が `ctrl` と `alt` を false にして渡す（配列ごとの扱いは OS が知っている）
    pub text: &'a str,
    pub is_repeat: bool,
    pub secure: bool,
}

/// 修飾キーそのもの（単独の押下は IME に送らない）
fn is_modifier_vk(code: u16) -> bool {
    matches!(
        code,
        vk::SHIFT
            | vk::CONTROL
            | vk::MENU
            | vk::CAPITAL
            | vk::LWIN
            | vk::RWIN
            | vk::LSHIFT
            | vk::RSHIFT
            | vk::LCONTROL
            | vk::RCONTROL
            | vk::LMENU
            | vk::RMENU
    )
}

/// VK だけで種類が決まるキー。文字のキーは `None`（`text` で読む）
fn kind_from_vk(code: u16) -> Option<KeyKind> {
    Some(match code {
        vk::SPACE => KeyKind::Space,
        vk::RETURN => KeyKind::Enter,
        vk::ESCAPE => KeyKind::Escape,
        vk::BACK => KeyKind::Backspace,
        vk::LEFT => KeyKind::Left,
        vk::RIGHT => KeyKind::Right,
        vk::UP => KeyKind::Up,
        vk::DOWN => KeyKind::Down,
        vk::OEM_ATTN => KeyKind::Eisu,
        vk::OEM_COPY | vk::KANA => KeyKind::Kana,
        // shell-core の Function は F1〜F20。F21〜F24 は持たない
        vk::F1..=vk::F20 => KeyKind::Function {
            number: u8::try_from(code - vk::F1 + 1).unwrap_or(0),
        },
        vk::F21..=vk::F24 | vk::TAB => KeyKind::Other,
        _ => return None,
    })
}

/// キー 1 回を `ShellEvent` にする。修飾キーだけの押下は `None`（shell-core に送らない）。
pub fn key_event(raw: &RawKey<'_>) -> Option<ShellEvent> {
    if is_modifier_vk(raw.vk) {
        return None;
    }
    let key = kind_from_vk(raw.vk).unwrap_or_else(|| {
        // 制御文字（Tab、Ctrl の組み合わせの結果など）は文字として送らない
        if raw.text.is_empty() || raw.text.chars().any(char::is_control) {
            KeyKind::Other
        } else {
            KeyKind::Character {
                text: raw.text.to_owned(),
            }
        }
    });
    Some(ShellEvent::Key(KeyInput {
        key,
        modifiers: Modifiers {
            shift: raw.modifiers.shift,
            control: raw.modifiers.ctrl,
            option: raw.modifiers.alt,
            command: raw.modifiers.win,
        },
        is_repeat: raw.is_repeat,
        secure: raw.secure,
        layout_text: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(code: u16, text: &str) -> RawKey<'_> {
        RawKey {
            vk: code,
            modifiers: WinModifiers::default(),
            text,
            is_repeat: false,
            secure: false,
        }
    }

    fn key_of(raw: &RawKey<'_>) -> KeyKind {
        match key_event(raw) {
            Some(ShellEvent::Key(input)) => input.key,
            other => panic!("Key でない: {other:?}"),
        }
    }

    fn character(text: &str) -> KeyKind {
        KeyKind::Character {
            text: text.to_owned(),
        }
    }

    #[test]
    fn fixed_keys_by_vk() {
        assert_eq!(key_of(&raw(vk::SPACE, " ")), KeyKind::Space);
        assert_eq!(key_of(&raw(vk::RETURN, "\r")), KeyKind::Enter);
        assert_eq!(key_of(&raw(vk::ESCAPE, "")), KeyKind::Escape);
        assert_eq!(key_of(&raw(vk::BACK, "\u{8}")), KeyKind::Backspace);
        assert_eq!(key_of(&raw(vk::LEFT, "")), KeyKind::Left);
        assert_eq!(key_of(&raw(vk::RIGHT, "")), KeyKind::Right);
        assert_eq!(key_of(&raw(vk::UP, "")), KeyKind::Up);
        assert_eq!(key_of(&raw(vk::DOWN, "")), KeyKind::Down);
        assert_eq!(key_of(&raw(vk::OEM_ATTN, "")), KeyKind::Eisu);
        assert_eq!(key_of(&raw(vk::OEM_COPY, "")), KeyKind::Kana);
        assert_eq!(key_of(&raw(vk::KANA, "")), KeyKind::Kana);
    }

    #[test]
    fn function_keys_f1_to_f20() {
        assert_eq!(key_of(&raw(vk::F1, "")), KeyKind::Function { number: 1 });
        assert_eq!(key_of(&raw(0x79, "")), KeyKind::Function { number: 10 });
        assert_eq!(key_of(&raw(vk::F20, "")), KeyKind::Function { number: 20 });
        assert_eq!(key_of(&raw(vk::F21, "")), KeyKind::Other); // F21
        assert_eq!(key_of(&raw(vk::F24, "")), KeyKind::Other);
    }

    #[test]
    fn modifier_only_presses_are_not_sent() {
        for code in [
            vk::SHIFT,
            vk::CONTROL,
            vk::MENU,
            vk::CAPITAL,
            vk::LWIN,
            vk::RWIN,
            vk::LSHIFT,
            vk::RSHIFT,
            vk::LCONTROL,
            vk::RCONTROL,
            vk::LMENU,
            vk::RMENU,
        ] {
            assert_eq!(key_event(&raw(code, "")), None, "VK {code:#x}");
        }
    }

    /// 同じ VK（OEM_1 = 0xBA）でも、配列が返す文字をそのまま使う。JIS は `:`、US は `;`、AZERTY は `$` など
    #[test]
    fn text_comes_from_os_not_vk_position() {
        const OEM_1: u16 = 0xBA;
        for text in [":", ";", "$", "ü", "ñ", "ö"] {
            assert_eq!(key_of(&raw(OEM_1, text)), character(text));
        }
        // 文字キー（A）も、Dvorak や AZERTY のように VK と文字が食い違ってよい
        assert_eq!(key_of(&raw(0x41, "q")), character("q"));
        assert_eq!(key_of(&raw(0x41, "Q")), character("Q"));
    }

    #[test]
    fn non_bmp_and_multi_char_text_kept() {
        assert_eq!(key_of(&raw(0xDF, "\u{1F600}")), character("\u{1F600}"));
        // ToUnicode が結合文字を 2 つ返すことがある
        assert_eq!(key_of(&raw(0xDF, "e\u{301}")), character("e\u{301}"));
    }

    #[test]
    fn no_text_or_control_text_is_other() {
        assert_eq!(key_of(&raw(0x41, "")), KeyKind::Other);
        assert_eq!(key_of(&raw(0x41, "\u{1}")), KeyKind::Other); // Ctrl+A の制御文字
        assert_eq!(key_of(&raw(vk::TAB, "\t")), KeyKind::Other);
        assert_eq!(key_of(&raw(0x70 + 20, "")), KeyKind::Other);
    }

    #[test]
    fn modifiers_map_to_shell_modifiers() {
        let mut key = raw(0x5A, "z");
        key.modifiers = WinModifiers {
            shift: true,
            ctrl: true,
            alt: true,
            win: true,
        };
        key.is_repeat = true;
        key.secure = true;
        let Some(ShellEvent::Key(input)) = key_event(&key) else {
            panic!("Key でない");
        };
        assert_eq!(
            input.modifiers,
            Modifiers {
                shift: true,
                control: true,
                option: true,
                command: true,
            }
        );
        assert!(input.is_repeat);
        assert!(input.secure);
        assert_eq!(input.layout_text, None);
        assert_eq!(input.key, character("z"));
    }

    #[test]
    fn chord_keeps_key_and_sets_only_pressed_modifiers() {
        let mut key = raw(0x5A, "z");
        key.modifiers.ctrl = true;
        let Some(ShellEvent::Key(input)) = key_event(&key) else {
            panic!("Key でない");
        };
        assert_eq!(
            input.modifiers,
            Modifiers {
                control: true,
                ..Modifiers::NONE
            }
        );
    }

    #[test]
    fn modifiers_apply_to_fixed_keys() {
        let mut key = raw(vk::LEFT, "");
        key.modifiers.shift = true;
        let Some(ShellEvent::Key(input)) = key_event(&key) else {
            panic!("Key でない");
        };
        assert_eq!(input.key, KeyKind::Left);
        assert!(input.modifiers.shift);
    }
}
