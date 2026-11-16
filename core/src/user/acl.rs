//! Windows のデータのフォルダの権限（ADR 0073 §3）。
//!
//! 継承を受けない ACL（本人、SYSTEM、組み込みの管理者だけ）をフォルダに付ける。中のファイルとフォルダは
//! 継承で同じ ACL になる。SDDL の組み立て（`private_dir_sddl`）は OS に依存しない。OS の呼び出しは
//! `cfg(windows)` の `os` だけ。Unix は 0700 / 0600（`util.rs`）で、ここは使わない。

/// 継承を受けず（`P`）、自動継承の印（`AI`）を付ける DACL の前置き
const DACL_PREFIX: &str = "D:PAI";
/// 全権（`FILE_ALL_ACCESS`）
const FULL_ACCESS: &str = "FA";
/// 子のファイルとフォルダへの継承の印（`OI` = オブジェクト、`CI` = コンテナ）
const INHERIT_FLAGS: &str = "OICI";
/// SDDL の別名: SYSTEM
const SYSTEM_ALIAS: &str = "SY";
/// SDDL の別名: 組み込みの管理者（Administrators）
const ADMINISTRATORS_ALIAS: &str = "BA";

/// フォルダに付ける DACL の SDDL。本人（`user_sid`）、SYSTEM、管理者だけに全権を許す。
///
/// ほかのユーザー、Everyone、匿名からは開けない（ACE が無いものは拒否される）。
/// `user_sid` は `S-` で始まり、英数字と `-` だけのものだけを受ける（SDDL の区切りを作らせない）。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn private_dir_sddl(user_sid: &str) -> Option<String> {
    let valid = user_sid.starts_with("S-")
        && user_sid.len() > "S-".len()
        && user_sid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !valid {
        return None;
    }
    let ace = |who: &str| format!("(A;{INHERIT_FLAGS};{FULL_ACCESS};;;{who})");
    Some(format!(
        "{DACL_PREFIX}{}{}{}",
        ace(user_sid),
        ace(SYSTEM_ALIAS),
        ace(ADMINISTRATORS_ALIAS)
    ))
}

#[cfg(windows)]
pub(crate) use os::secure_dir;

#[cfg(windows)]
mod os {
    use std::collections::HashSet;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::{Path, PathBuf};
    use std::ptr;
    use std::sync::{Mutex, PoisonError};

    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, FALSE, HANDLE, HLOCAL, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
    };
    use windows_sys::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, GetTokenInformation,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    use super::private_dir_sddl;

    /// このプロセスで、ACL を付け終えたフォルダ（書くたびに付け直さないため）
    static SECURED: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

    /// `LocalAlloc` された領域の持ち主（落ちたら `LocalFree`）
    struct LocalMem(*mut core::ffi::c_void);

    impl Drop for LocalMem {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: OS が LocalAlloc した領域で、ここで 1 度だけ解放する。
                unsafe { LocalFree(self.0 as HLOCAL) };
            }
        }
    }

    /// `path` のフォルダに、本人・SYSTEM・管理者だけの継承しない ACL を付ける（既にあれば付け直す）。
    /// 同じプロセスでは、同じフォルダに 1 度だけ付ける。失敗したら `Err`（付いたことを確かめられない）。
    pub(crate) fn secure_dir(path: &Path) -> io::Result<()> {
        {
            let guard = SECURED.lock().unwrap_or_else(PoisonError::into_inner);
            if guard.as_ref().is_some_and(|set| set.contains(path)) {
                return Ok(());
            }
        }
        apply(path)?;
        SECURED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(HashSet::new)
            .insert(path.to_path_buf());
        Ok(())
    }

    fn apply(path: &Path) -> io::Result<()> {
        let sid = current_user_sid()?;
        let sddl = private_dir_sddl(&sid).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "SID is not in the expected form",
            )
        })?;
        let wide_sddl = to_wide_str(&sddl);
        let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: wide_sddl は NUL で終わり、この呼び出しの間生きている。sd には OS が LocalAlloc した
        // 記述子を書く（解放は下の LocalMem）。大きさは要らないので null。
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide_sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                ptr::null_mut(),
            )
        };
        if ok == FALSE {
            return Err(io::Error::last_os_error());
        }
        let _sd_guard = LocalMem(sd);

        let mut present = FALSE;
        let mut defaulted = FALSE;
        let mut dacl: *mut ACL = ptr::null_mut();
        // SAFETY: sd は上の有効な記述子（_sd_guard が生きている）。dacl は sd の中を指し、下で使い終わるまで
        // sd は解放されない。
        let ok = unsafe { GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted) };
        if ok == FALSE {
            return Err(io::Error::last_os_error());
        }
        if present == FALSE || dacl.is_null() {
            // NULL の DACL は「全員に全権」。付けてはならない
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "DACL is missing",
            ));
        }

        let wide_path = to_wide_os(path);
        // SAFETY: wide_path は NUL で終わり、この呼び出しの間生きている。dacl は有効な ACL。
        // PROTECTED で、親からの継承を切る。所有者、グループ、SACL は変えない（null）。
        let status = unsafe {
            SetNamedSecurityInfoW(
                wide_path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                dacl,
                ptr::null(),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        Ok(())
    }

    fn to_wide_str(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn to_wide_os(p: &Path) -> Vec<u16> {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// このプロセスのユーザーの SID（`S-1-5-21-…`）。
    fn current_user_sid() -> io::Result<String> {
        let mut raw_token: HANDLE = ptr::null_mut();
        // SAFETY: GetCurrentProcess は閉じなくてよい疑似ハンドルを返す。raw_token には開いたトークンが
        // 書かれる（成功したときだけ。所有は下の OwnedHandle）。
        let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw_token) };
        if ok == FALSE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: 成功したので raw_token は開いたハンドルで、ここで所有を移す。
        let token = unsafe { OwnedHandle::from_raw_handle(raw_token) };

        let mut needed = 0u32;
        // SAFETY: 大きさを聞くだけ（バッファは null、長さ 0）。失敗（ERROR_INSUFFICIENT_BUFFER）で
        // needed が埋まる。
        unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                ptr::null_mut(),
                0,
                &mut needed,
            )
        };
        if (needed as usize) < size_of::<TOKEN_USER>() {
            return Err(io::Error::last_os_error());
        }
        // TOKEN_USER はポインタを含むので、ポインタの境界に揃ったバッファ（usize の並び）で受ける
        let mut buf = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
        // SAFETY: buf は needed バイト以上あり、usize の境界に揃っていて、この呼び出しの間生きている。
        let ok = unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                buf.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        };
        if ok == FALSE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: 成功したので、buf の先頭は TOKEN_USER（揃いと大きさは上で確かめた）。Sid は buf の中を
        // 指し、buf は下で使い終わるまで生きている。
        let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };

        let mut text: *mut u16 = ptr::null_mut();
        // SAFETY: sid は上の有効な SID。text には OS が LocalAlloc した文字列を書く（成功したときだけ。
        // 解放は下の LocalMem）。
        let ok = unsafe { ConvertSidToStringSidW(sid, &mut text) };
        if ok == FALSE {
            return Err(io::Error::last_os_error());
        }
        let _free = LocalMem(text.cast());
        let mut len = 0usize;
        // SAFETY: text は NUL で終わる UTF-16（ConvertSidToStringSidW の約束）で、_free が落ちるまで
        // 生きている。NUL より先は読まない。
        while unsafe { *text.add(len) } != 0 {
            len += 1;
        }
        // SAFETY: text から len 個は上で読めた範囲で、_free が落ちるまで生きていて、ほかから書かれない。
        let units = unsafe { std::slice::from_raw_parts(text, len) };
        String::from_utf16(units)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "SID is not valid UTF-16"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "S-1-5-21-1004336348-1177238915-682003330-1001";

    #[test]
    fn grants_only_user_system_and_administrators() {
        let sddl = private_dir_sddl(SID).unwrap();
        assert!(sddl.starts_with("D:P"), "{sddl}");
        assert_eq!(sddl.matches('(').count(), 3, "{sddl}");
        assert!(sddl.contains(&format!(";;;{SID})")));
        assert!(sddl.contains(";;;SY)"));
        assert!(sddl.contains(";;;BA)"));
        assert!(!sddl.contains(";;;WD)"), "Everyone は入れない: {sddl}");
    }

    #[test]
    fn rejects_sid_that_could_inject_sddl() {
        for bad in ["", "S-", "x", "S-1-5)(A;;FA;;;WD", "S-1-5-21 ", "S-1-あ"] {
            assert_eq!(private_dir_sddl(bad), None, "{bad:?}");
        }
    }
}
