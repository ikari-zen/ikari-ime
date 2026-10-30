//! `cargo xtask eval`（ADR 0043 §3・段 X4。レーン L-D）。全セットの 1 位の比較（`snapshot`）。

use crate::{XtaskError, not_yet};

/// 引数（L-D が `snapshot` の下位コマンドを足す）。
#[derive(Debug, clap::Args)]
pub struct Args {}

pub fn run(_args: Args) -> Result<(), XtaskError> {
    Err(not_yet("eval", "X4", "eval/scripts/snapshot.sh"))
}
