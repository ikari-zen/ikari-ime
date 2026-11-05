//! `webwords collect`（レーン A。ADR 0055 §5.1 の 2）。本文から連なりを数える（ADR 0039 §2.2）。

use std::path::PathBuf;

use super::WebwordsError;

#[derive(Debug, Clone)]
pub struct CollectConfig {
    /// pages-articles の bz2（分割の全ファイル）
    pub pages_articles: Vec<PathBuf>,
    /// 収集の一覧（`CollectedRow` の行）の出力先
    pub out: PathBuf,
    /// 出力する df の下限
    pub min_df: u32,
}

/// 収集を行い、出力した行数を返す。
pub fn collect(_config: &CollectConfig) -> Result<u64, WebwordsError> {
    Err(WebwordsError::NotImplemented("collect"))
}
