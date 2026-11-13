//! 評価セットの読み込み、実行、集計、レポート（ADR 0001 §6、ADR 0002 §15）。
//!
//! - `set`: 評価セット（TSV）の読み込みと絞り込み
//! - `run`: 実行（順位と応答時間の計測）
//! - `report`: 集計とレポート

pub mod ab;
pub mod ajimee;
pub mod boundary;
pub mod carry;
pub mod compare;
pub mod gates;
pub mod holdout;
pub mod lm_timing;
pub mod ops;
pub mod reach;
pub mod reading;
pub mod report;
pub mod rss;
pub mod run;
pub mod scenario;
pub mod segment_engine;
pub mod segments;
pub mod set;
pub mod setup;

#[cfg(test)]
mod testutil;

pub(crate) use report::cell;
pub use report::{
    Group, LatencyStats, LmReport, ReportMeta, Summary, render_cases_tsv, render_markdown,
    summarize,
};
pub use run::{CaseResult, EvalConfig, ProfileResolver, RunOptions, run, run_with};
pub use segments::{SegmentCase, load_segment_cases};
pub(crate) use set::validate_expected;
pub use set::{
    ALT_SEPARATOR, EvalCase, EvalSetError, Filtered, GUARD_KINDS, Split, SplitSel, TAG_UNREACHABLE,
    filter_cases, load_cases, split_of_id,
};
