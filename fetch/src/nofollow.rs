//! リンクを辿らずに、作業の場所を開く・確かめる・消す（ADR 0076 §3.2）。
//!
//! Windows の作業の場所は、低い整合性の者も書ける。中の整合性の `ikari-data` がパスのまま開いたり消したりすると、
//! リンク（ジャンクション、シンボリックリンク。reparse point のすべて）を辿って、向けた先に触れてしまう。そのため:
//! - ディレクトリは、リンクを辿らずにハンドルで開き、リンクでないことをハンドルで確かめ、持っている間は
//!   名前の付け替えと削除をさせない（`FILE_SHARE_DELETE` を渡さない）。
//! - 消すのは、持っているディレクトリの中の名前だけ。リンクを辿らずに開き、そのハンドルの実際の場所の親が
//!   持っているディレクトリであることを確かめてから、ハンドルで消す（パスで消さない）。
//!
//! macOS・その他の OS の作業の場所はデータの場所の下で、低い権限の別の書き手はいない。リンクを辿らずに
//! 種類を見るだけにする（ハンドルでの確かめは持たない）。
//!
//! 何を消してよいかの判断は `workspace` の純粋な関数（`NodeKind` を受ける）。ここは OS の呼び出しだけ。

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use crate::workspace::NodeKind;

/// リンクやディレクトリでないものを、ディレクトリやファイルとして使おうとしたときの失敗
pub(crate) fn not_plain() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "not a plain directory or file",
    )
}

/// `path` の種類を、リンクを辿らずに見る
pub(crate) fn node_kind(path: &Path) -> io::Result<NodeKind> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(kind_of(&meta)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(NodeKind::Missing),
        Err(e) => Err(e),
    }
}

/// 持っているディレクトリ（リンクでないことを確かめたもの）
#[derive(Debug)]
pub(crate) struct HeldDir {
    path: PathBuf,
    /// 持っている間、名前の付け替えと削除をさせないためのハンドル（読まない）
    #[cfg(windows)]
    _handle: File,
    #[cfg(windows)]
    final_path: PathBuf,
}

/// リンクを辿らずにファイルを開き、ふつうのファイルであることを確かめる（リンクなら開かずに失敗）
pub(crate) fn open_plain_file(options: &mut OpenOptions, path: &Path) -> io::Result<File> {
    let file = os::open_no_follow(options, path)?;
    if kind_of(&file.metadata()?) != NodeKind::File {
        return Err(not_plain());
    }
    Ok(file)
}

#[cfg(windows)]
fn kind_of(meta: &fs::Metadata) -> NodeKind {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    // reparse point は、種類（ジャンクション、シンボリックリンク、そのほか）に関わらずリンクとして扱う
    if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        NodeKind::Link
    } else if meta.is_dir() {
        NodeKind::Dir
    } else {
        NodeKind::File
    }
}

#[cfg(not(windows))]
fn kind_of(meta: &fs::Metadata) -> NodeKind {
    let t = meta.file_type();
    if t.is_symlink() {
        NodeKind::Link
    } else if t.is_dir() {
        NodeKind::Dir
    } else {
        NodeKind::File
    }
}

pub(crate) use os::hold_dir;

#[cfg(windows)]
mod os {
    use std::ffi::OsString;
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FileDispositionInfo, GetFinalPathNameByHandleW,
        SetFileInformationByHandle, VOLUME_NAME_DOS,
    };

    use super::{HeldDir, kind_of, not_plain};
    use crate::workspace::{NodeKind, removable_kind};

    /// 実際の場所を読む最初のバッファの長さ（足りなければ、OS が返す長さで読み直す）
    const FINAL_PATH_INITIAL_LEN: usize = 512;
    /// 読み直す回数の上限（そのあいだに場所が長くなり続けたら諦める）
    const FINAL_PATH_RETRIES: usize = 3;

    /// リンクを辿らずに開く（`FILE_FLAG_OPEN_REPARSE_POINT`。ディレクトリも開けるように `BACKUP_SEMANTICS`）
    pub(super) fn open_no_follow(options: &mut OpenOptions, path: &Path) -> io::Result<File> {
        options
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
    }

    /// `path` のディレクトリを、リンクを辿らずに開いて持つ。持っている間、名前の付け替えと削除をさせない
    pub(crate) fn hold_dir(path: &Path) -> io::Result<HeldDir> {
        let handle = open_no_follow(
            OpenOptions::new()
                .access_mode(FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE),
            path,
        )?;
        if kind_of(&handle.metadata()?) != NodeKind::Dir {
            return Err(not_plain());
        }
        let final_path = final_path(&handle)?;
        Ok(HeldDir {
            path: path.to_path_buf(),
            _handle: handle,
            final_path,
        })
    }

    impl HeldDir {
        /// `self` の実際の場所が、`parent` の実際の場所のすぐ下であること
        pub(crate) fn ensure_child_of(&self, parent: &HeldDir) -> io::Result<()> {
            if self.final_path.parent() == Some(parent.final_path.as_path()) {
                Ok(())
            } else {
                Err(not_plain())
            }
        }

        /// 中の `name` を、リンクを辿らずに開いて、ハンドルで消す。無ければ何もしない。
        /// ディレクトリは消さない（`removable_kind`）。開いたものの実際の場所の親が自分でなければ、消さずに失敗
        pub(crate) fn remove_entry(&self, name: &str) -> io::Result<()> {
            let opened = open_no_follow(
                OpenOptions::new()
                    .access_mode(DELETE | FILE_READ_ATTRIBUTES)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE),
                &self.path.join(name),
            );
            let file = match opened {
                Ok(file) => file,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e),
            };
            if !removable_kind(kind_of(&file.metadata()?)) {
                return Ok(());
            }
            if final_path(&file)?.parent() != Some(self.final_path.as_path()) {
                return Err(not_plain());
            }
            let info = FILE_DISPOSITION_INFO { DeleteFile: true };
            // SAFETY: `file` は DELETE つきで開いた生きているハンドル。`info` は呼び出しの間生きていて、大きさを正しく渡す
            let ok = unsafe {
                SetFileInformationByHandle(
                    file.as_raw_handle(),
                    FileDispositionInfo,
                    (&info as *const FILE_DISPOSITION_INFO).cast(),
                    size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }

    /// ハンドルが指す実際の場所（リンクを解いたあとの、正規化した場所）
    fn final_path(file: &File) -> io::Result<PathBuf> {
        let mut buf = vec![0u16; FINAL_PATH_INITIAL_LEN];
        for _ in 0..FINAL_PATH_RETRIES {
            let capacity = u32::try_from(buf.len()).unwrap_or(u32::MAX);
            // SAFETY: `file` は生きているハンドル。`buf` は `capacity` 個の u16 を書ける
            let n = unsafe {
                GetFinalPathNameByHandleW(
                    file.as_raw_handle(),
                    buf.as_mut_ptr(),
                    capacity,
                    FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
                )
            } as usize;
            if n == 0 {
                return Err(io::Error::last_os_error());
            }
            if n < buf.len() {
                buf.truncate(n);
                return Ok(PathBuf::from(OsString::from_wide(&buf)));
            }
            // 足りないときは、終わりの NUL を含む長さが返る
            buf.resize(n, 0);
        }
        Err(io::Error::other("final path keeps growing"))
    }
}

#[cfg(not(windows))]
mod os {
    use std::fs::{self, File, OpenOptions};
    use std::io;
    use std::path::Path;

    use super::{HeldDir, node_kind, not_plain};
    use crate::workspace::{NodeKind, removable_kind};

    /// リンクを辿らずに開く（unix は `O_NOFOLLOW`。最後の要素がリンクなら失敗する）
    pub(super) fn open_no_follow(options: &mut OpenOptions, path: &Path) -> io::Result<File> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        options.open(path)
    }

    /// `path` がリンクでないディレクトリであることを確かめる（ハンドルは持たない。モジュールの説明を見る）
    pub(crate) fn hold_dir(path: &Path) -> io::Result<HeldDir> {
        if node_kind(path)? != NodeKind::Dir {
            return Err(not_plain());
        }
        Ok(HeldDir {
            path: path.to_path_buf(),
        })
    }

    impl HeldDir {
        /// 実際の場所の比べはしない（ハンドルを持たないため）
        pub(crate) fn ensure_child_of(&self, _parent: &HeldDir) -> io::Result<()> {
            Ok(())
        }

        /// 中の `name` を消す。消す前に、自分がまだリンクでないディレクトリであることを見直す。
        /// 無ければ何もしない。ディレクトリは消さない。リンクは先を辿らずにリンクだけを消す
        pub(crate) fn remove_entry(&self, name: &str) -> io::Result<()> {
            if node_kind(&self.path)? != NodeKind::Dir {
                return Err(not_plain());
            }
            let path = self.path.join(name);
            if !removable_kind(node_kind(&path)?) {
                return Ok(());
            }
            match fs::remove_file(&path) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                other => other,
            }
        }
    }
}
