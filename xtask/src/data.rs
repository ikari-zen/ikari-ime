//! `cargo xtask data`（ADR 0043 §3・段 X3。レーン L-D）。データの取得（`fetch`）と配置（`install`）。

use crate::{XtaskError, not_yet};

/// 引数（L-D が `fetch`・`install` の下位コマンドを足す）。
#[derive(Debug, clap::Args)]
pub struct Args {}

pub fn run(_args: Args) -> Result<(), XtaskError> {
    Err(not_yet(
        "data",
        "X3",
        "scripts/fetch-data.sh と scripts/install-data.sh",
    ))
}
