//! テスト用の偽の `ikari-lm-host`（モデルなし）。配布物には入れない。
//!
//! 引数は本物と同じ（`--model <パス> --gpu on|off`）。`--model` のファイルを「台本」として読む（`key=value` の行）。
//! テストは台本を書き換えて、次に起動する子の挙動を変えられる。
//!
//!   load=fail              読み込みの失敗を返して終わる
//!   ready=never            Ready を出さずに止まる
//!   ready_delay_ms=N       Ready の前に N ms 待つ
//!   version=N              Ready の版を N にする
//!   gpu=fallback           `--gpu on` のとき、GPU で読めず CPU で読み直したことにする
//!   delay_ms=N             採点 1 回に N ms かける
//!   exit_after=N           N 回答えたあと、次の要求で（答えずに）終わる
//!   hang_after=N           N 回答えたあと、次の要求で（答えずに）止まる
//!   garbage_after=N        N 回答えたあと、次の要求に壊れた枠を返す
//!   fail_infer=1           すべての要求に採点の失敗を返す（子は生きたまま）
//!   nll.<表記>=<数>        その表記の NLL（なければ表記の文字数）
//!
//! `--gpu off` で起動したときは装置を CPU と報告する（`--gpu on` は GPU）。
//! 起動のたびに、`<台本のパス>.starts` に `--gpu` の値を 1 行足す（テストが、起動の回数と引数を見る）。

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use ikari_lm::host::proto::{Reply, write_frame};
use ikari_lm::host::{ARG_GPU, ARG_GPU_ON, ARG_MODEL, ReadyInfo, serve};
use ikari_lm::{GpuState, LmDevice, LmError, LmScorer};

const NLL_PREFIX: &str = "nll.";
const STARTS_SUFFIX: &str = ".starts";
const FOREVER: Duration = Duration::from_secs(3600);

#[derive(Default)]
struct Script {
    kv: HashMap<String, String>,
}

impl Script {
    fn read(path: &PathBuf) -> Script {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let kv = text
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
            .collect();
        Script { kv }
    }

    fn is(&self, key: &str, value: &str) -> bool {
        self.kv.get(key).is_some_and(|v| v == value)
    }

    fn number(&self, key: &str) -> Option<u64> {
        self.kv.get(key).and_then(|v| v.parse().ok())
    }
}

struct FakeScorer {
    script: Script,
    answered: u64,
}

impl FakeScorer {
    fn reached(&self, key: &str) -> bool {
        self.script.number(key).is_some_and(|n| self.answered >= n)
    }
}

impl LmScorer for FakeScorer {
    fn nll(&mut self, _prompt: &str, candidates: &[String]) -> Result<Vec<f32>, LmError> {
        if self.reached("exit_after") {
            std::process::exit(1);
        }
        if self.reached("hang_after") {
            thread::sleep(FOREVER);
        }
        if self.reached("garbage_after") {
            let mut out = std::io::stdout().lock();
            let _ = write_frame(&mut out, &[0x00, 0x01, 0x02]);
            let _ = out.flush();
            thread::sleep(FOREVER);
        }
        if let Some(ms) = self.script.number("delay_ms") {
            thread::sleep(Duration::from_millis(ms));
        }
        if self.script.is("fail_infer", "1") {
            return Err(LmError::Inference("fake failure".to_owned()));
        }
        self.answered += 1;
        Ok(candidates
            .iter()
            .map(|c| {
                self.script
                    .kv
                    .get(&format!("{NLL_PREFIX}{c}"))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(c.chars().count() as f32)
            })
            .collect())
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value_of = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let Some(model) = value_of(ARG_MODEL).map(PathBuf::from) else {
        return ExitCode::from(2);
    };
    let gpu_on = value_of(ARG_GPU).as_deref() == Some(ARG_GPU_ON);
    let script = Script::read(&model);
    let mut starts = model.clone().into_os_string();
    starts.push(STARTS_SUFFIX);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(starts)
    {
        let _ = writeln!(f, "{}", if gpu_on { "on" } else { "off" });
    }

    if script.is("ready", "never") {
        thread::sleep(FOREVER);
    }
    if let Some(ms) = script.number("ready_delay_ms") {
        thread::sleep(Duration::from_millis(ms));
    }
    if let Some(version) = script.number("version") {
        let ready = Reply::Ready {
            version: version as u16,
            device: LmDevice::Cpu,
            gpu: GpuState::NotBuilt,
            devices: "cpu".to_owned(),
        };
        let _ = write_frame(&mut std::io::stdout().lock(), &ready.encode());
        thread::sleep(FOREVER);
    }

    let load = move || {
        if script.is("load", "fail") {
            return Err(LmError::Load("fake load failure".to_owned()));
        }
        let fallback = gpu_on && script.is("gpu", "fallback");
        let info = ReadyInfo {
            device: if gpu_on && !fallback {
                LmDevice::Gpu
            } else {
                LmDevice::Cpu
            },
            gpu: match (gpu_on, fallback) {
                (true, true) => GpuState::FallbackLoadFailed,
                (true, false) => GpuState::On,
                (false, _) => GpuState::OffSetting,
            },
            devices: "cpu,fake:Gpu".to_owned(),
        };
        let scorer = FakeScorer {
            script,
            answered: 0,
        };
        Ok((Box::new(scorer) as Box<dyn LmScorer>, info))
    };
    match serve(std::io::stdin(), std::io::stdout().lock(), load, || {
        std::process::exit(0)
    }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
