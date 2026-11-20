//! 取得（`ikari-data`）の起動、進み具合、取り消しの口（ヘッダの `ikari_settings_fetch_*`。ADR 0079 §3.6）。
//! 名前と引数の並びは Windows の `fetch_abi.rs` と同じ。失敗の値は `IKARI_SHELL_ERR_*`、文の入れ物は `IkariShellBuf`。
//!
//! 起こすのは `fetch_launcher::ProductLauncher`（ADR 0079 §3.6）。核（`FetchTask`）は `shell-core` の共有の部分。
//! どの入口も panic を外へ出さない。
//!
//! 判断の置き場所: Rust（起動するか、どの exe か、引数、文）。Swift はボタンと描画だけ。

use std::panic::{AssertUnwindSafe, catch_unwind};

use ikari_shell_core::settings::fetch::text::{fetch_error, fetch_snapshot};
use ikari_shell_core::settings::fetch::{FetchRequest, FetchTask, Launcher, Snapshot};

use crate::IkariShellBuf;
use crate::fetch_launcher::ProductLauncher;
use crate::shell::{
    IKARI_SHELL_ERR_ARG, IKARI_SHELL_ERR_FAILED, IKARI_SHELL_ERR_NULL, IKARI_SHELL_ERR_PANIC,
    IKARI_SHELL_OK,
};

/// `ikari_settings_fetch_start` の `kind`: 表のモデル（`a` が ID）
pub const IKARI_SETTINGS_FETCH_PRESET: i32 = 1;
/// 自由指定の URL（`a` が URL。`format` が必要。`sha256` と `name` は空でよい）
pub const IKARI_SETTINGS_FETCH_URL: i32 = 2;

/// 走っている取得の不透明な値
pub struct IkariSettingsFetch(FetchTask);

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
unsafe fn read_str<'a>(ptr: *const u8, len: usize) -> Option<&'a str> {
    if ptr.is_null() {
        return (len == 0).then_some("");
    }
    // SAFETY: 呼び出しの約束
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(ptr, len) }).ok()
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_owned())
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
    let strings = unsafe {
        (
            read_str(a, a_len),
            read_str(format, format_len),
            read_str(sha256, sha256_len),
            read_str(name, name_len),
        )
    };
    let (Some(a), Some(format), Some(sha256), Some(name)) = strings else {
        return IKARI_SHELL_ERR_ARG;
    };
    let request = match kind {
        IKARI_SETTINGS_FETCH_PRESET if !a.is_empty() => FetchRequest::Preset { id: a.to_owned() },
        IKARI_SETTINGS_FETCH_URL if !a.is_empty() && !format.is_empty() => FetchRequest::Url {
            url: a.to_owned(),
            format: format.to_owned(),
            sha256: non_empty(sha256),
            name: non_empty(name),
        },
        _ => return IKARI_SHELL_ERR_ARG,
    };
    // SAFETY: 呼び出しの約束（`out` は null でなく書き込める。`msg` は null か書き込める）
    unsafe { start_and_write(&ProductLauncher, &request, out, msg) }
}

/// 起こして、成功なら `*out` に値を、失敗なら `msg` に文を書く
///
/// # Safety
/// `out` は書き込める。`msg` は null か書き込める。
unsafe fn start_and_write(
    launcher: &dyn Launcher,
    request: &FetchRequest,
    out: *mut *mut IkariSettingsFetch,
    msg: *mut IkariShellBuf,
) -> i32 {
    let out_addr = out as usize;
    let msg_addr = msg as usize;
    guarded(AssertUnwindSafe(|| {
        match start_with(launcher, request) {
            Ok(fetch) => {
                // SAFETY: `out` は null でなく、書き込める（呼び出しの約束）
                unsafe { (out_addr as *mut *mut IkariSettingsFetch).write(Box::into_raw(fetch)) };
                IKARI_SHELL_OK
            }
            Err(message) => {
                // SAFETY: `msg` は null か書き込める（呼び出しの約束）
                unsafe { write_msg(msg_addr as *mut IkariShellBuf, message) };
                IKARI_SHELL_ERR_FAILED
            }
        }
    }))
}

fn start_with(
    launcher: &dyn Launcher,
    request: &FetchRequest,
) -> Result<Box<IkariSettingsFetch>, &'static str> {
    FetchTask::start(launcher, request)
        .map(|task| Box::new(IkariSettingsFetch(task)))
        .map_err(fetch_error)
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
    // SAFETY: 呼び出しの約束
    unsafe { start_and_write(&ProductLauncher, &FetchRequest::VersionCheck, out, msg) }
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
    // SAFETY: 呼び出しの約束（null か、開いたもの）
    let Some(fetch) = (unsafe { fetch.as_ref() }) else {
        return IKARI_SHELL_ERR_NULL;
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
        unsafe { write_msg(text_addr as *mut IkariShellBuf, &fetch_snapshot(&snapshot)) };
        IKARI_SHELL_OK
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

/// 取得が終わったときの sha256（`done` の行の値。64 桁の 16 進）を `out` に書く。まだ終わっていなければ `ERR_FAILED`。
///
/// # Safety
/// `fetch` は開いたもの（null なら `ERR_NULL`）。`out` は null か書き込める。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_sha256(
    fetch: *const IkariSettingsFetch,
    out: *mut IkariShellBuf,
) -> i32 {
    // SAFETY: 呼び出しの約束
    let Some(fetch) = (unsafe { fetch.as_ref() }) else {
        return IKARI_SHELL_ERR_NULL;
    };
    let out_addr = out as usize;
    guarded(AssertUnwindSafe(|| {
        match fetch.0.snapshot().sha256.filter(|s| !s.is_empty()) {
            Some(sha256) => {
                // SAFETY: 呼び出しの約束
                unsafe { write_msg(out_addr as *mut IkariShellBuf, &sha256) };
                IKARI_SHELL_OK
            }
            None => IKARI_SHELL_ERR_FAILED,
        }
    }))
}

/// 取り消す（子に `SIGKILL`）。終わっていれば何も変えない。
///
/// # Safety
/// `fetch` は開いたもの（null なら `ERR_NULL`）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_cancel(fetch: *const IkariSettingsFetch) -> i32 {
    // SAFETY: 呼び出しの約束
    let Some(fetch) = (unsafe { fetch.as_ref() }) else {
        return IKARI_SHELL_ERR_NULL;
    };
    guarded(AssertUnwindSafe(|| {
        fetch.0.cancel();
        IKARI_SHELL_OK
    }))
}

/// 解放する（走っていれば取り消す）。null は何もしない。
///
/// # Safety
/// `fetch` は null か、`ikari_settings_fetch_start` が返してまだ解放していないもの。同じ値を 2 回渡さない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_fetch_free(fetch: *mut IkariSettingsFetch) {
    if !fetch.is_null() {
        // SAFETY: `Box::into_raw` で返したもので、1 回だけ渡される（呼び出しの約束）
        drop(unsafe { Box::from_raw(fetch) });
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor};
    #[cfg(unix)]
    use std::path::Path;
    use std::ptr;
    use std::thread;
    use std::time::Duration;

    use ikari_shell_core::settings::fetch::text::FetchErrorCode;
    use ikari_shell_core::settings::fetch::{Phase, Running};

    use super::*;
    #[cfg(unix)]
    use crate::fetch_launcher::spawn_running;

    /// 固定の行を返す偽の子
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

    /// 本物の子（`sh -c`）を起こす偽の ikari-data（Unix だけ）
    #[cfg(unix)]
    struct Shell(&'static str);

    #[cfg(unix)]
    impl Launcher for Shell {
        fn launch(&self, _args: &[String]) -> io::Result<Running> {
            spawn_running(
                Path::new("/bin/sh"),
                &["-c".to_owned(), self.0.to_owned()],
                &[],
            )
        }
    }

    fn preset() -> FetchRequest {
        FetchRequest::Preset {
            id: "zenz-small".into(),
        }
    }

    fn take_text(buf: IkariShellBuf) -> String {
        // SAFETY: 殻が返したものを 1 回だけ引き取る
        String::from_utf8(unsafe { buf.into_vec() }).unwrap()
    }

    fn open(launcher: &dyn Launcher) -> *mut IkariSettingsFetch {
        Box::into_raw(start_with(launcher, &preset()).unwrap())
    }

    /// 取得中でなくなるまで poll する。最後の様子と文を返す
    fn poll_until_finished(raw: *const IkariSettingsFetch) -> (IkariSettingsFetchStatus, String) {
        for _ in 0..500 {
            let mut status = IkariSettingsFetchStatus {
                phase: 0,
                received: 0,
                total: 0,
                has_total: 0,
            };
            let mut text = IkariShellBuf::EMPTY;
            // SAFETY: `raw` は開いたもの。`status` と `text` は書き込める
            let code = unsafe { ikari_settings_fetch_poll(raw, &mut status, &mut text) };
            assert_eq!(code, IKARI_SHELL_OK);
            let text = take_text(text);
            if status.phase != Phase::Running.code() {
                return (status, text);
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("not finished");
    }

    fn start_with_strings(kind: i32, a: &[u8], format: &[u8]) -> i32 {
        let mut out = ptr::null_mut();
        // SAFETY: 読める長さを渡し、`out` は書き込める
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
                ptr::null_mut(),
            )
        };
        assert!(out.is_null());
        code
    }

    #[test]
    fn bad_arguments_are_refused_before_launching() {
        assert_eq!(start_with_strings(99, b"x", b""), IKARI_SHELL_ERR_ARG);
        assert_eq!(
            start_with_strings(IKARI_SETTINGS_FETCH_PRESET, b"", b""),
            IKARI_SHELL_ERR_ARG
        );
        assert_eq!(
            start_with_strings(IKARI_SETTINGS_FETCH_URL, b"https://x", b""),
            IKARI_SHELL_ERR_ARG
        );
        assert_eq!(
            start_with_strings(IKARI_SETTINGS_FETCH_PRESET, &[0xff], b""),
            IKARI_SHELL_ERR_ARG
        );
    }

    #[test]
    fn null_arguments_do_not_crash() {
        // SAFETY: null を渡したときの約束を確かめる
        unsafe {
            assert_eq!(
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
                ),
                IKARI_SHELL_ERR_NULL
            );
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

    #[test]
    fn launch_failure_reports_the_tool_text_without_a_handle() {
        // テストの exe は束の中にないので、製品の起こし方は起こせない
        let mut out = ptr::null_mut();
        let mut msg = IkariShellBuf::EMPTY;
        let a = "zenz-small";
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
        assert_eq!(take_text(msg), fetch_error(FetchErrorCode::Tool));
    }

    #[test]
    fn lines_flow_to_poll_and_sha256_is_returned_when_done() {
        let sha = "a".repeat(64);
        let script: &'static str =
            Box::leak(format!("progress 5 10\ndone m {sha}\n").into_boxed_str());
        let raw = open(&Scripted(script));
        let (status, _) = poll_until_finished(raw);
        assert_eq!(status.phase, Phase::Done.code());
        let mut out = IkariShellBuf::EMPTY;
        // SAFETY: `raw` は開いたもの。`out` は書き込める
        unsafe {
            assert_eq!(ikari_settings_fetch_sha256(raw, &mut out), IKARI_SHELL_OK);
            assert_eq!(take_text(out), sha);
            // 終わったあとの取り消しは何も変えない
            assert_eq!(ikari_settings_fetch_cancel(raw), IKARI_SHELL_OK);
            ikari_settings_fetch_free(raw);
        }
    }

    #[test]
    #[cfg(unix)]
    fn sha256_fails_until_done_and_free_cancels_a_running_child() {
        let raw = open(&Shell("echo progress 1 2; exec sleep 60"));
        let mut out = IkariShellBuf::EMPTY;
        // SAFETY: `raw` は開いたもの。1 回だけ解放（走っている子は取り消される）
        unsafe {
            assert_eq!(
                ikari_settings_fetch_sha256(raw, &mut out),
                IKARI_SHELL_ERR_FAILED
            );
            ikari_settings_fetch_free(raw);
        }
    }

    #[test]
    fn error_line_is_reported_as_failure_text() {
        let raw = open(&Scripted("progress 5 10\nerror checksum\n"));
        let (status, text) = poll_until_finished(raw);
        assert_eq!(status.phase, Phase::Failed.code());
        assert_eq!(
            (status.received, status.total, status.has_total),
            (5, 10, 1)
        );
        assert_eq!(text, fetch_error(FetchErrorCode::Checksum));
        // SAFETY: 1 回だけ解放
        unsafe { ikari_settings_fetch_free(raw) };
    }

    #[test]
    #[cfg(unix)]
    fn child_dying_without_a_final_line_is_a_failure() {
        let raw = open(&Shell("echo progress 1 2; exit 9"));
        let (status, _) = poll_until_finished(raw);
        assert_eq!(status.phase, Phase::Failed.code());
        // SAFETY: 1 回だけ解放
        unsafe { ikari_settings_fetch_free(raw) };
    }

    #[test]
    #[cfg(unix)]
    fn cancel_kills_a_running_child() {
        let raw = open(&Shell("echo progress 1 2; exec sleep 60"));
        // SAFETY: `raw` は開いたもの
        unsafe { assert_eq!(ikari_settings_fetch_cancel(raw), IKARI_SHELL_OK) };
        let (status, _) = poll_until_finished(raw);
        assert_eq!(status.phase, Phase::Cancelled.code());
        // SAFETY: 取り消しのあとに 1 回だけ解放
        unsafe { ikari_settings_fetch_free(raw) };
    }
}
