//! デバッグの出力（`OutputDebugStringW`。DebugView などで読む）。
//!
//! **入力された文字列を出さない**（プライバシーの方針）。受けるのは `&'static str`（コードに書いた固定の文）と
//! HRESULT の値だけにして、入力の内容や周辺テキストを渡せない形にしている。

use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows_core::{HRESULT, HSTRING};

/// 出力の先頭（DebugView で絞り込むため）
const PREFIX: &str = "ikari-tip: ";

fn write(line: &str) {
    let text = HSTRING::from(line);
    // SAFETY: `text` は NUL で終わる UTF-16（HSTRING が保証する）で、呼び出しの間生きている。
    unsafe { OutputDebugStringW(&text) };
}

/// 固定の文を 1 行出す。
pub fn event(message: &'static str) {
    write(&format!("{PREFIX}{message}\n"));
}

/// 失敗を、場所の文と HRESULT の値で出す（誤りの文面は OS の言語に依存するので出さない）。
pub fn error(message: &'static str, error: &windows_core::Error) {
    hresult(message, error.code());
}

/// HRESULT を出す。
pub fn hresult(message: &'static str, code: HRESULT) {
    // HRESULT は符号つきの 32 ビット。16 進で読むのが通例なので、ビット列のまま出す
    write(&format!("{PREFIX}{message} (hr=0x{:08X})\n", code.0 as u32));
}
