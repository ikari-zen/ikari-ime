//! `cargo xtask perf`（ADR 0041 §6.3、ADR 0043 §3。レーン L-P）。
//!
//! - `perf shell`（P1 キー 1 回、P3 殻の起動、P4 メモリ。Rust 版の測り手を呼ぶ。Swift 版の測り手は M5 で消した）
//! - `perf engine`（P5。今の `check.sh --perf` の `run_perf` を包む）
//! - `perf smoke`（P2。`--allow-smoke` のときだけ）
//! - `perf compare --baseline <file>`（表を出し、`tolerance.toml` の幅を超えたら 0 以外で終わる）
//!
//! 結果のファイルの形は `record.rs`。基準値は `eval/reports/shell-perf/baseline.json`。
//! 基準値を作る手順: `cargo xtask perf shell --out eval/reports/shell-perf/baseline.json`、
//! 続けて `perf engine --out …`（辞書が要る）。どちらもほかの区間の指標を残して混ぜる。

use crate::XtaskError;

mod compare;
mod engine;
mod record;
mod shell;
mod smoke;

/// 引数（下位コマンド）。
#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// P1（キー 1 回）、P3（殻の起動）、P4（メモリ）
    Shell(shell::Args),
    /// P5（エンジンとコアの性能。実辞書が要る）
    Engine(engine::Args),
    /// P2（変換の往復。smoke を流すので、ユーザーの了解が要る）
    Smoke(smoke::Args),
    /// 基準値との比較の表
    Compare(compare::Args),
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    match args.command {
        Command::Shell(args) => shell::run(args),
        Command::Engine(args) => engine::run(args),
        Command::Smoke(args) => smoke::run(args),
        Command::Compare(args) => compare::run(args),
    }
}
