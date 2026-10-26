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
