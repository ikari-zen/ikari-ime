//! 取り込みファイルの読み込み（サイズの上限は読む前に確かめる）。

use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use ikari_core::user::fnv1a64;

use crate::schema::{MAX_FILE_BYTES, Parsed, SchemaError, parse_jsonl};

#[derive(Debug)]
pub enum ReadError {
    Io(std::io::Error),
    Schema(SchemaError),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::Io(e) => write!(f, "{e}"),
            ReadError::Schema(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<std::io::Error> for ReadError {
    fn from(e: std::io::Error) -> Self {
        ReadError::Io(e)
    }
}

impl From<SchemaError> for ReadError {
    fn from(e: SchemaError) -> Self {
        ReadError::Schema(e)
    }
}

/// 読み込んだ取り込みファイル
#[derive(Debug)]
pub struct IngestFile {
    pub parsed: Parsed,
    /// ファイル内容の FNV-1a 64（変更の検出用）
    pub fnv64: u64,
    pub bytes: u64,
}

pub fn read_ingest_file(path: &Path) -> Result<IngestFile, ReadError> {
    // シンボリックリンクと FIFO などを開く前に断る（open は辿る・FIFO では止まる）
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() {
        return Err(ReadError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "通常のファイルではない(シンボリックリンク、FIFO など)",
        )));
    }
    let file = File::open(path)?;
    let len = file.metadata()?.len();
    if len > MAX_FILE_BYTES {
        return Err(SchemaError::TooLarge { bytes: len }.into());
    }
    // メタデータの値を信用せず、読む量にも上限をかける
    let mut buf = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut buf)?;
    let bytes = buf.len() as u64;
    let fnv64 = fnv1a64(&buf);
    let parsed = parse_jsonl(&buf)?;
    Ok(IngestFile {
        parsed,
        fnv64,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.jsonl");
        std::fs::write(&real, b"").unwrap();
        let link = dir.path().join("link.jsonl");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        match read_ingest_file(&link) {
            Err(ReadError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rejects_fifo_without_blocking() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("fifo.jsonl");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc_mkfifo(c.as_ptr(), 0o600) }, 0);
        match read_ingest_file(&fifo) {
            Err(ReadError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn missing_stays_not_found() {
        let dir = tempfile::tempdir().unwrap();
        match read_ingest_file(&dir.path().join("none")) {
            Err(ReadError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            other => panic!("{other:?}"),
        }
    }

    unsafe extern "C" {
        #[link_name = "mkfifo"]
        fn libc_mkfifo(path: *const std::ffi::c_char, mode: u32) -> i32;
    }
}
