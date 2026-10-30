//! `cargo xtask eval`（ADR 0043 §3・段 X4。レーン L-D）。全セットの 1 位の比較（`snapshot`）と、N 位以内の確認（`rank-within`）。
//! `eval/scripts/snapshot.sh` と `eval/scripts/rank-within.sh` の移し先。

use crate::XtaskError;

mod rank_within;
mod snapshot;
mod textdiff;

/// 評価
#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// リファクタの前後で振る舞いが変わっていないことを確かめる snapshot を取る、または基準と比べる
    Snapshot(snapshot::Args),
    /// 評価セットの期待が N 位以内かを調べる（brand-katakana.tsv の回帰用）
    RankWithin(rank_within::Args),
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    match args.command {
        Command::Snapshot(args) => snapshot::run(args),
        Command::RankWithin(args) => {
            rank_within::run(args).map_err(|failure| XtaskError(failure.message))
        }
    }
}
