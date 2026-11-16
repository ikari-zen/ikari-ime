//! Windows の子の起動（`CreateProcessW` / `CreateProcessAsUserW`）。LM の檻の段 1（ADR 0072 §3）。
//!
//! - 渡すハンドルは、標準入出力の 3 つだけ（`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`。ADR 0067 §4.3、0068 §6.1）。
//!   親のパイプの端は継承させない。
//! - 実行ファイルは絶対パスを明示し、コマンドラインはすべて引用符で囲む。カレントディレクトリは実行ファイルのフォルダ。
//! - 子は止めた状態で作り、Job Object に入れてから動かす。Job: 親のハンドルが閉じたら子も終わる。プロセスは子の
//!   1 つだけ（子は、さらにプロセスを作れない）。クリップボード、ほかのプロセスの窓、デスクトップ、画面と
//!   システムの設定、グローバルなアトム、サインアウトに触れられない。
//! - [`low_integrity_enabled`] のとき、子を低い整合性で起動する（利用者のファイルに書けない）。
//! - 窓を出さない（`CREATE_NO_WINDOW`）。
//! - どれか 1 つでもかけられなければ、子を止めて失敗を返す（呼び出し側は、LM なしで動く）。

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::process::{ChildStdin, ChildStdout};
use std::ptr;

use windows_sys::Win32::Foundation::{HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation};
use windows_sys::Win32::Security::{
    CreateWellKnownSid, DuplicateTokenEx, SECURITY_MAX_SID_SIZE, SID_AND_ATTRIBUTES,
    SecurityImpersonation, SetTokenInformation, TOKEN_ADJUST_DEFAULT, TOKEN_ASSIGN_PRIMARY,
    TOKEN_DUPLICATE, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel, TokenPrimary,
    WinLowLabelSid,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_UILIMIT_DESKTOP,
    JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOBOBJECTINFOCLASS,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CreateProcessAsUserW, CreateProcessW,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, INFINITE,
    InitializeProcThreadAttributeList, OpenProcessToken, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
};

use super::cmdline;

/// 段 1 の「低い整合性」（ADR 0072 §3・Q3）を使うか。実機で GPU が使えると確かめるまでは仮の決定。
/// 製品のビルドは環境変数を読まず、常に使う。開発用のビルド（`dev-knobs`）だけ、環境変数
/// （`placement::LOW_INTEGRITY_ENV`）が `0` なら使わない（ほかの絞りは変わらない）。GPU で読めない子は、
/// 今までどおり失敗に数えられ、CPU だけの子に切り替わる（`scorer`）
pub(super) fn low_integrity_enabled() -> bool {
    #[cfg(feature = "dev-knobs")]
    let value = std::env::var(crate::placement::LOW_INTEGRITY_ENV).ok();
    #[cfg(not(feature = "dev-knobs"))]
    let value: Option<String> = None;
    crate::placement::low_integrity(value.as_deref())
}

/// Job の中のプロセスの上限（子そのもの 1 つ）
const MAX_ACTIVE_PROCESSES: u32 = 1;
/// Job の UI の制限（ADR 0072 §3。子は窓を持たない）
const UI_RESTRICTIONS: u32 = JOB_OBJECT_UILIMIT_HANDLES
    | JOB_OBJECT_UILIMIT_READCLIPBOARD
    | JOB_OBJECT_UILIMIT_WRITECLIPBOARD
    | JOB_OBJECT_UILIMIT_GLOBALATOMS
    | JOB_OBJECT_UILIMIT_DESKTOP
    | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
    | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
    | JOB_OBJECT_UILIMIT_EXITWINDOWS;
/// 整合性のラベルを表す属性（winnt.h の `SE_GROUP_INTEGRITY`）
const SE_GROUP_INTEGRITY: u32 = 0x0000_0020;
/// 標準エラーの捨て先
const NULL_DEVICE: &str = "NUL";
/// 止めた子の終了コード
const KILLED_EXIT_CODE: u32 = 1;
/// 子に渡すハンドルの数（標準入力、標準出力、標準エラー）
const STD_HANDLE_COUNT: usize = 3;

/// 起動した子。Job を閉じると（捨てると）中の子も終わる
pub(super) struct Proc {
    // 宣言順 = drop 順。先にプロセスのハンドルを閉じ、最後に Job を閉じる
    process: OwnedHandle,
    pid: u32,
    _job: Job,
}

impl Proc {
    pub(super) fn id(&self) -> u32 {
        self.pid
    }

    /// 子を止めて回収する。すでに終わっていても失敗にしない
    pub(super) fn reap(&mut self) {
        let process = raw(&self.process);
        // SAFETY: `process` は、この値が持つ、まだ閉じていないプロセスのハンドル
        unsafe {
            TerminateProcess(process, KILLED_EXIT_CODE);
            WaitForSingleObject(process, INFINITE);
        }
    }

    #[cfg(test)]
    pub(super) fn process_handle(&self) -> HANDLE {
        raw(&self.process)
    }
}

pub(super) fn spawn(
    program: &Path,
    args: &[OsString],
) -> io::Result<(Proc, ChildStdin, ChildStdout)> {
    let app = wide_nul(program.as_os_str())?;
    let args: Vec<Vec<u16>> = args.iter().map(|a| a.encode_wide().collect()).collect();
    let mut command_line = cmdline::command_line(&wide(program.as_os_str()), &args)?;
    let cwd = program
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| wide_nul(dir.as_os_str()))
        .transpose()?;

    let job = Job::confined()?;
    let token = if low_integrity_enabled() {
        Some(low_integrity_token()?)
    } else {
        None
    };

    // 子が読む側（標準入力）と、子が書く側（標準出力）。どちらも継承できない形で作り（`CreatePipe`）、
    // 子の側の端だけを、起動の直前に継承できるようにする。起動のあとはすぐ閉じる
    let (child_in, parent_in) = io::pipe()?;
    let (parent_out, child_out) = io::pipe()?;
    let child_in = OwnedHandle::from(child_in);
    let child_out = OwnedHandle::from(child_out);
    let child_err = OwnedHandle::from(File::options().write(true).open(NULL_DEVICE)?);
    let std_handles = [raw(&child_in), raw(&child_out), raw(&child_err)];
    for handle in std_handles {
        inheritable(handle)?;
    }
    let attrs = HandleList::new(std_handles)?;

    // SAFETY: すべて 0 の値は、この構造体として正しい（使う欄はこのあと埋める）
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = std_handles[0];
    startup.StartupInfo.hStdOutput = std_handles[1];
    startup.StartupInfo.hStdError = std_handles[2];
    startup.lpAttributeList = attrs.as_ptr();

    let flags = CREATE_NO_WINDOW | CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT;
    let cwd_ptr = cwd.as_ref().map_or(ptr::null(), |c| c.as_ptr());
    // SAFETY: すべて 0 の値は、この構造体として正しい（成功したら OS が埋める）
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: 文字列は NUL で終わり、この呼び出しの間生きている。`command_line` は書き換えてよい配列。
    // 継承は有効にするが、`attrs` の一覧のハンドルだけが渡る。`startup` と `attrs` はこの呼び出しの間生きている
    let created = unsafe {
        match &token {
            Some(token) => CreateProcessAsUserW(
                raw(token),
                app.as_ptr(),
                command_line.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                1,
                flags,
                ptr::null(),
                cwd_ptr,
                &startup.StartupInfo,
                &mut info,
            ),
            None => CreateProcessW(
                app.as_ptr(),
                command_line.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                1,
                flags,
                ptr::null(),
                cwd_ptr,
                &startup.StartupInfo,
                &mut info,
            ),
        }
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 成功した `CreateProcess*` が返した、まだ誰も持っていないハンドル
    let (process, thread) = unsafe {
        (
            OwnedHandle::from_raw_handle(info.hProcess),
            OwnedHandle::from_raw_handle(info.hThread),
        )
    };
    // 子の側の端は、親では閉じる（閉じないと、子が終わっても読み取りが終わらない）
    drop((attrs, child_in, child_out, child_err));
    let assigned = job.assign(&process);
    let mut proc = Proc {
        process,
        pid: info.dwProcessId,
        _job: job,
    };
    if let Err(e) = assigned {
        proc.reap();
        return Err(e);
    }
    // SAFETY: `thread` は、止めた状態で作った子の最初のスレッドのハンドル
    if unsafe { ResumeThread(raw(&thread)) } == u32::MAX {
        let e = io::Error::last_os_error();
        proc.reap();
        return Err(e);
    }
    Ok((
        proc,
        ChildStdin::from(OwnedHandle::from(parent_in)),
        ChildStdout::from(OwnedHandle::from(parent_out)),
    ))
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle() as HANDLE
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().collect()
}

fn wide_nul(s: &OsStr) -> io::Result<Vec<u16>> {
    let mut out = wide(s);
    if out.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "nul in command line",
        ));
    }
    out.push(0);
    Ok(out)
}

fn check(ok: windows_sys::core::BOOL) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// ハンドルを継承できるようにする（子に渡す 3 つにだけ使う）
fn inheritable(handle: HANDLE) -> io::Result<()> {
    // SAFETY: 呼び出し側が持つ、開いたハンドルの印を変えるだけ
    check(unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) })
}

/// 自分のトークンを複製し、整合性を低くしたもの
fn low_integrity_token() -> io::Result<OwnedHandle> {
    let mut current: HANDLE = ptr::null_mut();
    // SAFETY: 自分のプロセスの擬似ハンドルと、書き込み先を渡すだけ
    check(unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_DUPLICATE | TOKEN_QUERY,
            &mut current,
        )
    })?;
    // SAFETY: 成功した `OpenProcessToken` が返したハンドル
    let current = unsafe { OwnedHandle::from_raw_handle(current) };
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: 開いたトークンと、書き込み先を渡すだけ
    check(unsafe {
        DuplicateTokenEx(
            raw(&current),
            TOKEN_ASSIGN_PRIMARY | TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ADJUST_DEFAULT,
            ptr::null(),
            SecurityImpersonation,
            TokenPrimary,
            &mut token,
        )
    })?;
    // SAFETY: 成功した `DuplicateTokenEx` が返したハンドル
    let token = unsafe { OwnedHandle::from_raw_handle(token) };

    // SID は 4 バイトの境界に置く
    let mut sid = [0u32; SECURITY_MAX_SID_SIZE as usize / size_of::<u32>()];
    let mut sid_len = SECURITY_MAX_SID_SIZE;
    // SAFETY: `sid` は `sid_len` バイトの書き込める領域
    check(unsafe {
        CreateWellKnownSid(
            WinLowLabelSid,
            ptr::null_mut(),
            sid.as_mut_ptr().cast(),
            &mut sid_len,
        )
    })?;
    let label = TOKEN_MANDATORY_LABEL {
        Label: SID_AND_ATTRIBUTES {
            Sid: sid.as_mut_ptr().cast(),
            Attributes: SE_GROUP_INTEGRITY,
        },
    };
    // SAFETY: `label` と、それが指す `sid` は、この呼び出しの間生きている
    check(unsafe {
        SetTokenInformation(
            raw(&token),
            TokenIntegrityLevel,
            ptr::from_ref(&label).cast(),
            size_of::<TOKEN_MANDATORY_LABEL>() as u32 + sid_len,
        )
    })?;
    Ok(token)
}

/// `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` だけを持つ属性の一覧。一覧が指すハンドルの配列も持つ
/// （OS は、一覧を捨てるまで配列を読む）。捨てると破棄する
struct HandleList {
    // `Vec` の中身は動かないので、値ごと動かしてよい
    buf: Vec<usize>,
    handles: Box<[HANDLE; STD_HANDLE_COUNT]>,
}

impl HandleList {
    fn new(handles: [HANDLE; STD_HANDLE_COUNT]) -> io::Result<HandleList> {
        let mut size = 0usize;
        // SAFETY: 大きさを聞くだけの呼び出し（失敗が返るのは決まっていて、`size` だけを使う）
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size) };
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buf = vec![0usize; size.div_ceil(size_of::<usize>())];
        // SAFETY: `buf` は `size` バイト以上の、書き込める領域
        check(unsafe {
            InitializeProcThreadAttributeList(buf.as_mut_ptr().cast(), 1, 0, &mut size)
        })?;
        let list = HandleList {
            buf,
            handles: Box::new(handles),
        };
        // SAFETY: 初期化した一覧に、属性を 1 つ足す。配列は `list` が持ち、一覧より長く生きる
        check(unsafe {
            UpdateProcThreadAttribute(
                list.as_ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                list.handles.as_ptr().cast(),
                size_of::<[HANDLE; STD_HANDLE_COUNT]>(),
                ptr::null_mut(),
                ptr::null(),
            )
        })?;
        Ok(list)
    }

    fn as_ptr(&self) -> *mut core::ffi::c_void {
        self.buf.as_ptr().cast_mut().cast()
    }
}

impl Drop for HandleList {
    fn drop(&mut self) {
        // SAFETY: `new` で初期化した一覧を 1 回だけ破棄する
        unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
    }
}

/// 子を入れる Job Object。閉じると中のプロセスが終わる
struct Job(OwnedHandle);

impl Job {
    /// 制限をかけた Job を作る
    fn confined() -> io::Result<Job> {
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
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        limits.BasicLimitInformation.ActiveProcessLimit = MAX_ACTIVE_PROCESSES;
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
}
