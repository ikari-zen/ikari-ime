//! 失敗の理由（ADR 0076 §8.2）。外へ出すのは理由の語 1 つだけ（`FetchErrorCode`）。

use std::fmt;

use ikari_resource::StageError;
use ikari_resource::fetch_report::FetchErrorCode;

use crate::url::UrlError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchError(pub FetchErrorCode);

impl FetchError {
    pub fn code(self) -> FetchErrorCode {
        self.0
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.word())
    }
}

impl std::error::Error for FetchError {}

impl From<FetchErrorCode> for FetchError {
    fn from(code: FetchErrorCode) -> Self {
        Self(code)
    }
}

impl From<UrlError> for FetchError {
    fn from(_: UrlError) -> Self {
        Self(FetchErrorCode::BadUrl)
    }
}

impl From<StageError> for FetchError {
    fn from(e: StageError) -> Self {
        Self(match e {
            StageError::TooLarge { .. } => FetchErrorCode::TooLarge,
            StageError::Checksum => FetchErrorCode::Checksum,
            StageError::DiskFull => FetchErrorCode::DiskFull,
            // 同じ名前の束がすでにある（ほかの取得が先に入れた）。取得はしていないので busy と同じ扱い
            StageError::Exists(_) => FetchErrorCode::Busy,
            StageError::BadPlan(_) | StageError::Io { .. } | StageError::Resource(_) => {
                FetchErrorCode::Internal
            }
        })
    }
}
