//! LM の子の AppContainer（檻の段 2。ADR 0072 §3、§12 の実測）。プロファイルと、読み取りの許可。
//!
//! - プロファイルは利用者ごとに 1 つ（名前は [`PROFILE_NAME`]）。無ければ作り、あれば SID を名前から求める。
//!   プロファイルが無いと、AppContainer の子は起動できない（VM で `ERROR_FILE_NOT_FOUND`。ADR 0072 §12）。
//!   消すのはアンインストールのとき（[`delete_profile`]。消すのは、呼んだ利用者のものだけ）。
//! - 能力（capability）は 1 つも付けない。ネットワーク、ほかのプロセスの口、利用者のファイルに届かない。
//! - 読み取りを許すのは、渡されたファイル 1 つ（継承しない ACE。`FILE_GENERIC_READ`）。何度呼んでも ACE は増えない
//!   （`GRANT_ACCESS` は同じ相手の ACE にまとめる）。
//! - 実行ファイルのフォルダは、ここでは許さない。製品のインストール先（Program Files の下）は、すべてのアプリの
//!   パッケージ（`ALL APPLICATION PACKAGES`）に読み取りと実行が開いている前提（ADR 0067。§12 の実測）。

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_SUCCESS, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT,
    SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, FreeSid, NO_INHERITANCE, PSECURITY_DESCRIPTOR, PSID,
};
use windows_sys::Win32::Storage::FileSystem::FILE_GENERIC_READ;
use windows_sys::core::HRESULT;

/// プロファイルの名前（利用者ごとに 1 つ。英数字と `.` `-` で 64 文字まで）
#[cfg(not(test))]
pub(super) const PROFILE_NAME: &str = "io.github.ikari-zen.lm-host";
/// 単体テストの名前（製品のプロファイルに触れない）
#[cfg(test)]
pub(super) const PROFILE_NAME: &str = "io.github.ikari-zen.lm-host-test";
/// 設定の画面などに出る名前（利用者に見える場所はない見込み）
const DISPLAY_NAME: &str = "ikari LM host";

/// `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)`
const HRESULT_ALREADY_EXISTS: HRESULT = (0x8007_0000u32 | ERROR_ALREADY_EXISTS) as HRESULT;

/// AppContainer の SID。捨てると解放する（`CreateAppContainerProfile` と
/// `DeriveAppContainerSidFromAppContainerName` が返す SID は、どちらも `FreeSid` で解放する）
pub(super) struct AppContainerSid(PSID);

impl AppContainerSid {
    pub(super) fn as_psid(&self) -> PSID {
        self.0
    }
}

impl Drop for AppContainerSid {
    fn drop(&mut self) {
        // SAFETY: この値だけが持つ、OS が割り当てた SID を 1 回だけ解放する
        unsafe { FreeSid(self.0) };
    }
}

fn wide_nul(s: &OsStr) -> io::Result<Vec<u16>> {
    let mut out: Vec<u16> = s.encode_wide().collect();
    if out.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "nul in name"));
    }
    out.push(0);
    Ok(out)
}

fn hresult(hr: HRESULT) -> io::Result<()> {
    if hr < 0 {
        Err(io::Error::from_raw_os_error(hr))
    } else {
        Ok(())
    }
}

fn win32(code: u32) -> io::Result<()> {
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code as i32))
    }
}

/// プロファイルを用意して、その SID を返す（無ければ作る）
pub(super) fn ensure_profile() -> io::Result<AppContainerSid> {
    let name = wide_nul(OsStr::new(PROFILE_NAME))?;
    let display = wide_nul(OsStr::new(DISPLAY_NAME))?;
    let mut sid: PSID = ptr::null_mut();
    // SAFETY: 文字列は NUL で終わり、この呼び出しの間生きている。能力は 0 個（一覧は null）
    let hr = unsafe {
        CreateAppContainerProfile(
            name.as_ptr(),
            display.as_ptr(),
            display.as_ptr(),
            ptr::null(),
            0,
            &mut sid,
        )
    };
    if hr == HRESULT_ALREADY_EXISTS {
        // SAFETY: NUL で終わる名前と、書き込み先を渡すだけ
        hresult(unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) })?;
    } else {
        hresult(hr)?;
    }
    if sid.is_null() {
        return Err(io::Error::other("appcontainer sid unavailable"));
    }
    Ok(AppContainerSid(sid))
}

/// プロファイルを消す（アンインストールのとき。無くても成功）。呼んだ利用者のものだけが消える
pub(super) fn delete_profile() -> io::Result<()> {
    let name = wide_nul(OsStr::new(PROFILE_NAME))?;
    // SAFETY: NUL で終わる名前を渡すだけ
    hresult(unsafe { DeleteAppContainerProfile(name.as_ptr()) })
}

/// `path`（ファイル 1 つ）に、`sid` の読み取りを許す ACE を足す。継承させない
pub(super) fn grant_read(path: &Path, sid: &AppContainerSid) -> io::Result<()> {
    let path = wide_nul(path.as_os_str())?;
    let mut old: *mut ACL = ptr::null_mut();
    let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: NUL で終わるパスと、書き込み先を渡すだけ。`old` は `sd` の中を指す
    win32(unsafe {
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
    })?;
    let sd = LocalMem(sd);
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_GENERIC_READ,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid.as_psid().cast(),
        },
    };
    let mut new: *mut ACL = ptr::null_mut();
    // SAFETY: `entry` が指す SID は `sid` が持ち、この呼び出しの間生きている。`old` は `sd` が生きている間有効
    win32(unsafe { SetEntriesInAclW(1, &entry, old, &mut new) })?;
    let new = LocalMem(new.cast());
    // SAFETY: `new` は `SetEntriesInAclW` が作った ACL。所有者などは変えない（DACL だけ）
    let set = win32(unsafe {
        SetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            new.0.cast(),
            ptr::null(),
        )
    });
    drop((new, sd));
    set
}

/// `LocalAlloc` で割り当てられたもの。捨てると `LocalFree` する
struct LocalMem(*mut core::ffi::c_void);

impl Drop for LocalMem {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: OS が `LocalAlloc` で割り当てて渡した領域を 1 回だけ解放する
            unsafe { LocalFree(self.0) };
        }
    }
}
