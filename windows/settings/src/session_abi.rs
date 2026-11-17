//! 設定の口へのつながりの C の ABI（W4-s3 の第 2 段。ヘッダは `include/ikari_settings_core.h`）。
//!
//! - 文字列は UTF-8 のポインタと長さで受ける（呼び出しの間だけ借りる）。
//! - 結果の欄の文は `msg` に書く（null なら書かない。返したバイト列は `ikari_settings_core_buf_free` で返す）。
//! - `SettingsSession` の中の値は錠で守る（どのスレッドから呼んでもよい。画面は UI のスレッドの外で呼ぶ）。
//!
//! 判断の置き場所: Rust。何を送るか（`edit`）、文（`text`、状態は shell-core の `LinkStatus`）、相手の照合（`link`）。

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::{Mutex, MutexGuard};

use ikari_shell_core::bridge::LinkStatus;
use ikari_shell_core::bridge::proto::UpdateSettings;
use ikari_shell_core::settings::{Reflect, all_specs, export_file_name};

use crate::abi::{IKARI_SETTINGS_CORE_ERR_NULL, IKARI_SETTINGS_CORE_OK, IkariSettingsBuf, guarded};
use crate::edit::{UiValue, plan_edit, plan_reset};
use crate::link::{LinkError, SettingsLink, connect_installed};
use crate::text;
use crate::view::view_tsv;

/// 操作が通らなかった（理由は `msg` の文）
pub const IKARI_SETTINGS_CORE_ERR_FAILED: i32 = -4;
/// つながりが使えなくなった（状態の欄を「つながらない」にする。文は `msg`）
pub const IKARI_SETTINGS_CORE_ERR_DISCONNECTED: i32 = -5;
/// 引数が使えない（null の session、UTF-8 でない文字列、知らない部品の番号）
pub const IKARI_SETTINGS_CORE_ERR_ARG: i32 = -6;

/// `ikari_settings_core_edit` の `widget`
pub const IKARI_SETTINGS_CORE_WIDGET_BOOL: i32 = 1;
pub const IKARI_SETTINGS_CORE_WIDGET_INT: i32 = 2;
pub const IKARI_SETTINGS_CORE_WIDGET_CHOICE: i32 = 3;

/// 設定の口へのつながりと、最後に受けた値の一式（書き換えを作るときの「今の値」）
pub struct SettingsSession {
    link: SettingsLink,
    effective: Mutex<Vec<(String, String)>>,
}

impl SettingsSession {
    fn effective(&self) -> MutexGuard<'_, Vec<(String, String)>> {
        self.effective.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// # Safety
/// `out` は null か、書き込める `IkariSettingsBuf` を指すこと。
unsafe fn write_text(out: *mut IkariSettingsBuf, text: &str) {
    if !out.is_null() {
        // SAFETY: 呼び出しの約束（null でなければ書き込める）
        unsafe { out.write(IkariSettingsBuf::from_vec(text.as_bytes().to_vec())) };
    }
}

/// # Safety
/// `ptr` は null（`len` は 0）か、`len` バイトを読めること。
unsafe fn read_str<'a>(ptr: *const u8, len: usize) -> Option<&'a str> {
    if ptr.is_null() {
        return (len == 0).then_some("");
    }
    // SAFETY: 呼び出しの約束（`len` バイトを読める。呼び出しの間だけ借りる）
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(ptr, len) }).ok()
}

/// 誤りの番号（つながりが使えないなら DISCONNECTED）
fn failure_code(e: &LinkError) -> i32 {
    match e {
        LinkError::Status(_) => IKARI_SETTINGS_CORE_ERR_DISCONNECTED,
        _ => IKARI_SETTINGS_CORE_ERR_FAILED,
    }
}

/// 誤りの文を書いて番号を返す
fn fail(e: &LinkError, msg: *mut IkariSettingsBuf) -> i32 {
    // SAFETY: `msg` は null か書き込める（呼び出しの約束を受け継ぐ）
    unsafe { write_text(msg, &text::link_error(e)) };
    failure_code(e)
}

/// 設定の口につなぐ（相手の照合と `Hello` まで）。つながらなければ null で、`out_status` に `LinkStatus` の番号
/// （`ikari_settings_core_status_text` で文にする）。戻り値は `ikari_settings_core_close` で必ず解放する。
///
/// # Safety
/// `out_status` は null か、書き込める `i32` を指すこと。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_core_open(out_status: *mut i32) -> *mut SettingsSession {
    let opened = catch_unwind(|| connect_installed(env!("CARGO_PKG_VERSION")))
        .unwrap_or(Err(LinkStatus::Unavailable));
    let (session, status) = match opened {
        Ok(link) => {
            let effective = link.effective().unwrap_or_default();
            let session = Box::new(SettingsSession {
                link,
                effective: Mutex::new(effective),
            });
            (Box::into_raw(session), LinkStatus::Connected)
        }
        Err(status) => (ptr::null_mut(), status),
    };
    if !out_status.is_null() {
        // SAFETY: 呼び出しの約束（null でなければ書き込める）
        unsafe { out_status.write(status.code()) };
    }
    session
}

/// つながりを閉じて解放する。null は何もしない。
///
/// # Safety
/// `session` は null か、`ikari_settings_core_open` が返してまだ閉じていないもの。同じ値を 2 回渡さない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_core_close(session: *mut SettingsSession) {
    if !session.is_null() {
        // SAFETY: `Box::into_raw` で返したもので、1 回だけ渡される（呼び出しの約束）
        drop(unsafe { Box::from_raw(session) });
    }
}

/// 状態（`LinkStatus` の番号）の文。知らない番号は「返事がありません」の文。
///
/// # Safety
/// `out` は null か、書き込める `IkariSettingsBuf` を指すこと。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_core_status_text(
    status: i32,
    out: *mut IkariSettingsBuf,
) -> i32 {
    if out.is_null() {
        return IKARI_SETTINGS_CORE_ERR_NULL;
    }
    let status = LinkStatus::from_code(status).unwrap_or(LinkStatus::Unavailable);
    // SAFETY: `out` は書き込める
    unsafe { write_text(out, status.text()) };
    IKARI_SETTINGS_CORE_OK
}

/// 画面の行（`view` の TSV）。`session` が null なら既定値で作る（つながっていないとき）。つながっていれば、
/// 設定の口から読み直す。読めなければ、最後に受けた値で作り、`msg` に理由を書いて誤りの番号を返す。
///
/// # Safety
/// `session` は null か開いたもの。`out` と `msg` は null か書き込める `IkariSettingsBuf`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_core_view(
    session: *const SettingsSession,
    out: *mut IkariSettingsBuf,
    msg: *mut IkariSettingsBuf,
) -> i32 {
    if out.is_null() {
        return IKARI_SETTINGS_CORE_ERR_NULL;
    }
    // SAFETY: 呼び出しの約束（null か、開いたもの）
    let session = unsafe { session.as_ref() };
    guarded(AssertUnwindSafe(|| {
        let Some(session) = session else {
            // SAFETY: `out` は書き込める
            unsafe { write_text(out, &view_tsv(&[])) };
            return IKARI_SETTINGS_CORE_OK;
        };
        let code = match session.link.effective() {
            Ok(fresh) => {
                *session.effective() = fresh;
                IKARI_SETTINGS_CORE_OK
            }
            Err(e) => fail(&e, msg),
        };
        // SAFETY: `out` は書き込める
        unsafe { write_text(out, &view_tsv(&session.effective())) };
        code
    }))
}

/// 書き換えを送る（`widget` は `IKARI_SETTINGS_CORE_WIDGET_*`。真偽は `number` の 0 と 1、数は `number`、選択肢は
/// `choice`）。変わらなければ何も送らずに成功（`msg` は書かない）。読み直しは `ikari_settings_core_view`。
///
/// # Safety
/// `session` は開いたもの。`key` と `choice` は null（長さ 0）か読める UTF-8。`msg` は null か書き込める。
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn ikari_settings_core_edit(
    session: *const SettingsSession,
    key: *const u8,
    key_len: usize,
    widget: i32,
    number: i64,
    choice: *const u8,
    choice_len: usize,
    msg: *mut IkariSettingsBuf,
) -> i32 {
    // SAFETY: 呼び出しの約束
    let session = unsafe { session.as_ref() };
    // SAFETY: 呼び出しの約束
    let key = unsafe { read_str(key, key_len) };
    // SAFETY: 呼び出しの約束
    let choice = unsafe { read_str(choice, choice_len) };
    let (Some(session), Some(key), Some(choice)) = (session, key, choice) else {
        return IKARI_SETTINGS_CORE_ERR_ARG;
    };
    let value = match widget {
        IKARI_SETTINGS_CORE_WIDGET_BOOL => UiValue::Bool(number != 0),
        IKARI_SETTINGS_CORE_WIDGET_INT => UiValue::Int(number),
        IKARI_SETTINGS_CORE_WIDGET_CHOICE => UiValue::Choice(choice.to_owned()),
        _ => return IKARI_SETTINGS_CORE_ERR_ARG,
    };
    guarded(AssertUnwindSafe(|| {
        let planned = plan_edit(&session.effective(), key, &value);
        match planned {
            Ok(Some(update)) => send_update(session, update, key, msg),
            Ok(None) => IKARI_SETTINGS_CORE_OK,
            Err(e) => {
                // SAFETY: `msg` は null か書き込める
                unsafe { write_text(msg, text::edit_error(e)) };
                IKARI_SETTINGS_CORE_ERR_FAILED
            }
        }
    }))
}

/// `key` を既定に戻す。
///
/// # Safety
/// `ikari_settings_core_edit` と同じ。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_core_reset(
    session: *const SettingsSession,
    key: *const u8,
    key_len: usize,
    msg: *mut IkariSettingsBuf,
) -> i32 {
    // SAFETY: 呼び出しの約束
    let session = unsafe { session.as_ref() };
    // SAFETY: 呼び出しの約束
    let key = unsafe { read_str(key, key_len) };
    let (Some(session), Some(key)) = (session, key) else {
        return IKARI_SETTINGS_CORE_ERR_ARG;
    };
    guarded(AssertUnwindSafe(|| match plan_reset(key) {
        Ok(update) => send_update(session, update, key, msg),
        Err(e) => {
            // SAFETY: `msg` は null か書き込める
            unsafe { write_text(msg, text::edit_error(e)) };
            IKARI_SETTINGS_CORE_ERR_FAILED
        }
    }))
}

fn send_update(
    session: &SettingsSession,
    update: UpdateSettings,
    key: &str,
    msg: *mut IkariSettingsBuf,
) -> i32 {
    match session.link.update(update) {
        Ok(fresh) => {
            *session.effective() = fresh;
            let restart = all_specs()
                .iter()
                .any(|s| s.key == key && s.reflect == Reflect::EngineRestart);
            // SAFETY: `msg` は null か書き込める
            unsafe { write_text(msg, text::updated(restart)) };
            IKARI_SETTINGS_CORE_OK
        }
        Err(e) => fail(&e, msg),
    }
}

/// `path`（利用者がファイルを選ぶ窓で選んだもの）に書き出す。`replace` は、画面（または OS の窓）で上書きの確認を
/// 済ませたときだけ立てる。結果の文は `msg`。
///
/// # Safety
/// `ikari_settings_core_edit` と同じ。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_core_export(
    session: *const SettingsSession,
    path: *const u8,
    path_len: usize,
    replace: bool,
    msg: *mut IkariSettingsBuf,
) -> i32 {
    // SAFETY: 呼び出しの約束
    let session = unsafe { session.as_ref() };
    // SAFETY: 呼び出しの約束
    let path = unsafe { read_str(path, path_len) };
    let (Some(session), Some(path)) = (session, path) else {
        return IKARI_SETTINGS_CORE_ERR_ARG;
    };
    guarded(AssertUnwindSafe(|| {
        match session.link.export_to_file(path, replace) {
            Ok(reply) => {
                // SAFETY: `msg` は null か書き込める
                unsafe { write_text(msg, &text::exported(&reply)) };
                IKARI_SETTINGS_CORE_OK
            }
            Err(e) => fail(&e, msg),
        }
    }))
}

/// `path` から取り込む（`dry_run` は確かめて件数を返すだけ。画面は先に `dry_run` で見せ、確認のあとに本番を送る）。
/// 取り込めない内容なら、何も変えずに `IKARI_SETTINGS_CORE_ERR_FAILED`。結果の文は `msg`。
///
/// # Safety
/// `ikari_settings_core_edit` と同じ。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_core_import(
    session: *const SettingsSession,
    path: *const u8,
    path_len: usize,
    dry_run: bool,
    msg: *mut IkariSettingsBuf,
) -> i32 {
    // SAFETY: 呼び出しの約束
    let session = unsafe { session.as_ref() };
    // SAFETY: 呼び出しの約束
    let path = unsafe { read_str(path, path_len) };
    let (Some(session), Some(path)) = (session, path) else {
        return IKARI_SETTINGS_CORE_ERR_ARG;
    };
    guarded(AssertUnwindSafe(|| {
        match session.link.import_from_file(path, dry_run) {
            Ok(reply) => {
                // SAFETY: `msg` は null か書き込める
                unsafe { write_text(msg, &text::imported(&reply, dry_run)) };
                let blocked = reply.issue.as_ref().is_some_and(|i| i.kind != 0)
                    || (!dry_run && !reply.applied);
                if blocked {
                    IKARI_SETTINGS_CORE_ERR_FAILED
                } else {
                    IKARI_SETTINGS_CORE_OK
                }
            }
            Err(e) => fail(&e, msg),
        }
    }))
}

/// 自由指定のモデルをファイルから取り込む（ADR 0076 §7、ADR 0060 の段 4）。画面はファイルを選ぶ窓で得たパスと、
/// 形式の語、sha256（空なら照らさない。警告の確認を済ませたとき）、表示の名前（空でよい）を渡す。結果の文は `msg`。
///
/// 契約のスタブ（第 4 弾。中身はレーン D-4）: 今は送らずに `ERR_FAILED`。
///
/// # Safety
/// `ikari_settings_core_edit` と同じ。
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn ikari_settings_core_import_model(
    session: *const SettingsSession,
    path: *const u8,
    path_len: usize,
    format: *const u8,
    format_len: usize,
    sha256: *const u8,
    sha256_len: usize,
    name: *const u8,
    name_len: usize,
    msg: *mut IkariSettingsBuf,
) -> i32 {
    // SAFETY: 呼び出しの約束
    let session = unsafe { session.as_ref() };
    // SAFETY: 呼び出しの約束
    let strings = unsafe {
        (
            read_str(path, path_len),
            read_str(format, format_len),
            read_str(sha256, sha256_len),
            read_str(name, name_len),
        )
    };
    let (Some(session), (Some(path), Some(format), Some(sha256), Some(name))) = (session, strings)
    else {
        return IKARI_SETTINGS_CORE_ERR_ARG;
    };
    guarded(AssertUnwindSafe(|| {
        let sha256 = (!sha256.is_empty()).then_some(sha256);
        let name = (!name.is_empty()).then_some(name);
        match session.link.import_model_file(path, format, sha256, name) {
            Ok(_) => IKARI_SETTINGS_CORE_OK,
            Err(e) => fail(&e, msg),
        }
    }))
}

/// 自由指定のモデル（URL、ファイル）のボタンを出してよいか（1 なら出す、0 なら出さない）。判断は核
/// （段 2 のある OS でだけ出す。ADR 0072 §4.2、ADR 0076 §7）。`session` は null でよい（つながっていない）。
///
/// 契約のスタブ（第 4 弾。中身はレーン D-4）: 今は常に 0。
///
/// # Safety
/// `session` は null か、開いたもの。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_core_custom_model_offered(
    session: *const SettingsSession,
) -> i32 {
    // SAFETY: 呼び出しの約束
    let _ = unsafe { session.as_ref() };
    0
}

/// 書き出しの既定の名前（`ikari-export-YYYYMMDD.tsv`）。年月日は画面が利用者の暦と時刻帯で数えて渡す。
///
/// # Safety
/// `out` は null か、書き込める `IkariSettingsBuf` を指すこと。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_core_export_file_name(
    year: u32,
    month: u32,
    day: u32,
    out: *mut IkariSettingsBuf,
) -> i32 {
    if out.is_null() {
        return IKARI_SETTINGS_CORE_ERR_NULL;
    }
    // SAFETY: `out` は書き込める
    unsafe { write_text(out, &export_file_name(year, month, day)) };
    IKARI_SETTINGS_CORE_OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::ikari_settings_core_buf_free;

    fn take(buf: IkariSettingsBuf) -> String {
        // SAFETY: 核が返した ptr と len
        let bytes = unsafe { std::slice::from_raw_parts(buf.ptr, buf.len) }.to_vec();
        // SAFETY: 読んだあと 1 回だけ返す
        unsafe { ikari_settings_core_buf_free(buf) };
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn without_a_session_the_view_shows_the_defaults_and_open_says_why() {
        let mut out = IkariSettingsBuf::EMPTY;
        // SAFETY: null の session は約束のうち。`out` は書き込める
        let code = unsafe { ikari_settings_core_view(ptr::null(), &mut out, ptr::null_mut()) };
        assert_eq!(code, IKARI_SETTINGS_CORE_OK);
        assert_eq!(take(out), view_tsv(&[]));
        let mut status = -1;
        // SAFETY: `status` は書き込める
        let session = unsafe { ikari_settings_core_open(&mut status) };
        #[cfg(not(windows))]
        {
            assert!(session.is_null(), "Windows 以外には設定の口が無い");
            assert_eq!(status, LinkStatus::EngineNotRunning.code());
        }
        // Windows では、テストを流す権限とエンジンの有無で変わる。高い権限（GitHub のランナーは管理者）なら、つながずに断る
        #[cfg(windows)]
        if !crate::abi::self_allowed() {
            assert!(session.is_null());
            assert_eq!(status, LinkStatus::Elevated.code());
        }
        // SAFETY: open が返したもの（null も約束のうち）。1 回だけ閉じる
        unsafe { ikari_settings_core_close(session) };
        let expected = LinkStatus::from_code(status).expect("知っている番号");
        let mut out = IkariSettingsBuf::EMPTY;
        // SAFETY: `out` は書き込める
        unsafe { ikari_settings_core_status_text(status, &mut out) };
        assert_eq!(take(out), expected.text());
    }

    #[test]
    fn null_sessions_and_bad_strings_are_argument_errors() {
        let key = "paging.wrap";
        // SAFETY: null の session と、読める key
        let code = unsafe {
            ikari_settings_core_edit(
                ptr::null(),
                key.as_ptr(),
                key.len(),
                IKARI_SETTINGS_CORE_WIDGET_BOOL,
                1,
                ptr::null(),
                0,
                ptr::null_mut(),
            )
        };
        assert_eq!(code, IKARI_SETTINGS_CORE_ERR_ARG);
        let bad = [0xff_u8, 0xfe];
        // SAFETY: 2 バイトを読める
        assert_eq!(unsafe { read_str(bad.as_ptr(), bad.len()) }, None);
        // SAFETY: null は長さ 0 のときだけ空
        assert_eq!(unsafe { read_str(ptr::null(), 0) }, Some(""));
        // SAFETY: 同上
        assert_eq!(unsafe { read_str(ptr::null(), 3) }, None);
    }

    #[test]
    fn the_export_file_name_comes_from_shell_core() {
        let mut out = IkariSettingsBuf::EMPTY;
        // SAFETY: `out` は書き込める
        unsafe { ikari_settings_core_export_file_name(2026, 11, 16, &mut out) };
        assert_eq!(take(out), "ikari-export-20261116.tsv");
    }
}
