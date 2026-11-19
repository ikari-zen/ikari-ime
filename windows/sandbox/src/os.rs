//! Windows の起動の本体（`CreateProcessW` / `CreateProcessAsUserW`）。作りはクレートの説明（`lib.rs`）。

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{
    HANDLE, HANDLE_FLAG_INHERIT, LocalFree, SetHandleInformation,
};
use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows_sys::Win32::Security::{
    CreateWellKnownSid, DuplicateTokenEx, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
    SECURITY_MAX_SID_SIZE, SID_AND_ATTRIBUTES, SecurityImpersonation, SetTokenInformation,
    TOKEN_ADJUST_DEFAULT, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_MANDATORY_LABEL,
    TOKEN_QUERY, TokenIntegrityLevel, TokenPrimary, WinLowLabelSid,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_UILIMIT_DESKTOP,
    JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOBOBJECTINFOCLASS,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW,
    CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
    GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList, OpenProcessToken,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

use crate::{Integrity, JobLimits, cmdline};

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
/// 止めた子の終了コード
const KILLED_EXIT_CODE: u32 = 1;

/// 子のプロセスとスレッドの DACL（SDDL）。SYSTEM と管理者だけに全権、持ち主の権利は読むだけ。親は作成時のハンドルを持つので困らない
const CHILD_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;RC;;;OW)";
/// `SDDL_REVISION_1`
const SDDL_REVISION: u32 = 1;

/// `CreateProcess*` に渡す、子のプロセスとスレッドのセキュリティ記述子。捨てると解放する
struct ChildSecurity {
    descriptor: PSECURITY_DESCRIPTOR,
    attributes: SECURITY_ATTRIBUTES,
}

impl ChildSecurity {
    fn new() -> io::Result<ChildSecurity> {
        let sddl = wide_nul(OsStr::new(CHILD_SDDL))?;
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: NUL で終わる文字列と、書き込める出力先
        check(unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION,
                &mut descriptor,
                ptr::null_mut(),
            )
        })?;
        Ok(ChildSecurity {
            descriptor,
            attributes: SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
        })
    }

    fn as_ptr(&self) -> *const SECURITY_ATTRIBUTES {
        &self.attributes
    }
}

impl Drop for ChildSecurity {
    fn drop(&mut self) {
        // SAFETY: `ConvertStringSecurityDescriptorToSecurityDescriptorW` が確保したものを 1 回だけ解放する
        unsafe { LocalFree(self.descriptor) };
    }
}

/// 子に渡す標準入出力。`None` は渡さない（子では null）。どれも、起動の間だけ継承できるようにする
#[derive(Clone, Copy, Default)]
pub struct Stdio<'a> {
    pub input: Option<BorrowedHandle<'a>>,
    pub output: Option<BorrowedHandle<'a>>,
    pub error: Option<BorrowedHandle<'a>>,
}

/// 起動の求め
pub struct Launch<'a> {
    /// 実行ファイルの絶対パス（PATH から探さない）
    pub program: &'a Path,
    pub args: &'a [OsString],
    /// 子の環境（`environment_block` の形）。`None` なら親の環境を受け継ぐ
    pub environment: Option<&'a [u16]>,
    pub integrity: Integrity,
    pub job: JobLimits,
    pub stdio: Stdio<'a>,
    /// 標準入出力のほかに、子に継承させるハンドル（ADR 0078 §4.1）。`PROC_THREAD_ATTRIBUTE_HANDLE_LIST` に標準入出力と
    /// 合わせて入れる。継承の印は起動の直前に付け、起動のあと（失敗でも）に外す。値は子でも同じなので、呼ぶ側が
    /// 引数で知らせる（ここは引数を作らない）。同じハンドルが 2 度ある、標準入出力と重なる → `InvalidInput`
    pub inherit: &'a [BorrowedHandle<'a>],
}

/// 起動した子。捨てると（Job を閉じると）中のプロセスが終わる。`Send` と `Sync`（別のスレッドから止めてよい）
pub struct Confined {
    // 宣言順 = drop 順。先にプロセスのハンドルを閉じ、最後に Job を閉じる
    process: OwnedHandle,
    pid: u32,
    job: Job,
}

impl Confined {
    pub fn id(&self) -> u32 {
        self.pid
    }

    /// 子のプロセスのハンドル（調べるため）
    pub fn process(&self) -> BorrowedHandle<'_> {
        self.process.as_handle()
    }

    /// 子がこの Job に入っているか（調べるため）
    pub fn in_job(&self) -> io::Result<bool> {
        let mut result = 0;
        // SAFETY: 持っている 2 つのハンドルと、書き込める `result`
        check(unsafe { IsProcessInJob(raw(&self.process), raw(&self.job.0), &mut result) })?;
        Ok(result != 0)
    }

    /// 子が終わるのを待って、終了コードを返す
    pub fn wait(&self) -> io::Result<u32> {
        let process = raw(&self.process);
        // SAFETY: `process` は、この値が持つ、まだ閉じていないプロセスのハンドル
        unsafe { WaitForSingleObject(process, INFINITE) };
        let mut code = 0u32;
        // SAFETY: 同じハンドルと、書き込める `code`
        check(unsafe { GetExitCodeProcess(process, &mut code) })?;
        Ok(code)
    }

    /// Job の中のプロセス（子と、その子）をすべて止める。何度呼んでもよい。すでに終わっていても失敗にしない
    pub fn terminate(&self) {
        // SAFETY: 持っている Job のハンドル
        unsafe { TerminateJobObject(raw(&self.job.0), KILLED_EXIT_CODE) };
    }

    /// 子を止めて、終わるのを待つ（起動の途中の失敗の片づけ）
    fn reap(&self) {
        let process = raw(&self.process);
        // SAFETY: `process` は、この値が持つ、まだ閉じていないプロセスのハンドル
        unsafe {
            TerminateProcess(process, KILLED_EXIT_CODE);
            WaitForSingleObject(process, INFINITE);
        }
    }
}

/// 子を、止めた状態で作り → Job に入れ → 動かす。継承するのは `launch.stdio` と `launch.inherit` のハンドルだけ
pub fn spawn(launch: &Launch<'_>) -> io::Result<Confined> {
    let program = launch.program.as_os_str();
    let app = wide_nul(program)?;
    let args: Vec<Vec<u16>> = launch.args.iter().map(|a| wide(a)).collect();
    let mut command_line = cmdline::command_line(&wide(program), &args)?;
    let cwd = launch
        .program
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| wide_nul(dir.as_os_str()))
        .transpose()?;
    let mut environment = launch.environment.map(<[u16]>::to_vec);
    if let Some(block) = &environment
        && !block.ends_with(&[0, 0])
    {
        return Err(invalid("environment block must end with two NULs"));
    }

    let stdio = launch.stdio;
    let std_handle =
        |h: Option<BorrowedHandle<'_>>| h.map_or(ptr::null_mut(), |h| h.as_raw_handle() as HANDLE);
    let (input, output, error) = (
        std_handle(stdio.input),
        std_handle(stdio.output),
        std_handle(stdio.error),
    );
    let extra: Vec<HANDLE> = launch
        .inherit
        .iter()
        .map(|h| h.as_raw_handle() as HANDLE)
        .collect();
    let inherited = inherited_handles([input, output, error], &extra)?;

    let job = Job::confined(&launch.job)?;
    let token = match launch.integrity {
        Integrity::Low => Some(low_integrity_token()?),
        Integrity::Inherit => None,
    };

    let marks = InheritMarks::set(&inherited)?;
    let attrs = if inherited.is_empty() {
        None
    } else {
        Some(HandleList::new(inherited.clone())?)
    };

    // SAFETY: すべて 0 の値は、この構造体として正しい（使う欄はこのあと埋める）
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = input;
    startup.StartupInfo.hStdOutput = output;
    startup.StartupInfo.hStdError = error;
    let mut flags = CREATE_NO_WINDOW | CREATE_SUSPENDED;
    if let Some(attrs) = &attrs {
        startup.lpAttributeList = attrs.as_ptr();
        flags |= EXTENDED_STARTUPINFO_PRESENT;
    }
    if environment.is_some() {
        flags |= CREATE_UNICODE_ENVIRONMENT;
    }
    // 一覧があるときだけ継承を有効にする（一覧のハンドルだけが渡る）
    let inherit = i32::from(attrs.is_some());
    let env_ptr = environment
        .as_mut()
        .map_or(ptr::null_mut(), |e| e.as_mut_ptr().cast());
    let cwd_ptr = cwd.as_ref().map_or(ptr::null(), |c| c.as_ptr());
    let security = ChildSecurity::new()?;
    // SAFETY: すべて 0 の値は、この構造体として正しい（成功したら OS が埋める）
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: 文字列と環境の領域は NUL で終わり、この呼び出しの間生きている。`command_line` は書き換えてよい配列。
    // `startup` と `attrs`（と、それが指すハンドルの配列）は、この呼び出しの間生きている
    let created = unsafe {
        match &token {
            Some(token) => CreateProcessAsUserW(
                raw(token),
                app.as_ptr(),
                command_line.as_mut_ptr(),
                security.as_ptr(),
                security.as_ptr(),
                inherit,
                flags,
                env_ptr,
                cwd_ptr,
                &startup.StartupInfo,
                &mut info,
            ),
            None => CreateProcessW(
                app.as_ptr(),
                command_line.as_mut_ptr(),
                security.as_ptr(),
                security.as_ptr(),
                inherit,
                flags,
                env_ptr,
                cwd_ptr,
                &startup.StartupInfo,
                &mut info,
            ),
        }
    };
    let created_error = (created == 0).then(io::Error::last_os_error);
    // 継承の印は、起動のすぐあとに外す（呼び出し側がハンドルを持ち続けても、ほかの起動に漏れない）
    drop((attrs, marks));
    if let Some(e) = created_error {
        return Err(e);
    }
    // SAFETY: 成功した `CreateProcess*` が返した、まだ誰も持っていないハンドル
    let (process, thread) = unsafe {
        (
            OwnedHandle::from_raw_handle(info.hProcess),
            OwnedHandle::from_raw_handle(info.hThread),
        )
    };
    let assigned = job.assign(&process);
    let child = Confined {
        process,
        pid: info.dwProcessId,
        job,
    };
    if let Err(e) = assigned {
        child.reap();
        return Err(e);
    }
    // SAFETY: `thread` は、止めた状態で作った子の最初のスレッドのハンドル
    if unsafe { ResumeThread(raw(&thread)) } == u32::MAX {
        let e = io::Error::last_os_error();
        child.reap();
        return Err(e);
    }
    Ok(child)
}

/// 継承させるハンドルの一覧。標準入出力どうしの重なりは 1 つにまとめる（同じパイプを標準出力と標準エラーに渡す形）。
/// `extra` に同じものが 2 度ある、`extra` が標準入出力と重なる、null がある → `InvalidInput`（一覧に同じハンドルを
/// 2 度入れると OS が拒むので、先に断る）
fn inherited_handles(stdio: [HANDLE; 3], extra: &[HANDLE]) -> io::Result<Vec<HANDLE>> {
    let mut inherited: Vec<HANDLE> = Vec::new();
    for handle in stdio {
        if !handle.is_null() && !inherited.contains(&handle) {
            inherited.push(handle);
        }
    }
    for &handle in extra {
        if handle.is_null() {
            return Err(invalid("null handle to inherit"));
        }
        if inherited.contains(&handle) {
            return Err(invalid("handle to inherit is repeated or overlaps stdio"));
        }
        inherited.push(handle);
    }
    Ok(inherited)
}

/// `name=value\0…\0\0` の UTF-16 の環境。空でも、終わりの NUL を 2 つ置く。名前か値に NUL があれば失敗
pub fn environment_block(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> io::Result<Vec<u16>> {
    let mut block = Vec::new();
    for (name, value) in vars {
        let entry: Vec<u16> = name
            .encode_wide()
            .chain(std::iter::once(u16::from(b'=')))
            .chain(value.encode_wide())
            .collect();
        if entry.contains(&0) {
            return Err(invalid("nul in env"));
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

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().collect()
}

fn wide_nul(s: &OsStr) -> io::Result<Vec<u16>> {
    let mut out = wide(s);
    if out.contains(&0) {
        return Err(invalid("nul in path"));
    }
    out.push(0);
    Ok(out)
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what)
}

fn check(ok: windows_sys::core::BOOL) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// 渡すハンドルに付けた継承の印。捨てると外す
struct InheritMarks<'a>(&'a [HANDLE]);

impl<'a> InheritMarks<'a> {
    fn set(handles: &'a [HANDLE]) -> io::Result<InheritMarks<'a>> {
        for (done, &handle) in handles.iter().enumerate() {
            // SAFETY: 呼び出し側が持つ、開いたハンドルの印を変えるだけ
            let ok =
                unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
            if ok == 0 {
                let e = io::Error::last_os_error();
                drop(InheritMarks(&handles[..done]));
                return Err(e);
            }
        }
        Ok(InheritMarks(handles))
    }
}

impl Drop for InheritMarks<'_> {
    fn drop(&mut self) {
        for &handle in self.0 {
            // SAFETY: `set` で印を付けた、まだ開いているハンドル（呼び出し側が持つ）
            unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
        }
    }
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
    // `Vec` と `Box` の中身は動かないので、値ごと動かしてよい
    _buf: Vec<usize>,
    /// `_buf` の先頭。作るときに可変の借用から一度だけ取る（共有の参照から可変のポインタを作らない）
    list: *mut core::ffi::c_void,
    handles: Box<[HANDLE]>,
}

impl HandleList {
    fn new(handles: Vec<HANDLE>) -> io::Result<HandleList> {
        let mut size = 0usize;
        // SAFETY: 大きさを聞くだけの呼び出し（失敗が返るのは決まっていて、`size` だけを使う）
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size) };
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buf = vec![0usize; size.div_ceil(size_of::<usize>())];
        let list_ptr: *mut core::ffi::c_void = buf.as_mut_ptr().cast();
        // SAFETY: `buf` は `size` バイト以上の、書き込める領域
        check(unsafe { InitializeProcThreadAttributeList(list_ptr, 1, 0, &mut size) })?;
        // 初期化に成功したので、ここから先の失敗では `Drop` が一覧を破棄する
        let list = HandleList {
            _buf: buf,
            list: list_ptr,
            handles: handles.into_boxed_slice(),
        };
        // SAFETY: 初期化した一覧に、属性を 1 つ足す。配列は `list` が持ち、一覧より長く生きる
        check(unsafe {
            UpdateProcThreadAttribute(
                list.as_ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                list.handles.as_ptr().cast(),
                size_of_val::<[HANDLE]>(&list.handles),
                ptr::null_mut(),
                ptr::null(),
            )
        })?;
        Ok(list)
    }

    fn as_ptr(&self) -> *mut core::ffi::c_void {
        self.list
    }
}

impl Drop for HandleList {
    fn drop(&mut self) {
        // SAFETY: `new` で初期化した一覧を 1 回だけ破棄する
        unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
    }
}

/// 自分のプロセスが入っている Job の UI の制限。Job に入っていない、または読めなければ 0（制限なしとみなし、
/// 新しい Job にかける側に倒す）
fn own_job_ui_restrictions() -> u32 {
    let mut ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
        UIRestrictionsClass: 0,
    };
    // SAFETY: null は「自分の Job」の意味。`ui` は書き込める、クラスに合う大きさの値
    let ok = unsafe {
        QueryInformationJobObject(
            ptr::null_mut(),
            JobObjectBasicUIRestrictions,
            ptr::from_mut(&mut ui).cast(),
            size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
            ptr::null_mut(),
        )
    };
    if ok == 0 { 0 } else { ui.UIRestrictionsClass }
}

/// 子を入れる Job Object。閉じると中のプロセスが終わる
struct Job(OwnedHandle);

impl Job {
    /// 制限をかけた Job を作る
    fn confined(limits: &JobLimits) -> io::Result<Job> {
        // SAFETY: 名前なし・既定のセキュリティで作る。戻り値は検査する
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: 成功した `CreateJobObjectW` が返したハンドル
        let job = Job(unsafe { OwnedHandle::from_raw_handle(handle) });

        // SAFETY: すべて 0 の値は、この構造体として正しい（制限なし）
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        info.BasicLimitInformation.ActiveProcessLimit = limits.max_processes;
        job.set(JobObjectExtendedLimitInformation, &info)?;
        let wanted = UI_RESTRICTIONS;
        // 自分が UI の制限つきの Job の中にいると、UI の制限つきの Job には入れられない（VM で
        // `ERROR_NOT_SUPPORTED`。ADR 0076 §17）。入れ子の Job の中のプロセスには外の Job の制限もかかるので、
        // 外の Job がすでに全部かけていれば、ここではかけない。一部だけなら、かけて、入れるところで失敗にする
        if own_job_ui_restrictions() & wanted != wanted {
            let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
                UIRestrictionsClass: wanted,
            };
            job.set(JobObjectBasicUIRestrictions, &ui)?;
        }
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
mod tests;
