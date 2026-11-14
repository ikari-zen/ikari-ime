//! ログオフ・シャットダウンの通知（`WM_QUERYENDSESSION`、`WM_ENDSESSION`）を、メッセージ専用ウィンドウで受ける
//! （`cfg(windows)`。Windows の CI か実機で確かめる）。
//!
//! コンソールのないビルド（`hide-console`）には、コンソールの制御イベント（`ctrl.rs`）が届かない。見えないウィンドウ
//! （`HWND_MESSAGE`。画面にも Alt+Tab にも出ない）を専用スレッドで作ってメッセージを回し、終了の通知が来たら、
//! `ctrl.rs` と同じ止める合図を送る。後始末（学習の保存、`exit=clean` の記録）は `main` の流れが行う（判断は Rust の側）。
//!
//! `unsafe` はこのファイルの Win32 の呼び出しだけ。

#![allow(unsafe_code)]

use std::io;
use std::ptr;
use std::sync::mpsc;
use std::thread;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_MESSAGE, MSG,
    RegisterClassW, TranslateMessage, WM_ENDSESSION, WM_QUERYENDSESSION, WNDCLASSW,
};

use crate::ctrl;

/// ウィンドウクラス名（UTF-16。終端つき）。`w!` を使わず、ASCII だけを広げる
const CLASS_NAME: [u16; 22] = ascii_to_utf16(b"IkariEngineEndSession\0");
/// `WM_QUERYENDSESSION` への返事。終わってよい
const ALLOW_END: LRESULT = 1;

const fn ascii_to_utf16<const N: usize>(bytes: &[u8; N]) -> [u16; N] {
    let mut out = [0u16; N];
    let mut i = 0;
    while i < N {
        out[i] = bytes[i] as u16;
        i += 1;
    }
    out
}

/// 専用スレッドでメッセージ専用ウィンドウを作り、メッセージを回す。作れなかったときはエラー（動きは続けてよい）。
pub fn spawn() -> io::Result<()> {
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
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
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
    match message {
        WM_QUERYENDSESSION => ALLOW_END,
        WM_ENDSESSION => {
            // wparam が 0 なら、終了は取りやめになった（止めない）
            if wparam != 0 {
                ctrl::signal_stop();
                ctrl::wait_finished();
            }
            0
        }
        // SAFETY: 受け取った引数をそのまま渡す
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
