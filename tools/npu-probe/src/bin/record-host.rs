//! `npu-probe-record-host`: 入力を作るための LM の子（Mac。`scripts/make-inputs.sh`）。
//!
//! `ikari-lm-host` と同じ取り決め（`--model <GGUF> --gpu on|off|cpu`）で採点し、届いた頼み（プロンプトと候補）を
//! 1 行 1 件の JSON で、環境変数 `IKARI_NPU_PROBE_RECORD` のファイルに足す。件の id は `IKARI_NPU_PROBE_ID`。
//! 評価（`ikari-eval --lm-host`）が起動する。製品には入らない。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use ikari_lm::host::{ARG_GPU, ARG_GPU_CPU, ARG_GPU_OFF, ARG_MODEL, ReadyInfo, serve};
use ikari_lm::{CancelToken, LlamaScorer, LmDevice, LmError, LmScorer};

const RECORD_ENV: &str = "IKARI_NPU_PROBE_RECORD";
const ID_ENV: &str = "IKARI_NPU_PROBE_ID";
const EXIT_USAGE: u8 = 2;

struct Recording {
    inner: LlamaScorer,
    out: File,
    id: String,
}

impl LmScorer for Recording {
    fn nll(
        &mut self,
        prompt: &str,
        candidates: &[String],
        cancel: &CancelToken,
    ) -> Result<Vec<f32>, LmError> {
        let line = serde_json::json!({
            "id": self.id,
            "prompt": prompt,
            "candidates": candidates,
        });
        // 記録に失敗しても採点は続ける（評価を止めない）。足りない件は make-inputs が知らせる
        let _ = writeln!(self.out, "{line}");
        self.inner.nll(prompt, candidates, cancel)
    }

    fn device(&self) -> LmDevice {
        self.inner.device()
    }
}

fn main() -> ExitCode {
    let mut model = None;
    let mut gpu = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let Some(value) = args.next() else { break };
        match arg.as_str() {
            ARG_MODEL => model = Some(PathBuf::from(value)),
            ARG_GPU => gpu = Some(value),
            _ => {}
        }
    }
    let (Some(model), Some(gpu), Ok(record)) = (model, gpu, std::env::var(RECORD_ENV)) else {
        eprintln!(
            "usage: {RECORD_ENV}=<file> npu-probe-record-host {ARG_MODEL} <gguf> {ARG_GPU} <on|off|cpu>"
        );
        return ExitCode::from(EXIT_USAGE);
    };
    let id = std::env::var(ID_ENV).unwrap_or_default();
    let load = move || {
        let inner = match gpu.as_str() {
            ARG_GPU_CPU => LlamaScorer::load_cpu_only(&model)?,
            other => LlamaScorer::load_with(&model, other != ARG_GPU_OFF)?,
        };
        let out = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&record)
            .map_err(|e| LmError::Load(format!("{record}: {e}")))?;
        let info = ReadyInfo {
            device: inner.device(),
            gpu: ikari_lm::gpu_state(),
            devices: ikari_lm::devices().unwrap_or_default(),
        };
        let scorer = Recording { inner, out, id };
        Ok((Box::new(scorer) as Box<dyn LmScorer>, info))
    };
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
