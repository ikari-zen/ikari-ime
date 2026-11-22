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

fn is_valid_sid(user_sid: &str) -> bool {
    user_sid.starts_with("S-")
        && user_sid.len() > "S-".len()
        && user_sid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn private_sddl(user_sid: &str, flags: &str) -> Option<String> {
    if !is_valid_sid(user_sid) {
        return None;
    }
    let ace = |who: &str| format!("(A;{flags};{FULL_ACCESS};;;{who})");
    Some(format!(
        "{DACL_PREFIX}{}{}{}",
        ace(user_sid),
        ace(SYSTEM_ALIAS),
        ace(ADMINISTRATORS_ALIAS)
    ))
}

/// フォルダに付ける DACL の SDDL。本人（`user_sid`）、SYSTEM、管理者だけに全権を許す。
///
/// ほかのユーザー、Everyone、匿名からは開けない（ACE が無いものは拒否される）。
/// `user_sid` は `S-` で始まり、英数字と `-` だけのものだけを受ける（SDDL の区切りを作らせない）。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn private_dir_sddl(user_sid: &str) -> Option<String> {
    private_sddl(user_sid, INHERIT_FLAGS)
}

/// 通常ファイルに付ける DACL の SDDL。本人（`user_sid`）、SYSTEM、管理者だけに全権を許す（継承フラグなし）。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn private_file_sddl(user_sid: &str) -> Option<String> {
    private_sddl(user_sid, "")
}

#[cfg(windows)]
pub(crate) use os::{secure_dir, tighten_dir};

#[cfg(windows)]
mod os {
    use std::collections::HashSet;
    use std::fs;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
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
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    use super::{private_dir_sddl, private_file_sddl};

    /// このプロセスで、ACL を付け終えたフォルダ（書くたびに付け直さないため）
    static SECURED: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

    /// `LocalAlloc` された領域の持ち主（落ちたら `LocalFree`）
    pub(super) struct LocalMem(pub(super) *mut core::ffi::c_void);

    impl Drop for LocalMem {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: OS が LocalAlloc した領域で、ここで 1 度だけ解放する。
                unsafe { LocalFree(self.0 as HLOCAL) };
            }
        }
    }

    /// SDDL から組み立てた DACL（`_sd` が生きている間 `dacl` は有効）。
    pub(super) struct ParsedDacl {
        _sd: LocalMem,
        dacl: *mut ACL,
    }

    impl ParsedDacl {
        pub(super) fn from_sddl(sddl: &str) -> io::Result<Self> {
            let wide_sddl = to_wide_str(sddl);
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
            let sd_guard = LocalMem(sd);

            let mut present = FALSE;
            let mut defaulted = FALSE;
            let mut dacl: *mut ACL = ptr::null_mut();
            // SAFETY: sd は上の有効な記述子（sd_guard が生きている）。dacl は sd の中を指し、
            // ParsedDacl が生きている間 sd は解放されない。
            let ok =
                unsafe { GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted) };
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
            Ok(Self {
                _sd: sd_guard,
                dacl,
            })
        }

        pub(super) fn apply(&self, path: &Path) -> io::Result<()> {
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
                    self.dacl,
                    ptr::null(),
                )
            };
            if status != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
            Ok(())
        }
    }

    fn is_reparse_point(meta: &fs::Metadata) -> bool {
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    fn current_private_sddl(builder: fn(&str) -> Option<String>) -> io::Result<String> {
        let sid = current_user_sid()?;
        builder(&sid).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "SID is not in the expected form",
            )
        })
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
        let dacl = ParsedDacl::from_sddl(&current_private_sddl(private_dir_sddl)?)?;
        dacl.apply(path)?;
        SECURED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(HashSet::new)
            .insert(path.to_path_buf());
        Ok(())
    }

    /// `dir` とその直下の通常ファイル・サブフォルダに、本人・SYSTEM・管理者だけの保護済み DACL を付け直す
    /// （best effort。既存の子に残る明示的な ACE も置き換える。リパースポイントは辿らない）。
    pub(crate) fn tighten_dir(dir: &Path) {
        let Ok(dir_meta) = fs::symlink_metadata(dir) else {
            return;
        };
        if is_reparse_point(&dir_meta) || !dir_meta.is_dir() {
            return;
        }
        let Ok(sid) = current_user_sid() else {
            return;
        };
        let (Some(dir_sddl), Some(file_sddl)) = (private_dir_sddl(&sid), private_file_sddl(&sid))
        else {
            return;
        };
        let (Ok(dir_dacl), Ok(file_dacl)) = (
            ParsedDacl::from_sddl(&dir_sddl),
            ParsedDacl::from_sddl(&file_sddl),
        ) else {
            return;
        };
        if dir_dacl.apply(dir).is_ok() {
            SECURED
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get_or_insert_with(HashSet::new)
                .insert(dir.to_path_buf());
        }
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if is_reparse_point(&meta) {
                continue;
            }
            if meta.is_file() {
                let _ = file_dacl.apply(&path);
            } else if meta.is_dir() {
                let _ = dir_dacl.apply(&path);
            }
        }
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
    pub(super) fn current_user_sid() -> io::Result<String> {
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
    fn file_sddl_grants_only_user_system_and_administrators_without_inheritance() {
        let sddl = private_file_sddl(SID).unwrap();
        assert!(sddl.starts_with("D:P"), "{sddl}");
        assert_eq!(sddl.matches('(').count(), 3, "{sddl}");
        assert!(!sddl.contains("OICI"), "{sddl}");
        assert!(sddl.contains(&format!("(A;;FA;;;{SID})")), "{sddl}");
        assert!(sddl.contains("(A;;FA;;;SY)"), "{sddl}");
        assert!(sddl.contains("(A;;FA;;;BA)"), "{sddl}");
        assert!(!sddl.contains(";;;WD)"), "Everyone は入れない: {sddl}");
    }

    #[test]
    fn rejects_sid_that_could_inject_sddl() {
        for bad in ["", "S-", "x", "S-1-5)(A;;FA;;;WD", "S-1-5-21 ", "S-1-あ"] {
            assert_eq!(private_dir_sddl(bad), None, "{bad:?}");
            assert_eq!(private_file_sddl(bad), None, "{bad:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn tighten_dir_removes_extra_explicit_ace_from_children() {
        use std::fs;
        use std::ptr;

        use windows_sys::Win32::Foundation::{ERROR_SUCCESS, FALSE};
        use windows_sys::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetNamedSecurityInfoW,
            SDDL_REVISION_1, SE_FILE_OBJECT,
        };
        use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};

        fn read_dacl_sddl(path: &std::path::Path) -> String {
            let wide: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
            // SAFETY: wide は NUL で終わる UTF-16。sd は OS が LocalAlloc し、下で LocalFree する。
            let status = unsafe {
                GetNamedSecurityInfoW(
                    wide.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    &mut sd,
                )
            };
            assert_eq!(status, ERROR_SUCCESS);
            let _sd_guard = os::LocalMem(sd);
            let mut text: *mut u16 = ptr::null_mut();
            let mut len = 0u32;
            // SAFETY: sd は有効なセキュリティ記述子。text は OS が LocalAlloc し、下で LocalFree する。
            let ok = unsafe {
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    sd,
                    SDDL_REVISION_1,
                    DACL_SECURITY_INFORMATION,
                    &mut text,
                    &mut len,
                )
            };
            assert_ne!(ok, FALSE);
            let _text_guard = os::LocalMem(text.cast());
            // SAFETY: text は len 文字以上の NUL 終端 UTF-16。
            let units = unsafe { std::slice::from_raw_parts(text, len as usize) };
            let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());
            String::from_utf16(&units[..end]).unwrap()
        }

        use std::os::windows::ffi::OsStrExt;

        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("data.tsv");
        let sub = t.path().join("sub");
        fs::write(&file, "x").unwrap();
        fs::create_dir(&sub).unwrap();

        let sid = os::current_user_sid().unwrap();
        let wide_file = format!("D:P(A;;FA;;;{sid})(A;;FR;;;WD)");
        let wide_sub = format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FR;;;WD)");
        os::ParsedDacl::from_sddl(&wide_file)
            .unwrap()
            .apply(&file)
            .unwrap();
        os::ParsedDacl::from_sddl(&wide_sub)
            .unwrap()
            .apply(&sub)
            .unwrap();
        assert!(read_dacl_sddl(&file).contains(";;;WD)"));
        assert!(read_dacl_sddl(&sub).contains(";;;WD)"));

        tighten_dir(t.path());

        let file_after = read_dacl_sddl(&file);
        let sub_after = read_dacl_sddl(&sub);
        let has_user = |sddl: &str| {
            sddl.contains(&format!(";;;{sid})"))
                || (sid.ends_with("-500") && sddl.contains(";;;LA)"))
        };
        assert!(!file_after.contains(";;;WD)"), "{file_after}");
        assert!(has_user(&file_after), "{file_after}");
        assert!(file_after.contains(";;;SY)"), "{file_after}");
        assert!(file_after.contains(";;;BA)"), "{file_after}");
        assert!(!sub_after.contains(";;;WD)"), "{sub_after}");
        assert!(has_user(&sub_after), "{sub_after}");
        assert!(sub_after.contains(";;;SY)"), "{sub_after}");
        assert!(sub_after.contains(";;;BA)"), "{sub_after}");
    }
}
