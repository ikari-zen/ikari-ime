//! `cargo xtask dict`（ADR 0043 §3・段 X2。レーン L-D）。辞書のビルド。

use crate::{XtaskError, not_yet};

/// 引数（L-D が足す）。
#[derive(Debug, clap::Args)]
pub struct Args {}

pub fn run(_args: Args) -> Result<(), XtaskError> {
    Err(not_yet("dict", "X2", "scripts/build-dict.sh"))
}
