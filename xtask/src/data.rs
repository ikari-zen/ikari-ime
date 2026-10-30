//! `cargo xtask data`（ADR 0043 §3・段 X3。レーン L-D）。データの取得（`fetch`）と配置（`install`）。
//! 取得と配置は未着手。このモジュールは今、`check` と `dict` が共有する部品（`scripts/lib.sh` の定数、sha256）を持つ。

use crate::{XtaskError, not_yet};

pub(crate) mod consts;
pub(crate) mod run;
pub(crate) mod sha256;

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
