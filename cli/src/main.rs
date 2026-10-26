//! `ikari` CLI（ADR 0001 §5, ADR 0002 §14）。引数解析は `args.rs`、整形は `output.rs` に純粋関数として置く。
//!
//! 終了コード: 0 成功（候補0件も含む）、1 辞書の読み込みエラーなど、2 引数の誤り。

mod args;
mod convert;
mod engine_lock;
mod ingest_cmd;
mod output;
mod reading;
mod session;
mod stack;
mod user_cmds;

use std::error::Error;
use std::process::ExitCode;

use args::{Command, parse_command};

const EXIT_FAILURE: u8 = 1;

fn main() -> ExitCode {
    let command = match parse_command(std::env::args_os()) {
        Ok(c) => c,
        // --help / --version は 0、引数の誤りは 2 で終わる
        Err(e) => e.exit(),
    };
    match run(&command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

fn run(command: &Command) -> Result<(), Box<dyn Error>> {
    match command {
        Command::Convert(a) => convert::run_convert(a),
        Command::Session(a) => session::run_session(a),
        Command::UserDict(a) => user_cmds::run_user_dict(a),
        Command::Learn(a) => user_cmds::run_learn(a),
        Command::Profile(a) => user_cmds::run_profile(a),
        Command::Ingest(a) => ingest_cmd::run_ingest(a),
        Command::Data(a) => user_cmds::run_data(a),
    }
}
