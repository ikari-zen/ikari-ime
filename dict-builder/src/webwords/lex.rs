//! `webwords lex`（レーン C。ADR 0055 §5.3 の 2）。一覧から辞書に足す lex.csv を作る。
//! ID は固定の数値を持たず、Sudachi の行から数える（§4.2）。

use std::path::PathBuf;

use super::{WebClass, WebwordsError};

#[derive(Debug, Clone)]
pub struct LexConfig {
    /// schema=2 の一覧
    pub list: PathBuf,
    /// Sudachi（small、core、notcore）と ABR の lex.csv（F1 をその場でもう一度当てる。ID も数える）
    pub lex: Vec<PathBuf>,
    /// 出力する `webwords_lex.csv`
    pub out: PathBuf,
    /// レポート（Markdown）の出力先
    pub report: Option<PathBuf>,
    /// df の下限（F2）。None なら `MIN_DF_FLOOR`
    pub min_df: Option<u32>,
    /// 普通名詞のコスト（掃引で決める。None なら既定）
    pub cost: Option<i16>,
    /// 区分ごとの出発点からの差（掃引用。§4.3）
    pub proper_offsets: Vec<(WebClass, i16)>,
    /// F5（連結を足すかの規則。0039 Q4）を当てない
    pub no_f5: bool,
}

/// 行を作り、出力した行数を返す。
pub fn lex(_config: &LexConfig) -> Result<u64, WebwordsError> {
    Err(WebwordsError::NotImplemented("lex"))
}
