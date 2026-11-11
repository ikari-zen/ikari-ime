//! Windows の仮想キー（VK）と修飾、OS が返した文字 → `ShellEvent`（純粋）。
//!
//! 文字は VK の位置ではなく、OS が現在の配列で返した文字（`ToUnicode` 相当）から作る。JIS、US、ほかのどの配列でも同じ。

use ikari_shell_core::ShellEvent;

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
    /// OS が現在の配列で返した文字。Shift だけを適用し、Ctrl と Alt は外したもの。出ないキーは空
    pub text: &'a str,
    pub is_repeat: bool,
    pub secure: bool,
}

/// キー 1 回を `ShellEvent` にする。修飾キーだけの押下は `None`（shell-core に送らない）。
pub fn key_event(_raw: &RawKey<'_>) -> Option<ShellEvent> {
    todo!("W2-a")
}
