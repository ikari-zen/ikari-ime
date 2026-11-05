//! `cargo xtask`: ビルドと検証の道具（ADR 0043）。Windows でも要るもの（check、辞書のビルド、データの取得と配置、評価、
//! 性能の計測）を、シェルに依存しない Rust にまとめる。macOS 専用のもの（署名、install、smoke）はシェルのまま。
//!
//! # 持ち主（ADR 0041 §7.2、ADR 0043 §3）
//! - この `main.rs` は M1-a で固めた（下位コマンドの入口だけ）。レーンは変えない。足りなければリードに戻す
//! - L-D: `check`、`dict`、`data`、`eval`（段 X1〜X4。各モジュールの `Args` と `run` の中身）
//! - L-P: `perf`（ADR 0041 §6.3）
//!
//! 引数はモジュールごとの `Args` が持つので、レーンは `main.rs` を変えずに引数を足せる。

use std::process::ExitCode;

use clap::{Parser, Subcommand};

mod check;
mod cov;
mod data;
mod dict;
mod eval;
mod perf;
mod webwords;

#[derive(Debug, Parser)]
#[command(
    name = "xtask",
    bin_name = "cargo xtask",
    about = "ikari のビルドと検証の道具（ADR 0043）"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// fmt、clippy、cargo test、swift test（macOS のみ）、文書の検査と、結果の記録（段 X1。今は scripts/check.sh）
    Check(check::Args),
    /// 辞書のビルド（段 X2。今は scripts/build-dict.sh）
    Dict(dict::Args),
    /// データの取得と配置（段 X3。今は scripts/fetch-data.sh、scripts/install-data.sh）
    Data(data::Args),
    /// 評価（段 X4。今は eval/scripts/snapshot.sh）
    Eval(eval::Args),
    /// 性能の計測と基準値との比較（ADR 0041 §6.3。L-P）
    Perf(perf::Args),
    /// テストの行カバレッジの測定（cargo-llvm-cov が要る。issue #158）
    Cov(cov::Args),
    /// Wikipedia 日本語版のダンプから外来語の一覧を作る（メンテナー用。ADR 0055 §5.1。issue #113）
    Webwords(webwords::Args),
}

/// xtask の誤り。メッセージは利用者に出す。
#[derive(Debug)]
pub struct XtaskError(String);

impl std::fmt::Display for XtaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for XtaskError {}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Check(args) => check::run(args),
        Command::Dict(args) => dict::run(args),
        Command::Data(args) => data::run(args),
        Command::Eval(args) => eval::run(args),
        Command::Perf(args) => perf::run(args),
        Command::Cov(args) => cov::run(args),
        Command::Webwords(args) => webwords::run(args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn cli_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn subcommands_are_the_ones_in_adr_0043() {
        let names: Vec<String> = Cli::command()
            .get_subcommands()
            .map(|command| command.get_name().to_owned())
            .collect();
        assert_eq!(
            names,
            ["check", "dict", "data", "eval", "perf", "cov", "webwords"]
        );
    }
}
