//! 取得の C の ABI の通常版の中身（feature `online`。ADR 0080 §7.1）。`ikari-data.exe` を起こす（`fetch_os`）。
//!
//! 判断の置き場所: Rust（引数の組み立ては `fetch::model_fetch_args`、文は `text`）。

use std::panic::AssertUnwindSafe;

use super::{IKARI_SETTINGS_FETCH_PRESET, IKARI_SETTINGS_FETCH_URL, IkariSettingsFetchStatus};
use crate::abi::{IKARI_SETTINGS_CORE_ERR_NULL, IKARI_SETTINGS_CORE_OK, IkariSettingsBuf, guarded};
use crate::fetch::{FetchRequest, FetchTask, Launcher, Snapshot};
use crate::fetch_os::ProductLauncher;
use crate::session_abi::IKARI_SETTINGS_CORE_ERR_ARG;
use crate::session_abi::IKARI_SETTINGS_CORE_ERR_FAILED;
use crate::text;

/// 走っている取得の不透明な値
pub struct SettingsFetch(FetchTask);

/// # Safety
/// `ptr` は null（`len` は 0）か、`len` バイトを読めること。
unsafe fn read_str<'a>(ptr: *const u8, len: usize) -> Option<&'a str> {
    if ptr.is_null() {
        return (len == 0).then_some("");
    }
    // SAFETY: 呼び出しの約束
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(ptr, len) }).ok()
}

/// # Safety
/// `out` は null か、書き込める `IkariSettingsBuf` を指すこと。
unsafe fn write_text(out: *mut IkariSettingsBuf, text: &str) {
    if !out.is_null() {
        // SAFETY: 呼び出しの約束
        unsafe { out.write(IkariSettingsBuf::from_vec(text.as_bytes().to_vec())) };
    }
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_owned())
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
    out: *mut *mut SettingsFetch,
    msg: *mut IkariSettingsBuf,
) -> i32 {
    if out.is_null() {
        return IKARI_SETTINGS_CORE_ERR_NULL;
    }
    // SAFETY: 呼び出しの約束
    let strings = unsafe {
        (
            read_str(a, a_len),
            read_str(format, format_len),
            read_str(sha256, sha256_len),
            read_str(name, name_len),
        )
    };
    let (Some(a), Some(format), Some(sha256), Some(name)) = strings else {
        return IKARI_SETTINGS_CORE_ERR_ARG;
    };
    let request = match kind {
        IKARI_SETTINGS_FETCH_PRESET if !a.is_empty() => FetchRequest::Preset { id: a.to_owned() },
        IKARI_SETTINGS_FETCH_URL if !a.is_empty() && !format.is_empty() => FetchRequest::Url {
            url: a.to_owned(),
            format: format.to_owned(),
            sha256: non_empty(sha256),
            name: non_empty(name),
        },
        _ => return IKARI_SETTINGS_CORE_ERR_ARG,
    };
    let out_addr = out as usize;
    let msg_addr = msg as usize;
    guarded(AssertUnwindSafe(move || {
        match start_with(&ProductLauncher, &request) {
            Ok(fetch) => {
                // SAFETY: `out` は null でなく、書き込める（呼び出しの約束）
                unsafe { (out_addr as *mut *mut SettingsFetch).write(Box::into_raw(fetch)) };
                IKARI_SETTINGS_CORE_OK
            }
            Err(message) => {
                // SAFETY: `msg` は null か書き込める（呼び出しの約束）
                unsafe { write_text(msg_addr as *mut IkariSettingsBuf, message) };
                IKARI_SETTINGS_CORE_ERR_FAILED
            }
        }
    }))
}

fn start_with(
    launcher: &dyn Launcher,
    request: &FetchRequest,
) -> Result<Box<SettingsFetch>, &'static str> {
    FetchTask::start(launcher, request)
        .map(|task| Box::new(SettingsFetch(task)))
        .map_err(text::fetch_error)
}

/// 様子を読む。`status` に数、`text` に文（どちらも null なら書かない）。
///
/// # Safety
/// `fetch` は開いたもの（null なら `ERR_NULL`）。`status` と `text` は null か書き込める。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_poll(
    fetch: *const SettingsFetch,
    status: *mut IkariSettingsFetchStatus,
    text: *mut IkariSettingsBuf,
) -> i32 {
    // SAFETY: 呼び出しの約束（null か、開いたもの）
    let Some(fetch) = (unsafe { fetch.as_ref() }) else {
        return IKARI_SETTINGS_CORE_ERR_NULL;
    };
    let status_addr = status as usize;
    let text_addr = text as usize;
    guarded(AssertUnwindSafe(|| {
        let snapshot = fetch.0.snapshot();
        if status_addr != 0 {
            // SAFETY: 呼び出しの約束（null でなければ書き込める）
            unsafe { (status_addr as *mut IkariSettingsFetchStatus).write(to_status(&snapshot)) };
        }
        // SAFETY: 呼び出しの約束
        unsafe {
            write_text(
                text_addr as *mut IkariSettingsBuf,
                &text::fetch_snapshot(&snapshot),
            )
        };
        IKARI_SETTINGS_CORE_OK
    }))
}

fn to_status(s: &Snapshot) -> IkariSettingsFetchStatus {
    IkariSettingsFetchStatus {
        phase: s.phase.code(),
        received: s.received,
        total: s.total.unwrap_or(0),
        has_total: u8::from(s.total.is_some()),
    }
}

/// 版の確認を始める（`ikari-data version-check`。ADR 0076 §6）。成功したら `*out` に値を書く。様子と文は
/// `ikari_settings_fetch_poll`、解放は `ikari_settings_fetch_free`（取得と同じ）。失敗は `msg` に文を書く。
///
/// # Safety
/// `out` は書き込める。`msg` は null か書き込める。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_version_check_start(
    out: *mut *mut SettingsFetch,
    msg: *mut IkariSettingsBuf,
) -> i32 {
    if out.is_null() {
        return IKARI_SETTINGS_CORE_ERR_NULL;
    }
    let out_addr = out as usize;
    let msg_addr = msg as usize;
    guarded(AssertUnwindSafe(move || {
        match start_with(&ProductLauncher, &FetchRequest::VersionCheck) {
            Ok(fetch) => {
                // SAFETY: `out` は null でなく、書き込める（呼び出しの約束）
                unsafe { (out_addr as *mut *mut SettingsFetch).write(Box::into_raw(fetch)) };
                IKARI_SETTINGS_CORE_OK
            }
            Err(message) => {
                // SAFETY: `msg` は null か書き込める（呼び出しの約束）
                unsafe { write_text(msg_addr as *mut IkariSettingsBuf, message) };
                IKARI_SETTINGS_CORE_ERR_FAILED
            }
        }
    }))
}

/// 取得が終わったときの sha256（`done` の行の値。64 桁の 16 進）を `out` に書く。まだ終わっていなければ `ERR_FAILED`。
/// sha256 なしで取ったときに、確認の画面に出す（ADR 0076 §18.1）。
///
/// 契約のスタブ（第 4 弾。中身はレーン D-4）: 今は常に `ERR_FAILED`。
///
/// # Safety
/// `fetch` は開いたもの（null なら `ERR_NULL`）。`out` は null か書き込める。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_sha256(
    fetch: *const SettingsFetch,
    out: *mut IkariSettingsBuf,
) -> i32 {
    // SAFETY: 呼び出しの約束
    let Some(_fetch) = (unsafe { fetch.as_ref() }) else {
        return IKARI_SETTINGS_CORE_ERR_NULL;
    };
    let _ = out;
    IKARI_SETTINGS_CORE_ERR_FAILED
}

/// 取り消す（子の木を終わらせる）。終わっていれば何も変えない。
///
/// # Safety
/// `fetch` は開いたもの（null なら `ERR_NULL`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_cancel(fetch: *const SettingsFetch) -> i32 {
    // SAFETY: 呼び出しの約束
    let Some(fetch) = (unsafe { fetch.as_ref() }) else {
        return IKARI_SETTINGS_CORE_ERR_NULL;
    };
    guarded(AssertUnwindSafe(|| {
        fetch.0.cancel();
        IKARI_SETTINGS_CORE_OK
    }))
}

/// 解放する（走っていれば取り消す）。null は何もしない。
///
/// # Safety
/// `fetch` は null か、`ikari_settings_fetch_start` が返してまだ解放していないもの。同じ値を 2 回渡さない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_free(fetch: *mut SettingsFetch) {
    if !fetch.is_null() {
        // SAFETY: `Box::into_raw` で返したもので、1 回だけ渡される（呼び出しの約束）
        drop(unsafe { Box::from_raw(fetch) });
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor};
    use std::ptr;

    use ikari_resource::fetch_report::FetchErrorCode;

    use super::*;
    use crate::fetch::{Phase, Running};

    struct Scripted(&'static str);

    impl Launcher for Scripted {
        fn launch(&self, _args: &[String]) -> io::Result<Running> {
            Ok(Running {
                stdout: Box::new(Cursor::new(self.0.as_bytes().to_vec())),
                kill: Box::new(|| {}),
                wait: Box::new(|| Some(0)),
            })
        }
    }

    fn start_url(a: &str, format: &str, out: &mut *mut SettingsFetch) -> i32 {
        let mut msg = IkariSettingsBuf::EMPTY;
        // SAFETY: 文字列はこの呼び出しの間生きていて、`out` と `msg` は書き込める
        let code = unsafe {
            ikari_settings_fetch_start(
                IKARI_SETTINGS_FETCH_URL,
                a.as_ptr(),
                a.len(),
                format.as_ptr(),
                format.len(),
                ptr::null(),
                0,
                ptr::null(),
                0,
                out,
                &mut msg,
            )
        };
        // SAFETY: 核が返したものを 1 回だけ返す
        unsafe { crate::abi::ikari_settings_core_buf_free(msg) };
        code
    }

    #[test]
    fn bad_arguments_are_refused_before_launching() {
        let mut out = ptr::null_mut();
        assert_eq!(start_url("", "zenz", &mut out), IKARI_SETTINGS_CORE_ERR_ARG);
        assert_eq!(
            start_url("https://e.com/m", "", &mut out),
            IKARI_SETTINGS_CORE_ERR_ARG
        );
        assert!(out.is_null());
        // SAFETY: 約束を確かめる（null）
        let code = unsafe {
            ikari_settings_fetch_start(
                9,
                ptr::null(),
                0,
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
        assert_eq!(code, IKARI_SETTINGS_CORE_ERR_ARG);
        // UTF-8 でない
        let bad = [0xffu8, 0xfe];
        // SAFETY: 読める 2 バイト
        let code = unsafe {
            ikari_settings_fetch_start(
                IKARI_SETTINGS_FETCH_PRESET,
                bad.as_ptr(),
                2,
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
        assert_eq!(code, IKARI_SETTINGS_CORE_ERR_ARG);
    }

    #[test]
    fn null_handles_are_refused_and_free_of_null_is_a_no_op() {
        // SAFETY: null を渡したときの約束を確かめる
        unsafe {
            assert_eq!(
                ikari_settings_fetch_poll(ptr::null(), ptr::null_mut(), ptr::null_mut()),
                IKARI_SETTINGS_CORE_ERR_NULL
            );
            assert_eq!(
                ikari_settings_fetch_cancel(ptr::null()),
                IKARI_SETTINGS_CORE_ERR_NULL
            );
            ikari_settings_fetch_free(ptr::null_mut());
        }
    }

    #[test]
    fn poll_reports_numbers_and_text_then_free() {
        let launcher = Scripted("progress 5 10\nerror checksum\n");
        let request = FetchRequest::Preset {
            id: "zenz-small".into(),
        };
        let fetch = start_with(&launcher, &request).unwrap();
        let raw = Box::into_raw(fetch);
        // 読み終わるまで待つ（`Failed` になる）
        let mut status = IkariSettingsFetchStatus {
            phase: 0,
            received: 0,
            total: 0,
            has_total: 0,
        };
        for _ in 0..200 {
            let mut text = IkariSettingsBuf::EMPTY;
            // SAFETY: `raw` は開いたもの。`status` と `text` は書き込める
            let code = unsafe { ikari_settings_fetch_poll(raw, &mut status, &mut text) };
            assert_eq!(code, IKARI_SETTINGS_CORE_OK);
            // SAFETY: 核が返したものを 1 回だけ返す
            let message = unsafe { std::slice::from_raw_parts(text.ptr, text.len) }.to_vec();
            unsafe { crate::abi::ikari_settings_core_buf_free(text) };
            if status.phase == Phase::Failed.code() {
                assert_eq!(
                    String::from_utf8(message).unwrap(),
                    text::fetch_error(FetchErrorCode::Checksum)
                );
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(status.phase, Phase::Failed.code());
        assert_eq!(
            (status.received, status.total, status.has_total),
            (5, 10, 1)
        );
        // SAFETY: 取り消しと解放（1 回）
        unsafe {
            assert_eq!(ikari_settings_fetch_cancel(raw), IKARI_SETTINGS_CORE_OK);
            ikari_settings_fetch_free(raw);
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn product_launcher_cannot_start_outside_windows() {
        let request = FetchRequest::Preset {
            id: "zenz-small".into(),
        };
        assert_eq!(
            start_with(&ProductLauncher, &request).err(),
            Some(text::fetch_error(FetchErrorCode::Tool))
        );
    }
}
