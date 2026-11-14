//! ログオフ・シャットダウンの通知（`WM_QUERYENDSESSION`、`WM_ENDSESSION`）を、非表示の最上位ウィンドウで受ける
//! （`cfg(windows)`。Windows の CI か実機で確かめる）。
//!
//! コンソールのないビルド（`hide-console`）には、コンソールの制御イベント（`ctrl.rs`）が届かない。見えないウィンドウを
//! 専用スレッドで作ってメッセージを回し、終了の通知が来たら、`ctrl.rs` と同じ止める合図を送る。
//! ウィンドウは、親も持ち主もない通常の最上位（`WS_OVERLAPPED`、`WS_VISIBLE` なし）。表示されず、タスクバーにも出ない。
//! メッセージ専用ウィンドウ（`HWND_MESSAGE`）は、ブロードキャストの `WM_QUERYENDSESSION` / `WM_ENDSESSION` を受けない
//! ので使わない（#372。#385 の実機で `exit=clean` が書かれなかった）。
//! 後始末（学習の保存、`exit=clean` の記録）は `main` の流れが行う。反応の判断は `session_end.rs`（判断は Rust の側）。
//!
//! `unsafe` はこのファイルの Win32 の呼び出しだけ。

#![allow(unsafe_code)]

use std::io;
use std::ptr;
use std::sync::mpsc;
use std::sync::{OnceLock, Weak};
use std::thread;

use ikari_engine::Engine;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, RegisterClassW,
    TranslateMessage, WM_ENDSESSION, WM_QUERYENDSESSION, WNDCLASSW, WS_OVERLAPPED,
};

use crate::{ctrl, session_end};

// 判断の側（`session_end`）の定数が、OS の値と同じであることの確認（ずれたらビルドが止まる）
const _: () = {
    use windows_sys::Win32::UI::WindowsAndMessaging as wm;
    assert!(session_end::WM_QUERYENDSESSION == WM_QUERYENDSESSION);
    assert!(session_end::WM_ENDSESSION == WM_ENDSESSION);
    assert!(session_end::ENDSESSION_CLOSEAPP == wm::ENDSESSION_CLOSEAPP);
    assert!(session_end::ENDSESSION_CRITICAL == wm::ENDSESSION_CRITICAL);
    assert!(session_end::ENDSESSION_LOGOFF == wm::ENDSESSION_LOGOFF);
};

/// 診断の行を書く先（`Engine` を生かし続けないよう、弱い参照）。ウィンドウの手続きから使う
static ENGINE: OnceLock<Weak<Engine>> = OnceLock::new();

/// ウィンドウクラス名（UTF-16。終端つき）。`w!` を使わず、ASCII だけを広げる
const CLASS_NAME: [u16; 22] = ascii_to_utf16(b"IkariEngineEndSession\0");

const fn ascii_to_utf16<const N: usize>(bytes: &[u8; N]) -> [u16; N] {
    let mut out = [0u16; N];
    let mut i = 0;
    while i < N {
        out[i] = bytes[i] as u16;
        i += 1;
    }
    out
}

/// `engine.log` に 1 行書く（`host <note>`）。`Engine` がすでに落ちていれば何もしない
fn record(note: &str) {
    if let Some(engine) = ENGINE.get().and_then(Weak::upgrade) {
        engine.record_host_event(note);
    }
}

/// 専用スレッドで非表示の最上位ウィンドウを作り、メッセージを回す。作れなかったときはエラー（動きは続けてよい）。
/// 作成の成否は `engine.log` に書く（`window=created` / `window=failed os_error=N`）。
pub fn spawn(engine: Weak<Engine>) -> io::Result<()> {
    let _ = ENGINE.set(engine);
    let result = spawn_thread();
    match &result {
        Ok(()) => record("window=created kind=toplevel"),
        Err(e) => record(&format!(
            "window=failed os_error={}",
            e.raw_os_error().unwrap_or(0)
        )),
    }
    result
}

fn spawn_thread() -> io::Result<()> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("ikari-endsession".to_owned())
        .spawn(move || {
            let created = create_window();
            let ok = created.is_ok();
            let _ = tx.send(created);
            if ok {
                message_loop();
            }
        })?;
    rx.recv()
        .unwrap_or_else(|_| Err(io::Error::other("the window thread ended")))
}

fn create_window() -> io::Result<()> {
    // SAFETY: 引数は有効なポインタか null。クラス名はこのプロセスの間ずっと有効（const の配列）
    unsafe {
        let instance = GetModuleHandleW(ptr::null());
        let class = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: ptr::null_mut(),
            hCursor: ptr::null_mut(),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: CLASS_NAME.as_ptr(),
        };
        if RegisterClassW(&class) == 0 {
            return Err(io::Error::last_os_error());
        }
        let hwnd = CreateWindowExW(
            0,
            CLASS_NAME.as_ptr(),
            CLASS_NAME.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        if hwnd.is_null() {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn message_loop() {
    // SAFETY: `msg` はこのスレッドのスタックにある。GetMessageW は 0 か -1 で戻る（終了、またはエラー）
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// OS が（このスレッドで）呼ぶ。止める合図を送り、後始末が終わるまで待ってから返す
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let Some(reaction) = session_end::react(message, wparam, lparam) else {
        // SAFETY: 受け取った引数をそのまま渡す
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    };
    record(&reaction.note);
    if reaction.stop {
        ctrl::signal_stop();
        ctrl::wait_finished();
    }
    reaction.result as LRESULT
}
