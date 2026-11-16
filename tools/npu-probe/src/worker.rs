//! 1 つの装置を測る子のプロセス（`ikari-npu-probe --worker <装置>`）。親は装置ごとに自分を起動し直す
//! （ADR 0075 §8「失敗しても次の行に進む」）。子が落ちても、ほかの行は測れる。メモリの最大も装置ごとに分かれる。
//! 結果は標準出力の最後の行に 1 つの JSON で書く。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::device::Device;
use crate::input::Item;
use crate::score::TokenSet;

/// 子に頼む測り方
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// 読み込み、初回、暖機、全件 × 回数
    Full,
    /// 読み込みと初回だけ（2 回目の起動。OpenVINO のキャッシュありの初回）
    First,
    /// 1 件だけ（檻の中で動くかの確認）
    Quick,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Full => "full",
            Mode::First => "first",
            Mode::Quick => "quick",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        [Mode::Full, Mode::First, Mode::Quick]
            .into_iter()
            .find(|m| m.name() == s)
    }
}

/// 子に頼む仕事
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub device: Device,
    /// OpenVINO の列の数 B（llama は無し）
    pub batch: Option<usize>,
    pub mode: Mode,
    /// 暖機のあとに全件を流す回数の上限
    pub reps: usize,
    /// 回数を流す時間の上限（超えたら、その回で終える）
    pub budget: Duration,
    /// ツールのフォルダ（`models\` を読む）
    pub dir: PathBuf,
    /// 入力（無ければ埋めたもの）
    pub inputs: Option<PathBuf>,
    /// OpenVINO のモデルのキャッシュ（`CACHE_DIR`）。無ければキャッシュしない
    pub cache: Option<PathBuf>,
    /// OpenVINO の C API の DLL（無ければ exe の隣）
    pub ov_lib: Option<PathBuf>,
}

/// 子の結果
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkerResult {
    pub device: String,
    pub batch: Option<usize>,
    pub mode: String,
    pub ok: bool,
    /// 失敗の理由（パスを伏せる前の文。親が伏せて切り詰める）
    pub error: Option<String>,
    /// 実際に動いた装置（llama は `gpu` / `cpu`、OpenVINO は装置の名前）
    pub ran_on: Option<String>,
    /// 読み込み（OpenVINO はコンパイルを含む）の時間
    pub load_ms: Option<f64>,
    /// 読み込みの始めから、最初の 1 件の結果までの時間
    pub first_ms: Option<f64>,
    /// OpenVINO の長さごとのコンパイルの時間（T, ms）
    pub compile_ms: Vec<(usize, f64)>,
    /// 件ごとの、暖機のあとの 1 回の時間（µs）
    pub times_us: Vec<Vec<u64>>,
    /// 件ごとの NLL（評価できない候補は None）
    pub nll: Vec<Vec<Option<f32>>>,
    /// 暖機のあとに流せた回数
    pub reps: usize,
    /// 長すぎて採点しなかった候補の数
    pub too_long: usize,
    /// プロセスの作業メモリの最大（バイト。Windows は Private Bytes の最大）
    pub peak_mem: Option<u64>,
    /// llama-cpu の子が出すトークン列
    pub tokens: Option<TokenSet>,
    /// 補足（GPU に載らず CPU に落ちた、など）
    pub note: Option<String>,
}

impl WorkerResult {
    pub fn failed(job_device: Device, batch: Option<usize>, mode: Mode, error: String) -> Self {
        WorkerResult {
            device: job_device.to_string(),
            batch,
            mode: mode.name().to_owned(),
            ok: false,
            error: Some(error),
            ..WorkerResult::default()
        }
    }
}

/// 1 件ずつ採点する口（llama と OpenVINO の違いを隠す）
pub trait ItemScorer {
    /// `index` 番目の件の、候補ごとの NLL
    fn score(&mut self, index: usize) -> Result<Vec<Option<f32>>, String>;
}

/// 開いた採点器
pub struct Opened {
    pub scorer: Box<dyn ItemScorer>,
    pub load: Duration,
    pub ran_on: Option<String>,
    pub compile_ms: Vec<(usize, f64)>,
    pub too_long: usize,
    pub tokens: Option<TokenSet>,
    pub note: Option<String>,
    /// 測れない（GPU に載らなかった、など）。測りはするが、失敗として出す
    pub not_ok: Option<String>,
}

/// 測った結果
#[derive(Debug, Default)]
pub struct Measured {
    pub first: Option<Duration>,
    pub times_us: Vec<Vec<u64>>,
    pub nll: Vec<Vec<Option<f32>>>,
    pub reps: usize,
    pub error: Option<String>,
}

fn finite(v: Vec<Option<f32>>) -> Vec<Option<f32>> {
    v.into_iter().map(|x| x.filter(|f| f.is_finite())).collect()
}

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// 初回、暖機、回数の順に流す
pub fn measure(
    scorer: &mut dyn ItemScorer,
    items: usize,
    mode: Mode,
    reps: usize,
    budget: Duration,
) -> Measured {
    let mut m = Measured {
        nll: vec![Vec::new(); items],
        times_us: vec![Vec::new(); items],
        ..Measured::default()
    };
    if items == 0 {
        m.error = Some("入力が無い".into());
        return m;
    }
    let started = Instant::now();
    match scorer.score(0) {
        Ok(v) => m.nll[0] = finite(v),
        Err(e) => {
            m.error = Some(e);
            return m;
        }
    }
    m.first = Some(started.elapsed());
    if mode != Mode::Full {
        return m;
    }
    // 暖機（全件を 1 回。長さごとの初回を測る回に混ぜない）。ここの NLL を一致度に使う
    for i in 0..items {
        match scorer.score(i) {
            Ok(v) => m.nll[i] = finite(v),
            Err(e) => {
                m.error = Some(format!("{} 件目: {e}", i + 1));
                return m;
            }
        }
    }
    let started = Instant::now();
    for _ in 0..reps {
        for i in 0..items {
            let t = Instant::now();
            if let Err(e) = scorer.score(i) {
                m.error = Some(format!("{} 件目: {e}", i + 1));
                return m;
            }
            m.times_us[i].push(micros(t.elapsed()));
        }
        m.reps += 1;
        if started.elapsed() > budget {
            break;
        }
    }
    m
}

/// 子の仕事をする。どこで失敗しても結果を返す（標準出力に書くのは呼び出し側）
pub fn run(job: &Job, items: &[Item]) -> WorkerResult {
    let opened = match open(job, items) {
        Ok(o) => o,
        Err(e) => {
            let mut r = WorkerResult::failed(job.device, job.batch, job.mode, e);
            r.peak_mem = crate::mem::peak_bytes();
            return r;
        }
    };
    let Opened {
        mut scorer,
        load,
        ran_on,
        compile_ms,
        too_long,
        tokens,
        note,
        not_ok,
    } = opened;
    let n = if job.mode == Mode::Quick {
        1
    } else {
        items.len()
    };
    let measured = measure(scorer.as_mut(), n, job.mode, job.reps, job.budget);
    let first_ms = measured.first.map(|f| ms(load) + ms(f));
    let error = measured.error.or(not_ok);
    WorkerResult {
        device: job.device.to_string(),
        batch: job.batch,
        mode: job.mode.name().to_owned(),
        ok: error.is_none(),
        error,
        ran_on,
        load_ms: Some(ms(load)),
        first_ms,
        compile_ms,
        times_us: measured.times_us,
        nll: measured.nll,
        reps: measured.reps,
        too_long,
        peak_mem: crate::mem::peak_bytes(),
        tokens,
        note,
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn open(job: &Job, items: &[Item]) -> Result<Opened, String> {
    match job.device {
        Device::LlamaCpu | Device::LlamaVulkan => crate::llama_dev::open(job, items),
        Device::Ov(target, precision) => crate::ov::open(job, target, precision, &token_set(items)),
        Device::LlamaOvNpu => {
            Err("このツールでは作っていない（後回し。ADR 0075 §4.3 の A）".into())
        }
    }
}

/// 入力（fixture）のトークン列。OpenVINO の子はこれを使う（契約の「トークン化」。llama.cpp で作ったもの）
pub fn token_set(items: &[Item]) -> TokenSet {
    TokenSet {
        eos: crate::score::END_TOKEN,
        items: items
            .iter()
            .map(|i| crate::score::ItemTokens {
                prompt: i.prompt_tokens.clone(),
                candidates: i.candidate_tokens.clone(),
            })
            .collect(),
    }
}

/// `models\` の中の GGUF（決まった名前、無ければ最初の `.gguf`）
pub fn gguf_path(dir: &Path) -> Option<PathBuf> {
    let models = dir.join(crate::MODELS_DIR);
    let fixed = models.join(crate::GGUF_NAME);
    if fixed.is_file() {
        return Some(fixed);
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir(&models)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("gguf"))
        })
        .collect();
    found.sort();
    found.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        calls: usize,
        fail_at: Option<usize>,
    }

    impl ItemScorer for Fake {
        fn score(&mut self, index: usize) -> Result<Vec<Option<f32>>, String> {
            self.calls += 1;
            if self.fail_at == Some(self.calls) {
                return Err("boom".into());
            }
            Ok(vec![Some(index as f32), Some(f32::INFINITY)])
        }
    }

    #[test]
    fn full_runs_first_warm_and_reps() {
        let mut f = Fake {
            calls: 0,
            fail_at: None,
        };
        let m = measure(&mut f, 3, Mode::Full, 2, Duration::from_secs(60));
        assert_eq!(f.calls, 1 + 3 + 2 * 3);
        assert_eq!(m.reps, 2);
        assert!(m.times_us.iter().all(|t| t.len() == 2));
        assert_eq!(m.nll[2], vec![Some(2.0), None]);
        assert!(m.error.is_none());
    }

    #[test]
    fn first_mode_runs_once() {
        let mut f = Fake {
            calls: 0,
            fail_at: None,
        };
        let m = measure(&mut f, 3, Mode::First, 5, Duration::from_secs(60));
        assert_eq!(f.calls, 1);
        assert!(m.first.is_some());
        assert_eq!(m.reps, 0);
    }

    #[test]
    fn budget_stops_after_one_rep() {
        let mut f = Fake {
            calls: 0,
            fail_at: None,
        };
        let m = measure(&mut f, 2, Mode::Full, 10, Duration::ZERO);
        assert_eq!(m.reps, 1);
    }

    #[test]
    fn error_is_reported() {
        let mut f = Fake {
            calls: 0,
            fail_at: Some(3),
        };
        let m = measure(&mut f, 3, Mode::Full, 1, Duration::from_secs(60));
        assert_eq!(m.error.as_deref(), Some("2 件目: boom"));
    }

    #[test]
    fn mode_names_round_trip() {
        for m in [Mode::Full, Mode::First, Mode::Quick] {
            assert_eq!(Mode::parse(m.name()), Some(m));
        }
    }
}
