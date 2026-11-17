//! 作業の場所と錠、片づけ、ディスクの見積もり（ADR 0076 §3.1、§3.2、§8.3）。
//!
//! - 作業の場所（`fetch/`）: Windows は `%USERPROFILE%\AppData\LocalLow\ikari-ime\fetch\`（低い整合性の子が書ける場所。
//!   ikari は ACL を変えない）、macOS は `<データの場所>/fetch/`。**ここのファイルは信じない**（`stage_model_bundle` が
//!   写しながら数え直す）。
//! - inbox: `<データの場所>/inbox/`（エンジンが見回る。ADR 0058）。
//! - 錠（`fetch/.lock`）: 同時に走る取得は 1 つ。OS の排他の錠なので、落ちても残らない。

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

use ikari_core::user::UserDir;
use ikari_resource::PARTIAL_SUFFIX;
use ikari_resource::fetch_report::FetchErrorCode;

use crate::error::FetchError;
use crate::limits::DISK_MARGIN;

const INBOX_DIR: &str = "inbox";
const WORK_DIR: &str = "fetch";
const LOCK_FILE: &str = ".lock";
/// 作業の場所に落とすファイルの名前
pub const DOWNLOAD_FILE: &str = "download.bin";
#[cfg(windows)]
const LOCAL_LOW_RELATIVE: &str = r"AppData\LocalLow\ikari-ime";
#[cfg(windows)]
const USERPROFILE_ENV: &str = "USERPROFILE";

/// 使う場所
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    pub inbox: PathBuf,
    pub work: PathBuf,
}

impl Dirs {
    pub fn new(inbox: impl Into<PathBuf>, work: impl Into<PathBuf>) -> Self {
        Self {
            inbox: inbox.into(),
            work: work.into(),
        }
    }

    /// 製品の場所（データの場所は引数で変えられない。ADR 0067）。決められなければ `None`
    pub fn resolve() -> Option<Self> {
        let user = UserDir::resolve(None)?;
        let work = work_root(user.root())?.join(WORK_DIR);
        Some(Self::new(user.root().join(INBOX_DIR), work))
    }

    /// 作業の場所の錠。取れなければ `busy`
    pub fn lock(&self) -> Result<WorkLock, FetchError> {
        fs::create_dir_all(&self.work).map_err(|_| FetchError(FetchErrorCode::Internal))?;
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(self.work.join(LOCK_FILE))
            .map_err(|_| FetchError(FetchErrorCode::Internal))?;
        match file.try_lock() {
            Ok(()) => Ok(WorkLock { _file: file }),
            Err(TryLockError::WouldBlock) => Err(FetchError(FetchErrorCode::Busy)),
            Err(TryLockError::Error(_)) => Err(FetchError(FetchErrorCode::Internal)),
        }
    }

    /// 落ちたあとに残った、作業の場所のファイルと inbox の `.partial` を消す（錠は残す）。**錠を持っているときだけ呼ぶ**
    pub fn cleanup(&self, _lock: &WorkLock) -> io::Result<()> {
        remove_children(&self.work, |name| name != LOCK_FILE)?;
        remove_children(&self.inbox, |name| name.ends_with(PARTIAL_SUFFIX))
    }

    /// 作業の場所に落とすファイルの場所
    pub fn download_path(&self) -> PathBuf {
        self.work.join(DOWNLOAD_FILE)
    }

    /// 要る空きが、作業の場所と inbox の両方にあるか（同じボリュームなら 2 倍必要になるので、どちらも
    /// `required` を見る）。足りなければ `disk-full`。空きを調べられないときは、止めずに進む（書くときの失敗で受ける）
    pub fn check_space(&self, required: u64) -> Result<(), FetchError> {
        for dir in [&self.work, &self.inbox] {
            fs::create_dir_all(dir).map_err(|_| FetchError(FetchErrorCode::Internal))?;
            if let Ok(free) = free_space(dir)
                && free < required
            {
                return Err(FetchError(FetchErrorCode::DiskFull));
            }
        }
        Ok(())
    }
}

/// 取得の錠。持っている間、ほかの取得は `busy` になる
#[derive(Debug)]
pub struct WorkLock {
    _file: File,
}

/// 要る空き（バイト）。大きさが分かっていれば（プリセット）その 2 倍（作業の場所と写し）と余裕、
/// 分からなければ（自由指定）上限の 1 倍と余裕（写す前にもう一度、受けた大きさで見る。ADR 0076 §8.3）
pub fn required_space(known_size: Option<u64>, max_size: u64) -> u64 {
    match known_size {
        Some(size) => size.saturating_mul(2).saturating_add(DISK_MARGIN),
        None => max_size.saturating_add(DISK_MARGIN),
    }
}

/// 写す前に見る空き（受けた大きさの分と余裕）
pub fn required_space_to_copy(received: u64) -> u64 {
    received.saturating_add(DISK_MARGIN)
}

/// 親ディレクトリの中身を、`pick(名前)` が真のものだけ消す。親がなければ何もしない
fn remove_children(dir: &Path, pick: impl Fn(&str) -> bool) -> io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !pick(name) {
            continue;
        }
        let path = entry.path();
        // シンボリックリンクは、先を辿らずにリンクだけを消す（`file_type` はリンクを辿らない）
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn work_root(_data_root: &Path) -> Option<PathBuf> {
    let profile = std::env::var_os(USERPROFILE_ENV).filter(|v| !v.is_empty())?;
    Some(PathBuf::from(profile).join(LOCAL_LOW_RELATIVE))
}

#[cfg(not(windows))]
fn work_root(data_root: &Path) -> Option<PathBuf> {
    Some(data_root.to_path_buf())
}

#[cfg(unix)]
fn free_space(dir: &Path) -> io::Result<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(dir.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "nul in path"))?;
    // SAFETY: すべて 0 の値は `statvfs` として正しい。`path` は NUL で終わり、呼び出しの間生きている
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: 上の `path` と、書き込める `stat` を渡す
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // 一般の利用者が使える分（`f_bavail`）
    Ok((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

#[cfg(windows)]
fn free_space(dir: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut wide: Vec<u16> = dir.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "nul in path"));
    }
    wide.push(0);
    let mut available = 0u64;
    // SAFETY: `wide` は NUL で終わり、呼び出しの間生きている。`available` は書き込める。ほかの 2 つは使わない（null）
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(available)
}

#[cfg(not(any(unix, windows)))]
fn free_space(_dir: &Path) -> io::Result<u64> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "free space"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(root: &Path) -> Dirs {
        Dirs::new(root.join("inbox"), root.join("fetch"))
    }

    #[test]
    fn only_one_lock_at_a_time() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        let first = d.lock().unwrap();
        assert_eq!(d.lock().unwrap_err().code(), FetchErrorCode::Busy);
        drop(first);
        assert!(d.lock().is_ok(), "閉じれば取れる");
    }

    #[test]
    fn cleanup_removes_work_files_and_partials_but_keeps_the_lock_and_bundles() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        let lock = d.lock().unwrap();
        fs::write(d.download_path(), b"half").unwrap();
        fs::create_dir_all(d.work.join("sub")).unwrap();
        fs::write(d.work.join("sub").join("x"), b"x").unwrap();
        fs::create_dir_all(d.inbox.join("user-aaa.partial")).unwrap();
        fs::write(d.inbox.join("user-aaa.partial").join("m.gguf"), b"x").unwrap();
        fs::create_dir_all(d.inbox.join("user-bbb")).unwrap();
        fs::write(d.inbox.join("status.json"), b"{}").unwrap();

        d.cleanup(&lock).unwrap();

        assert!(d.work.join(LOCK_FILE).exists());
        assert!(!d.download_path().exists());
        assert!(!d.work.join("sub").exists());
        assert!(!d.inbox.join("user-aaa.partial").exists());
        // 完成した束と、エンジンの状態は触らない
        assert!(d.inbox.join("user-bbb").exists());
        assert!(d.inbox.join("status.json").exists());
    }

    #[test]
    fn cleanup_with_nothing_there_is_fine() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        let lock = d.lock().unwrap();
        d.cleanup(&lock).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_does_not_follow_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        let lock = d.lock().unwrap();
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep"), b"x").unwrap();
        std::os::unix::fs::symlink(&outside, d.work.join("link")).unwrap();
        d.cleanup(&lock).unwrap();
        assert!(!d.work.join("link").exists());
        assert!(outside.join("keep").exists(), "リンクの先は消さない");
    }

    #[test]
    fn space_estimates() {
        assert_eq!(required_space(Some(100), 999), 200 + DISK_MARGIN);
        assert_eq!(required_space(None, 999), 999 + DISK_MARGIN);
        assert_eq!(required_space_to_copy(10), 10 + DISK_MARGIN);
        assert_eq!(required_space(Some(u64::MAX), 0), u64::MAX, "あふれない");
    }

    #[test]
    fn plenty_of_space_passes_and_absurd_requirements_fail() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        assert!(d.check_space(1).is_ok());
        assert_eq!(
            d.check_space(u64::MAX).unwrap_err().code(),
            FetchErrorCode::DiskFull
        );
    }
}
