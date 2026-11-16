//! 低い整合性・AppContainer（能力なし）での子の起動（Windows。ADR 0075 §6、ADR 0072 §3）。
//!
//! `lm/src/host/os/windows.rs` の流儀に合わせる: 渡すハンドルは標準入出力の 3 つだけ
//! （`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`）、実行ファイルは絶対パス、窓を出さない。製品の子とは違い、
//! Job Object の絞りはかけない（測るのは「装置に触れるか」で、ほかの制限は段 1・段 2 の本体が持つ）。

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Read};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    ERROR_SUCCESS, HANDLE, HANDLE_FLAG_INHERIT, LocalFree, SetHandleInformation, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, NO_MULTIPLE_TRUSTEE, REVOKE_ACCESS,
    SE_FILE_OBJECT, SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_SID,
    TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::{
    ACL, CreateWellKnownSid, DACL_SECURITY_INFORMATION, DuplicateTokenEx, FreeSid,
    PSECURITY_DESCRIPTOR, PSID, SECURITY_CAPABILITIES, SECURITY_MAX_SID_SIZE, SID_AND_ATTRIBUTES,
    SUB_CONTAINERS_AND_OBJECTS_INHERIT, SecurityImpersonation, SetTokenInformation,
    TOKEN_ADJUST_DEFAULT, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_MANDATORY_LABEL,
    TOKEN_QUERY, TokenIntegrityLevel, TokenPrimary, WinLowLabelSid,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CreateProcessAsUserW, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetExitCodeProcess, INFINITE,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcessToken,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
    PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
};

use super::Captured;

/// 整合性のラベルを表す属性（winnt.h の `SE_GROUP_INTEGRITY`）
const SE_GROUP_INTEGRITY: u32 = 0x0000_0020;
/// `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)`
const HR_ALREADY_EXISTS: i32 = 0x8007_00B7_u32 as i32;
/// 止めた子の終了コード
const KILLED_EXIT_CODE: u32 = 1;
const NULL_DEVICE: &str = "NUL";
const STD_HANDLE_COUNT: usize = 3;

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().collect()
}

fn wide_nul(s: &OsStr) -> io::Result<Vec<u16>> {
    let mut out = wide(s);
    if out.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "nul in string"));
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

fn check_hr(hr: i32, what: &str) -> io::Result<()> {
    if hr < 0 {
        Err(io::Error::other(format!(
            "{what}: HRESULT 0x{:08x}",
            hr as u32
        )))
    } else {
        Ok(())
    }
}

fn check_win32(rc: u32, what: &str) -> io::Result<()> {
    if rc == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{what}: {}",
            io::Error::from_raw_os_error(rc as i32)
        )))
    }
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle() as HANDLE
}

/// コマンドラインの 1 語を引用符で囲む（`CommandLineToArgvW` の規則。引用符の前の `\` は倍にする）
fn quote(arg: &[u16], out: &mut Vec<u16>) {
    const QUOTE: u16 = b'"' as u16;
    const BACKSLASH: u16 = b'\\' as u16;
    out.push(QUOTE);
    let mut backslashes = 0usize;
    for &c in arg {
        if c == BACKSLASH {
            backslashes += 1;
            continue;
        }
        if c == QUOTE {
            out.extend(std::iter::repeat_n(BACKSLASH, backslashes * 2 + 1));
        } else {
            out.extend(std::iter::repeat_n(BACKSLASH, backslashes));
        }
        backslashes = 0;
        out.push(c);
    }
    out.extend(std::iter::repeat_n(BACKSLASH, backslashes * 2));
    out.push(QUOTE);
}

fn command_line(program: &Path, args: &[OsString]) -> io::Result<Vec<u16>> {
    let mut out = Vec::new();
    quote(&wide(program.as_os_str()), &mut out);
    for a in args {
        out.push(u16::from(b' '));
        quote(&wide(a), &mut out);
    }
    if out.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "nul in args"));
    }
    out.push(0);
    Ok(out)
}

/// 自分のトークンを複製し、整合性を低くしたもの（`lm/src/host/os/windows.rs` と同じ手順）
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

/// 属性の一覧（ハンドルの一覧と、AppContainer のときは能力の指定）。指す値も持つ
struct Attributes {
    buf: Vec<usize>,
    handles: Box<[HANDLE; STD_HANDLE_COUNT]>,
    caps: Option<Box<SECURITY_CAPABILITIES>>,
}

impl Attributes {
    fn new(handles: [HANDLE; STD_HANDLE_COUNT], container: Option<PSID>) -> io::Result<Self> {
        let count = if container.is_some() { 2 } else { 1 };
        let mut size = 0usize;
        // SAFETY: 大きさを聞くだけの呼び出し（失敗が返るのは決まっていて、`size` だけを使う）
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), count, 0, &mut size) };
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buf = vec![0usize; size.div_ceil(size_of::<usize>())];
        // SAFETY: `buf` は `size` バイト以上の、書き込める領域
        check(unsafe {
            InitializeProcThreadAttributeList(buf.as_mut_ptr().cast(), count, 0, &mut size)
        })?;
        let mut attrs = Attributes {
            buf,
            handles: Box::new(handles),
            caps: container.map(|sid| {
                Box::new(SECURITY_CAPABILITIES {
                    AppContainerSid: sid,
                    Capabilities: ptr::null_mut(),
                    CapabilityCount: 0,
                    Reserved: 0,
                })
            }),
        };
        // SAFETY: 初期化した一覧に属性を足す。指す値は `attrs` が持ち、一覧より長く生きる
        check(unsafe {
            UpdateProcThreadAttribute(
                attrs.as_ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                attrs.handles.as_ptr().cast(),
                size_of::<[HANDLE; STD_HANDLE_COUNT]>(),
                ptr::null_mut(),
                ptr::null(),
            )
        })?;
        if let Some(caps) = attrs.caps.as_mut() {
            let caps_ptr: *mut SECURITY_CAPABILITIES = &mut **caps;
            // SAFETY: 同上
            check(unsafe {
                UpdateProcThreadAttribute(
                    attrs.as_ptr(),
                    0,
                    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                    caps_ptr.cast(),
                    size_of::<SECURITY_CAPABILITIES>(),
                    ptr::null_mut(),
                    ptr::null(),
                )
            })?;
        }
        Ok(attrs)
    }

    fn as_ptr(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.buf.as_ptr().cast_mut().cast()
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: `new` で初期化した一覧を 1 回だけ破棄する
        unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
    }
}

/// `container` が無ければ低い整合性、あれば AppContainer（能力なし）で起動し、終わるまで待つ
pub(super) fn run(
    program: &Path,
    args: &[OsString],
    container: Option<&Profile>,
    timeout: Duration,
) -> io::Result<Captured> {
    let app = wide_nul(program.as_os_str())?;
    let mut cmd = command_line(program, args)?;
    let cwd = program
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| wide_nul(d.as_os_str()))
        .transpose()?;
    let token = match container {
        None => Some(low_integrity_token()?),
        Some(_) => None,
    };

    let (parent_out, child_out) = io::pipe()?;
    let child_out = OwnedHandle::from(child_out);
    let child_in = OwnedHandle::from(File::open(NULL_DEVICE)?);
    let child_err = OwnedHandle::from(File::options().write(true).open(NULL_DEVICE)?);
    let std_handles = [raw(&child_in), raw(&child_out), raw(&child_err)];
    for h in std_handles {
        // SAFETY: 開いたハンドルの印を変えるだけ
        check(unsafe { SetHandleInformation(h, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) })?;
    }
    let attrs = Attributes::new(std_handles, container.map(|p| p.sid))?;

    // SAFETY: すべて 0 の値は、この構造体として正しい（使う欄はこのあと埋める）
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = std_handles[0];
    startup.StartupInfo.hStdOutput = std_handles[1];
    startup.StartupInfo.hStdError = std_handles[2];
    startup.lpAttributeList = attrs.as_ptr();
    let flags = CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT;
    let cwd_ptr = cwd.as_ref().map_or(ptr::null(), |c| c.as_ptr());
    // SAFETY: すべて 0 の値は、この構造体として正しい（成功したら OS が埋める）
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: 文字列は NUL で終わり、この呼び出しの間生きている。`cmd` は書き換えてよい配列。
    // 継承は有効にするが、`attrs` の一覧のハンドルだけが渡る
    let created = unsafe {
        match &token {
            Some(token) => CreateProcessAsUserW(
                raw(token),
                app.as_ptr(),
                cmd.as_mut_ptr(),
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
                cmd.as_mut_ptr(),
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
    let (process, _thread) = unsafe {
        (
            OwnedHandle::from_raw_handle(info.hProcess),
            OwnedHandle::from_raw_handle(info.hThread),
        )
    };
    // 子の側の端は閉じる（閉じないと、子が終わっても読み取りが終わらない）
    drop((attrs, child_in, child_out, child_err));
    let mut reader = parent_out;
    let reading = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
        buf
    });
    let ms = u32::try_from(timeout.as_millis()).unwrap_or(INFINITE - 1);
    // SAFETY: まだ閉じていないプロセスのハンドル
    let waited = unsafe { WaitForSingleObject(raw(&process), ms) };
    let timed_out = waited == WAIT_TIMEOUT;
    if timed_out {
        // SAFETY: 同上
        unsafe {
            TerminateProcess(raw(&process), KILLED_EXIT_CODE);
            WaitForSingleObject(raw(&process), INFINITE);
        }
    }
    let mut code = 0u32;
    // SAFETY: 同上。`code` は書き込める変数
    let got = unsafe { GetExitCodeProcess(raw(&process), &mut code) };
    let stdout = reading.join().unwrap_or_default();
    Ok(Captured {
        code: (got != 0).then_some(code as i32),
        stdout,
        timed_out,
    })
}

/// フォルダの DACL に、`sid` の許可を足す（`GRANT_ACCESS`）か外す（`REVOKE_ACCESS`）。下のフォルダとファイルに継承させる
fn set_access(dir: &Path, sid: PSID, grant: bool) -> io::Result<()> {
    let path = wide_nul(dir.as_os_str())?;
    let mut old: *mut ACL = ptr::null_mut();
    let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: パスは NUL で終わる。書き込み先はこの関数の中の変数
    check_win32(
        unsafe {
            GetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut old,
                ptr::null_mut(),
                &mut sd,
            )
        },
        "GetNamedSecurityInfo",
    )?;
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: if grant {
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE
        } else {
            0
        },
        grfAccessMode: if grant { GRANT_ACCESS } else { REVOKE_ACCESS },
        grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
            ptstrName: sid.cast(),
        },
    };
    let mut new: *mut ACL = ptr::null_mut();
    // SAFETY: `entry` と `old`（`sd` の中）は、この呼び出しの間生きている
    let rc = unsafe { SetEntriesInAclW(1, &entry, old, &mut new) };
    let result = check_win32(rc, "SetEntriesInAcl").and_then(|()| {
        // SAFETY: パスは NUL で終わる。`new` は `SetEntriesInAclW` が作った ACL
        check_win32(
            unsafe {
                SetNamedSecurityInfoW(
                    path.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    new,
                    ptr::null(),
                )
            },
            "SetNamedSecurityInfo",
        )
    });
    // SAFETY: どちらも OS が `LocalAlloc` で確保したもの（無ければ NULL で、何もしない）
    unsafe {
        LocalFree(new.cast());
        LocalFree(sd);
    }
    result
}

/// AppContainer のプロファイルと、フォルダに足した許可
pub struct Profile {
    name: Vec<u16>,
    sid: PSID,
    dir: PathBuf,
}

impl Profile {
    pub fn create(name: &str, dir: &Path) -> io::Result<Profile> {
        let name_w = wide_nul(OsStr::new(name))?;
        let mut sid: PSID = ptr::null_mut();
        // SAFETY: 文字列は NUL で終わる。能力は無し（0 個）。書き込み先はこの関数の中の変数
        let mut hr = unsafe {
            CreateAppContainerProfile(
                name_w.as_ptr(),
                name_w.as_ptr(),
                name_w.as_ptr(),
                ptr::null(),
                0,
                &mut sid,
            )
        };
        if hr == HR_ALREADY_EXISTS {
            // SAFETY: 同上
            hr = unsafe { DeriveAppContainerSidFromAppContainerName(name_w.as_ptr(), &mut sid) };
        }
        check_hr(hr, "CreateAppContainerProfile")?;
        let profile = Profile {
            name: name_w,
            sid,
            dir: dir.to_path_buf(),
        };
        set_access(dir, sid, true)?;
        Ok(profile)
    }

    pub fn remove(self) -> io::Result<()> {
        let revoked = set_access(&self.dir, self.sid, false);
        // SAFETY: 文字列は NUL で終わる
        let hr = unsafe { DeleteAppContainerProfile(self.name.as_ptr()) };
        revoked.and(check_hr(hr, "DeleteAppContainerProfile"))
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        if !self.sid.is_null() {
            // SAFETY: `CreateAppContainerProfile` / `Derive…` が返した SID を 1 回だけ解放する
            unsafe { FreeSid(self.sid) };
        }
    }
}

/// 残ったプロファイルを消す。`dir` の許可も外す（無くても失敗にしない）
pub(super) fn cleanup(name: &str, dir: &Path) -> io::Result<bool> {
    let name_w = wide_nul(OsStr::new(name))?;
    let mut sid: PSID = ptr::null_mut();
    // SAFETY: 文字列は NUL で終わる。書き込み先はこの関数の中の変数
    let hr = unsafe { DeriveAppContainerSidFromAppContainerName(name_w.as_ptr(), &mut sid) };
    if hr >= 0 && !sid.is_null() {
        let _ = set_access(dir, sid, false);
        // SAFETY: `Derive…` が返した SID を 1 回だけ解放する
        unsafe { FreeSid(sid) };
    }
    // SAFETY: 文字列は NUL で終わる
    let hr = unsafe { DeleteAppContainerProfile(name_w.as_ptr()) };
    Ok(hr >= 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(s: &str) -> String {
        let mut out = Vec::new();
        quote(&s.encode_utf16().collect::<Vec<_>>(), &mut out);
        String::from_utf16(&out).unwrap()
    }

    #[test]
    fn quoting_follows_argv_rules() {
        assert_eq!(q("a b"), "\"a b\"");
        assert_eq!(q(r"C:\dir\"), r#""C:\dir\\""#);
        assert_eq!(q(r#"a"b"#), r#""a\"b""#);
    }
}
