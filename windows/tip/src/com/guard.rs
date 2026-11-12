//! COM の入口を `catch_unwind` で包む（ADR 0041 §4.2。アプリのプロセスを落とさない）。
//!
//! COM の入口は `extern "system"` の関数（`#[implement]` の展開）から呼ばれる。panic が OS の枠をまたぐと
//! プロセスが落ちるので、入口の中で止めて、「その回は素通し」の値を返す。

use std::panic::{AssertUnwindSafe, catch_unwind};

use super::log;

/// `f` を呼び、panic したら `fallback` を返す（`where_` は出力に使う固定の文）。
///
/// `AssertUnwindSafe`: panic のあとの状態（`Shell` の中など）は信用しない。呼ぶ側が、panic を知ったら
/// `Shell` を捨てる（`service` の `poisoned`）。ここでは値を返すことだけを保証する。
pub fn guard<T>(where_: &'static str, fallback: T, f: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(_) => {
            log::event(where_);
            fallback
        }
    }
}

/// `guard` の、`Result` を返す入口用。panic したら `E_UNEXPECTED`（呼び出し側には「失敗」として見える）。
pub fn guard_result<T>(
    where_: &'static str,
    f: impl FnOnce() -> windows_core::Result<T>,
) -> windows_core::Result<T> {
    guard(
        where_,
        Err(windows::Win32::Foundation::E_UNEXPECTED.into()),
        f,
    )
}
