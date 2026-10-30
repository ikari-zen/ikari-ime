//! `cargo xtask perf`（ADR 0041 §6.3、ADR 0043 §3。レーン L-P）。
//! `perf shell`（P1・P3・P4）、`perf engine`（P5。今の `check.sh --perf` の `run_perf`）、`perf smoke`（P2。`--allow-smoke` のときだけ）、
//! `perf compare --baseline <file>`。

use crate::{XtaskError, not_yet};

/// 引数（L-P が `shell`・`engine`・`smoke`・`compare` の下位コマンドを足す）。
#[derive(Debug, clap::Args)]
pub struct Args {}

pub fn run(_args: Args) -> Result<(), XtaskError> {
    Err(not_yet("perf", "L-P", "scripts/check.sh --perf"))
}
