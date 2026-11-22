//! 取得の口のオフライン版の中身（feature `online` なし。ADR 0080 §7.1）。何も起こさない。取得の核
//! （`ikari_shell_core::settings::fetch`）も、`ikari-data` の場所を決める部分（`fetch_launcher`）も持たない。
//!
//! ハンドルは作らないので、`poll`・`sha256`・`cancel` に null でない値が来たら、それは知らない値（`ERR_FAILED`）。
//! `free` は何もしない（解放するものが無い）。

use ikari_shell_core::edition::FETCH_UNAVAILABLE_IN_OFFLINE;

use super::IkariSettingsFetchStatus;
use crate::shell::{IKARI_SHELL_ERR_FAILED, IKARI_SHELL_ERR_NULL};
use crate::{IkariShellBuf, write_error};

/// 取得のハンドルの型（オフライン版では作らない）
pub struct IkariSettingsFetch {
    _never: (),
}

/// 始める口の共通: `out` は書かず、`msg` に文を書いて `ERR_FAILED`
fn refuse(out: *mut *mut IkariSettingsFetch, msg: *mut IkariShellBuf) -> i32 {
    if out.is_null() {
        return IKARI_SHELL_ERR_NULL;
    }
    write_error(msg, FETCH_UNAVAILABLE_IN_OFFLINE);
    IKARI_SHELL_ERR_FAILED
}

fn unknown_handle(fetch: *const IkariSettingsFetch) -> i32 {
    if fetch.is_null() {
        IKARI_SHELL_ERR_NULL
    } else {
        IKARI_SHELL_ERR_FAILED
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
    out: *mut *mut IkariSettingsFetch,
    msg: *mut IkariShellBuf,
) -> i32 {
    refuse(out, msg)
}

/// オフライン版: 起こさない（`ERR_FAILED` と文）。
///
/// # Safety
/// `out` は null か書き込める。`msg` は null か書き込める。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_version_check_start(
    out: *mut *mut IkariSettingsFetch,
    msg: *mut IkariShellBuf,
) -> i32 {
    refuse(out, msg)
}

/// オフライン版: ハンドルは無い（null は `ERR_NULL`、ほかは知らない値で `ERR_FAILED`。何も書かない）。
///
/// # Safety
/// 引数を読み書きしない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_poll(
    fetch: *const IkariSettingsFetch,
    _status: *mut IkariSettingsFetchStatus,
    _text: *mut IkariShellBuf,
) -> i32 {
    unknown_handle(fetch)
}

/// オフライン版: ハンドルは無い（`ikari_settings_fetch_poll` と同じ）。
///
/// # Safety
/// 引数を読み書きしない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_sha256(
    fetch: *const IkariSettingsFetch,
    _out: *mut IkariShellBuf,
) -> i32 {
    unknown_handle(fetch)
}

/// オフライン版: ハンドルは無い（`ikari_settings_fetch_poll` と同じ）。
///
/// # Safety
/// 引数を読み書きしない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_cancel(fetch: *const IkariSettingsFetch) -> i32 {
    unknown_handle(fetch)
}

/// オフライン版: 解放するものが無い（何もしない）。
///
/// # Safety
/// 引数を読み書きしない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_free(_fetch: *mut IkariSettingsFetch) {}

#[cfg(test)]
mod tests {
    use std::ptr;

    use super::super::IKARI_SETTINGS_FETCH_PRESET;
    use super::*;

    fn take_text(buf: IkariShellBuf) -> String {
        // SAFETY: 殻が返したものを 1 回だけ引き取る
        String::from_utf8(unsafe { buf.into_vec() }).unwrap()
    }

    #[test]
    fn starting_is_refused_with_a_text_and_no_handle() {
        let a = "zenz-small";
        let mut out = ptr::null_mut();
        let mut msg = IkariShellBuf::EMPTY;
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
        assert_eq!(code, IKARI_SHELL_ERR_FAILED);
        assert!(out.is_null());
        assert_eq!(take_text(msg), FETCH_UNAVAILABLE_IN_OFFLINE);

        let mut msg = IkariShellBuf::EMPTY;
        // SAFETY: `out` と `msg` は書き込める
        let code = unsafe { ikari_settings_version_check_start(&mut out, &mut msg) };
        assert_eq!(code, IKARI_SHELL_ERR_FAILED);
        assert!(out.is_null());
        assert_eq!(take_text(msg), FETCH_UNAVAILABLE_IN_OFFLINE);
    }

    #[test]
    fn null_arguments_do_not_crash() {
        // SAFETY: null を渡したときの約束を確かめる
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
        let mut out = ptr::null_mut();
        // `msg` が null でも、起こさずに失敗を返す
        // SAFETY: `out` は書き込める
        let code = unsafe { ikari_settings_version_check_start(&mut out, ptr::null_mut()) };
        assert_eq!(code, IKARI_SHELL_ERR_FAILED);
        assert!(out.is_null());
    }
}
