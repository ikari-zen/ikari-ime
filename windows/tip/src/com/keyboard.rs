//! キーの押下（`ITfKeyEventSink` の `wParam`・`lParam`）→ `keys::RawKey`。
//!
//! 文字は VK の位置ではなく、`ToUnicodeEx` が現在の配列で返したもの（JIS、US、ほかのどの配列でも同じ）。
//!
//! - Ctrl と Alt を外した状態で `ToUnicodeEx` を呼ぶ（Shift と CapsLock は残す）。`RawKey.text` の約束。
//! - AltGr: Windows では AltGr は Ctrl+Alt として届く（右 Alt が左 Ctrl を伴う）。Ctrl と Alt が両方押されていて、
//!   押したままの状態で `ToUnicodeEx` が文字を返すときは、その文字を使い、`ctrl` と `alt` を false にする
//!   （`RawKey.text` の doc の約束。配列ごとの扱いは OS が知っている）。US や JIS のように AltGr の層がない配列では
//!   文字が出ないので、ふつうの Ctrl+Alt の組み合わせのままになる。
//! - `ToUnicodeEx` には「キーボードの状態を変えない」旗（`TOUNICODE_NO_STATE_CHANGE`。Windows 10 1607 以降）を付ける。
//!   付けないと、デッドキーのたまりを TIP が先に消費し、アプリが受け取る文字が変わる。
//!   デッドキー（戻り値が負）は文字なしにする（アプリに渡り、アプリ側でデッドキーとして合成される）。
//!
//! `secure`（パスワード欄）は S8（W4）で見分ける。いまは常に false。

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, GetKeyboardLayout, GetKeyboardState, ToUnicodeEx,
};

use crate::keys::{RawKey, WinModifiers, vk};

/// `ToUnicodeEx` の `wFlags`: キーボードの状態を変えない（bit 2。Windows 10 1607 以降）
const TOUNICODE_NO_STATE_CHANGE: u32 = 1 << 2;
/// `ToUnicodeEx` の出力の長さ（UTF-16。合字や結合文字で 2 文字以上になることがある）
const UNICODE_BUFFER_LEN: usize = 8;
/// キーボードの状態の配列の「押されている」ビット
const KEY_DOWN_BIT: u8 = 0x80;
/// `lParam` のスキャンコード（bit 16〜23）
const LPARAM_SCAN_SHIFT: u32 = 16;
const LPARAM_SCAN_MASK: isize = 0xFF;
/// `lParam` の直前の状態（bit 30。1 なら押したまま = 自動の繰り返し）
const LPARAM_PREVIOUS_STATE: isize = 1 << 30;
/// 今のスレッドの配列（`GetKeyboardLayout` の引数）
const CURRENT_THREAD: u32 = 0;

/// 修飾と文字を読み終えたキーの押下（`RawKey` は文字を借りるので、持ち主をここに置く）。
pub struct KeyPress {
    pub vk: u16,
    pub modifiers: WinModifiers,
    pub text: String,
    pub is_repeat: bool,
}

impl KeyPress {
    pub fn raw(&self) -> RawKey<'_> {
        RawKey {
            vk: self.vk,
            modifiers: self.modifiers,
            text: &self.text,
            is_repeat: self.is_repeat,
            // TODO(S8、W4): secure な入力欄の見分け
            secure: false,
        }
    }
}

fn is_down(code: u16) -> bool {
    // SAFETY: 引数は VK の値。戻り値の最上位ビットが「押されている」
    unsafe { GetKeyState(i32::from(code)) < 0 }
}

fn modifiers_now() -> WinModifiers {
    WinModifiers {
        shift: is_down(vk::SHIFT),
        ctrl: is_down(vk::CONTROL),
        alt: is_down(vk::MENU),
        win: is_down(vk::LWIN) || is_down(vk::RWIN),
    }
}

/// `state` のとおりに押したときの文字。出なければ空（デッドキーも空）。
fn to_unicode(code: u16, scan: u32, state: &[u8; 256]) -> String {
    let mut buffer = [0u16; UNICODE_BUFFER_LEN];
    // SAFETY: `state` は 256 バイトのキーボードの状態、`buffer` は書ける領域。配列は今のスレッドのもの
    let written = unsafe {
        ToUnicodeEx(
            u32::from(code),
            scan,
            state,
            &mut buffer,
            TOUNICODE_NO_STATE_CHANGE,
            Some(GetKeyboardLayout(CURRENT_THREAD)),
        )
    };
    match usize::try_from(written) {
        Ok(len) if len > 0 => String::from_utf16_lossy(&buffer[..len.min(UNICODE_BUFFER_LEN)]),
        // 0 は文字なし、負はデッドキー
        _ => String::new(),
    }
}

fn clear(state: &mut [u8; 256], codes: &[u16]) {
    for &code in codes {
        state[usize::from(code)] &= !KEY_DOWN_BIT;
    }
}

/// キーの押下を読む。VK が 16 ビットに収まらなければ `None`（OS は送らない）。
pub fn read(wparam: WPARAM, lparam: LPARAM) -> Option<KeyPress> {
    let code = u16::try_from(wparam.0).ok()?;
    let scan = u32::try_from((lparam.0 >> LPARAM_SCAN_SHIFT) & LPARAM_SCAN_MASK).unwrap_or(0);
    let is_repeat = lparam.0 & LPARAM_PREVIOUS_STATE != 0;
    let mut modifiers = modifiers_now();

    let mut state = [0u8; 256];
    // SAFETY: `state` は 256 バイトの書ける領域
    if unsafe { GetKeyboardState(&mut state) }.is_err() {
        return Some(KeyPress {
            vk: code,
            modifiers,
            text: String::new(),
            is_repeat,
        });
    }

    let altgr_text = if modifiers.ctrl && modifiers.alt {
        let text = to_unicode(code, scan, &state);
        (!text.is_empty() && !text.chars().any(char::is_control)).then_some(text)
    } else {
        None
    };
    let text = match altgr_text {
        Some(text) => {
            modifiers.ctrl = false;
            modifiers.alt = false;
            text
        }
        None => {
            clear(
                &mut state,
                &[
                    vk::CONTROL,
                    vk::LCONTROL,
                    vk::RCONTROL,
                    vk::MENU,
                    vk::LMENU,
                    vk::RMENU,
                ],
            );
            to_unicode(code, scan, &state)
        }
    };
    Some(KeyPress {
        vk: code,
        modifiers,
        text,
        is_repeat,
    })
}
