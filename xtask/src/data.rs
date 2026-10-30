//! `cargo xtask data`（ADR 0043 §3・段 X3。レーン L-D）。データの取得（`fetch`）と配置（`install`）。
//! `scripts/fetch-data.sh` と `scripts/install-data.sh` の移し先。このモジュールは `check` と `dict` が共有する部品
//! （`scripts/lib.sh` の定数、sha256、外部コマンドの実行）も持つ。

use std::path::Path;

use crate::XtaskError;

pub(crate) mod consts;
pub(crate) mod fetch;
pub(crate) mod install;
pub(crate) mod run;
pub(crate) mod sha256;

/// データの取得と配置
#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// 辞書とモデルの元データを data/raw/ に取得する（開発の道具。製品は通信しない: NFR-17）。通信するのはこの下位コマンドだけ
    Fetch(fetch::Args),
    /// 辞書とモデルを、ユーザーのデータのディレクトリに入れる（ADR 0004 §8.3）
    Install(install::Args),
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    let result = match args.command {
        Command::Fetch(args) => fetch::run(args),
        Command::Install(args) => install::run(args),
    };
    result.map_err(|failure| XtaskError(failure.message))
}

/// `--data` / `--dest` などの空のパスを弾く。
pub(crate) fn non_empty(path: &Path, flag: &str) -> Result<(), run::Failure> {
    if path.as_os_str().is_empty() {
        Err(run::Failure::new(format!("{flag} が空です")))
    } else {
        Ok(())
    }
}
