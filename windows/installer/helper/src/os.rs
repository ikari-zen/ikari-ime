//! OS 層（`cfg(windows)`。Windows の CI か実機で確かめる）。記述子を読む、プロセスを探して止める、エラー報告の除外。
//!
//! `unsafe` はこのファイルの Win32 の呼び出しだけ。規則（何を問題とするか、だれを止めるか）は `acl`・`stop`・`tree`。

use std::io;
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::ptr;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_SUCCESS, FALSE, HANDLE, HLOCAL, INVALID_HANDLE_VALUE, LUID, LocalFree,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
    AdjustTokenPrivileges, DACL_SECURITY_INFORMATION, GetAce, GetAclInformation,
    GetTokenInformation, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, OWNER_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SE_DEBUG_NAME, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES,
    TOKEN_PRIVILEGES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ErrorReporting::{
    WerAddExcludedApplication, WerRemoveExcludedApplication,
};
use windows_sys::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories,
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::{
    EVENT_MODIFY_STATE, GetCurrentProcess, GetCurrentProcessId, OpenEventW, OpenProcess,
    OpenProcessToken, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE, QueryFullProcessImageNameW, SetEvent, TerminateProcess, WaitForSingleObject,
};

use ikari_setup_helper::acl::{Ace, Descriptor};
use ikari_setup_helper::stop::{
    CHILD_EXES, ENGINE_EXE, FORCED_EXIT_CODE, Outcome, SIGNAL_WAIT_MS, TERMINATE_WAIT_MS,
    event_name_for, exe_path, same_path,
};

/// ACE の種類（winnt.h の ACCESS_ALLOWED_ACE_TYPE、ACCESS_DENIED_ACE_TYPE）
const ALLOWED_ACE_TYPE: u8 = 0;
const DENIED_ACE_TYPE: u8 = 1;
/// QueryFullProcessImageNameW の受け皿（長いパスも受ける）
const IMAGE_PATH_CAPACITY: usize = 32_768;

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 持っているハンドル（落ちるときに 1 回だけ閉じる）
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: 自分が開いた有効なハンドルを 1 回だけ閉じる
        unsafe { CloseHandle(self.0) };
    }
}

/// OS が LocalAlloc したものの解放
struct LocalMem(*mut core::ffi::c_void);

impl Drop for LocalMem {
    fn drop(&mut self) {
        // SAFETY: OS が LocalAlloc したものを 1 回だけ解放する
        unsafe { LocalFree(self.0 as HLOCAL) };
    }
}

/// 以後の DLL の読み込みを system32 に限る（ADR 0067 §4.4 と同じ考え。管理者で動くので、なおさら）。
pub fn restrict_dll_search() {
    // SAFETY: 引数は定数。失敗しても既定の探索のまま続ける（起動時の取り込みは済んでいる）
    unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) };
}

/// SID を `S-1-…` の文字列に
fn sid_string(sid: PSID) -> io::Result<String> {
    let mut text: *mut u16 = ptr::null_mut();
    // SAFETY: sid は呼び出し側が持つ有効な SID。text には成功したときだけ LocalAlloc した文字列が書かれる
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == FALSE {
        return Err(io::Error::last_os_error());
    }
    let _free = LocalMem(text.cast());
    let mut len = 0usize;
    // SAFETY: text は NUL で終わる UTF-16（ConvertSidToStringSidW の約束）。NUL より先は読まない
    while unsafe { *text.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: text から len 個は上で読めた範囲で、_free が落ちるまで生きている
    let units = unsafe { std::slice::from_raw_parts(text, len) };
    String::from_utf16(units).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
}

/// ファイルかフォルダの持ち主と DACL を読む（リンクかどうかは std の属性で見る。リンクの先は読まない）。
pub fn read_descriptor(path: &Path) -> io::Result<Descriptor> {
    let reparse_point =
        std::fs::symlink_metadata(path)?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    if reparse_point {
        // 先を読まない（行き先の ACL は意味がない）。リンクであることだけで失敗になる
        return Ok(Descriptor {
            owner: None,
            dacl: Some(Vec::new()),
            reparse_point,
        });
    }
    let wide = to_wide(&path.to_string_lossy());
    let mut owner: PSID = ptr::null_mut();
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: wide は NUL で終わる UTF-16。owner と dacl は descriptor の中を指し、descriptor は下の LocalMem が解放する
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let _free = LocalMem(descriptor);
    let owner = if owner.is_null() {
        None
    } else {
        sid_string(owner).ok()
    };
    let dacl = if dacl.is_null() {
        None
    } else {
        Some(read_aces(dacl)?)
    };
    Ok(Descriptor {
        owner,
        dacl,
        reparse_point,
    })
}

fn read_aces(acl: *mut ACL) -> io::Result<Vec<Ace>> {
    let mut info = ACL_SIZE_INFORMATION {
        AceCount: 0,
        AclBytesInUse: 0,
        AclBytesFree: 0,
    };
    // SAFETY: acl は GetNamedSecurityInfoW が返した有効な ACL。info は書ける大きさの構造体
    let ok = unsafe {
        GetAclInformation(
            acl,
            (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    };
    if ok == FALSE {
        return Err(io::Error::last_os_error());
    }
    let mut aces = Vec::with_capacity(info.AceCount as usize);
    for index in 0..info.AceCount {
        let mut slot = core::mem::MaybeUninit::<*mut core::ffi::c_void>::uninit();
        // SAFETY: index は AceCount より小さい。成功したとき、slot には ACL の中の ACE の先頭が書かれる
        if unsafe { GetAce(acl, index, slot.as_mut_ptr()) } == FALSE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: GetAce が成功したので、slot は書かれた
        let raw = unsafe { slot.assume_init() };
        // GetAce が成功すれば raw は書かれるが、使う前に空でないことを確かめる
        if raw.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "GetAce returned a null ACE",
            ));
        }
        // SAFETY: どの ACE も ACE_HEADER で始まる。raw は上で空でないと確かめた
        let header = unsafe { *(raw as *const ACE_HEADER) };
        let ace = match header.AceType {
            ALLOWED_ACE_TYPE => {
                // SAFETY: 種類が ACCESS_ALLOWED_ACE なので、その形。SidStart は ACE の中の SID の先頭
                let allowed = raw as *const ACCESS_ALLOWED_ACE;
                let mask = unsafe { (*allowed).Mask };
                let sid = unsafe { ptr::addr_of!((*allowed).SidStart) } as PSID;
                Ace::Allowed {
                    sid: sid_string(sid)?,
                    mask,
                    flags: header.AceFlags,
                }
            }
            DENIED_ACE_TYPE => Ace::Denied,
            other => Ace::Other { ace_type: other },
        };
        aces.push(ace);
    }
    Ok(aces)
}

/// このプロセスの SeDebugPrivilege を有効にする（ほかの利用者のエンジンを開いて止めるため。管理者のトークンに
/// あり、既定では無効）。有効にできなくても続ける（同じ利用者のエンジンは開ける）。
fn enable_debug_privilege() {
    let mut raw: HANDLE = ptr::null_mut();
    // SAFETY: GetCurrentProcess は疑似ハンドル。raw には成功したときだけトークンが書かれる
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES, &mut raw) } == FALSE
    {
        return;
    }
    let token = Owned(raw);
    let mut luid = LUID {
        LowPart: 0,
        HighPart: 0,
    };
    // SAFETY: 名前は定数の NUL 終わりの UTF-16
    if unsafe { LookupPrivilegeValueW(ptr::null(), SE_DEBUG_NAME, &mut luid) } == FALSE {
        return;
    }
    let privileges = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: SE_PRIVILEGE_ENABLED,
        }],
    };
    // SAFETY: token は調整の権利つきで開いた。privileges は呼び出しの間生きている
    unsafe {
        AdjustTokenPrivileges(
            token.0,
            FALSE,
            &privileges,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
}

/// 名前（`ENGINE_EXE` など、最後の部分）が一致するプロセスの番号
fn processes_named(file_name: &str) -> io::Result<Vec<u32>> {
    // SAFETY: 引数は定数
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let snapshot = Owned(snapshot);
    // SAFETY: PROCESSENTRY32W はすべて 0 で正しい値（dwSize は下で入れる）
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut pids = Vec::new();
    // SAFETY: snapshot は有効。entry は dwSize を入れた書ける構造体
    let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) } != FALSE;
    while more {
        let len = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
        if name.eq_ignore_ascii_case(file_name) {
            pids.push(entry.th32ProcessID);
        }
        // SAFETY: 同上
        more = unsafe { Process32NextW(snapshot.0, &mut entry) } != FALSE;
    }
    Ok(pids)
}

/// 開いたプロセスの実行ファイルのパス
fn image_path(process: HANDLE) -> Option<String> {
    let mut buf = vec![0u16; IMAGE_PATH_CAPACITY];
    let mut len = buf.len() as u32;
    // SAFETY: process は照会の権利つきで開いた。buf は len 個の u16 を書ける
    let ok = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len)
    };
    (ok != FALSE).then(|| String::from_utf16_lossy(&buf[..len as usize]))
}

/// プロセスのユーザーの SID
fn process_user_sid(process: HANDLE) -> Option<String> {
    let mut raw: HANDLE = ptr::null_mut();
    // SAFETY: process は有効。raw には成功したときだけトークンが書かれる
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw) } == FALSE {
        return None;
    }
    let token = Owned(raw);
    let mut needed = 0u32;
    // SAFETY: 大きさを聞くだけ
    unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut needed) };
    if (needed as usize) < size_of::<TOKEN_USER>() {
        return None;
    }
    let mut buf = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
    // SAFETY: buf は needed バイト以上で、ポインタの境界に揃っている
    let ok = unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buf.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    };
    if ok == FALSE {
        return None;
    }
    // SAFETY: 成功したので buf の先頭は TOKEN_USER。Sid は buf の中を指し、buf はこの関数の終わりまで生きている
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    sid_string(sid).ok()
}

/// `path` の exe で動いているプロセスを開く（番号の使い回しを避けるため、ハンドルを持ったまま比べる）。
/// 名前は合うが開けないものの数も返す（パスを確かめられないので、残っているものに数える）。
fn open_matching(path: &str, access: u32) -> io::Result<(Vec<(u32, Owned)>, usize)> {
    let file_name = path.rsplit('\\').next().unwrap_or(path);
    let mut opened = Vec::new();
    let mut unopenable = 0;
    for pid in processes_named(file_name)? {
        // SAFETY: 引数は値だけ。失敗は null
        let raw = unsafe { OpenProcess(access, FALSE, pid) };
        if raw.is_null() {
            unopenable += 1;
            continue;
        }
        let handle = Owned(raw);
        match image_path(handle.0) {
            Some(image) if same_path(&image, path) => opened.push((pid, handle)),
            Some(_) => {}
            None => unopenable += 1,
        }
    }
    Ok((opened, unopenable))
}

/// 合図する（開けなければ `false`。強制の終了に回る）
fn signal(pid: u32, process: HANDLE) -> bool {
    let Some(sid) = process_user_sid(process) else {
        return false;
    };
    let mut session = 0u32;
    // SAFETY: session は書ける u32
    if unsafe { ProcessIdToSessionId(pid, &mut session) } == FALSE {
        return false;
    }
    // SAFETY: 引数なし。自分のプロセスの番号
    let own_pid = unsafe { GetCurrentProcessId() };
    let mut own_session = 0u32;
    // SAFETY: own_session は書ける u32
    if unsafe { ProcessIdToSessionId(own_pid, &mut own_session) } == FALSE {
        return false;
    }
    let Some(name) = event_name_for(&sid, session, own_session) else {
        return false;
    };
    let wide = to_wide(&name);
    // SAFETY: wide は NUL で終わる UTF-16。合図の権利だけで開く
    let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, FALSE, wide.as_ptr()) };
    if event.is_null() {
        return false;
    }
    let event = Owned(event);
    // SAFETY: event は合図の権利つきで開いた
    unsafe { SetEvent(event.0) != FALSE }
}

/// 期限までに終わるのを待つ。終わったら `true`
fn wait_until(process: HANDLE, deadline: Instant) -> bool {
    let left = deadline.saturating_duration_since(Instant::now());
    let ms = u32::try_from(left.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: process は同期の権利つきで開いた
    unsafe { WaitForSingleObject(process, ms) == WAIT_OBJECT_0 }
}

/// 残っているものを強制的に終わらせ、消えるのを待つ。消えなかった数を返す
fn terminate(handles: &[&Owned]) -> (usize, usize) {
    let mut terminated = 0;
    for handle in handles {
        // SAFETY: handle は終了の権利つきで開いた
        if unsafe { TerminateProcess(handle.0, FORCED_EXIT_CODE) } != FALSE {
            terminated += 1;
        }
    }
    let deadline = Instant::now() + Duration::from_millis(u64::from(TERMINATE_WAIT_MS));
    let remaining = handles
        .iter()
        .filter(|h| !wait_until(h.0, deadline))
        .count();
    (terminated, remaining)
}

/// インストール先のエンジンを、すべてのセッションで止める（ADR 0068 §6.2）。そのあと子を終わらせる。
pub fn stop_engines(install_dir: &str) -> io::Result<Outcome> {
    enable_debug_privilege();
    let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE;
    let mut outcome = Outcome::default();

    let (engines, unopenable) = open_matching(&exe_path(install_dir, ENGINE_EXE), access)?;
    outcome.found = engines.len() + unopenable;
    outcome.remaining += unopenable;
    for (pid, handle) in &engines {
        if signal(*pid, handle.0) {
            outcome.signaled += 1;
        }
    }
    let deadline = Instant::now() + Duration::from_millis(u64::from(SIGNAL_WAIT_MS));
    let mut left = Vec::new();
    for (_, handle) in &engines {
        if wait_until(handle.0, deadline) {
            outcome.exited += 1;
        } else {
            left.push(handle);
        }
    }
    let (terminated, remaining) = terminate(&left);
    outcome.terminated += terminated;
    outcome.remaining += remaining;

    for child in CHILD_EXES {
        let (children, unopenable) = open_matching(&exe_path(install_dir, child), access)?;
        outcome.remaining += unopenable;
        let refs: Vec<&Owned> = children.iter().map(|(_, h)| h).collect();
        let (terminated, remaining) = terminate(&refs);
        outcome.terminated += terminated;
        outcome.remaining += remaining;
    }
    Ok(outcome)
}

/// AppContainer のプロファイルを消す（動かした利用者のもの。無くても成功。ADR 0072 §12.5）。失敗は HRESULT
pub fn delete_appcontainer_profile(name: &str) -> Result<(), i32> {
    let wide = to_wide(name);
    // SAFETY: wide は NUL で終わる UTF-16
    let hr = unsafe {
        windows_sys::Win32::Security::Isolation::DeleteAppContainerProfile(wide.as_ptr())
    };
    if hr >= 0 { Ok(()) } else { Err(hr) }
}

/// OS のエラー報告の対象から外す・戻す（機械全体。ADR 0073 §4）。失敗は HRESULT
pub fn error_reporting(exe_name: &str, exclude: bool) -> Result<(), i32> {
    let wide = to_wide(exe_name);
    const ALL_USERS: i32 = 1;
    // SAFETY: wide は NUL で終わる UTF-16
    let hr = unsafe {
        if exclude {
            WerAddExcludedApplication(wide.as_ptr(), ALL_USERS)
        } else {
            WerRemoveExcludedApplication(wide.as_ptr(), ALL_USERS)
        }
    };
    if hr >= 0 { Ok(()) } else { Err(hr) }
}
