//! テスト用の偽の `ikari-lm-host`（モデルなし）。配布物には入れない。
//!
//! 引数は本物と同じ（`--model <パス> --gpu on|off|cpu`）。`--model` のファイルを「台本」として読む（`key=value` の行）。
//! テストは、次に起動する子の挙動を `<台本のパス>.next` に書いて変えられる（あれば台本より優先する）。台本そのものは、
//! エンジンが照合したモデルとして開いたまま持つので（Windows は書き込みの共有なし）、書き換えない。
//!
//!   load=fail              読み込みの失敗を返して終わる
//!   ready=never            Ready を出さずに止まる
//!   ready_delay_ms=N       Ready の前に N ms 待つ
//!   version=N              Ready の版を N にする
//!   gpu=fallback           `--gpu on` のとき、GPU で読めず CPU で読み直したことにする
//!   gpu=unused             `--gpu on` でも、装置を CPU と報告する（GPU のない機械）
//!   die_on_gpu=1           `--gpu on` のとき、最初の要求で（答えずに）終わる（GPU のドライバで落ちる子）
//!   delay_ms=N             採点 1 回に N ms かける
//!   delay_steps=N          `delay_ms` を N 個の区切り（decode に見立てる）に分け、区切りで取り消しの印を見る（既定 1）
//!   exit_after=N           N 回答えたあと、次の要求で（答えずに）終わる
//!   hang_after=N           N 回答えたあと、次の要求で（答えずに）止まる
//!   garbage_after=N        N 回答えたあと、次の要求に壊れた枠を返す
//!   fail_infer=1           すべての要求に採点の失敗を返す（子は生きたまま）
//!   nll.<表記>=<数>        その表記の NLL（なければ表記の文字数）
//!
//! `--gpu off` で起動したときは装置を CPU と報告する（`--gpu on` は GPU）。
//! 要求が来たときに `<台本のパス>.die` があれば、答えずに終わる（動いている子を、テストが外から落とす）。
//! 起動のたびに、`<台本のパス>.starts` に `--gpu` の値（`on` / `off` / `cpu`）を 1 行足す（テストが、起動の回数と引数を見る）。

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use ikari_lm::host::proto::{Reply, write_frame};
use ikari_lm::host::{ARG_GPU, ARG_GPU_ON, ARG_MODEL, ReadyInfo, serve};
use ikari_lm::{CancelToken, GpuState, LmDevice, LmError, LmScorer};

const NLL_PREFIX: &str = "nll.";
const DIE_SUFFIX: &str = ".die";
const STARTS_SUFFIX: &str = ".starts";
const NEXT_SUFFIX: &str = ".next";
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
    die_flag: PathBuf,
    /// `--gpu on` で起動し、台本が `die_on_gpu=1`
    die_now: bool,
    answered: u64,
}

impl FakeScorer {
    fn reached(&self, key: &str) -> bool {
        self.script.number(key).is_some_and(|n| self.answered >= n)
    }
}

impl LmScorer for FakeScorer {
    fn nll(
        &mut self,
        _prompt: &str,
        candidates: &[String],
        cancel: &CancelToken,
    ) -> Result<Vec<f32>, LmError> {
        if self.die_now || self.die_flag.exists() || self.reached("exit_after") {
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
            let steps = self.script.number("delay_steps").unwrap_or(1).max(1);
            for _ in 0..steps {
                if cancel.is_cancelled() {
                    return Err(LmError::Cancelled);
                }
                thread::sleep(Duration::from_millis(ms / steps));
            }
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
    let gpu_arg = value_of(ARG_GPU).unwrap_or_default();
    let gpu_on = gpu_arg == ARG_GPU_ON;
    let next = {
        let mut path = model.clone().into_os_string();
        path.push(NEXT_SUFFIX);
        PathBuf::from(path)
    };
    let script = Script::read(if next.exists() { &next } else { &model });
    let die_flag = {
        let mut path = model.clone().into_os_string();
        path.push(DIE_SUFFIX);
        PathBuf::from(path)
    };
    let mut starts = model.clone().into_os_string();
    starts.push(STARTS_SUFFIX);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(starts)
    {
        let _ = writeln!(f, "{gpu_arg}");
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

    let die_now = gpu_on && script.is("die_on_gpu", "1");
    let load = move || {
        if script.is("load", "fail") {
            return Err(LmError::Load("fake load failure".to_owned()));
        }
        let fallback = gpu_on && (script.is("gpu", "fallback") || script.is("gpu", "unused"));
        let load_failed = gpu_on && script.is("gpu", "fallback");
        let info = ReadyInfo {
            device: if gpu_on && !fallback {
                LmDevice::Gpu
            } else {
                LmDevice::Cpu
            },
            gpu: match (gpu_on, fallback) {
                (true, true) if load_failed => GpuState::FallbackLoadFailed,
                (true, true) => GpuState::NoGpu,
                (true, false) => GpuState::On,
                (false, _) => GpuState::OffSetting,
            },
            devices: "cpu,fake:Gpu".to_owned(),
        };
        let scorer = FakeScorer {
            script,
            die_flag,
            die_now,
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
