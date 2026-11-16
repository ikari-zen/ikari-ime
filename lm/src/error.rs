use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LmError {
    /// モデルの読み込みに失敗した
    Load(String),
    /// 推論に失敗した
    Inference(String),
    /// 取り消しの印が立って、推論を途中でやめた（ADR 0074。失敗には数えない）
    Cancelled,
    /// 設定の値が不正
    Config { key: String, message: String },
}

impl fmt::Display for LmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LmError::Load(m) => write!(f, "lm load failed: {m}"),
            LmError::Inference(m) => write!(f, "lm inference failed: {m}"),
            LmError::Cancelled => write!(f, "lm inference cancelled"),
            LmError::Config { key, message } => write!(f, "lm config `{key}`: {message}"),
        }
    }
}

impl std::error::Error for LmError {}
