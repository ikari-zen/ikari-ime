//! `webwords classify`（レーン B。ADR 0055 §2、§5.1 の 3）。§2.2 の行 0〜9 を当て、schema=2 の一覧を作る。
//! SQL ダンプの読み込みは同じレーンの `sqldump.rs`（契約のコミットには置かない）。

use std::path::PathBuf;

use super::{ListHeader, WebwordsError};

#[derive(Debug, Clone)]
pub struct ClassifyConfig {
    /// `collect` の出力
    pub collected: PathBuf,
    /// jawiki の SQL ダンプ（同じ日付）。page、redirect、page_props、categorylinks、linktarget
    pub page_sql: PathBuf,
    pub redirect_sql: PathBuf,
    pub page_props_sql: PathBuf,
    pub categorylinks_sql: PathBuf,
    pub linktarget_sql: PathBuf,
    /// カテゴリの規則表。None なら同梱の `DEFAULT_CATEGORIES_TEXT`
    pub categories: Option<PathBuf>,
    /// Sudachi（small、core、notcore）と ABR の lex.csv（F1 の照合）
    pub lex: Vec<PathBuf>,
    /// 一覧の 1 行目に書く値（`rules` は規則表から計算するのでここでは使わない）
    pub header: ListHeader,
    /// df の下限（既定は `MIN_DF_FLOOR`）
    pub min_df: u32,
    /// schema=2 の一覧の出力先
    pub out: PathBuf,
    /// レポート（Markdown。理由ごとの件数と §4.1 の代理の測定）の出力先
    pub report: Option<PathBuf>,
}

/// 判定を行い、一覧に出した行数を返す。
pub fn classify(_config: &ClassifyConfig) -> Result<u64, WebwordsError> {
    Err(WebwordsError::NotImplemented("classify"))
}
