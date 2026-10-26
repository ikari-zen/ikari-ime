//! ikari-import: LLM が出力した JSONL の検証、強制アライメント、初期学習（ADR 0002 §11）。
//!
//! core は import に依存しない。流れは次のとおり。
//! 1. `prompt`: 同梱のプロンプト（版つき。業務文書の注意を含む）
//! 2. `schema`: JSONL の検証（信用しない入力。理由ごとに捨てた件数を数える）
//! 3. `align`: 辞書で読みを再現する強制アライメント
//! 4. `pipeline`: 評価用の選別（ハッシュで決定的）、初期学習（`initial` 層）、登録候補、分野の推定
//! 5. `commands`: 台帳（`ingest_sources.tsv`）と登録候補の更新、取り込み元の削除
//! 6. `holdout`: 評価用の 2 割を、取り込みファイルのパスから読み出す（複製しない）
//!
//! 原文は取り込みファイルにだけ残る（NFR-19）。学習ファイルに入るのは、語単位の記録と
//! ソルト付きの特徴量のハッシュだけ。

pub mod align;
pub mod commands;
pub mod holdout;
pub mod io;
pub mod pipeline;
pub mod prompt;
pub mod schema;

#[cfg(test)]
mod test_support;

pub use align::{AlignedToken, Alignment, TokenKind, align};
pub use commands::{ImportError, RunOptions, RunReport, remove_source, reset_all, run_file};
pub use holdout::{
    HOLDOUT_MODULUS, HoldoutSentence, HoldoutSet, holdout_sentences, is_holdout, load_holdout,
};
pub use io::{IngestFile, ReadError, read_ingest_file};
pub use pipeline::{DomainScore, Env, IngestOutcome, ScoreMethod, ingest, suggest_profile};
pub use prompt::{CLOUD_LLM_WARNING, INGEST_PROMPT, PROMPT_VERSION};
pub use schema::{DropCounts, DropReason, Parsed, SchemaError, Sentence, parse_jsonl};
