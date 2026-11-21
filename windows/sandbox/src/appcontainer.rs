//! AppContainer（檻の段 2。ADR 0072 §3、§12 の実測。ADR 0078 §3.2、§4.1）。プロファイル、能力の SID、読み取りの許可。
//!
//! - プロファイルは利用者ごとに 1 つずつ（名前は呼ぶ側が [`ContainerProfile`] で決める。LM の子と取得の子で分ける）。
//!   無ければ作り、あれば SID を名前から求める。プロファイルが無いと、AppContainer の子は起動できない（VM で
//!   `ERROR_FILE_NOT_FOUND`。ADR 0072 §12.5）。消すのはアンインストールのとき（[`delete_profile`]。消すのは、呼んだ
//!   利用者のものだけ）。
//! - 能力（capability）は、呼ぶ側が [`Capability`] で明示したものだけ（LM の子は 0 個、取得の子は
//!   `internetClient` だけ）。
//! - 読み取りを許すのは、渡されたファイル 1 つ（継承しない ACE。`FILE_GENERIC_READ`）。何度呼んでも ACE は増えない
//!   （`GRANT_ACCESS` は同じ相手の ACE にまとめる）。外すのは [`revoke_read`]。特定の AppContainer の SID の ACE が
//!   あると、AppContainer でない低い整合性のプロセスはそのファイルを読めない（ADR 0072 §12.5）。
//! - 実行ファイルのフォルダは、ここでは許さない。製品のインストール先（Program Files の下）は、すべてのアプリの
//!   パッケージ（`ALL APPLICATION PACKAGES`）に読み取りと実行が開いている前提（ADR 0067。ADR 0072 §12 の実測）。
//!
//! 起動の属性（`PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES`）は、`Launch::container` に渡すと `spawn` が組み立てる。
//! LM の子は、まだ自分で組み立てる（`lm/src/host/os/windows.rs`。ADR 0076 §17 の移行まで 2 か所）。

/// AppContainer のプロファイル（利用者ごと）。名前は英数字と `.` `-` で 64 文字まで
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContainerProfile {
    pub name: &'static str,
    /// 設定の画面などに出る名前（利用者に見える場所はない見込み）。説明にも同じものを使う
    pub display_name: &'static str,
}

/// 子に付ける能力
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    /// インターネットへの送信（`WinCapabilityInternetClientSid`）。LAN の中とループバックには届かない
    InternetClient,
}

#[cfg(windows)]
pub use imp::{Container, delete_profile, grant_read, revoke_read};

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr;

    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_SUCCESS, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ACCESS_MODE, EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, NO_MULTIPLE_TRUSTEE,
        REVOKE_ACCESS, SE_FILE_OBJECT, SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_SID,
        TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::Isolation::{
        CreateAppContainerProfile, DeleteAppContainerProfile,
        DeriveAppContainerSidFromAppContainerName,
    };
    use windows_sys::Win32::Security::{
        ACL, CreateWellKnownSid, DACL_SECURITY_INFORMATION, FreeSid, NO_INHERITANCE,
        PSECURITY_DESCRIPTOR, PSID, SECURITY_CAPABILITIES, SECURITY_MAX_SID_SIZE,
        SID_AND_ATTRIBUTES, WELL_KNOWN_SID_TYPE, WinCapabilityInternetClientSid,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_GENERIC_READ;
    use windows_sys::core::HRESULT;

    use super::{Capability, ContainerProfile};

    /// `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)`
    const HRESULT_ALREADY_EXISTS: HRESULT = (0x8007_0000u32 | ERROR_ALREADY_EXISTS) as HRESULT;
    /// 能力の SID の属性（winnt.h の `SE_GROUP_ENABLED`）
    const SE_GROUP_ENABLED: u32 = 0x0000_0004;

    /// AppContainer の SID と、能力の SID。捨てると解放する（`CreateAppContainerProfile` と
    /// `DeriveAppContainerSidFromAppContainerName` が返す SID は、どちらも `FreeSid` で解放する）
    pub struct Container {
        sid: PSID,
        /// 能力の SID の領域（4 バイトの境界に置く。`Box` の中身は動かないので、`attributes` が指してよい）
        _capability_sids: Vec<Box<[u32]>>,
        /// `SECURITY_CAPABILITIES` が指す能力の一覧
        attributes: Box<[SID_AND_ATTRIBUTES]>,
    }

    // SAFETY: 中身は OS が割り当てた SID と、自分が持つ領域だけ。どのスレッドから読んでも、解放は Drop の 1 回だけ
    unsafe impl Send for Container {}
    // SAFETY: 共有の参照からは読むだけ
    unsafe impl Sync for Container {}

    impl Container {
        /// プロファイルが無ければ作り、あれば名前から SID を求める。能力の SID を作る
        pub fn ensure(
            profile: &ContainerProfile,
            capabilities: &[Capability],
        ) -> io::Result<Container> {
            let mut sids: Vec<Box<[u32]>> = Vec::with_capacity(capabilities.len());
            for &capability in capabilities {
                sids.push(well_known_sid(capability_sid_type(capability))?);
            }
            let attributes: Box<[SID_AND_ATTRIBUTES]> = sids
                .iter_mut()
                .map(|sid| SID_AND_ATTRIBUTES {
                    Sid: sid.as_mut_ptr().cast(),
                    Attributes: SE_GROUP_ENABLED,
                })
                .collect();
            let sid = ensure_profile(profile, &attributes)?;
            Ok(Container {
                sid,
                _capability_sids: sids,
                attributes,
            })
        }

        /// AppContainer の SID（この値が生きている間だけ有効）
        pub fn as_psid(&self) -> PSID {
            self.sid
        }

        /// 起動の属性の値。中のポインタは、この値が生きている間だけ有効
        pub(crate) fn security_capabilities(&self) -> SECURITY_CAPABILITIES {
            SECURITY_CAPABILITIES {
                AppContainerSid: self.sid,
                Capabilities: if self.attributes.is_empty() {
                    ptr::null_mut()
                } else {
                    self.attributes.as_ptr().cast_mut()
                },
                CapabilityCount: self.attributes.len() as u32,
                Reserved: 0,
            }
        }
    }

    impl Drop for Container {
        fn drop(&mut self) {
            // SAFETY: この値だけが持つ、OS が割り当てた SID を 1 回だけ解放する
            unsafe { FreeSid(self.sid) };
        }
    }

    fn capability_sid_type(capability: Capability) -> WELL_KNOWN_SID_TYPE {
        match capability {
            Capability::InternetClient => WinCapabilityInternetClientSid,
        }
    }

    /// よく知られた SID（4 バイトの境界に置いた領域）
    fn well_known_sid(kind: WELL_KNOWN_SID_TYPE) -> io::Result<Box<[u32]>> {
        let mut sid =
            vec![0u32; SECURITY_MAX_SID_SIZE as usize / size_of::<u32>()].into_boxed_slice();
        let mut len = SECURITY_MAX_SID_SIZE;
        // SAFETY: `sid` は `len` バイトの書き込める領域
        let ok =
            unsafe { CreateWellKnownSid(kind, ptr::null_mut(), sid.as_mut_ptr().cast(), &mut len) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(sid)
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

    /// プロファイルを用意して、その SID を返す（無ければ、能力の一覧つきで作る）
    fn ensure_profile(
        profile: &ContainerProfile,
        capabilities: &[SID_AND_ATTRIBUTES],
    ) -> io::Result<PSID> {
        let name = wide_nul(OsStr::new(profile.name))?;
        let display = wide_nul(OsStr::new(profile.display_name))?;
        let mut sid: PSID = ptr::null_mut();
        let list = if capabilities.is_empty() {
            ptr::null()
        } else {
            capabilities.as_ptr()
        };
        // SAFETY: 文字列は NUL で終わり、この呼び出しの間生きている。能力の一覧（とそれが指す SID）も同じ
        let hr = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                display.as_ptr(),
                display.as_ptr(),
                list,
                capabilities.len() as u32,
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
        Ok(sid)
    }

    /// プロファイルを消す（アンインストールのとき。無くても成功）。呼んだ利用者のものだけが消える
    pub fn delete_profile(profile: &ContainerProfile) -> io::Result<()> {
        let name = wide_nul(OsStr::new(profile.name))?;
        // SAFETY: NUL で終わる名前を渡すだけ
        hresult(unsafe { DeleteAppContainerProfile(name.as_ptr()) })
    }

    /// `path`（ファイル 1 つ）に、`container` の読み取りを許す ACE を足す。継承させない
    pub fn grant_read(path: &Path, container: &Container) -> io::Result<()> {
        set_entry(path, container.as_psid(), GRANT_ACCESS)
    }

    /// `path` から、`profile` の SID の ACE を外す（無くても成功）。プロファイルが無くても SID は名前から決まる
    pub fn revoke_read(path: &Path, profile: &ContainerProfile) -> io::Result<()> {
        let name = wide_nul(OsStr::new(profile.name))?;
        let mut sid: PSID = ptr::null_mut();
        // SAFETY: NUL で終わる名前と、書き込み先を渡すだけ
        hresult(unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) })?;
        if sid.is_null() {
            return Err(io::Error::other("appcontainer sid unavailable"));
        }
        let sid = OwnedSid(sid);
        set_entry(path, sid.0, REVOKE_ACCESS)
    }

    /// `FreeSid` で解放する SID
    struct OwnedSid(PSID);

    impl Drop for OwnedSid {
        fn drop(&mut self) {
            // SAFETY: この値だけが持つ、OS が割り当てた SID を 1 回だけ解放する
            unsafe { FreeSid(self.0) };
        }
    }

    fn set_entry(path: &Path, sid: PSID, mode: ACCESS_MODE) -> io::Result<()> {
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
            grfAccessMode: mode,
            grfInheritance: NO_INHERITANCE,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: ptr::null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_UNKNOWN,
                ptstrName: sid.cast(),
            },
        };
        let mut new: *mut ACL = ptr::null_mut();
        // SAFETY: `entry` が指す SID は呼び出し側が持ち、この呼び出しの間生きている。`old` は `sd` が生きている間有効
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
}
