//! 画面の要約（10 行前後）と `report.txt`（ADR 0075 §4.4）。
//!
//! 会社 PC から持ち出せるのは目で見たものだけなので、要約だけで判断できるようにする: 装置ごとに 1 行
//! （載ったか、p50・p99、初回、メモリ、llama-cpu を基準にした 1 位の一致）、檻の 1 行、最後に撤退の基準の判定。

use std::fmt::Write as _;
use std::time::Duration;

use crate::device::{Device, OvTarget, Precision};
use crate::ov::OvInfo;
use crate::stats::{self, Agreement};
use crate::worker::WorkerResult;

/// 撤退の基準（**仮置き**。ADR 0075 §5、2026-11-16 ユーザー決定）: NPU の p50 が Vulkan のこの倍率以下でなければ採らない
pub const SPEED_RATIO_MAX: f64 = 0.8;
/// キャッシュありの初回（読み込みとコンパイルと最初の 1 件）がこれを超えたら採らない（ms）
pub const CACHED_FIRST_MAX_MS: f64 = 2000.0;
/// llama-cpu と比べた 1 位の一致がこれ未満なら採らない
pub const TOP1_MIN: f64 = 0.95;
/// 要約に出す失敗の理由の長さ（文字）
const SUMMARY_REASON_CHARS: usize = 48;
/// 報告に出す失敗の理由の長さ（文字。ADR 0075 §4.4）
pub const REPORT_REASON_CHARS: usize = 200;
const BYTES_PER_MB: f64 = 1024.0 * 1024.0;

/// 先頭の情報
#[derive(Clone, Debug, Default)]
pub struct Header {
    pub tool: String,
    pub os: String,
    pub cpu: String,
    pub ov: Option<OvInfo>,
    pub ov_error: Option<String>,
    pub llama_devices: Option<String>,
    /// （ファイルの名前、sha256 の先頭 12 桁）
    pub models: Vec<(String, String)>,
    pub inputs: usize,
    pub candidates: usize,
    /// llama のトークン列と、入力の `prompt_ids` の一致（一致した件、比べた件）
    pub token_match: Option<(usize, usize)>,
}

/// 1 つの装置の結果
#[derive(Clone, Debug)]
pub struct DeviceRun {
    pub device: Device,
    /// 暖機のあと全件を流した子（OpenVINO は B ごと）
    pub full: Vec<WorkerResult>,
    /// キャッシュありの初回を測った子（OpenVINO だけ。B ごと）
    pub first: Vec<WorkerResult>,
    /// 測らなかった理由（モデルが無い、など）
    pub skipped: Option<String>,
}

/// 檻の確認の 1 つ
#[derive(Clone, Debug)]
pub struct SandboxRow {
    pub confine: &'static str,
    pub device: Device,
    pub ok: bool,
    pub reason: Option<String>,
}

/// 全体
#[derive(Clone, Debug, Default)]
pub struct Run {
    pub header: Header,
    pub devices: Vec<DeviceRun>,
    pub sandbox: Vec<SandboxRow>,
    /// 檻の確認を飛ばした理由（Windows でない、`--no-appcontainer` など）
    pub sandbox_notes: Vec<String>,
    pub elapsed: Duration,
}

/// 時間の要約（µs）
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub samples: usize,
}

pub fn timing(r: &WorkerResult) -> Option<Timing> {
    let all: Vec<u64> = r.times_us.iter().flatten().copied().collect();
    Some(Timing {
        p50: stats::percentile(&all, 50.0)?,
        p95: stats::percentile(&all, 95.0)?,
        p99: stats::percentile(&all, 99.0)?,
        samples: all.len(),
    })
}

impl DeviceRun {
    /// 成功した子のうち p50 の最も小さいもの（OpenVINO は B を選ぶ）
    pub fn best(&self) -> Option<&WorkerResult> {
        self.full
            .iter()
            .filter(|r| r.ok)
            .filter_map(|r| timing(r).map(|t| (r, t)))
            .min_by_key(|(_, t)| t.p50)
            .map(|(r, _)| r)
    }

    /// `full` と同じ B の、キャッシュありの初回
    pub fn first_for(&self, batch: Option<usize>) -> Option<&WorkerResult> {
        self.first.iter().find(|r| r.batch == batch)
    }

    /// 載らなかった理由（成功した子が無いとき）
    pub fn failure(&self) -> Option<String> {
        if self.best().is_some() {
            return None;
        }
        if let Some(s) = &self.skipped {
            return Some(s.clone());
        }
        self.full
            .iter()
            .find_map(|r| r.error.clone())
            .or_else(|| Some("結果が無い".into()))
    }
}

impl Run {
    pub fn get(&self, device: Device) -> Option<&DeviceRun> {
        self.devices.iter().find(|d| d.device == device)
    }

    fn base(&self) -> Option<&WorkerResult> {
        self.get(Device::LlamaCpu).and_then(DeviceRun::best)
    }

    /// llama-cpu を基準にした一致度
    pub fn agreement(&self, r: &WorkerResult) -> Option<Agreement> {
        let base = self.base()?;
        Some(stats::agreement(&base.nll, &r.nll))
    }

    /// 同じ精度の ov-cpu を基準にした一致度（装置の差を量子化の差から分ける）
    pub fn agreement_ov(&self, device: Device, r: &WorkerResult) -> Option<Agreement> {
        let Device::Ov(t, p) = device else {
            return None;
        };
        if t == OvTarget::Cpu {
            return None;
        }
        let base = self.get(Device::Ov(OvTarget::Cpu, p))?.best()?;
        Some(stats::agreement(&base.nll, &r.nll))
    }
}

fn truncate(s: &str, chars: usize) -> String {
    let one_line = s.replace(['\r', '\n'], " ");
    if one_line.chars().count() <= chars {
        return one_line;
    }
    let mut out: String = one_line.chars().take(chars).collect();
    out.push('…');
    out
}

fn ms(us: u64) -> String {
    format!("{:.1}", us as f64 / 1000.0)
}

fn secs(ms: f64) -> String {
    format!("{:.2}s", ms / 1000.0)
}

fn mb(bytes: Option<u64>) -> String {
    bytes.map_or("-".into(), |b| format!("{:.0}MB", b as f64 / BYTES_PER_MB))
}

fn percent(rate: Option<f64>) -> String {
    rate.map_or("-".into(), |r| format!("{:.1}%", r * 100.0))
}

/// 要約に並べる装置（INT8 の CPU・GPU は報告だけ）
fn summary_devices() -> Vec<Device> {
    vec![
        Device::LlamaCpu,
        Device::LlamaVulkan,
        Device::Ov(OvTarget::Cpu, Precision::Fp16),
        Device::Ov(OvTarget::Gpu, Precision::Fp16),
        Device::Ov(OvTarget::Npu, Precision::Fp16),
        Device::Ov(OvTarget::Npu, Precision::Int8),
        Device::LlamaOvNpu,
    ]
}

/// 檻の要約での短い名前
fn short(device: Device) -> String {
    match device {
        Device::LlamaCpu => "cpu".into(),
        Device::LlamaVulkan => "vk".into(),
        Device::Ov(OvTarget::Cpu, _) => "ovc".into(),
        Device::Ov(OvTarget::Gpu, _) => "ovg".into(),
        Device::Ov(OvTarget::Npu, _) => "npu".into(),
        Device::LlamaOvNpu => "lovn".into(),
    }
}

fn device_line(run: &Run, d: &DeviceRun) -> String {
    let name = format!("{:<13}", d.device.to_string());
    let Some(best) = d.best() else {
        let reason = d.failure().unwrap_or_default();
        let mark = if d.skipped.is_some() { "-" } else { "×" };
        return format!("{name}{mark} {}", truncate(&reason, SUMMARY_REASON_CHARS));
    };
    let t = timing(best).expect("best は時間がある");
    let batch = best.batch.map_or(String::new(), |b| format!("B{b} "));
    let first = best.first_ms.map_or("-".into(), secs);
    let cached = if d.device.is_ov() {
        let c = d
            .first_for(best.batch)
            .filter(|f| f.ok)
            .and_then(|f| f.first_ms)
            .map_or("-".into(), secs);
        format!("/キャッシュ {c}")
    } else {
        String::new()
    };
    let agree = if d.device == Device::LlamaCpu {
        "基準".into()
    } else {
        percent(run.agreement(best).and_then(|a| a.top1_rate()))
    };
    format!(
        "{name}○ {batch}p50 {}ms p99 {}ms 初回 {first}{cached} mem {} 1位一致 {agree}",
        ms(t.p50),
        ms(t.p99),
        mb(best.peak_mem)
    )
}

fn sandbox_line(run: &Run) -> Option<String> {
    if run.sandbox.is_empty() {
        return run
            .sandbox_notes
            .first()
            .map(|n| format!("檻   {}", truncate(n, SUMMARY_REASON_CHARS)));
    }
    let mut parts = Vec::new();
    for confine in ["低い整合性", "AppContainer"] {
        let rows: Vec<String> = run
            .sandbox
            .iter()
            .filter(|r| r.confine == confine)
            .map(|r| format!("{}{}", short(r.device), if r.ok { "○" } else { "×" }))
            .collect();
        if !rows.is_empty() {
            let label = if confine == "低い整合性" {
                "低IL"
            } else {
                "AC"
            };
            parts.push(format!("{label}: {}", rows.join(" ")));
        }
    }
    Some(format!("檻   {}", parts.join(" | ")))
}

/// 撤退の基準の判定
#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub device: Device,
    pub batch: Option<usize>,
    /// （○/×/?、説明）
    pub speed: (char, String),
    pub cached_first: (char, String),
    pub top1: (char, String),
    /// 載らなかった
    pub failed: Option<String>,
}

impl Verdict {
    /// 全体: どれか × なら ×、すべて ○ なら ○、ほかは ?
    pub fn overall(&self) -> char {
        if self.failed.is_some() {
            return '×';
        }
        let marks = [self.speed.0, self.cached_first.0, self.top1.0];
        if marks.contains(&'×') {
            '×'
        } else if marks.iter().all(|&m| m == '○') {
            '○'
        } else {
            '?'
        }
    }
}

pub fn verdict(run: &Run, device: Device) -> Verdict {
    let mut v = Verdict {
        device,
        batch: None,
        speed: ('?', "測れない".into()),
        cached_first: ('?', "測れない".into()),
        top1: ('?', "測れない".into()),
        failed: None,
    };
    let Some(d) = run.get(device) else {
        v.failed = Some("測っていない".into());
        return v;
    };
    let Some(best) = d.best() else {
        v.failed = d.failure();
        return v;
    };
    v.batch = best.batch;
    let p50 = timing(best).map(|t| t.p50 as f64);
    let vulkan = run
        .get(Device::LlamaVulkan)
        .and_then(DeviceRun::best)
        .and_then(timing)
        .map(|t| t.p50 as f64);
    v.speed = match (p50, vulkan) {
        (Some(a), Some(b)) if b > 0.0 => {
            let ratio = a / b;
            let mark = if ratio <= SPEED_RATIO_MAX {
                '○'
            } else {
                '×'
            };
            (mark, format!("Vulkan の {ratio:.2} 倍"))
        }
        _ => ('?', "Vulkan と比べられない".into()),
    };
    v.cached_first = match d
        .first_for(best.batch)
        .filter(|f| f.ok)
        .and_then(|f| f.first_ms)
    {
        Some(f) => (
            if f <= CACHED_FIRST_MAX_MS {
                '○'
            } else {
                '×'
            },
            secs(f),
        ),
        None => ('?', "キャッシュありを測れない".into()),
    };
    v.top1 = match run.agreement(best).and_then(|a| a.top1_rate()) {
        Some(r) => (if r >= TOP1_MIN { '○' } else { '×' }, percent(Some(r))),
        None => ('?', "基準が無い".into()),
    };
    v
}

fn verdict_line(v: &Verdict) -> String {
    let batch = v.batch.map_or(String::new(), |b| format!("(B{b})"));
    let head = format!("判定 {}{batch}:", v.device);
    let tail = match v.overall() {
        '○' => "○ 基準を満たす（採否はユーザーが決める）",
        '×' => "× 採らない",
        _ => "? 判定できない",
    };
    if let Some(f) = &v.failed {
        return format!("{head} 載らなかった（{}） → {tail}", truncate(f, 24));
    }
    format!(
        "{head} 速さ{}({}) 初回{}({}) 一致{}({}) → {tail}",
        v.speed.0, v.speed.1, v.cached_first.0, v.cached_first.1, v.top1.0, v.top1.1
    )
}

/// NPU の判定をする装置
pub fn verdict_devices() -> [Device; 2] {
    [
        Device::Ov(OvTarget::Npu, Precision::Fp16),
        Device::Ov(OvTarget::Npu, Precision::Int8),
    ]
}

fn header_line(h: &Header) -> String {
    let ov = h.ov.as_ref().map_or("OpenVINO なし".into(), |o| {
        format!("OpenVINO {}", short_version(&o.version))
    });
    let npu =
        h.ov.as_ref()
            .and_then(|o| o.devices.iter().find(|(n, _, _)| n.starts_with("NPU")))
            .map_or("NPU 見えない".into(), |(_, _, drv)| {
                format!("NPU ドライバ {}", drv.as_deref().unwrap_or("?"))
            });
    format!("{} / {} / {ov} / {npu}", h.tool, h.os)
}

/// OpenVINO の版の先頭（`2025.3.0-19807-…` の `2025.3.0`）
fn short_version(v: &str) -> &str {
    v.split('-').next().unwrap_or(v)
}

/// 画面の要約（10 行前後）
pub fn summary(run: &Run) -> Vec<String> {
    let mut lines = vec![header_line(&run.header)];
    for device in summary_devices() {
        if let Some(d) = run.get(device) {
            lines.push(device_line(run, d));
        }
    }
    lines.extend(sandbox_line(run));
    for device in verdict_devices() {
        lines.push(verdict_line(&verdict(run, device)));
    }
    lines
}

/// `report.txt` の本文（目安 80 行）
pub fn report(run: &Run) -> String {
    let h = &run.header;
    let mut s = String::new();
    let _ = writeln!(s, "== ikari-npu-probe の報告（ADR 0075 §4.4）==");
    let _ = writeln!(s, "ツール: {}", h.tool);
    let _ = writeln!(s, "OS: {}", h.os);
    let _ = writeln!(s, "CPU: {}", h.cpu);
    match (&h.ov, &h.ov_error) {
        (Some(o), _) => {
            let _ = writeln!(s, "OpenVINO: {}", o.version);
            for (name, full, drv) in &o.devices {
                let drv = drv
                    .as_ref()
                    .map_or(String::new(), |d| format!("（ドライバ {d}）"));
                let _ = writeln!(s, "  {name}: {full}{drv}");
            }
        }
        (None, Some(e)) => {
            let _ = writeln!(
                s,
                "OpenVINO: 読めない（{}）",
                truncate(e, REPORT_REASON_CHARS)
            );
        }
        (None, None) => {
            let _ = writeln!(s, "OpenVINO: 読めない");
        }
    }
    let _ = writeln!(
        s,
        "llama.cpp の装置: {}",
        h.llama_devices.as_deref().unwrap_or("-")
    );
    for (name, sha) in &h.models {
        let _ = writeln!(s, "モデル: {name} sha256 {sha}");
    }
    let _ = writeln!(
        s,
        "入力: {} の {} 件（候補 {} 個）",
        crate::input::SET_NAME,
        h.inputs,
        h.candidates
    );
    match h.token_match {
        Some((same, all)) => {
            let _ = writeln!(
                s,
                "トークン列: llama.cpp と入力の prompt_ids の一致 {same}/{all} 件"
            );
        }
        None => {
            let _ = writeln!(
                s,
                "トークン列: llama.cpp の語彙で作った（入力に prompt_ids なし）"
            );
        }
    }

    let _ = writeln!(
        s,
        "\n== 装置ごと（暖機のあとの 1 件の時間。ms）==\n装置 B 結果 読込 初回 キャッシュ初回 p50 p95 p99 回数 メモリ 長すぎ 動いた装置"
    );
    for d in &run.devices {
        if let Some(skip) = &d.skipped {
            let _ = writeln!(
                s,
                "{} - 測らない: {}",
                d.device,
                truncate(skip, REPORT_REASON_CHARS)
            );
            continue;
        }
        for r in &d.full {
            let t = timing(r);
            let cached = d
                .first_for(r.batch)
                .and_then(|f| f.first_ms)
                .map_or("-".into(), secs);
            let _ = writeln!(
                s,
                "{} {} {} {} {} {cached} {} {} {} {} {} {} {}",
                d.device,
                r.batch.map_or("-".into(), |b| b.to_string()),
                if r.ok { "○" } else { "×" },
                r.load_ms.map_or("-".into(), secs),
                r.first_ms.map_or("-".into(), secs),
                t.map_or("-".into(), |t| ms(t.p50)),
                t.map_or("-".into(), |t| ms(t.p95)),
                t.map_or("-".into(), |t| ms(t.p99)),
                r.reps,
                mb(r.peak_mem),
                r.too_long,
                r.ran_on.as_deref().map_or("-".into(), |x| truncate(x, 60)),
            );
        }
    }

    let _ = writeln!(
        s,
        "\n== 一致度（基準 llama-cpu / OpenVINO は同じ精度の ov-cpu も）==\n装置 B 件数 1位一致 τ中央値 NLL差最大 NLL差中央値"
    );
    for d in &run.devices {
        for r in d
            .full
            .iter()
            .filter(|r| r.ok && d.device != Device::LlamaCpu)
        {
            let mut rows = vec![("llama-cpu", run.agreement(r))];
            rows.push(("ov-cpu", run.agreement_ov(d.device, r)));
            for (base, a) in rows {
                let Some(a) = a else { continue };
                let f = |x: Option<f64>| x.map_or("-".into(), |v| format!("{v:.3}"));
                let _ = writeln!(
                    s,
                    "{} {} 対{base} {} {} {} {} {}",
                    d.device,
                    r.batch.map_or("-".into(), |b| b.to_string()),
                    a.items,
                    percent(a.top1_rate()),
                    f(a.tau_median),
                    f(a.diff_max),
                    f(a.diff_median)
                );
            }
        }
    }

    let _ = writeln!(
        s,
        "\n== 長さごとのコンパイル（ms。初回 / キャッシュあり）=="
    );
    for d in run.devices.iter().filter(|d| d.device.is_ov()) {
        for r in d.full.iter().filter(|r| !r.compile_ms.is_empty()) {
            let cached = d.first_for(r.batch).map(|f| &f.compile_ms);
            let parts: Vec<String> = r
                .compile_ms
                .iter()
                .map(|(t, c)| {
                    let w = cached
                        .and_then(|cs| cs.iter().find(|(t2, _)| t2 == t))
                        .map_or("-".into(), |(_, x)| format!("{x:.0}"));
                    format!("T{t} {c:.0}/{w}")
                })
                .collect();
            let _ = writeln!(
                s,
                "{} B{}: {}",
                d.device,
                r.batch.unwrap_or(0),
                parts.join(" ")
            );
        }
    }

    let _ = writeln!(s, "\n== 檻（ADR 0075 §6。1 件だけ流す）==");
    for n in &run.sandbox_notes {
        let _ = writeln!(s, "{n}");
    }
    for r in &run.sandbox {
        let _ = writeln!(
            s,
            "{} {} {} {}",
            r.confine,
            r.device,
            if r.ok { "○" } else { "×" },
            r.reason
                .as_deref()
                .map_or(String::new(), |x| truncate(x, REPORT_REASON_CHARS))
        );
    }

    let _ = writeln!(s, "\n== 失敗の理由 ==");
    for d in &run.devices {
        for r in d.full.iter().chain(&d.first).filter(|r| !r.ok) {
            let _ = writeln!(
                s,
                "{} B{} {}: {}",
                d.device,
                r.batch.unwrap_or(0),
                r.mode,
                truncate(r.error.as_deref().unwrap_or("?"), REPORT_REASON_CHARS)
            );
        }
        for r in d.full.iter().filter_map(|r| r.note.as_ref()) {
            let _ = writeln!(s, "{} 補足: {}", d.device, truncate(r, REPORT_REASON_CHARS));
        }
    }

    let _ = writeln!(
        s,
        "\n== 判定（仮置きの基準: p50 が Vulkan の {SPEED_RATIO_MAX} 倍以下、キャッシュありの初回 {:.0} 秒以下、1 位の一致 {:.0}% 以上）==",
        CACHED_FIRST_MAX_MS / 1000.0,
        TOP1_MIN * 100.0
    );
    for device in verdict_devices() {
        let _ = writeln!(s, "{}", verdict_line(&verdict(run, device)));
    }
    let _ = writeln!(s, "\n所要時間: {:.1} 分", run.elapsed.as_secs_f64() / 60.0);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(
        device: Device,
        batch: Option<usize>,
        us: u64,
        nll: Vec<Vec<Option<f32>>>,
    ) -> WorkerResult {
        WorkerResult {
            device: device.to_string(),
            batch,
            mode: "full".into(),
            ok: true,
            load_ms: Some(100.0),
            first_ms: Some(500.0),
            times_us: vec![vec![us; 20]],
            nll,
            reps: 20,
            peak_mem: Some(200 * 1024 * 1024),
            ..WorkerResult::default()
        }
    }

    fn nll(top: usize) -> Vec<Vec<Option<f32>>> {
        (0..20)
            .map(|_| {
                (0..3)
                    .map(|i| Some(if i == top { 1.0 } else { 5.0 }))
                    .collect()
            })
            .collect()
    }

    fn sample_run() -> Run {
        let npu16 = Device::Ov(OvTarget::Npu, Precision::Fp16);
        let npu8 = Device::Ov(OvTarget::Npu, Precision::Int8);
        let mut first = result(npu16, Some(8), 0, vec![]);
        first.mode = "first".into();
        first.first_ms = Some(800.0);
        Run {
            header: Header {
                tool: "ikari-npu-probe 0.1.0 (abc)".into(),
                os: "Windows 11".into(),
                ..Header::default()
            },
            devices: vec![
                DeviceRun {
                    device: Device::LlamaCpu,
                    full: vec![result(Device::LlamaCpu, None, 100_000, nll(0))],
                    first: vec![],
                    skipped: None,
                },
                DeviceRun {
                    device: Device::LlamaVulkan,
                    full: vec![result(Device::LlamaVulkan, None, 40_000, nll(0))],
                    first: vec![],
                    skipped: None,
                },
                DeviceRun {
                    device: npu16,
                    full: vec![
                        result(npu16, Some(1), 50_000, nll(0)),
                        result(npu16, Some(8), 20_000, nll(0)),
                    ],
                    first: vec![first],
                    skipped: None,
                },
                DeviceRun {
                    device: npu8,
                    full: vec![WorkerResult::failed(
                        npu8,
                        Some(1),
                        crate::worker::Mode::Full,
                        "compile failed".into(),
                    )],
                    first: vec![],
                    skipped: None,
                },
                DeviceRun {
                    device: Device::LlamaOvNpu,
                    full: vec![],
                    first: vec![],
                    skipped: Some("作っていない（後回し）".into()),
                },
            ],
            sandbox: vec![
                SandboxRow {
                    confine: "低い整合性",
                    device: Device::LlamaCpu,
                    ok: true,
                    reason: None,
                },
                SandboxRow {
                    confine: "AppContainer",
                    device: npu16,
                    ok: false,
                    reason: Some("x".into()),
                },
            ],
            sandbox_notes: vec![],
            elapsed: Duration::from_secs(600),
        }
    }

    #[test]
    fn best_picks_lowest_p50() {
        let run = sample_run();
        let d = run.get(Device::Ov(OvTarget::Npu, Precision::Fp16)).unwrap();
        assert_eq!(d.best().unwrap().batch, Some(8));
    }

    #[test]
    fn verdict_passes_and_fails() {
        let run = sample_run();
        let v = verdict(&run, Device::Ov(OvTarget::Npu, Precision::Fp16));
        assert_eq!(v.speed.0, '○', "{v:?}");
        assert_eq!(v.cached_first.0, '○');
        assert_eq!(v.top1.0, '○');
        assert_eq!(v.overall(), '○');
        let v = verdict(&run, Device::Ov(OvTarget::Npu, Precision::Int8));
        assert_eq!(v.overall(), '×');
        assert_eq!(v.failed.as_deref(), Some("compile failed"));
    }

    #[test]
    fn slow_npu_is_rejected() {
        let mut run = sample_run();
        let d = run
            .devices
            .iter_mut()
            .find(|d| d.device == Device::Ov(OvTarget::Npu, Precision::Fp16))
            .unwrap();
        for r in &mut d.full {
            r.times_us = vec![vec![39_000; 20]];
        }
        let v = verdict(&run, Device::Ov(OvTarget::Npu, Precision::Fp16));
        assert_eq!(v.speed.0, '×');
        assert_eq!(v.overall(), '×');
    }

    #[test]
    fn summary_is_short() {
        let run = sample_run();
        let lines = summary(&run);
        assert!(lines.len() <= 12, "{lines:#?}");
        assert!(lines[0].starts_with("ikari-npu-probe"));
        let all = lines.join("\n");
        assert!(all.contains("llama-cpu    ○ p50 100.0ms"), "{all}");
        assert!(all.contains("ov-npu-fp16  ○ B8 p50 20.0ms"), "{all}");
        assert!(all.contains("1位一致 100.0%"), "{all}");
        assert!(all.contains("ov-npu-int8  × compile failed"), "{all}");
        assert!(all.contains("llama-ov-npu - 作っていない"), "{all}");
        assert!(all.contains("檻   低IL: cpu○ | AC: npu×"), "{all}");
        assert!(
            all.contains("判定 ov-npu-fp16(B8): 速さ○(Vulkan の 0.50 倍)"),
            "{all}"
        );
    }

    #[test]
    fn report_has_sections() {
        let text = report(&sample_run());
        for section in ["装置ごと", "一致度", "檻", "失敗の理由", "判定", "所要時間"]
        {
            assert!(text.contains(section), "{section}");
        }
        assert!(text.lines().count() < 120);
    }

    #[test]
    fn truncate_adds_ellipsis() {
        assert_eq!(truncate("abcdef", 3), "abc…");
        assert_eq!(truncate("a\nb", 5), "a b");
    }
}
