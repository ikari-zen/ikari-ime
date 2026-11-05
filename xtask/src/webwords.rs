//! `cargo xtask webwords`（メンテナー用。ADR 0055 §5.1）。Wikipedia 日本語版のダンプから外来語の一覧を作る手順。
//! 契約のコミットでは入口だけ（中身は レーン D）。利用者の辞書のビルドは `cargo xtask dict` が一覧を使う（§5.3）。

use std::path::PathBuf;

use clap::Subcommand;

use crate::XtaskError;

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// dumps.wikimedia.org から pages-articles と SQL ダンプを空の作業用ディレクトリに取る（sha1 を照合し、sha256 を記録する）
    Fetch {
        /// ダンプの日付（例: 20261001）
        #[arg(long, value_name = "YYYYMMDD")]
        dump: String,
        /// 作業用ディレクトリ（空であること。data/raw には置かない）
        #[arg(long, value_name = "DIR")]
        work: PathBuf,
    },
    /// 取ったダンプから collect と classify を流し、`webwords-<日付>.tsv` とレポートを作る
    Build {
        /// ダンプの日付
        #[arg(long, value_name = "YYYYMMDD")]
        dump: String,
        /// `fetch` の作業用ディレクトリ
        #[arg(long, value_name = "DIR")]
        work: PathBuf,
        /// 出力先ディレクトリ
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
    },
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    let name = match args.command {
        Command::Fetch { .. } => "fetch",
        Command::Build { .. } => "build",
    };
    Err(XtaskError(format!(
        "webwords {name} は未実装です（ADR 0055 §7 のレーン D）"
    )))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Debug, Parser)]
    struct Cli {
        #[command(flatten)]
        args: Args,
    }

    #[test]
    fn fetch_and_build_parse_and_are_not_implemented() {
        let fetch =
            Cli::try_parse_from(["x", "fetch", "--dump", "20261001", "--work", "w"]).unwrap();
        assert!(run(fetch.args).is_err());
        let build = Cli::try_parse_from([
            "x", "build", "--dump", "20261001", "--work", "w", "--out", "o",
        ])
        .unwrap();
        assert!(run(build.args).is_err());
    }
}
