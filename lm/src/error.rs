use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LmError {
    /// モデルの読み込みに失敗した
    Load(String),
    /// 推論に失敗した
    Inference(String),
    /// 設定の値が不正
    Config { key: String, message: String },
}

impl fmt::Display for LmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LmError::Load(m) => write!(f, "lm load failed: {m}"),
            LmError::Inference(m) => write!(f, "lm inference failed: {m}"),
            LmError::Config { key, message } => write!(f, "lm config `{key}`: {message}"),
        }
    }
}

impl std::error::Error for LmError {}
