//! `ikari-data` の実行ファイル（ADR 0076 §4.1）。引数を読んで仕事を 1 つして終わる。
//! 標準出力に報告の行（`resource::fetch_report`）、終了コードは 0（成功）、1（`error`）、2（引数が不正）。

use std::io::stdout;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = match std::env::args_os()
        .skip(1)
        .map(|a| a.into_string())
        .collect()
    {
        Ok(args) => args,
        // UTF-8 でない引数は、読めない引数として扱う
        Err(_) => return ExitCode::from(ikari_resource::fetch_report::EXIT_USAGE as u8),
    };
    let code = ikari_data::run(&args, &mut stdout().lock());
    ExitCode::from(code as u8)
}
