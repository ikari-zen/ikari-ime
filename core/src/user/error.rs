//! ユーザーデータの読み書きのエラー。

use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
pub enum UserDataError {
    Io(std::io::Error),
    /// ファイルの内容が不正（行番号は 1 始まり）
    Parse {
        file: PathBuf,
        line: usize,
        message: String,
    },
    /// スキーマのバージョンが一致しない
    Schema {
        found: u32,
        expected: u32,
    },
    /// 品詞が見つからない、活用規則に合わない語尾など、値の不正
    Invalid(String),
}

impl fmt::Display for UserDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UserDataError::Io(e) => write!(f, "I/O error: {e}"),
            UserDataError::Parse {
                file,
                line,
                message,
            } => write!(f, "{}:{line}: {message}", file.display()),
            UserDataError::Schema { found, expected } => {
                write!(
                    f,
                    "unsupported schema version {found} (expected {expected})"
                )
            }
            UserDataError::Invalid(message) => write!(f, "invalid data: {message}"),
        }
    }
}

impl std::error::Error for UserDataError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            UserDataError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for UserDataError {
    fn from(e: std::io::Error) -> Self {
        UserDataError::Io(e)
    }
}
