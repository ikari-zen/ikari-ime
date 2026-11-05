//! Wikipedia 日本語版から集めた外来語の収集・判定・辞書の行の生成（ADR 0055。issue #113）。
//!
//! 3 つの段（`ikari-dict-builder webwords collect|classify|lex`）が、2 種類の一覧でつながる。
//!
//! ```text
//! collect : pages-articles (bz2)               -> 収集の一覧（CollectedRow: 表記 tf df cue）      レーン A
//! classify: 収集の一覧 + SQL ダンプ + Sudachi  -> schema=2 の一覧（ListRow）とレポート           レーン B
//! lex     : schema=2 の一覧 + Sudachi + ABR    -> webwords_lex.csv とレポート                   レーン C
//! ```
//!
//! # schema=2 の一覧の形式（ADR 0055 §5.2。ADR 0039 §5.1 を上書きする）
//!
//! - UTF-8、タブ区切り。ファイル名は `webwords-<ダンプの日付>.tsv`
//! - 1 行目（[`ListHeader`]）:
//!   `#ikari-webwords<TAB>schema=2<TAB>source=jawiki-<日付><TAB>sha256=<pages-articles の各ファイルの sha256 をつないだものの sha256>`
//!   `<TAB>articles=<数えた記事数><TAB>sudachi=<F1 に使った版><TAB>rules=<categories.tsv の sha256>`
//!   `<TAB>license=CC-BY-SA-4.0<TAB>attribution=Wikipedia 日本語版の執筆者（https://ja.wikipedia.org/）`
//! - 2 行目以降（[`ListRow`]）: `表記<TAB>tf<TAB>df<TAB>区分`
//!   - 区分は `common`、`proper.person`、`proper.place`、`proper.general` の 4 つだけ（[`WebClass`]）。
//!     既知・形・迷う語は一覧に出さない
//!   - 並びは表記のコードポイント順（重複なし）
//! - 記事の名前、カテゴリの名前、文、リンクは持たない。読みは持たない（ビルドのときに表記から作る）
//! - 大きさの見込み: 約 9 万行、約 3.2MB、xz で約 0.5MB（researcher の調査 R3、2026-11-06）
//!
//! 一覧は CC BY-SA 4.0 の表示つきで GitHub の Release に置く。辞書（system.dict）は利用者の手元で作る。
//!
//! # 持ち主
//! 契約のコミット（この `mod.rs`、`types.rs`、`rules.rs`、各サブコマンドの `*Config` と関数のシグネチャ）はレーンで変えない。
//! 足りなければリードに戻す。中身は A: `collect.rs`、B: `classify.rs`（と `sqldump.rs`）、C: `lex.rs`。

use std::fmt;

mod classify;
mod collect;
mod lex;
mod rules;
mod types;

pub use classify::{ClassifyConfig, classify};
pub use collect::{CollectConfig, collect};
pub use lex::{LexConfig, lex};
pub use rules::{
    DEFAULT_CATEGORIES_TEXT, MAINTENANCE_CATEGORY_MARKERS, Rule, RuleKind, RuleTable,
    is_maintenance_category,
};
pub use types::{
    ATTRIBUTION, CollectedRow, Division, LICENSE, LIST_MAGIC, LIST_SCHEMA, ListHeader, ListRow,
    MIN_DF_FLOOR, WebClass,
};

/// `webwords` の誤り。
#[derive(Debug)]
pub enum WebwordsError {
    /// 契約のコミットの段階で、中身がまだない
    NotImplemented(&'static str),
    /// 一覧や規則表の形が正しくない
    Format(String),
    /// 入出力
    Io(String),
}

impl fmt::Display for WebwordsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotImplemented(what) => write!(f, "webwords {what} は未実装です（ADR 0055）"),
            Self::Format(message) | Self::Io(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for WebwordsError {}
