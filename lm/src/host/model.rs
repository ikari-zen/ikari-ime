//! 照合したモデルのハンドル（契約。第 4 弾の追加。FC-3、C-03）。
//!
//! エンジンは、照合のときに開いたハンドルを、エンジンの寿命の間持ち、照合はそのハンドルで数える
//! （`ikari_resource::verify_model_open`）。子に渡すのは、そのハンドルから作ったもの:
//! - macOS: ハンドルの写し（[`ModelHandle::for_child`]）を子の fd 3 にする。写しは読み書きの位置を共有するので、
//!   起動のたびに位置を先頭へ戻してから渡す。パスで開く道が無いので、照合のあとに名前ごと差し替えられても、
//!   子が読むのは照合した物になる。
//! - Windows: 子（llama.cpp）はパスでしか開けない。エンジンは、書き込みと削除の共有を許さずに開いたまま持つ
//!   （読み取りの共有だけ）。持っている間、同じファイルの書き換え・削除・名前の付け替えは OS が断るので、
//!   子がパスで開いても、照合した物を読む。
//!
//! 判断の置き場所: Rust（エンジン）。

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 照合のときに開いた、モデルの読み取りのハンドルと、そのパス（ログ、Windows の子の `--model`、段 2 の許可）
#[derive(Clone)]
pub struct ModelHandle {
    file: Arc<File>,
    path: PathBuf,
}

impl fmt::Debug for ModelHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelHandle")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl ModelHandle {
    /// 読み取りだけで開く。Windows は読み取りの共有だけを許す（書き込みと削除の共有なし）。
    /// ふつうのファイルでなければ誤り。呼ぶ前に種類を見ておく（unix は止まらないよう非同期の印で開く）
    pub fn open(path: &Path) -> io::Result<ModelHandle> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ);
        }
        let file = options.open(path)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "model is not a regular file",
            ));
        }
        Ok(ModelHandle {
            file: Arc::new(file),
            path: path.to_owned(),
        })
    }

    /// 開いたときのパス
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 照合に使うハンドル（位置を動かさない読み方で読む）
    pub fn file(&self) -> &File {
        &self.file
    }

    /// 子に渡す写し（macOS の fd 3）。位置を共有するので、先頭へ戻してから返す。起動のたびに呼ぶ
    pub fn for_child(&self) -> io::Result<File> {
        let mut copy = self.file.try_clone()?;
        copy.seek(SeekFrom::Start(0))?;
        Ok(copy)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::*;

    const BODY: &[u8] = b"GGUF model body";

    fn model() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.gguf");
        File::create(&path).unwrap().write_all(BODY).unwrap();
        (dir, path)
    }

    #[test]
    fn each_launch_reads_from_the_start() {
        let (_dir, path) = model();
        let handle = ModelHandle::open(&path).unwrap();
        // 前の子が終わりまで読んでも、次の子は先頭から読む（同じハンドルで 2 回続けて起動）
        for _ in 0..2 {
            let mut out = Vec::new();
            handle.for_child().unwrap().read_to_end(&mut out).unwrap();
            assert_eq!(out, BODY);
        }
    }

    #[test]
    fn a_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(ModelHandle::open(dir.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn the_child_still_reads_the_verified_file_after_a_rename() {
        let (dir, path) = model();
        let handle = ModelHandle::open(&path).unwrap();
        std::fs::rename(&path, dir.path().join("old.gguf")).unwrap();
        std::fs::write(&path, b"other").unwrap();
        let mut out = Vec::new();
        handle.for_child().unwrap().read_to_end(&mut out).unwrap();
        assert_eq!(out, BODY);
    }

    #[cfg(unix)]
    #[test]
    fn the_handle_is_read_only() {
        use crate::host::cage::{FdAccess, FdKind, probe_fd};
        use std::os::fd::AsFd;
        let (_dir, path) = model();
        let child = ModelHandle::open(&path).unwrap().for_child().unwrap();
        let facts = probe_fd(child.as_fd()).unwrap();
        assert_eq!(
            (facts.kind, facts.access),
            (FdKind::Regular, FdAccess::Read)
        );
    }

    #[cfg(windows)]
    #[test]
    fn while_held_the_file_cannot_be_written_removed_or_renamed() {
        let (dir, path) = model();
        let _handle = ModelHandle::open(&path).unwrap();
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        assert!(std::fs::remove_file(&path).is_err());
        assert!(std::fs::rename(&path, dir.path().join("old.gguf")).is_err());
        // 読み取りは通る（子はパスで開く）
        let mut out = Vec::new();
        File::open(&path).unwrap().read_to_end(&mut out).unwrap();
        assert_eq!(out, BODY);
    }
}
