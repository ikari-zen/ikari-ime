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

/// compartment の変更の通知 1 件（診断用。#371）。種類と結果は固定の文、値は compartment の数値
/// （入力の内容ではない）。
pub fn compartment(kind: &'static str, value: Option<i32>, outcome: &'static str) {
    match value {
        Some(value) => write(&format!(
            "{PREFIX}OnChange {kind} value={value} {outcome}\n"
        )),
        None => write(&format!("{PREFIX}OnChange {kind} value=none {outcome}\n")),
    }
}

/// HRESULT を出す。
pub fn hresult(message: &'static str, code: HRESULT) {
    // HRESULT は符号つきの 32 ビット。16 進で読むのが通例なので、ビット列のまま出す
    write(&format!("{PREFIX}{message} (hr=0x{:08X})\n", code.0 as u32));
}

/// 前回出した入力スコープの値（同じ並びを打鍵のたびに出さないため）
static LAST_SCOPES: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());

/// 読んだ入力スコープの値（数値だけ。欄の内容は含まない）を、前回と違うときだけ 1 行出す。secure の判定の調査用。
pub fn input_scopes(scopes: &[i32]) {
    let Ok(mut last) = LAST_SCOPES.lock() else {
        return;
    };
    if last.as_slice() == scopes {
        return;
    }
    *last = scopes.to_vec();
    write(&format!("{PREFIX}input scopes={scopes:?}\n"));
}
