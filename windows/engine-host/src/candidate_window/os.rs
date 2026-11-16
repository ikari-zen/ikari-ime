//! 候補窓の子の起動と待ち（`CreateProcessW`、Job Object）。`unsafe` はこのファイルに閉じる。
//!
//! - 実行ファイルは絶対パスを明示し、コマンドラインはパスを引用符で囲んだものだけ（引数なし。ADR 0067 §4.3）。
//!   カレントディレクトリは実行ファイルのフォルダ。ハンドルは 1 つも継承させない（`bInheritHandles = FALSE`）。
//! - 子は止めた状態で作り、Job に入れてから動かす。Job に入れられなければ、子を止めて失敗を返す。
//! - Job（engine-host が 1 つ持ち、起動し直した子も同じ Job に入れる）: 閉じたら中の子も終わる（ADR 0062 §5.4 の
//!   決定 4）。扱えない例外で落ちた子は、エラーの報告の窓を出さずに終わる（見張りが起動し直す）。クリップボード、
//!   画面の設定、システムの設定の書き換え、サインアウトに触れられない（候補窓は文字を描くだけ。ADR 0068 §5）。
//!   **窓を描くのに要るもの（デスクトップ、ほかのプロセスの窓のハンドル、グローバルなアトム）は制限しない**
//!   （WinUI 3 が使う。LM の子の Job とは違う。ADR 0072 §3）。
//!
//! 推測（実機で確かめる。`docs/windows-real-device-check.md`）: この UI の制限のもとで WinUI 3 の窓が描ける。
//! engine-host がほかの Job の中（端末や SSH から起動したとき）でも、入れ子の Job に入れられる。

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::PathBuf;
use std::ptr;
use std::sync::Arc;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS,
    JOB_OBJECT_UILIMIT_EXITWINDOWS, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOBOBJECTINFOCLASS,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, SetInformationJobObject,
    TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CreateProcessW, GetExitCodeProcess, INFINITE, PROCESS_INFORMATION,
    ResumeThread, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
};

use super::Launcher;

/// Job の UI の制限（上の説明のとおり。窓を描くのに要るものは入れない）
const UI_RESTRICTIONS: u32 = JOB_OBJECT_UILIMIT_READCLIPBOARD
    | JOB_OBJECT_UILIMIT_WRITECLIPBOARD
    | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
    | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
    | JOB_OBJECT_UILIMIT_EXITWINDOWS;
/// 止めた子と、Job を閉じるときの終了コード
const KILLED_EXIT_CODE: u32 = 1;
/// 引用符（コマンドライン）
const QUOTE: u16 = b'"' as u16;

fn raw(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle() as HANDLE
}

fn check(ok: windows_sys::core::BOOL) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn wide_nul(s: &OsStr) -> io::Result<Vec<u16>> {
    let mut out: Vec<u16> = s.encode_wide().collect();
    if out.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "nul in path"));
    }
    out.push(0);
    Ok(out)
}

/// 候補窓を入れる Job Object。閉じると（捨てると）中の子も終わる。
#[derive(Debug)]
pub struct Job(OwnedHandle);

// SAFETY: Job のハンドルは、どのスレッドから使ってもよい（OS のカーネルのオブジェクト）
unsafe impl Send for Job {}
// SAFETY: 同上。このハンドルに対する呼び出しは、OS が排他する
unsafe impl Sync for Job {}

impl Job {
    /// 制限をかけた Job を作る。
    pub fn new() -> io::Result<Job> {
        // SAFETY: 名前なし・既定のセキュリティで作る。戻り値は検査する
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: 成功した `CreateJobObjectW` が返したハンドル
        let job = Job(unsafe { OwnedHandle::from_raw_handle(handle) });
        // SAFETY: すべて 0 の値は、この構造体として正しい（制限なし）
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
        job.set(JobObjectExtendedLimitInformation, &limits)?;
        let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
            UIRestrictionsClass: UI_RESTRICTIONS,
        };
        job.set(JobObjectBasicUIRestrictions, &ui)?;
        Ok(job)
    }

    fn set<T>(&self, class: JOBOBJECTINFOCLASS, value: &T) -> io::Result<()> {
        // SAFETY: `value` はこの呼び出しの間生きていて、`class` に合う型で、大きさを正しく渡す
        check(unsafe {
            SetInformationJobObject(
                raw(&self.0),
                class,
                ptr::from_ref(value).cast(),
                size_of::<T>() as u32,
            )
        })
    }

    fn assign(&self, process: &OwnedHandle) -> io::Result<()> {
        // SAFETY: どちらも、まだ閉じていないハンドル
        check(unsafe { AssignProcessToJobObject(raw(&self.0), raw(process)) })
    }

    /// 中の子をすべて終わらせる（engine-host が見張りをやめるとき）。失敗しても、Job を閉じれば終わる。
    pub fn terminate(&self) {
        // SAFETY: まだ閉じていない Job のハンドル
        unsafe { TerminateJobObject(raw(&self.0), KILLED_EXIT_CODE) };
    }
}

/// 起動した子のプロセスのハンドル。
pub struct ChildProcess(OwnedHandle);

/// 候補窓の実行ファイルを、`job` に入れて起動する手段。
pub struct JobLauncher {
    exe: PathBuf,
    job: Arc<Job>,
}

impl JobLauncher {
    pub fn new(exe: PathBuf, job: Arc<Job>) -> Self {
        Self { exe, job }
    }
}

impl Launcher for JobLauncher {
    type Child = ChildProcess;

    fn launch(&mut self) -> io::Result<ChildProcess> {
        if !self.exe.is_file() {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        let app = wide_nul(self.exe.as_os_str())?;
        // パスに `"` があれば失敗にする（Windows のパスには使えない文字。囲めない）
        if app.contains(&QUOTE) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "quote in path"));
        }
        let mut command_line = Vec::with_capacity(app.len() + 2);
        command_line.push(QUOTE);
        command_line.extend_from_slice(&app[..app.len() - 1]);
        command_line.push(QUOTE);
        command_line.push(0);
        let cwd = self
            .exe
            .parent()
            .map(|dir| wide_nul(dir.as_os_str()))
            .transpose()?;
        let cwd_ptr = cwd.as_ref().map_or(ptr::null(), |c| c.as_ptr());

        // SAFETY: すべて 0 の値は、この構造体として正しい（使う欄はこのあと埋める）
        let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
        startup.cb = size_of::<STARTUPINFOW>() as u32;
        // SAFETY: すべて 0 の値は、この構造体として正しい（成功したら OS が埋める）
        let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: 文字列は NUL で終わり、この呼び出しの間生きている。`command_line` は書き換えてよい配列。
        // ハンドルは継承させない
        check(unsafe {
            CreateProcessW(
                app.as_ptr(),
                command_line.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                0,
                CREATE_SUSPENDED,
                ptr::null(),
                cwd_ptr,
                &startup,
                &mut info,
            )
        })?;
        // SAFETY: 成功した `CreateProcessW` が返した、まだ誰も持っていないハンドル
        let (process, thread) = unsafe {
            (
                OwnedHandle::from_raw_handle(info.hProcess),
                OwnedHandle::from_raw_handle(info.hThread),
            )
        };
        let stop = |process: &OwnedHandle| {
            // SAFETY: まだ閉じていないプロセスのハンドル
            unsafe {
                TerminateProcess(raw(process), KILLED_EXIT_CODE);
                WaitForSingleObject(raw(process), INFINITE);
            }
        };
        if let Err(e) = self.job.assign(&process) {
            stop(&process);
            return Err(e);
        }
        // SAFETY: `thread` は、止めた状態で作った子の最初のスレッドのハンドル
        if unsafe { ResumeThread(raw(&thread)) } == u32::MAX {
            let e = io::Error::last_os_error();
            stop(&process);
            return Err(e);
        }
        Ok(ChildProcess(process))
    }

    fn wait(&mut self, child: ChildProcess) -> Option<u32> {
        let process = raw(&child.0);
        // SAFETY: まだ閉じていないプロセスのハンドル（`child` が持つ）
        unsafe { WaitForSingleObject(process, INFINITE) };
        let mut code = 0u32;
        // SAFETY: `code` は書ける場所
        check(unsafe { GetExitCodeProcess(process, &mut code) })
            .ok()
            .map(|()| code)
    }
}
