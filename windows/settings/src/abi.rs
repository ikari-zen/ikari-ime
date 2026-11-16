//! 画面（C#。`LibraryImport`）が呼ぶ C の ABI。ヘッダは `include/ikari_settings_core.h`（同じに保つ）。
//!
//! - 受け渡しは、数と、バイト列（Rust が確保し、`ikari_settings_core_buf_free` で返してもらう）だけ。Rust の型を境界に
//!   出さない（`shell-ffi`、候補窓の核と同じ作法）。
//! - どの入口も panic を外へ出さない（`catch_unwind`）。
//! - ここは版、権限、設定の一覧（`shell-core` の `settings::specs_tsv`）、バイト列の返却。
//!   設定の口へのつながりは `session_abi`。

use std::panic::{UnwindSafe, catch_unwind};
use std::ptr;

use ikari_shell_core::settings::specs_tsv;

/// ABI の版。互換を壊す変更で上げる。
pub const ABI_VERSION: u32 = 1;

/// 成功
pub const IKARI_SETTINGS_CORE_OK: i32 = 0;
/// 書き込む先が null
pub const IKARI_SETTINGS_CORE_ERR_NULL: i32 = -1;
/// 中で panic した（核の誤り）
pub const IKARI_SETTINGS_CORE_ERR_PANIC: i32 = -2;
/// 高い権限で動いている、または権限を確かめられない（ADR 0067 §4.3。画面は何もせずに終わる）
pub const IKARI_SETTINGS_CORE_ERR_ELEVATED: i32 = -3;

/// Rust が確保したバイト列。`ikari_settings_core_buf_free` で 1 回だけ返す。
#[repr(C)]
pub struct IkariSettingsBuf {
    pub ptr: *mut u8,
    pub len: usize,
}

impl IkariSettingsBuf {
    /// 空（null）のバイト列
    pub const EMPTY: IkariSettingsBuf = IkariSettingsBuf {
        ptr: ptr::null_mut(),
        len: 0,
    };

    pub(crate) fn from_vec(v: Vec<u8>) -> IkariSettingsBuf {
        let boxed = v.into_boxed_slice();
        let len = boxed.len();
        IkariSettingsBuf {
            ptr: Box::into_raw(boxed) as *mut u8,
            len,
        }
    }
}

pub(crate) fn guarded(f: impl FnOnce() -> i32 + UnwindSafe) -> i32 {
    catch_unwind(f).unwrap_or(IKARI_SETTINGS_CORE_ERR_PANIC)
}

#[unsafe(no_mangle)]
pub extern "C" fn ikari_settings_core_abi_version() -> u32 {
    ABI_VERSION
}

/// 画面を続けてよいか（高い権限では動かない）。Windows 以外は常に `IKARI_SETTINGS_CORE_OK`（確かめる手段がないため）。
#[unsafe(no_mangle)]
pub extern "C" fn ikari_settings_core_check_self() -> i32 {
    guarded(|| {
        if self_allowed() {
            IKARI_SETTINGS_CORE_OK
        } else {
            IKARI_SETTINGS_CORE_ERR_ELEVATED
        }
    })
}

#[cfg(windows)]
fn self_allowed() -> bool {
    use ikari_pipe::elevation::check_self;
    use ikari_pipe::os::windows::current_integrity_rid;
    check_self(current_integrity_rid()).is_ok()
}

#[cfg(not(windows))]
fn self_allowed() -> bool {
    true
}

/// 画面に出す設定の一覧（UTF-8 の TSV。1 行 1 件。列: キー、種類、節、画面、既定値（なしは空）、反映。並びは画面での順）。
/// `out` に書き、`ikari_settings_core_buf_free` で返してもらう。
///
/// # Safety
/// `out` は null か、書き込める `IkariSettingsBuf` を指すこと。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_core_specs(out: *mut IkariSettingsBuf) -> i32 {
    if out.is_null() {
        return IKARI_SETTINGS_CORE_ERR_NULL;
    }
    let out_addr = out as usize;
    guarded(move || {
        let buf = IkariSettingsBuf::from_vec(specs_tsv().into_bytes());
        // SAFETY: `out` は null でなく、書き込める `IkariSettingsBuf` を指す（呼び出しの約束）。
        unsafe { (out_addr as *mut IkariSettingsBuf).write(buf) };
        IKARI_SETTINGS_CORE_OK
    })
}

/// `ikari_settings_core_specs` などが返したバイト列を返す。`ptr` が null なら何もしない。
///
/// # Safety
/// `buf` は核が返したものを 1 回だけ渡すこと。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_core_buf_free(buf: IkariSettingsBuf) {
    if buf.ptr.is_null() {
        return;
    }
    // SAFETY: `buf` は `IkariSettingsBuf::from_vec` が `Box<[u8]>` から作ったもので、まだ返されていない（呼び出しの約束）。
    drop(unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(buf.ptr, buf.len)) });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_come_back_as_the_shared_tsv_and_can_be_freed() {
        let mut out = IkariSettingsBuf::EMPTY;
        // SAFETY: `out` は書き込める
        assert_eq!(
            unsafe { ikari_settings_core_specs(&mut out) },
            IKARI_SETTINGS_CORE_OK
        );
        assert!(!out.ptr.is_null());
        // SAFETY: 核が返した ptr と len
        let bytes = unsafe { std::slice::from_raw_parts(out.ptr, out.len) };
        let text = std::str::from_utf8(bytes).unwrap();
        assert_eq!(text, specs_tsv());
        // どの行も 6 列（画面が分けて描く）
        assert!(text.lines().all(|line| line.split('\t').count() == 6));
        // SAFETY: 1 回だけ返す
        unsafe { ikari_settings_core_buf_free(out) };
    }

    #[test]
    fn null_out_is_refused_and_empty_free_is_a_no_op() {
        // SAFETY: null を渡したときの約束を確かめる
        assert_eq!(
            unsafe { ikari_settings_core_specs(ptr::null_mut()) },
            IKARI_SETTINGS_CORE_ERR_NULL
        );
        // SAFETY: null の ptr は何もしない
        unsafe { ikari_settings_core_buf_free(IkariSettingsBuf::EMPTY) };
    }

    #[test]
    fn version_and_self_check() {
        assert_eq!(ikari_settings_core_abi_version(), ABI_VERSION);
        assert_eq!(ikari_settings_core_check_self(), IKARI_SETTINGS_CORE_OK);
    }
}
