//! `ikari-lm-host`: LM の推論だけをするプロセス（ADR 0060 Q4）。エンジンが起動し、標準入出力のパイプで採点を頼む。
//!
//!   ikari-lm-host --model <GGUF のパス> --gpu on|off
//!
//! 標準入力が閉じたら終わる（エンジンが子を手放した、またはエンジンが死んだ）。ファイルはモデルを読むだけで、
//! 通信はしない（NFR-17）。取り決めは `ikari_lm::host::proto`。

use std::path::PathBuf;
use std::process::ExitCode;

use ikari_lm::host::{ARG_GPU, ARG_GPU_OFF, ARG_GPU_ON, ARG_MODEL, ReadyInfo, serve};
use ikari_lm::{LlamaScorer, LmScorer};

const EXIT_USAGE: u8 = 2;

struct Args {
    model: PathBuf,
    gpu: bool,
}

fn parse_args() -> Option<Args> {
    let mut model = None;
    let mut gpu = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let value = args.next()?;
        match arg.to_str()? {
            ARG_MODEL => model = Some(PathBuf::from(value)),
            ARG_GPU => {
                gpu = Some(match value.to_str()? {
                    ARG_GPU_ON => true,
                    ARG_GPU_OFF => false,
                    _ => return None,
                });
            }
            _ => return None,
        }
    }
    Some(Args {
        model: model?,
        gpu: gpu?,
    })
}

fn main() -> ExitCode {
    let Some(args) = parse_args() else {
        eprintln!(
            "usage: ikari-lm-host {ARG_MODEL} <model.gguf> {ARG_GPU} {ARG_GPU_ON}|{ARG_GPU_OFF}"
        );
        return ExitCode::from(EXIT_USAGE);
    };
    let load = move || {
        let scorer = LlamaScorer::load_with(&args.model, args.gpu)?;
        let info = ReadyInfo {
            device: scorer.device(),
            gpu: ikari_lm::gpu_state(),
            devices: ikari_lm::devices().unwrap_or_default(),
        };
        Ok((Box::new(scorer) as Box<dyn LmScorer>, info))
    };
    // 入力が閉じたのに本体が戻らない（推論の中で固まっている）ときは、ここでプロセスを終わらせる
    let on_input_end = || std::process::exit(0);
    match serve(
        std::io::stdin(),
        std::io::stdout().lock(),
        load,
        on_input_end,
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
