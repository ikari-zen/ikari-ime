//! ikari-shell-ffi: 殻が使う C ABI（ヘッダは `include/ikari_shell.h`）。
//!
//! - このファイル: 共通の型（`IkariStr`、`IkariShellBuf`）と解放（`ikari_shell_buf_free`）
//! - `shell`: 殻の 1 本の口（`ikari_shell_*`。ADR 0041 §4.4）
//! - `settings`: 設定アプリ用（`ikari_settings_validate`・`rewrite`。ADR 0041 §2.3）
//! - `settings_app`: 設定アプリの Model 用（`ikari_settings_app`。入出力は `ikari.settings.v1`）
//! - `panel`: 候補ウィンドウの配置（`ikari_panel_*`）
//! - `convert`: `shell-core` の型と proto（`ikari.shell.v1`）の写し
//!
//! すべての入口で `catch_unwind` を使い、panic を FFI の外へ出さない。
//! ヘッダと一致させること（シグネチャの変更は契約の変更）。

pub mod convert;
mod panel;
mod settings;
mod settings_app;
mod shell;

pub use settings::{ikari_settings_rewrite, ikari_settings_validate};
pub use settings_app::ikari_settings_app;
pub use shell::*;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::slice;

/// 借用の文字列。
#[repr(C)]
pub struct IkariStr {
    pub ptr: *const u8,
    pub len: usize,
}

/// Rust が確保したバイト列。`ikari_shell_buf_free` で解放する。
#[repr(C)]
pub struct IkariShellBuf {
    pub ptr: *mut u8,
    pub len: usize,
}

impl IkariStr {
    pub const EMPTY: IkariStr = IkariStr {
        ptr: ptr::null(),
        len: 0,
    };

    /// `s` を借用する（`s` より長く使わない）。
    pub fn of(s: &str) -> IkariStr {
        Self::of_bytes(s.as_bytes())
    }

    /// `b` を借用する（`b` より長く使わない）。
    pub fn of_bytes(b: &[u8]) -> IkariStr {
        IkariStr {
            ptr: b.as_ptr(),
            len: b.len(),
        }
    }
}

impl IkariShellBuf {
    pub const EMPTY: IkariShellBuf = IkariShellBuf {
        ptr: ptr::null_mut(),
        len: 0,
    };

    pub(crate) fn from_vec(v: Vec<u8>) -> IkariShellBuf {
        let boxed = v.into_boxed_slice();
        let len = boxed.len();
        IkariShellBuf {
            ptr: Box::into_raw(boxed) as *mut u8,
            len,
        }
    }

    /// Rust が確保したバイト列を引き取って `Vec` にする（解放の責任も移る）。`ptr` が NULL なら空。
    ///
    /// # Safety
    /// `self` は Rust が返したもの（`ikari_shell_buf_copy` などで作ったもの）で、まだ解放していないこと。
    /// 引き取ったあと、同じ `ptr` を `ikari_shell_buf_free` に渡さないこと。
    pub unsafe fn into_vec(self) -> Vec<u8> {
        if self.ptr.is_null() {
            return Vec::new();
        }
        // SAFETY: `from_vec` が `Box<[u8]>` から作ったもの（呼び出し側の約束）
        unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(self.ptr, self.len)) }.into_vec()
    }
}

/// `(ptr, len)` を借用する。NULL か len 0 は空。
///
/// # Safety
/// `ptr` が非 NULL なら、`len` バイトが読めること。
pub(crate) unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(ptr, len) }
    }
}

/// panic を FFI の外へ出さない。panic したら `fallback` を返す。
pub(crate) fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

pub(crate) fn write_error(err: *mut IkariShellBuf, message: &str) {
    if !err.is_null() {
        // SAFETY: 呼び出し側が書き込める `IkariShellBuf` を渡す（契約）。
        unsafe { err.write(IkariShellBuf::from_vec(message.as_bytes().to_vec())) };
    }
}

/// # Safety
/// `buf` は Rust が返したものを1回だけ渡すこと。`ptr == NULL` は何もしない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_shell_buf_free(buf: IkariShellBuf) {
    if buf.ptr.is_null() {
        return;
    }
    guard((), || {
        drop(unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(buf.ptr, buf.len)) });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_declares_every_exported_function() {
        let header = include_str!("../include/ikari_shell.h");
        for name in [
            "ikari_shell_buf_free",
            "ikari_shell_new",
            "ikari_shell_dispatch",
            "ikari_shell_engine_down",
            "ikari_shell_note_exit",
            "ikari_shell_free",
            "ikari_shell_buf_copy",
            "ikari_settings_validate",
            "ikari_settings_rewrite",
            "ikari_settings_app",
            "ikari_panel_resolve_anchor",
            "ikari_panel_layout",
        ] {
            assert!(
                header.contains(name),
                "{name} is missing from ikari_shell.h"
            );
        }
    }

    #[test]
    fn shell_constants_match_the_header() {
        let header = include_str!("../include/ikari_shell.h");
        for (name, value) in [
            ("IKARI_SHELL_OK", IKARI_SHELL_OK),
            ("IKARI_SHELL_ERR_NULL", IKARI_SHELL_ERR_NULL),
            (
                "IKARI_SHELL_ERR_INVALID_HANDLE",
                IKARI_SHELL_ERR_INVALID_HANDLE,
            ),
            ("IKARI_SHELL_ERR_DECODE", IKARI_SHELL_ERR_DECODE),
            ("IKARI_SHELL_ERR_BUSY", IKARI_SHELL_ERR_BUSY),
            ("IKARI_SHELL_ERR_PANIC", IKARI_SHELL_ERR_PANIC),
            (
                "IKARI_SHELL_ERR_UNIMPLEMENTED",
                IKARI_SHELL_ERR_UNIMPLEMENTED,
            ),
            ("IKARI_SHELL_TRANSPORT_OK", IKARI_SHELL_TRANSPORT_OK),
            (
                "IKARI_SHELL_TRANSPORT_UNAVAILABLE",
                IKARI_SHELL_TRANSPORT_UNAVAILABLE,
            ),
            (
                "IKARI_SHELL_TRANSPORT_TIMEOUT",
                IKARI_SHELL_TRANSPORT_TIMEOUT,
            ),
            (
                "IKARI_SHELL_DOWN_INTERRUPTED",
                i32::from(IKARI_SHELL_DOWN_INTERRUPTED),
            ),
            (
                "IKARI_SHELL_DOWN_INVALIDATED",
                i32::from(IKARI_SHELL_DOWN_INVALIDATED),
            ),
        ] {
            let line = format!("{name} = {value},");
            assert!(
                header.contains(&line),
                "{line} is missing from ikari_shell.h"
            );
        }
    }
}
