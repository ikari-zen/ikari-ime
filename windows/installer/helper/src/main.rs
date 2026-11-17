//! ikari-setup-helper.exe: インストーラが管理者で呼ぶ道具（説明は `lib.rs`）。
//!
//! 出力は標準出力に 1 行ずつ、固定の語と数（と、検査で問題のあったパス）だけ。終了コードは `exit`。

use std::process::ExitCode;

use ikari_setup_helper::{USAGE, exit, parse};

#[cfg(windows)]
#[allow(unsafe_code)]
mod os;

#[cfg(not(windows))]
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(message) = parse(&args) {
        eprintln!("{message}\n{USAGE}");
        return ExitCode::from(exit::USAGE);
    }
    eprintln!("ikari-setup-helper runs only on Windows");
    ExitCode::from(exit::UNSUPPORTED)
}

#[cfg(windows)]
fn main() -> ExitCode {
    use ikari_setup_helper::{Command, tree};

    os::restrict_dll_search();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match parse(&args) {
        Ok(c) => c,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(exit::USAGE);
        }
    };
    let code = match command {
        Command::CheckDir(dir) => {
            let report = tree::check(std::path::Path::new(&dir), &os::read_descriptor);
            for finding in &report.findings {
                println!(
                    "untrusted {} {}",
                    finding.path.display(),
                    finding.problems.join(",")
                );
            }
            println!(
                "checked={} problems={}",
                report.checked,
                report.findings.len()
            );
            if report.ok() {
                exit::OK
            } else {
                exit::UNTRUSTED
            }
        }
        Command::Stop(dir) => match os::stop_engines(&dir) {
            Ok(outcome) => {
                println!("{outcome}");
                if outcome.all_gone() {
                    exit::OK
                } else {
                    exit::REMAINING
                }
            }
            Err(e) => {
                println!("stop failed: {e}");
                exit::OS_ERROR
            }
        },
        Command::WerExclude(names) => wer(&names, true),
        Command::WerInclude(names) => wer(&names, false),
        Command::CageProfileDelete => {
            match os::delete_appcontainer_profile(ikari_setup_helper::LM_CAGE_PROFILE) {
                Ok(()) => {
                    println!("cage profile deleted");
                    exit::OK
                }
                Err(hr) => {
                    println!("cage profile delete failed hr={hr:#010x}");
                    exit::OS_ERROR
                }
            }
        }
    };
    ExitCode::from(code)
}

#[cfg(windows)]
fn wer(names: &[String], exclude: bool) -> u8 {
    let mut code = exit::OK;
    for name in names {
        match os::error_reporting(name, exclude) {
            Ok(()) => println!(
                "wer {} {name}",
                if exclude { "excluded" } else { "included" }
            ),
            Err(hr) => {
                println!("wer failed {name} hr={hr:#010x}");
                code = exit::OS_ERROR;
            }
        }
    }
    code
}
