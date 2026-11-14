//! コンソールの制御イベント（Ctrl+C、Ctrl+Break、コンソールを閉じる、ログオフ、シャットダウン）を受けて、
//! 止める合図に変える（`cfg(windows)`。Windows の CI か実機で確かめる）。
//!
//! コンソールのないビルド（`hide-console`）にはこの通知が届かない。ログオフとシャットダウンは、
//! 非表示の最上位ウィンドウが受ける（`endsession.rs`。#372）。止める合図と「後始末が終わった印」はここで共有する。
//! 受け取れずにプロセスが強制的に終わっても、学習は 5 秒ごとに保存されていて、`engine.lock` は OS が解放する。
//!
//! `unsafe` はこのファイルの Win32 の呼び出しだけ。

#![allow(unsafe_code)]

use std::io;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{FALSE, TRUE};
use windows_sys::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
    SetConsoleCtrlHandler,
};

use crate::run::StopHandle;

/// コンソールを閉じる、ログオフ、シャットダウンのとき、OS は返事を待ってプロセスを終える（待ちは長くない）。
/// 後始末が終わるのをこの時間まで待ってから返す
const CLEANUP_WAIT: Duration = Duration::from_secs(4);
const CLEANUP_POLL: Duration = Duration::from_millis(50);

static STOP: OnceLock<StopHandle> = OnceLock::new();
static FINISHED: AtomicBool = AtomicBool::new(false);

/// 制御イベントの受け取りを入れる。2 回目以降は、最初の `stop` のまま（プロセスで 1 回だけ呼ぶ）。
pub fn install(stop: StopHandle) -> io::Result<()> {
    let _ = STOP.set(stop);
    // SAFETY: `handler` はこのプロセスの間ずっと有効な関数で、シグネチャは `PHANDLER_ROUTINE` と同じ
    let ok = unsafe { SetConsoleCtrlHandler(Some(handler), TRUE) };
    if ok == FALSE {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// 後始末が終わった印。待っている制御イベントの受け取りを返す
pub fn mark_finished() {
    FINISHED.store(true, Ordering::SeqCst);
}

/// OS が別のスレッドで呼ぶ。止める合図を送るだけ（判断も後始末も `main` の流れ）
unsafe extern "system" fn handler(ctrl_type: u32) -> i32 {
    match ctrl_type {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => {
            signal_stop();
            TRUE
        }
        CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => {
            signal_stop();
            wait_finished();
            TRUE
        }
        _ => FALSE,
    }
}

/// 後始末が終わる（`mark_finished`）まで、`CLEANUP_WAIT` を上限に待つ
pub(crate) fn wait_finished() {
    let started = Instant::now();
    while !FINISHED.load(Ordering::SeqCst) && started.elapsed() < CLEANUP_WAIT {
        thread::sleep(CLEANUP_POLL);
    }
}

pub(crate) fn signal_stop() {
    if let Some(stop) = STOP.get() {
        stop.stop();
    }
}
