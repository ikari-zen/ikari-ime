//! `cargo xtask check`（ADR 0043 §2-3・段 X1。レーン L-D）。
//! 記録の書式（`ikari-check/<SHA>` に `CHECK_EXIT=`・`SCOPE=` の 2 行、最後に `CHECK_EXIT=<コード>` を出す）を保つ。

use crate::{XtaskError, not_yet};

/// 引数（L-D が足す。`--docs-only`、`--real`、`--perf`、`--no-swift`、`--lm`、`--eval <基準>`）。
#[derive(Debug, clap::Args)]
pub struct Args {}

pub fn run(_args: Args) -> Result<(), XtaskError> {
    Err(not_yet("check", "X1", "scripts/check.sh"))
}
