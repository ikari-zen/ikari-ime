//! 作業の場所と錠、片づけ、ディスクの見積もり（ADR 0076 §3.1、§3.2、§8.3）。
//!
//! - 作業の場所（`fetch/`）: 両 OS で `<データの場所>/fetch/`（ADR 0078 §3.1 の 1。Windows の取得の子は、ここに
//!   場所を持たず、親が作った `download.bin` の追記だけのハンドルに書く）。**ここのファイルは信じない**
//!   （`stage_model_bundle` が写しながら数え直す）。根から順に、リンクを辿らずに 1 段ずつ作って持つ（`nofollow`。
//!   D-01。同じ利用者の中の整合性の者に対する保険として残す）。持っている間、付け替えと削除をさせない。
//! - inbox: `<データの場所>/inbox/`（エンジンが見回る。ADR 0058）。
//! - 錠（`fetch/.lock`）: 同時に走る取得は 1 つ。OS の排他の錠なので、落ちても残らない。

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::Component;
use std::path::{Path, PathBuf};

#[cfg(not(target_os = "macos"))]
use ikari_core::user::UserDir;
use ikari_resource::PARTIAL_SUFFIX;
use ikari_resource::fetch_report::FetchErrorCode;

use crate::error::FetchError;
use crate::limits::DISK_MARGIN;
use crate::nofollow::{self, HeldDir};

const INBOX_DIR: &str = "inbox";
const WORK_DIR: &str = "fetch";
const LOCK_FILE: &str = ".lock";
/// 作業の場所に落とすファイルの名前
pub const DOWNLOAD_FILE: &str = "download.bin";

/// 見た物の種類（リンクは辿らずに見る）。何を消してよいかの判断は `removable_kind`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeKind {
    Missing,
    File,
    Dir,
    /// リンク（シンボリックリンク。Windows は reparse point のすべて）
    Link,
}

/// 作業の場所の中で、消してよい種類。ファイルとリンク（リンクそのもの）だけ。ディレクトリは、知らないものとして残す
pub(crate) fn removable_kind(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::File | NodeKind::Link)
}

/// 使う場所
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    pub inbox: PathBuf,
    pub work: PathBuf,
    /// 作業の場所の根（`work` の祖先。ここから `work` まで 1 段ずつ作って持つ）。根そのものはリンクでないことだけ確かめる
    base: PathBuf,
}

impl Dirs {
    /// 作業の場所の根は、`work` の親
    pub fn new(inbox: impl Into<PathBuf>, work: impl Into<PathBuf>) -> Self {
        let work = work.into();
        let base = work.parent().map(Path::to_path_buf).unwrap_or_default();
        Self {
            inbox: inbox.into(),
            work,
            base,
        }
    }

    fn with_base(
        inbox: impl Into<PathBuf>,
        work: impl Into<PathBuf>,
        base: impl Into<PathBuf>,
    ) -> Self {
        Self {
            inbox: inbox.into(),
            work: work.into(),
            base: base.into(),
        }
    }

    /// 製品の場所（データの場所は引数で変えられない。ADR 0067）。決められなければ `None`。
    /// 作業の場所は、両 OS でデータの場所の下（ADR 0078 §3.1 の 1）
    pub fn resolve() -> Option<Self> {
        // macOS は `HOME` から決めない（ADR 0079 §3.4。Sandbox は `HOME` をコンテナにする）
        #[cfg(target_os = "macos")]
        let user = crate::macos::resolve_user_dir()?;
        #[cfg(not(target_os = "macos"))]
        let user = UserDir::resolve(None)?;
        let base = user.root().to_path_buf();
        Some(Self::with_base(
            base.join(INBOX_DIR),
            base.join(WORK_DIR),
            base,
        ))
    }

    /// 作業の場所の錠。取れなければ `busy`。根から作業の場所まで、リンクを辿らずに 1 段ずつ「作る → 持つ → 確かめる」。
    /// 返す錠が、持った場所も持ち続ける
    pub fn lock(&self) -> Result<WorkLock, FetchError> {
        let internal = |_| FetchError(FetchErrorCode::Internal);
        let held = self.hold_work_chain().map_err(internal)?;
        let file = nofollow::open_plain_file(
            OpenOptions::new().create(true).write(true).truncate(false),
            &self.work.join(LOCK_FILE),
        )
        .map_err(internal)?;
        match file.try_lock() {
            Ok(()) => Ok(WorkLock { _file: file, held }),
            Err(TryLockError::WouldBlock) => Err(FetchError(FetchErrorCode::Busy)),
            Err(TryLockError::Error(_)) => Err(FetchError(FetchErrorCode::Internal)),
        }
    }

    /// 根（リンクでないこと）から `work` まで、持った順の場所。最後が `work`
    fn hold_work_chain(&self) -> io::Result<Vec<HeldDir>> {
        let rest = self
            .work
            .strip_prefix(&self.base)
            .map_err(|_| nofollow::not_plain())?;
        // macOS（Sandbox）は根を作らず、あってリンクでないことだけを確かめる（ADR 0079 §3.5。`hold_dir` が `lstat` で見る）
        #[cfg(not(target_os = "macos"))]
        fs::create_dir_all(&self.base)?;
        let mut held = vec![nofollow::hold_dir(&self.base)?];
        let mut path = self.base.clone();
        for component in rest.components() {
            let Component::Normal(name) = component else {
                return Err(nofollow::not_plain());
            };
            path.push(name);
            match fs::create_dir(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
            let dir = nofollow::hold_dir(&path)?;
            dir.ensure_child_of(held.last().ok_or_else(nofollow::not_plain)?)?;
            held.push(dir);
        }
        // inbox も根の下に 1 段だけ自分で作る（ADR 0079 §3.5）
        #[cfg(target_os = "macos")]
        if let Some(name) = self.inbox.file_name().and_then(|n| n.to_str())
            && self.inbox.parent() == Some(self.base.as_path())
        {
            crate::macos::ensure_child_dir(&self.base, name)?;
        }
        Ok(held)
    }

    /// 落ちたあとに残った、作業の場所のファイルと inbox の `.partial` を消す（錠は残す）。**錠を持っているときだけ呼ぶ**。
    /// 作業の場所の中は、持っている場所の中の名前だけを、リンクを辿らずに消す。知らないディレクトリは消さずに続ける
    pub fn cleanup(&self, lock: &WorkLock) -> io::Result<()> {
        let work = lock.work_dir()?;
        for name in child_names(&self.work)? {
            if name != LOCK_FILE {
                work.remove_entry(&name)?;
            }
        }
        remove_partials(&self.inbox)
    }

    /// 作業の場所に、落とすファイル（`download.bin`）を新しく作って開く（読み書き。すでにあれば失敗）。
    /// 錠が持つ場所の中に、リンクを辿らずに作る。返すハンドルを最後まで持ち、パスで開き直さない（ADR 0078 §3.1 の 2）。
    /// 消すのは `cleanup`
    pub fn create_download(&self, lock: &WorkLock) -> io::Result<File> {
        lock.work_dir()?.create_file(DOWNLOAD_FILE)
    }

    /// 作業の場所に落とすファイルの場所（テストで中を見るため。製品はパスで開かない）
    #[cfg(test)]
    pub(crate) fn download_path(&self) -> PathBuf {
        self.work.join(DOWNLOAD_FILE)
    }

    /// 要る空きが、作業の場所と inbox の両方にあるか（同じボリュームなら 2 倍必要になるので、どちらも
    /// `required` を見る）。足りなければ `disk-full`。空きを調べられないときは、止めずに進む（書くときの失敗で受ける）
    pub fn check_space(&self, required: u64) -> Result<(), FetchError> {
        for dir in [&self.work, &self.inbox] {
            let dir = if dir == &self.work {
                // 作業の場所は錠のときに作る（ここでは作らない）。まだ無ければ根で見る
                if dir.exists() { dir } else { &self.base }
            } else {
                fs::create_dir_all(dir).map_err(|_| FetchError(FetchErrorCode::Internal))?;
                dir
            };
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
    /// 持っている場所（根から作業の場所まで。最後が作業の場所）。持っている間、付け替えと削除をさせない
    held: Vec<HeldDir>,
}

impl WorkLock {
    fn work_dir(&self) -> io::Result<&HeldDir> {
        self.held.last().ok_or_else(nofollow::not_plain)
    }
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

/// `dir` の直下の名前（UTF-8 でないものは、扱わない）。`dir` が無ければ空
fn child_names(dir: &Path) -> io::Result<Vec<String>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut names = Vec::new();
    for entry in entries {
        if let Ok(name) = entry?.file_name().into_string() {
            names.push(name);
        }
    }
    Ok(names)
}

/// inbox（データの場所の下。低い整合性の書き手はいない）の `.partial` を消す。リンクは先を辿らずにリンクだけを消す
fn remove_partials(inbox: &Path) -> io::Result<()> {
    for name in child_names(inbox)? {
        if !name.ends_with(PARTIAL_SUFFIX) {
            continue;
        }
        let path = inbox.join(&name);
        if nofollow::node_kind(&path)? == NodeKind::Dir {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
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
        fs::create_dir_all(d.inbox.join("user-aaa.partial")).unwrap();
        fs::write(d.inbox.join("user-aaa.partial").join("m.gguf"), b"x").unwrap();
        fs::create_dir_all(d.inbox.join("user-bbb")).unwrap();
        fs::write(d.inbox.join("status.json"), b"{}").unwrap();

        d.cleanup(&lock).unwrap();

        assert!(d.work.join(LOCK_FILE).exists());
        assert!(!d.download_path().exists());
        assert!(!d.inbox.join("user-aaa.partial").exists());
        // 完成した束と、エンジンの状態は触らない
        assert!(d.inbox.join("user-bbb").exists());
        assert!(d.inbox.join("status.json").exists());
    }

    #[test]
    fn the_download_is_created_once_under_the_held_work_dir_and_cleanup_removes_it() {
        use std::io::{Read, Seek, SeekFrom, Write};
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        let lock = d.lock().unwrap();
        let mut file = d.create_download(&lock).unwrap();
        file.write_all(b"abc").unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut back = Vec::new();
        file.read_to_end(&mut back).unwrap();
        assert_eq!(back, b"abc", "読み書きで開く");
        assert!(d.create_download(&lock).is_err(), "すでにあれば作らない");
        drop(file);
        d.cleanup(&lock).unwrap();
        assert!(!d.download_path().exists());
    }

    #[cfg(unix)]
    #[test]
    fn create_download_refuses_a_planted_link() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        let lock = d.lock().unwrap();
        let target = tmp.path().join("target");
        fs::write(&target, b"keep").unwrap();
        std::os::unix::fs::symlink(&target, d.download_path()).unwrap();
        assert!(d.create_download(&lock).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"keep");
    }

    #[test]
    fn cleanup_with_nothing_there_is_fine() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        let lock = d.lock().unwrap();
        d.cleanup(&lock).unwrap();
    }

    #[test]
    fn cleanup_keeps_unknown_directories_and_does_not_enter_them() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        let lock = d.lock().unwrap();
        fs::create_dir_all(d.work.join("sub")).unwrap();
        fs::write(d.work.join("sub").join("x"), b"x").unwrap();
        fs::write(d.download_path(), b"half").unwrap();
        d.cleanup(&lock).unwrap();
        assert!(
            d.work.join("sub").join("x").exists(),
            "知らないディレクトリは消さずに続ける"
        );
        assert!(!d.download_path().exists());
    }

    #[test]
    fn lock_creates_missing_levels_and_rejects_a_plain_file_in_the_way() {
        let tmp = tempfile::tempdir().unwrap();
        let d = Dirs::with_base(
            tmp.path().join("inbox"),
            tmp.path().join("a").join("fetch"),
            tmp.path(),
        );
        assert!(d.lock().is_ok());
        let tmp2 = tempfile::tempdir().unwrap();
        fs::write(tmp2.path().join("fetch"), b"x").unwrap();
        assert!(dirs(tmp2.path()).lock().is_err(), "ファイルがあれば失敗");
    }

    /// macOS（ADR 0079 §3.5）: 根は作らず（無ければ失敗）、根があれば `fetch/` と `inbox/` を作る
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_lock_does_not_create_the_root_but_creates_fetch_and_inbox() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ikari-ime");
        assert!(dirs(&root).lock().is_err(), "根が無ければ作らずに失敗");
        assert!(!root.exists());
        fs::create_dir(&root).unwrap();
        let _lock = dirs(&root).lock().unwrap();
        assert!(root.join("fetch").is_dir());
        assert!(root.join("inbox").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn lock_refuses_a_linked_work_dir_and_a_linked_lock_file() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, tmp.path().join("fetch")).unwrap();
        assert!(dirs(tmp.path()).lock().is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0, "先に作らない");

        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        fs::create_dir_all(&d.work).unwrap();
        let target = tmp.path().join("target");
        fs::write(&target, b"keep").unwrap();
        std::os::unix::fs::symlink(&target, d.work.join(LOCK_FILE)).unwrap();
        assert!(d.lock().is_err());
        assert_eq!(fs::read(&target).unwrap(), b"keep");
    }

    #[cfg(windows)]
    fn junction(link: &Path, target: &Path) {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap()
            .status;
        assert!(status.success(), "ジャンクションを作れなかった");
    }

    #[cfg(windows)]
    #[test]
    fn windows_junctions_are_not_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside");
        fs::create_dir_all(outside.join("fetch")).unwrap();
        fs::write(outside.join("keep"), b"x").unwrap();
        fs::write(outside.join("fetch").join("keep"), b"x").unwrap();

        // (a) `fetch` がジャンクション
        let root_a = tmp.path().join("a");
        fs::create_dir_all(root_a.join("ikari-ime")).unwrap();
        junction(
            &root_a.join("ikari-ime").join("fetch"),
            &outside.join("fetch"),
        );
        let d = Dirs::with_base(
            tmp.path().join("inbox"),
            root_a.join("ikari-ime").join("fetch"),
            &root_a,
        );
        assert!(d.lock().is_err());
        assert!(outside.join("fetch").join("keep").exists());

        // (b) `ikari-ime`（途中の要素）がジャンクション
        let root_b = tmp.path().join("b");
        fs::create_dir_all(&root_b).unwrap();
        junction(&root_b.join("ikari-ime"), &outside);
        let d = Dirs::with_base(
            tmp.path().join("inbox"),
            root_b.join("ikari-ime").join("fetch"),
            &root_b,
        );
        assert!(d.lock().is_err());
        assert!(outside.join("keep").exists());

        // (c) 中身がジャンクション。リンクだけを消し、先は残る
        let d = dirs(&tmp.path().join("c"));
        fs::create_dir_all(&d.work).unwrap();
        let lock = d.lock().unwrap();
        junction(&d.work.join("link"), &outside);
        d.cleanup(&lock).unwrap();
        assert!(outside.join("keep").exists());
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
