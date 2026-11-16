//! Windows: 自分（`ikari-data`）を、**低い整合性**の子（`ikari-data transfer …`）として起こす（ADR 0076 §3.2）。
//!
//! 作りは `lm/src/host/os/windows.rs`（LM の子）と同じ。`ikari-data` は `lm` に依存しないので、必要な部分だけ
//! ここに置いている（共有の置き場所に移すかは、リードの判断待ち）。
//!
//! - トークンを複製して整合性を低くし（`WinLowLabelSid`）、`CreateProcessAsUserW` で起こす。
//! - 継承するハンドルは、標準出力のパイプだけ（`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`）。標準入力と標準エラーは渡さない。
//! - 環境は、プロキシの変数と OS が動くのに要るものだけ（`child::filter_env`）。
//! - 止めた状態で作り、Job Object に入れてから動かす。Job: 親のハンドルが閉じたら子も終わる。子の数の上限は 1。
//!   窓、クリップボード、デスクトップ、画面と設定、グローバルなアトム、サインアウトに触れられない。
//! - 窓を出さない。実行ファイルは絶対パスで明示し、PATH から探さない。シェルを通さない。

use std::ffi::OsString;
use std::io::{self, PipeReader};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
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
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
    GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList, OpenProcessToken,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

use crate::child::{command_line, filter_env};

/// Job の中のプロセスの上限（子そのもの 1 つ）
const MAX_ACTIVE_PROCESSES: u32 = 1;
/// Job の UI の制限（子は窓を持たない）
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
/// 止めた子の終了コード
const KILLED_EXIT_CODE: u32 = 1;

/// 起こした子。捨てると（Job を閉じると）中の子も終わる
pub struct LowChild {
    // 宣言順 = drop 順。先にプロセスのハンドルを閉じ、最後に Job を閉じる
    process: OwnedHandle,
    _job: Job,
}

impl LowChild {
    /// 終わるのを待って、終了コードを返す
    pub fn wait(&self) -> io::Result<u32> {
        let process = raw(&self.process);
        // SAFETY: `process` は、この値が持つ、まだ閉じていないプロセスのハンドル
        unsafe { WaitForSingleObject(process, INFINITE) };
        let mut code = 0u32;
        // SAFETY: 同じハンドルと、書き込める `code`
        check(unsafe { GetExitCodeProcess(process, &mut code) })?;
        Ok(code)
    }

    /// 止める（すでに終わっていても失敗にしない）
    pub fn kill(&self) {
        // SAFETY: 持っているプロセスのハンドル
        unsafe { TerminateProcess(raw(&self.process), KILLED_EXIT_CODE) };
    }
}

/// `program args…` を低い整合性で起こす。子の標準出力の読み取り側を返す
pub fn spawn_low(program: &Path, args: &[String]) -> io::Result<(LowChild, PipeReader)> {
    let app = wide_nul(program)?;
    let mut line = command_line(program, args)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad command line"))?;
    let cwd = program
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(wide_nul)
        .transpose()?;
    let mut env = environment_block(std::env::vars_os())?;

    let job = Job::confined()?;
    let token = low_integrity_token()?;

    // 子が書く側（標準出力）だけを、起動の直前に継承できるようにする。起動のあとはすぐ閉じる
    let (parent_out, child_out) = io::pipe()?;
    let child_out = OwnedHandle::from(child_out);
    let out_handle = raw(&child_out);
    inheritable(out_handle)?;
    let attrs = HandleList::new([out_handle])?;

    // SAFETY: すべて 0 の値は、この構造体として正しい（使う欄はこのあと埋める）
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    // 標準入力と標準エラーは渡さない（null）
    startup.StartupInfo.hStdOutput = out_handle;
    startup.lpAttributeList = attrs.as_ptr();

    let flags = CREATE_NO_WINDOW
        | CREATE_SUSPENDED
        | CREATE_UNICODE_ENVIRONMENT
        | EXTENDED_STARTUPINFO_PRESENT;
    let cwd_ptr = cwd.as_ref().map_or(ptr::null(), |c| c.as_ptr());
    // SAFETY: すべて 0 の値は、この構造体として正しい（成功したら OS が埋める）
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: 文字列と環境の領域は NUL で終わり、この呼び出しの間生きている。`line` は書き換えてよい配列。
    // 継承は有効にするが、`attrs` の一覧のハンドルだけが渡る。`startup` と `attrs` はこの呼び出しの間生きている
    let created = unsafe {
        CreateProcessAsUserW(
            raw(&token),
            app.as_ptr(),
            line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            flags,
            env.as_mut_ptr().cast(),
            cwd_ptr,
            &startup.StartupInfo,
            &mut info,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 成功した `CreateProcessAsUserW` が返した、まだ誰も持っていないハンドル
    let (process, thread) = unsafe {
        (
            OwnedHandle::from_raw_handle(info.hProcess),
            OwnedHandle::from_raw_handle(info.hThread),
        )
    };
    // 子の側の端は、親では閉じる（閉じないと、子が終わっても読み取りが終わらない）
    drop((attrs, child_out));
    let assigned = job.assign(&process);
    let child = LowChild { process, _job: job };
    if let Err(e) = assigned {
        child.kill();
        return Err(e);
    }
    // SAFETY: `thread` は、止めた状態で作った子の最初のスレッドのハンドル
    if unsafe { ResumeThread(raw(&thread)) } == u32::MAX {
        let e = io::Error::last_os_error();
        child.kill();
        return Err(e);
    }
    Ok((child, parent_out))
}

/// `name=value\0…\0\0` の UTF-16 の環境（`filter_env` で選んだものだけ）。空でも、終わりの NUL を 2 つ置く
fn environment_block(vars: impl Iterator<Item = (OsString, OsString)>) -> io::Result<Vec<u16>> {
    let mut block = Vec::new();
    for (name, value) in filter_env(vars) {
        let entry: Vec<u16> = name
            .encode_wide()
            .chain(std::iter::once(u16::from(b'=')))
            .chain(value.encode_wide())
            .collect();
        if entry.contains(&0) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "nul in env"));
        }
        block.extend(entry);
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle() as HANDLE
}

fn wide_nul(path: &Path) -> io::Result<Vec<u16>> {
    let mut out: Vec<u16> = path.as_os_str().encode_wide().collect();
    if out.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "nul in path"));
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

/// ハンドルを継承できるようにする（子に渡す 1 つにだけ使う）
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
    handles: Box<[HANDLE; 1]>,
}

impl HandleList {
    fn new(handles: [HANDLE; 1]) -> io::Result<HandleList> {
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
                size_of::<[HANDLE; 1]>(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn os(s: &str) -> OsString {
        OsString::from(s)
    }

    #[test]
    fn environment_block_is_double_nul_terminated() {
        let block = environment_block(
            [(os("HTTPS_PROXY"), os("http://p:1")), (os("PATH"), os("x"))].into_iter(),
        )
        .unwrap();
        let text = String::from_utf16(&block).unwrap();
        assert_eq!(text, "HTTPS_PROXY=http://p:1\0\0");
        let empty = environment_block(std::iter::empty()).unwrap();
        assert_eq!(empty, [0, 0]);
    }
}
