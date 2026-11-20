//! 取得（`ikari-data`）の起動、進み具合、取り消しの口（ヘッダの `ikari_settings_fetch_*`。ADR 0079 §3.6）。
//! 名前と引数の並びは Windows の `fetch_abi.rs` と同じ。失敗の値は `IKARI_SHELL_ERR_*`、文の入れ物は `IkariShellBuf`。
//!
//! **契約のスタブ**（#489 の契約コミット。中身は ADR 0079 §7 のレーン S）: `start` は起こせない失敗
//! （`IKARI_SHELL_ERR_FAILED` と `TOOL_MESSAGE`）を返し、ハンドルは作らない。ハンドルを受ける関数は、NULL なら
//! `IKARI_SHELL_ERR_NULL`、それ以外は `IKARI_SHELL_ERR_UNIMPLEMENTED`。どの入口も panic を外へ出さない。
//!
//! 判断の置き場所: Rust（起動するか、どの exe か、引数、文）。Swift はボタンと描画だけ。

use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::IkariShellBuf;
use crate::shell::{
    IKARI_SHELL_ERR_ARG, IKARI_SHELL_ERR_FAILED, IKARI_SHELL_ERR_NULL, IKARI_SHELL_ERR_PANIC,
    IKARI_SHELL_ERR_UNIMPLEMENTED,
};

/// `ikari_settings_fetch_start` の `kind`: 表のモデル（`a` が ID）
pub const IKARI_SETTINGS_FETCH_PRESET: i32 = 1;
/// 自由指定の URL（`a` が URL。`format` が必要。`sha256` と `name` は空でよい）
pub const IKARI_SETTINGS_FETCH_URL: i32 = 2;

/// 起こせないときの文（Windows の `text::fetch_error(Tool)` と同じ。共有の核へ移すときに一本化する）
const TOOL_MESSAGE: &str = "取得の部品を起動できませんでした";

/// 走っている取得の不透明な値（スタブでは作らない）
pub struct IkariSettingsFetch {
    _private: (),
}

/// `ikari_settings_fetch_poll` が書く様子
#[repr(C)]
pub struct IkariSettingsFetchStatus {
    /// 0 取得中、1 取得できた、2 失敗、3 取り消した
    pub phase: i32,
    /// 受けたバイト
    pub received: u64,
    /// 全体のバイト（`has_total` が 0 なら使わない）
    pub total: u64,
    pub has_total: u8,
}

fn guarded(f: impl FnOnce() -> i32) -> i32 {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(IKARI_SHELL_ERR_PANIC)
}

/// # Safety
/// `ptr` は null（`len` は 0）か、`len` バイトを読めること。
unsafe fn is_valid_str(ptr: *const u8, len: usize) -> bool {
    if ptr.is_null() {
        return len == 0;
    }
    // SAFETY: 呼び出しの約束
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(ptr, len) }).is_ok()
}

/// # Safety
/// `msg` は null か、書き込める `IkariShellBuf` を指すこと。
unsafe fn write_msg(msg: *mut IkariShellBuf, text: &str) {
    if !msg.is_null() {
        // SAFETY: 呼び出しの約束
        unsafe { msg.write(IkariShellBuf::from_vec(text.as_bytes().to_vec())) };
    }
}

/// 取得を始める。成功したら `*out` に値を書く（`ikari_settings_fetch_free` で必ず解放する）。
/// 失敗（`ERR_ARG`: 引数が使えない、`ERR_FAILED`: 起こせない）は `msg` に文を書く。
///
/// # Safety
/// 文字列は null（長さ 0）か読める UTF-8。`out` は書き込める。`msg` は null か書き込める。
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn ikari_settings_fetch_start(
    kind: i32,
    a: *const u8,
    a_len: usize,
    format: *const u8,
    format_len: usize,
    sha256: *const u8,
    sha256_len: usize,
    name: *const u8,
    name_len: usize,
    out: *mut *mut IkariSettingsFetch,
    msg: *mut IkariShellBuf,
) -> i32 {
    if out.is_null() {
        return IKARI_SHELL_ERR_NULL;
    }
    // SAFETY: 呼び出しの約束
    let strings_ok = unsafe {
        is_valid_str(a, a_len)
            && is_valid_str(format, format_len)
            && is_valid_str(sha256, sha256_len)
            && is_valid_str(name, name_len)
    };
    let kind_ok = match kind {
        IKARI_SETTINGS_FETCH_PRESET => a_len > 0,
        IKARI_SETTINGS_FETCH_URL => a_len > 0 && format_len > 0,
        _ => false,
    };
    if !strings_ok || !kind_ok {
        return IKARI_SHELL_ERR_ARG;
    }
    let msg_addr = msg as usize;
    guarded(move || {
        // SAFETY: `msg` は null か書き込める（呼び出しの約束）
        unsafe { write_msg(msg_addr as *mut IkariShellBuf, TOOL_MESSAGE) };
        IKARI_SHELL_ERR_FAILED
    })
}

/// 版の確認を始める（`ikari-data version-check`。ADR 0076 §6）。様子と文は `ikari_settings_fetch_poll`、
/// 解放は `ikari_settings_fetch_free`（取得と同じ）。
///
/// # Safety
/// `out` は書き込める。`msg` は null か書き込める。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_version_check_start(
    out: *mut *mut IkariSettingsFetch,
    msg: *mut IkariShellBuf,
) -> i32 {
    if out.is_null() {
        return IKARI_SHELL_ERR_NULL;
    }
    let msg_addr = msg as usize;
    guarded(move || {
        // SAFETY: `msg` は null か書き込める（呼び出しの約束）
        unsafe { write_msg(msg_addr as *mut IkariShellBuf, TOOL_MESSAGE) };
        IKARI_SHELL_ERR_FAILED
    })
}

/// 様子を読む。`status` に数、`text` に文（どちらも null なら書かない）。
///
/// # Safety
/// `fetch` は開いたもの（null なら `ERR_NULL`）。`status` と `text` は null か書き込める。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_poll(
    fetch: *const IkariSettingsFetch,
    status: *mut IkariSettingsFetchStatus,
    text: *mut IkariShellBuf,
) -> i32 {
    let _ = (status, text);
    stub_with_handle(fetch)
}

/// 取得が終わったときの sha256（`done` の行の値。64 桁の 16 進）を `out` に書く。まだ終わっていなければ `ERR_FAILED`。
///
/// # Safety
/// `fetch` は開いたもの（null なら `ERR_NULL`）。`out` は null か書き込める。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_sha256(
    fetch: *const IkariSettingsFetch,
    out: *mut IkariShellBuf,
) -> i32 {
    let _ = out;
    stub_with_handle(fetch)
}

/// 取り消す（子に `SIGKILL`）。終わっていれば何も変えない。
///
/// # Safety
/// `fetch` は開いたもの（null なら `ERR_NULL`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_cancel(fetch: *const IkariSettingsFetch) -> i32 {
    stub_with_handle(fetch)
}

fn stub_with_handle(fetch: *const IkariSettingsFetch) -> i32 {
    if fetch.is_null() {
        IKARI_SHELL_ERR_NULL
    } else {
        IKARI_SHELL_ERR_UNIMPLEMENTED
    }
}

/// 解放する（走っていれば取り消す）。null は何もしない。スタブはハンドルを作らないので、何も解放しない。
///
/// # Safety
/// `fetch` は null か、`ikari_settings_fetch_start` が返してまだ解放していないもの。同じ値を 2 回渡さない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_free(fetch: *mut IkariSettingsFetch) {
    let _ = fetch;
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use super::*;

    fn start(kind: i32, a: &str, format: &str) -> (i32, *mut IkariSettingsFetch, IkariShellBuf) {
        let mut out: *mut IkariSettingsFetch = ptr::null_mut();
        let mut msg = IkariShellBuf::EMPTY;
        let code = unsafe {
            ikari_settings_fetch_start(
                kind,
                a.as_ptr(),
                a.len(),
                format.as_ptr(),
                format.len(),
                ptr::null(),
                0,
                ptr::null(),
                0,
                &mut out,
                &mut msg,
            )
        };
        (code, out, msg)
    }

    #[test]
    fn start_stub_reports_tool_failure_without_handle() {
        let (code, out, msg) = start(IKARI_SETTINGS_FETCH_PRESET, "model", "");
        assert_eq!(code, IKARI_SHELL_ERR_FAILED);
        assert!(out.is_null());
        let text = String::from_utf8(unsafe { msg.into_vec() }).unwrap();
        assert_eq!(text, TOOL_MESSAGE);
    }

    #[test]
    fn start_rejects_bad_arguments() {
        assert_eq!(start(99, "x", "").0, IKARI_SHELL_ERR_ARG);
        assert_eq!(
            start(IKARI_SETTINGS_FETCH_PRESET, "", "").0,
            IKARI_SHELL_ERR_ARG
        );
        assert_eq!(
            start(IKARI_SETTINGS_FETCH_URL, "https://x", "").0,
            IKARI_SHELL_ERR_ARG
        );
        let invalid = [0xffu8];
        let mut out: *mut IkariSettingsFetch = ptr::null_mut();
        let code = unsafe {
            ikari_settings_fetch_start(
                IKARI_SETTINGS_FETCH_PRESET,
                invalid.as_ptr(),
                1,
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
                &mut out,
                ptr::null_mut(),
            )
        };
        assert_eq!(code, IKARI_SHELL_ERR_ARG);
    }

    #[test]
    fn null_arguments_do_not_crash() {
        let code = unsafe {
            ikari_settings_fetch_start(
                IKARI_SETTINGS_FETCH_PRESET,
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        assert_eq!(code, IKARI_SHELL_ERR_NULL);
        unsafe {
            assert_eq!(
                ikari_settings_version_check_start(ptr::null_mut(), ptr::null_mut()),
                IKARI_SHELL_ERR_NULL
            );
            assert_eq!(
                ikari_settings_fetch_poll(ptr::null(), ptr::null_mut(), ptr::null_mut()),
                IKARI_SHELL_ERR_NULL
            );
            assert_eq!(
                ikari_settings_fetch_sha256(ptr::null(), ptr::null_mut()),
                IKARI_SHELL_ERR_NULL
            );
            assert_eq!(
                ikari_settings_fetch_cancel(ptr::null()),
                IKARI_SHELL_ERR_NULL
            );
            ikari_settings_fetch_free(ptr::null_mut());
        }
    }
}
