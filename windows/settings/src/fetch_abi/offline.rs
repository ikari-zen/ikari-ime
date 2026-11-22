//! 取得の C の ABI のオフライン版の中身（feature `online` なし。ADR 0080 §7.1）。何も起こさない。取得の核
//! （`ikari_shell_core::settings::fetch`）も、`ikari-data.exe` の起こし方（`fetch_os`）も持たない。
//!
//! ハンドルは作らないので、`poll`・`sha256`・`cancel` に null でない値が来たら、それは知らない値（`ERR_FAILED`）。
//! `free` は何もしない（解放するものが無い）。

use ikari_shell_core::edition::FETCH_UNAVAILABLE_IN_OFFLINE;

use super::IkariSettingsFetchStatus;
use crate::abi::{IKARI_SETTINGS_CORE_ERR_NULL, IkariSettingsBuf};
use crate::session_abi::IKARI_SETTINGS_CORE_ERR_FAILED;

/// 取得のハンドルの型（オフライン版では作らない）
pub struct SettingsFetch {
    _never: (),
}

/// 始める口の共通: `out` は書かず、`msg` に文を書いて `ERR_FAILED`
fn refuse(out: *mut *mut SettingsFetch, msg: *mut IkariSettingsBuf) -> i32 {
    if out.is_null() {
        return IKARI_SETTINGS_CORE_ERR_NULL;
    }
    if !msg.is_null() {
        let text = FETCH_UNAVAILABLE_IN_OFFLINE.as_bytes().to_vec();
        // SAFETY: `msg` は null か書き込める（呼び出しの約束）
        unsafe { msg.write(IkariSettingsBuf::from_vec(text)) };
    }
    IKARI_SETTINGS_CORE_ERR_FAILED
}

fn unknown_handle(fetch: *const SettingsFetch) -> i32 {
    if fetch.is_null() {
        IKARI_SETTINGS_CORE_ERR_NULL
    } else {
        IKARI_SETTINGS_CORE_ERR_FAILED
    }
}

/// オフライン版: 起こさない（`ERR_FAILED` と文）。
///
/// # Safety
/// `out` は null か書き込める。`msg` は null か書き込める（文字列の引数は読まない）。
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn ikari_settings_fetch_start(
    _kind: i32,
    _a: *const u8,
    _a_len: usize,
    _format: *const u8,
    _format_len: usize,
    _sha256: *const u8,
    _sha256_len: usize,
    _name: *const u8,
    _name_len: usize,
    out: *mut *mut SettingsFetch,
    msg: *mut IkariSettingsBuf,
) -> i32 {
    refuse(out, msg)
}

/// オフライン版: 起こさない（`ERR_FAILED` と文）。
///
/// # Safety
/// `out` は null か書き込める。`msg` は null か書き込める。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_version_check_start(
    out: *mut *mut SettingsFetch,
    msg: *mut IkariSettingsBuf,
) -> i32 {
    refuse(out, msg)
}

/// オフライン版: ハンドルは無い（null は `ERR_NULL`、ほかは知らない値で `ERR_FAILED`。何も書かない）。
///
/// # Safety
/// 引数を読み書きしない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_poll(
    fetch: *const SettingsFetch,
    _status: *mut IkariSettingsFetchStatus,
    _text: *mut IkariSettingsBuf,
) -> i32 {
    unknown_handle(fetch)
}

/// オフライン版: ハンドルは無い（`ikari_settings_fetch_poll` と同じ）。
///
/// # Safety
/// 引数を読み書きしない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_sha256(
    fetch: *const SettingsFetch,
    _out: *mut IkariSettingsBuf,
) -> i32 {
    unknown_handle(fetch)
}

/// オフライン版: ハンドルは無い（`ikari_settings_fetch_poll` と同じ）。
///
/// # Safety
/// 引数を読み書きしない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_cancel(fetch: *const SettingsFetch) -> i32 {
    unknown_handle(fetch)
}

/// オフライン版: 解放するものが無い（何もしない）。
///
/// # Safety
/// 引数を読み書きしない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_free(_fetch: *mut SettingsFetch) {}

#[cfg(test)]
mod tests {
    use std::ptr;

    use super::super::IKARI_SETTINGS_FETCH_PRESET;
    use super::*;

    fn take_text(buf: IkariSettingsBuf) -> String {
        // SAFETY: 核が返したものを読む
        let text = unsafe { std::slice::from_raw_parts(buf.ptr, buf.len) }.to_vec();
        // SAFETY: 核が返したものを 1 回だけ返す
        unsafe { crate::abi::ikari_settings_core_buf_free(buf) };
        String::from_utf8(text).unwrap()
    }

    #[test]
    fn starting_is_refused_with_a_text_and_no_handle() {
        let a = "zenz-small";
        let mut out = ptr::null_mut();
        let mut msg = IkariSettingsBuf::EMPTY;
        // SAFETY: 文字列は生きていて、`out` と `msg` は書き込める
        let code = unsafe {
            ikari_settings_fetch_start(
                IKARI_SETTINGS_FETCH_PRESET,
                a.as_ptr(),
                a.len(),
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
                &mut out,
                &mut msg,
            )
        };
        assert_eq!(code, IKARI_SETTINGS_CORE_ERR_FAILED);
        assert!(out.is_null());
        assert_eq!(take_text(msg), FETCH_UNAVAILABLE_IN_OFFLINE);

        let mut msg = IkariSettingsBuf::EMPTY;
        // SAFETY: `out` と `msg` は書き込める
        let code = unsafe { ikari_settings_version_check_start(&mut out, &mut msg) };
        assert_eq!(code, IKARI_SETTINGS_CORE_ERR_FAILED);
        assert!(out.is_null());
        assert_eq!(take_text(msg), FETCH_UNAVAILABLE_IN_OFFLINE);
    }

    #[test]
    fn null_handles_are_refused_and_free_of_null_is_a_no_op() {
        // SAFETY: null を渡したときの約束を確かめる
        unsafe {
            assert_eq!(
                ikari_settings_version_check_start(ptr::null_mut(), ptr::null_mut()),
                IKARI_SETTINGS_CORE_ERR_NULL
            );
            assert_eq!(
                ikari_settings_fetch_poll(ptr::null(), ptr::null_mut(), ptr::null_mut()),
                IKARI_SETTINGS_CORE_ERR_NULL
            );
            assert_eq!(
                ikari_settings_fetch_sha256(ptr::null(), ptr::null_mut()),
                IKARI_SETTINGS_CORE_ERR_NULL
            );
            assert_eq!(
                ikari_settings_fetch_cancel(ptr::null()),
                IKARI_SETTINGS_CORE_ERR_NULL
            );
            ikari_settings_fetch_free(ptr::null_mut());
        }
    }
}
