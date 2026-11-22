//! ストリームでの sha256。大きいファイル（辞書で数百 MB）でもメモリは読み取りバッファの分だけ。
//! 340MB 級の所要時間は S0（#254）で測る。

use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

const READ_BUFFER_BYTES: usize = 8 * 1024;
pub(crate) const SHA256_HEX_LEN: usize = 64;

/// 小文字 16 進 64 文字。
pub fn sha256_reader(mut reader: impl Read) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buf = [0u8; READ_BUFFER_BYTES];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(SHA256_HEX_LEN);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

/// リンクを辿らず、通常のファイルだけを読み取りで開く。
fn open_regular_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let meta = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a regular file",
            ));
        }
    }
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    Ok(file)
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    sha256_reader(open_regular_file(path)?)
}

/// manifest の sha256 の欄の形（小文字 16 進 64 文字）
pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == SHA256_HEX_LEN
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(
            sha256_reader(&b""[..]).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_reader(&b"abc"[..]).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn spans_buffer_boundaries() {
        // 8KB のバッファをまたぐ長さでも、一括と同じ値になる
        let data = vec![0x5au8; READ_BUFFER_BYTES * 3 + 17];
        let streamed = sha256_reader(&data[..]).unwrap();
        // sha2 0.11 のダイジェストは `LowerHex` を持たないので、1 バイトずつ十六進にする
        let whole: String = Sha256::digest(&data)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(streamed, whole);
    }

    #[test]
    fn sha256_file_hashes_regular_file_and_rejects_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.bin");
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // ディレクトリは通常ファイルではないため拒否する
        assert!(sha256_file(dir.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn sha256_file_rejects_symlink_and_fifo() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.bin");
        std::fs::write(&target, b"abc").unwrap();
        let link = dir.path().join("link.bin");
        symlink(&target, &link).unwrap();
        assert!(sha256_file(&link).is_err());

        let fifo = dir.path().join("pipe.fifo");
        let c_fifo = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: c_fifo は有効な NUL 終端パス文字列。
        let rc = unsafe { libc::mkfifo(c_fifo.as_ptr(), 0o600) };
        assert_eq!(rc, 0);
        let err = sha256_file(&fifo).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
