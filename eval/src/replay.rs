//! 再生の評価（`ikari-eval replay`。ADR 0065 §3.4）。
//!
//! 評価セットの各行で、読みを 1 文字ずつ Prefetch し（間隔は打鍵の間隔の分位数の表から引く）、最後の Prefetch の
//! あと Space までの間（同じく表から引く）を空けて Convert する。**製品と同じ `Engine`**（先行計算、待ち行列、
//! Space からの待ち）に、本物の時間で（実時間の sleep で）要求を送る。LM の人工の遅延は `--ranker-config` の
//! `zenz.debug_infer_scale` と `zenz.debug_infer_extra_ms`（`ikari-lm` の feature `dev-knobs`。製品には入らない）。
//!
//! 再現性: 乱数の種は固定で、結果に書く。行ごとの時刻の計画は「種 + 行の id」だけで決まる（ほかの行や LM の
//! 速さに依らない）ので、設定を替えても同じ行は同じ時刻で打たれる。
//!
//! 実機のログや利用者の学習は使わない。学習の保存先は一時ディレクトリで、確定（Commit）は送らない。

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ikari_core::LmBudget;
use ikari_engine::{Engine, EngineParts, MAX_PAGE_SIZE, Settings, stack_options};
use ikari_proto::SCHEMA_VERSION;
use ikari_proto::v1 as pb;
use ikari_proto::v1::{Envelope, LmState, LmVerdict, envelope};
use ikari_stack::{StackPaths, build_stack};
use prost::Message;

use crate::EvalCase;

/// 同梱の分位数の表（ADR 0065 §1.3 の数字）
pub const DEFAULT_QUANTILES: &str = include_str!("../data/replay-quantiles.tsv");
/// 乱数の種の既定（任意の固定値。結果に書く）
pub const DEFAULT_SEED: u64 = 486;
/// 行と行の間の静まり待ちの既定（前の行の古い推論を持ち越さないため。仮置き。結果に書く）
pub const DEFAULT_SETTLE_MS: u64 = 1000;
/// LM が `Ready` になるのを待つ上限（モデルの読み込みと暖機）
const LM_READY_LIMIT: Duration = Duration::from_secs(180);
const LM_READY_POLL: Duration = Duration::from_millis(50);
/// 表の列の見出し（読み飛ばす行）
const TABLE_HEADER_FIRST_CELL: &str = "kind";
const KIND_INTERVAL: &str = "interval";
const KIND_GAP: &str = "gap";
const SESSION_ID: u64 = 1;
const SHELL_VERSION: &str = "ikari-eval-replay";
/// SplitMix64 の定数
const SPLITMIX_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

// ---- 分位数の表 ----

/// 分位 → ミリ秒の曲線。点の間は直線で補い、端より外は端の値のまま
#[derive(Clone, Debug, PartialEq)]
pub struct Curve(Vec<(f64, f64)>);

impl Curve {
    /// `u`（0..=1）の分位の値
    pub fn at(&self, u: f64) -> Duration {
        let pts = &self.0;
        let ms = match pts.iter().position(|&(p, _)| p >= u) {
            None => pts[pts.len() - 1].1,
            Some(0) => pts[0].1,
            Some(i) => {
                let (p0, v0) = pts[i - 1];
                let (p1, v1) = pts[i];
                v0 + (v1 - v0) * (u - p0) / (p1 - p0)
            }
        };
        Duration::from_secs_f64(ms / 1000.0)
    }
}

/// 打鍵の間隔と、Space までの間の分位数
#[derive(Clone, Debug, PartialEq)]
pub struct Quantiles {
    pub interval: Curve,
    pub gap: Curve,
}

/// 分位数の表（TSV。列は kind, p, ms）を読む
pub fn parse_quantiles(text: &str) -> Result<Quantiles, String> {
    let mut interval = Vec::new();
    let mut gap = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let cells: Vec<&str> = line.split('\t').collect();
        if cells.first() == Some(&TABLE_HEADER_FIRST_CELL) {
            continue;
        }
        let at = |msg: &str| format!("line {}: {msg}", i + 1);
        let [kind, p, ms] = cells[..] else {
            return Err(at("expected 3 columns (kind, p, ms)"));
        };
        let p: f64 = p.parse().map_err(|_| at("p is not a number"))?;
        let ms: f64 = ms.parse().map_err(|_| at("ms is not a number"))?;
        if !(p > 0.0 && p <= 1.0) || !ms.is_finite() || ms < 0.0 {
            return Err(at("p must be in (0, 1] and ms must be non-negative"));
        }
        let target = match kind {
            KIND_INTERVAL => &mut interval,
            KIND_GAP => &mut gap,
            other => return Err(at(&format!("unknown kind `{other}`"))),
        };
        if let Some(&(prev_p, prev_ms)) = target.last()
            && (p <= prev_p || ms < prev_ms)
        {
            return Err(at(
                "rows of a kind must be ascending in p and non-decreasing in ms",
            ));
        }
        target.push((p, ms));
    }
    if interval.is_empty() || gap.is_empty() {
        return Err(format!(
            "the table needs `{KIND_INTERVAL}` and `{KIND_GAP}` rows"
        ));
    }
    Ok(Quantiles {
        interval: Curve(interval),
        gap: Curve(gap),
    })
}

// ---- 乱数と計画 ----

/// SplitMix64（外部の乱数の crate を入れない。再現性のため、手順をここに固定する）
#[derive(Clone, Debug)]
pub struct SplitMix64(u64);

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        SplitMix64(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(SPLITMIX_GAMMA);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// 0 以上 1 未満
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(FNV_OFFSET, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(FNV_PRIME)
    })
}

/// 1 行の時刻の計画
#[derive(Clone, Debug, PartialEq)]
pub struct RowPlan {
    /// i 番目の Prefetch（i = 0..chars）のあとの待ち。最後は Space までの間
    pub waits: Vec<Duration>,
}

/// 行の計画。`chars` 文字なら Prefetch は `chars` 回、待ちも `chars` 個（最後が Space までの間）。
/// 種と id だけで決まる
pub fn plan_row(q: &Quantiles, seed: u64, id: &str, chars: usize) -> RowPlan {
    let mut rng = SplitMix64::new(seed ^ fnv1a(id));
    let mut waits = Vec::with_capacity(chars);
    for _ in 1..chars {
        waits.push(q.interval.at(rng.next_f64()));
    }
    waits.push(q.gap.at(rng.next_f64()));
    RowPlan { waits }
}

/// 行を決まった方法で `limit` 件に絞る（種と id の混ぜ合わせで選び、元の並びに戻す）。
/// `None` か件数以上なら全部
pub fn select_rows(cases: &[EvalCase], seed: u64, limit: Option<usize>) -> Vec<&EvalCase> {
    let Some(limit) = limit.filter(|&l| l < cases.len()) else {
        return cases.iter().collect();
    };
    let mut keyed: Vec<(u64, usize)> = cases
        .iter()
        .enumerate()
        .map(|(i, c)| (SplitMix64::new(seed ^ fnv1a(&c.id)).next_u64(), i))
        .collect();
    keyed.sort_unstable();
    let mut picked: Vec<usize> = keyed.into_iter().take(limit).map(|(_, i)| i).collect();
    picked.sort_unstable();
    picked.into_iter().map(|i| &cases[i]).collect()
}

// ---- 結果 ----

/// 1 行の結果
#[derive(Clone, Debug, PartialEq)]
pub struct RowOutcome {
    pub id: String,
    pub verdict: LmVerdict,
    pub top1: bool,
    /// Space（Convert）から 1 ページ目が返るまで
    pub first_page: Duration,
    /// `LmReport.infer_micros`（先行計算の推論の時間の和）
    pub infer: Duration,
}

/// nearest-rank 法の分位（昇順の `ceil(p * n) - 1` 番目）。空なら 0
pub fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let rank = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1]
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReplaySummary {
    pub rows: usize,
    /// `LmVerdict` の名前 → 件数（0 件の名前は入れない）
    pub verdicts: BTreeMap<String, usize>,
    pub applied: usize,
    pub top1: usize,
    pub first_page_p50: Duration,
    pub first_page_p99: Duration,
    /// 推論の時間（0 でない行だけ）
    pub infer_n: usize,
    pub infer_p50: Duration,
    pub infer_p90: Duration,
}

impl ReplaySummary {
    pub fn applied_rate(&self) -> f64 {
        ratio(self.applied, self.rows)
    }

    pub fn top1_rate(&self) -> f64 {
        ratio(self.top1, self.rows)
    }
}

fn ratio(n: usize, d: usize) -> f64 {
    if d == 0 { 0.0 } else { n as f64 / d as f64 }
}

pub fn summarize(outcomes: &[RowOutcome]) -> ReplaySummary {
    let mut s = ReplaySummary {
        rows: outcomes.len(),
        ..ReplaySummary::default()
    };
    let mut pages: Vec<Duration> = Vec::with_capacity(outcomes.len());
    let mut infers: Vec<Duration> = Vec::new();
    for o in outcomes {
        *s.verdicts
            .entry(o.verdict.as_str_name().to_owned())
            .or_default() += 1;
        s.applied += usize::from(o.verdict == LmVerdict::LmApplied);
        s.top1 += usize::from(o.top1);
        pages.push(o.first_page);
        if !o.infer.is_zero() {
            infers.push(o.infer);
        }
    }
    pages.sort_unstable();
    infers.sort_unstable();
    s.first_page_p50 = percentile(&pages, 0.50);
    s.first_page_p99 = percentile(&pages, 0.99);
    s.infer_n = infers.len();
    s.infer_p50 = percentile(&infers, 0.50);
    s.infer_p90 = percentile(&infers, 0.90);
    s
}

/// レポートの前書き（再現に要る値。パスやユーザーの情報は入れない）
#[derive(Clone, Debug, Default)]
pub struct ReplayMeta {
    pub set_name: String,
    pub rows_in_set: usize,
    pub seed: u64,
    pub settle: Duration,
    pub limit: Option<usize>,
    pub lm_deadline: Duration,
    pub convert_wait: Duration,
    /// `zenz.debug_infer_*` の指定（あれば）
    pub debug_infer: BTreeMap<String, String>,
    pub model_name: String,
    pub load_before: Option<String>,
    pub load_after: Option<String>,
    pub elapsed: Duration,
}

fn ms(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64() * 1000.0)
}

pub fn render_markdown(meta: &ReplayMeta, s: &ReplaySummary) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# 再生の評価（ADR 0065 §3.4）\n");
    let _ = writeln!(
        out,
        "- セット: {}（{} 行のうち {} 行を再生）",
        meta.set_name, meta.rows_in_set, s.rows
    );
    let limit = meta.limit.map_or("なし".to_owned(), |l| l.to_string());
    let _ = writeln!(
        out,
        "- 乱数の種: {}、行数の上限: {limit}、行の間の静まり: {} ms、所要: {:.0} 秒",
        meta.seed,
        meta.settle.as_millis(),
        meta.elapsed.as_secs_f64()
    );
    let _ = writeln!(
        out,
        "- LM: {}、LM の締め切り {} ms、Space からの待ち {} ms",
        meta.model_name,
        meta.lm_deadline.as_millis(),
        meta.convert_wait.as_millis()
    );
    let debug = if meta.debug_infer.is_empty() {
        "なし".to_owned()
    } else {
        meta.debug_infer
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let _ = writeln!(out, "- 人工の遅延: {debug}");
    let _ = writeln!(
        out,
        "- 負荷平均（開始前 / 終了後）: {} / {}\n",
        meta.load_before.as_deref().unwrap_or("?"),
        meta.load_after.as_deref().unwrap_or("?")
    );
    let _ = writeln!(out, "| 項目 | 値 |\n| --- | ---: |");
    let _ = writeln!(
        out,
        "| LM が付いた率（LM_APPLIED） | {:.1}%（{}/{}） |",
        s.applied_rate() * 100.0,
        s.applied,
        s.rows
    );
    let _ = writeln!(
        out,
        "| 1 位 | {:.1}%（{}/{}） |",
        s.top1_rate() * 100.0,
        s.top1,
        s.rows
    );
    let _ = writeln!(
        out,
        "| Space から 1 ページ目まで p50 / p99 (ms) | {} / {} |",
        ms(s.first_page_p50),
        ms(s.first_page_p99)
    );
    let _ = writeln!(
        out,
        "| 先行計算の推論の時間 p50 / p90 (ms。n={}) | {} / {} |\n",
        s.infer_n,
        ms(s.infer_p50),
        ms(s.infer_p90)
    );
    let _ = writeln!(
        out,
        "| LmReport.verdict | 件数 | 率 |\n| --- | ---: | ---: |"
    );
    for (name, n) in &s.verdicts {
        let _ = writeln!(out, "| {name} | {n} | {:.1}% |", ratio(*n, s.rows) * 100.0);
    }
    out
}

/// ケース別の TSV（id, verdict, top1, first_page_us, infer_us）
pub fn render_cases_tsv(outcomes: &[RowOutcome]) -> String {
    let mut out = String::from("id\tverdict\ttop1\tfirst_page_us\tinfer_us\n");
    for o in outcomes {
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}",
            o.id,
            o.verdict.as_str_name(),
            u8::from(o.top1),
            o.first_page.as_micros(),
            o.infer.as_micros()
        );
    }
    out
}

/// `uptime` の負荷平均の部分（取れなければ None。表示だけに使う）
pub fn load_average() -> Option<String> {
    let out = std::process::Command::new("uptime").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (_, tail) = text.split_once("load average")?;
    Some(tail.trim_start_matches(['s', ':', ' ']).trim().to_owned())
}

// ---- エンジンの組み立てと再生 ----

/// 再生に使うエンジンの設定
#[derive(Clone, Debug)]
pub struct EngineSetup {
    pub dict_dir: PathBuf,
    /// None なら LM なし（テスト用）
    pub lm_model: Option<PathBuf>,
    /// 別プロセスの子の実行ファイル。None なら同じプロセス（feature `lm` が要る）
    pub lm_host: Option<PathBuf>,
    /// `--ranker-config` の key=value（`zenz.debug_infer_*` など）
    pub ranker_config: BTreeMap<String, String>,
    /// LM の締め切り。None なら設定の既定（`engine.lm_deadline_ms`）
    pub lm_deadline: Option<Duration>,
}

/// 一時ディレクトリ（学習の保存先。終了時に消す）
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 再生用のエンジン。`Engine` は製品と同じ（先行計算、待ち行列、Space からの待ち）
pub struct ReplayEngine {
    engine: Engine,
    pub lm_deadline: Duration,
    pub convert_wait: Duration,
    next_request: u64,
    _temp: TempDir,
}

impl ReplayEngine {
    /// エンジンを組み立て、LM があれば `Ready` まで待つ。値の既定は製品の設定の既定（`Settings::default()`）
    pub fn open(setup: &EngineSetup) -> Result<ReplayEngine, String> {
        let settings = Settings::default();
        let lm_deadline = setup
            .lm_deadline
            .unwrap_or_else(|| settings.lm_deadline_or_default());
        let convert_wait = settings.convert_wait_or_default();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let temp =
            std::env::temp_dir().join(format!("ikari-eval-replay-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&temp).map_err(|e| format!("cannot create the temp dir: {e}"))?;
        let temp = TempDir(temp);
        let budget = LmBudget {
            max: lm_deadline,
            ..LmBudget::default()
        };
        let paths = StackPaths {
            dict_dir: setup.dict_dir.clone(),
            user_dir: Some(temp.0.clone()),
            vectors: None,
            lm_model: setup.lm_model.clone(),
            lm_host: setup.lm_host.clone(),
            no_packs: false,
        };
        let mut opts = stack_options(0, budget);
        opts.ranker_config = setup.ranker_config.clone();
        let stack = build_stack(&paths, &opts).map_err(|e| e.detail())?;
        let lm_requested = setup.lm_model.is_some();
        let engine = Engine::from_parts(EngineParts {
            stack,
            rebuild: None,
            data_dir: None,
            lm_requested,
            lm_budget: budget,
            convert_wait,
            lock: None,
        });
        let mut me = ReplayEngine {
            engine,
            lm_deadline,
            convert_wait,
            next_request: 1,
            _temp: temp,
        };
        me.call(envelope::Body::BeginSession(pb::BeginSession {
            session_id: SESSION_ID,
            client_bundle_id: SHELL_VERSION.into(),
            secure: false,
        }))?;
        if lm_requested {
            me.wait_lm_ready()?;
        }
        Ok(me)
    }

    fn call(&mut self, body: envelope::Body) -> Result<envelope::Body, String> {
        let id = self.next_request;
        self.next_request += 1;
        let req = Envelope {
            schema_version: SCHEMA_VERSION,
            request_id: id,
            body: Some(body),
        }
        .encode_to_vec();
        let reply = Envelope::decode(self.engine.handle(&req).as_slice())
            .map_err(|e| format!("the reply does not decode: {e}"))?;
        match reply.body {
            Some(envelope::Body::Error(e)) => Err(format!("engine error: {}", e.message)),
            Some(body) => Ok(body),
            None => Err("empty reply".into()),
        }
    }

    fn wait_lm_ready(&mut self) -> Result<(), String> {
        let started = Instant::now();
        loop {
            let body = self.call(envelope::Body::Hello(pb::Hello {
                shell_version: SHELL_VERSION.into(),
            }))?;
            if let envelope::Body::HelloReply(r) = body {
                match LmState::try_from(r.lm) {
                    Ok(LmState::LmReady) => return Ok(()),
                    Ok(LmState::LmFailed) => return Err("the LM failed to load".into()),
                    _ => {}
                }
            }
            if started.elapsed() > LM_READY_LIMIT {
                return Err("the LM did not become ready in time".into());
            }
            std::thread::sleep(LM_READY_POLL);
        }
    }

    /// 1 行を再生する（実時間）。`rev` は行ごとに替える文脈の番号
    pub fn replay_row(
        &mut self,
        case: &EvalCase,
        plan: &RowPlan,
        rev: u64,
        page_size: u32,
    ) -> Result<RowOutcome, String> {
        let context = pb::SurroundingText {
            left: case.context.clone(),
            right: case.right.clone(),
            available: true,
        };
        let chars: Vec<char> = case.reading.chars().collect();
        if chars.is_empty() || plan.waits.len() != chars.len() {
            return Err(format!("{}: the plan does not match the reading", case.id));
        }
        let mut due = Instant::now();
        for (i, wait) in plan.waits.iter().enumerate() {
            let prefix: String = chars[..=i].iter().collect();
            self.call(envelope::Body::Prefetch(pb::Prefetch {
                session_id: SESSION_ID,
                reading: prefix,
                context: Some(context.clone()),
                context_rev: rev,
            }))?;
            due += *wait;
            if let Some(rest) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(rest);
            }
        }
        let started = Instant::now();
        let reply = self.call(envelope::Body::Convert(pb::Convert {
            session_id: SESSION_ID,
            reading: case.reading.clone(),
            context: Some(context),
            context_rev: rev,
            page_size,
        }))?;
        let first_page = started.elapsed();
        let envelope::Body::ConvertReply(reply) = reply else {
            return Err(format!(
                "{}: Convert did not return a ConvertReply",
                case.id
            ));
        };
        let report = reply.lm_report.unwrap_or_default();
        let outcome = RowOutcome {
            id: case.id.clone(),
            verdict: LmVerdict::try_from(report.verdict).unwrap_or(LmVerdict::Unspecified),
            top1: reply
                .candidates
                .first()
                .is_some_and(|c| case.matches(&c.surface)),
            first_page,
            infer: Duration::from_micros(u64::from(report.infer_micros)),
        };
        // 変換は使い終わった（殻が候補窓を閉じたときと同じ）。確定（学習）は送らない
        self.call(envelope::Body::Cancel(pb::Cancel {
            conversion_id: reply.conversion_id,
        }))?;
        Ok(outcome)
    }
}

/// 再生の指定
#[derive(Clone, Debug)]
pub struct ReplayOptions {
    pub seed: u64,
    pub settle: Duration,
    pub limit: Option<usize>,
}

/// セットを再生する。経過は `progress` に渡す（件数ごと）
pub fn run_replay(
    engine: &mut ReplayEngine,
    cases: &[EvalCase],
    q: &Quantiles,
    opts: &ReplayOptions,
    mut progress: impl FnMut(usize, usize),
) -> Result<Vec<RowOutcome>, String> {
    let rows = select_rows(cases, opts.seed, opts.limit);
    let mut outcomes = Vec::with_capacity(rows.len());
    for (i, case) in rows.iter().enumerate() {
        let chars = case.reading.chars().count();
        let plan = plan_row(q, opts.seed, &case.id, chars);
        let rev = i as u64 + 1;
        outcomes.push(engine.replay_row(case, &plan, rev, MAX_PAGE_SIZE)?);
        progress(i + 1, rows.len());
        std::thread::sleep(opts.settle);
    }
    Ok(outcomes)
}

/// 設定ファイル（`--ranker-config`）から、人工の遅延のキーだけを取り出す（レポートの前書き用）
pub fn debug_infer_keys(kv: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    kv.iter()
        .filter(|(k, _)| k.starts_with("zenz.debug_infer_"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// 辞書のフォルダ名などパスを書かずに、ファイル名だけを返す
pub fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q() -> Quantiles {
        parse_quantiles(DEFAULT_QUANTILES).expect("the bundled table parses")
    }

    fn case(id: &str, reading: &str) -> EvalCase {
        EvalCase {
            id: id.into(),
            kind: String::new(),
            tags: Vec::new(),
            profile: None,
            context: String::new(),
            right: String::new(),
            field: None,
            history: String::new(),
            reading: reading.into(),
            expected: String::new(),
            note: String::new(),
            split: None,
        }
    }

    #[test]
    fn the_bundled_table_matches_adr_0065_1_3() {
        let q = q();
        // 表にある分位は、そのままの値（ADR 0065 §1.3）
        assert_eq!(q.interval.at(0.50), Duration::from_millis(280));
        assert_eq!(q.interval.at(0.99), Duration::from_millis(1526));
        assert_eq!(q.gap.at(0.50), Duration::from_millis(136));
        assert_eq!(q.gap.at(0.90), Duration::from_millis(310));
    }

    #[test]
    fn curves_interpolate_linearly_and_clamp_at_the_ends() {
        let q = q();
        // gap の p25 = 117、p50 = 136 の真ん中（p37.5）は 126.5
        let mid = q.gap.at(0.375).as_secs_f64() * 1000.0;
        assert!((mid - 126.5).abs() < 1e-6, "{mid}");
        // 端より外は端の値
        assert_eq!(q.interval.at(0.0), Duration::from_millis(116));
        assert_eq!(q.interval.at(0.999), Duration::from_millis(1526));
        assert_eq!(q.gap.at(0.01), Duration::from_millis(77));
    }

    #[test]
    fn bad_tables_are_rejected() {
        for text in [
            "interval\t0.5\t100\n",                                  // gap がない
            "interval\t0.5\tx\ngap\t0.5\t1\n",                       // 数字でない
            "interval\t0.5\t100\ninterval\t0.4\t200\ngap\t0.5\t1\n", // p が戻る
            "interval\t0.5\t100\ninterval\t0.6\t50\ngap\t0.5\t1\n",  // ms が減る
            "interval\t1.5\t100\ngap\t0.5\t1\n",                     // p が範囲外
            "other\t0.5\t1\n",                                       // 未知の kind
        ] {
            assert!(parse_quantiles(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn plans_depend_only_on_seed_and_id() {
        let q = q();
        let a = plan_row(&q, 1, "row-a", 5);
        assert_eq!(a, plan_row(&q, 1, "row-a", 5));
        assert_ne!(a, plan_row(&q, 2, "row-a", 5));
        assert_ne!(a, plan_row(&q, 1, "row-b", 5));
        assert_eq!(
            a.waits.len(),
            5,
            "one wait per Prefetch; the last one is the gap"
        );
        // 間隔の下限は表の下端、Space までの間の上限は表の上端
        let lo = Duration::from_millis(116);
        assert!(a.waits[..4].iter().all(|w| *w >= lo));
        assert!(a.waits[4] <= Duration::from_millis(779));
        assert_eq!(plan_row(&q, 1, "x", 1).waits.len(), 1);
    }

    #[test]
    fn select_rows_is_stable_and_keeps_the_order() {
        let cases: Vec<EvalCase> = (0..50).map(|i| case(&format!("c{i}"), "あ")).collect();
        let a = select_rows(&cases, 7, Some(10));
        let b = select_rows(&cases, 7, Some(10));
        assert_eq!(a.len(), 10);
        assert!(a.iter().zip(&b).all(|(x, y)| x.id == y.id));
        let positions: Vec<usize> = a
            .iter()
            .map(|c| cases.iter().position(|d| d.id == c.id).unwrap())
            .collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "{positions:?}");
        assert_eq!(select_rows(&cases, 7, None).len(), 50);
        assert_eq!(select_rows(&cases, 7, Some(99)).len(), 50);
    }

    #[test]
    fn percentile_is_nearest_rank() {
        let v: Vec<Duration> = (1..=10).map(Duration::from_millis).collect();
        assert_eq!(percentile(&v, 0.50), Duration::from_millis(5));
        assert_eq!(percentile(&v, 0.90), Duration::from_millis(9));
        assert_eq!(percentile(&v, 0.99), Duration::from_millis(10));
        assert_eq!(percentile(&[], 0.5), Duration::ZERO);
    }

    #[test]
    fn summary_counts_verdicts_and_rates() {
        let row = |verdict, top1, page_ms, infer_ms| RowOutcome {
            id: "x".into(),
            verdict,
            top1,
            first_page: Duration::from_millis(page_ms),
            infer: Duration::from_millis(infer_ms),
        };
        let s = summarize(&[
            row(LmVerdict::LmApplied, true, 10, 50),
            row(LmVerdict::LmApplied, false, 30, 70),
            row(LmVerdict::LmTimedOut, true, 330, 0),
            row(LmVerdict::LmPrefetchLate, true, 20, 0),
        ]);
        assert_eq!(s.rows, 4);
        assert_eq!(s.applied, 2);
        assert_eq!(s.top1, 3);
        assert_eq!(s.applied_rate(), 0.5);
        assert_eq!(s.verdicts["LM_APPLIED"], 2);
        assert_eq!(s.verdicts["LM_TIMED_OUT"], 1);
        assert_eq!(s.infer_n, 2);
        assert_eq!(s.first_page_p99, Duration::from_millis(330));
        let md = render_markdown(&ReplayMeta::default(), &s);
        assert!(md.contains("LM_APPLIED") && md.contains("50.0%"), "{md}");
    }

    #[test]
    fn only_debug_infer_keys_are_echoed() {
        let kv: BTreeMap<String, String> = [
            ("zenz.debug_infer_scale", "3"),
            ("zenz.alpha", "0.5"),
            ("topic.weight", "1"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let d = debug_infer_keys(&kv);
        assert_eq!(d.len(), 1);
        assert_eq!(d["zenz.debug_infer_scale"], "3");
    }
}
