//! 通常ファイルだけを開く（辞書・ベクトルの読み込み用）。
//!
//! FIFO やデバイスを開くと、読み手が来るまで無期限に止まるおそれがある。
//! Unix では `O_NONBLOCK` で開き、開いたハンドルの種類を見て、通常ファイルでなければ `InvalidInput` にする。

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

pub(crate) fn open_regular_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|e| io::Error::new(e.kind(), format!("cannot open {}: {e}", path.display())))?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let err = open_regular_file(dir.path()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_regular_file_is_opened() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, b"x").unwrap();
        assert!(open_regular_file(&path).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_is_rejected_without_blocking() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fifo");
        let c = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let err = open_regular_file(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
