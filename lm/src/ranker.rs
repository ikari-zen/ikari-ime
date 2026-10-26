//! `ZenzRanker`: zenz による話題スコアの Ranker（ADR 0002 §13.2）。
//!
//! - モデルは専用のワーカースレッドが持つ（llama の context は Sync ではないため）。
//! - `rank()` は要求を送り、`min(budget, 締め切りまでの残り)` だけ待つ。間に合わなければ何もしない（NFR-04）。
//!   締め切りは変換1回につき1つ（`RankInput.deadline`。ADR 0004 §3.4）で、`budget` は待ちの上限として残す。
//!   締め切りが `None` または過ぎていれば、要求を送らない（遅れて届いた結果が既にあれば、待たずに使う）。
//!   遅れて届いた結果は (読み, 文脈の末尾) をキーに1件だけ保持し、同じ変換が来たら使う。
//! - `warm_up()` は短い採点を1回済ませて、モデルの読み込みと Metal の初期化を終わらせる（ADR 0003 §6-4）。
//! - 最後の要求から `idle_unload` が経つと、ワーカーがモデルを解放する（NFR-11）。
//!   次の要求で読み直し、読み直している間は時間切れとして扱う（NFR-15: n-gram 相当の順位のまま）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ikari_core::{Candidate, CandidateOrigin, RankInput, Ranker};

use crate::config::{COST_PER_NAT, ModelKind, ZenzConfig};
use crate::prompt::{
    PromptInput, build_prompt_for, head_chars, join_topic, tail_chars, topic_words,
};
use crate::scorer::{LmLoader, LmScorer};

const RANKER_NAME: &str = "zenz";
const WORKER_THREAD_NAME: &str = "ikari-zenz";
const PROFILE_NAME_SEPARATOR: &str = "、";

/// 要求の同一性（読みと、文脈の末尾）
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RequestKey {
    reading: String,
    context_tail: String,
}

struct Request {
    key: RequestKey,
    prompt: String,
    candidates: Vec<String>,
    reply: SyncSender<Scored>,
}

/// `warm_up()` が採点させる読みと候補（短い読みで、候補は2つ）
const WARM_UP_READING: &str = "きょう";
const WARM_UP_CANDIDATES: [&str; 2] = ["今日", "京"];
/// `warm_up()` が読み込みと採点を待つ時間の上限（読み込みに失敗したままブロックしないため）
const WARM_UP_TIMEOUT: Duration = Duration::from_secs(60);

/// 表記ごとの NLL
type Scored = Arc<HashMap<String, f32>>;

#[derive(Default)]
struct Counters {
    /// 採点の対象があった `rank()` の呼び出し数（時間切れ率の分母）
    calls: AtomicU64,
    requests: AtomicU64,
    completed: AtomicU64,
    timeouts: AtomicU64,
    expired: AtomicU64,
    declined: AtomicU64,
    warm_ups: AtomicU64,
    late_hits: AtomicU64,
    superseded: AtomicU64,
    loads: AtomicU64,
    load_failures: AtomicU64,
    unloads: AtomicU64,
    inference_errors: AtomicU64,
}

struct Shared {
    counters: Counters,
    loaded: AtomicBool,
    /// 遅れて届いた結果（1件だけ）
    late: Mutex<Option<(RequestKey, Scored)>>,
    /// 最後に送った要求のキー（同じ要求を重ねて送らないため）
    inflight: Mutex<Option<RequestKey>>,
}

/// 動作の統計（eval のレポートと、モックのテストで使う）
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LmStats {
    /// 採点の対象があった `rank()` の呼び出し数。`timeouts` は 1 回の呼び出しで最大 1 回なので、`timeouts <= calls`
    pub calls: u64,
    /// ワーカーへ送った要求の数
    pub requests: u64,
    /// ワーカーが採点を終えた数（時間切れで捨てられたものも含む）
    pub completed: u64,
    /// `rank()` が間に合わずに戻った回数（`expired` を含む）
    pub timeouts: u64,
    /// 締め切りを過ぎていて、要求を送らずに戻った回数（`timeouts` の内訳。`calls` に含む）
    pub expired: u64,
    /// 締め切りが `None`（LM を使わない）で、何もせずに戻った回数（`calls` に含まない）
    pub declined: u64,
    /// `warm_up()` が採点を終えた回数
    pub warm_ups: u64,
    /// 遅れて届いた結果を使った回数
    pub late_hits: u64,
    /// 新しい要求に置き換えられて、処理されなかった要求の数
    pub superseded: u64,
    pub loads: u64,
    pub load_failures: u64,
    pub unloads: u64,
    pub inference_errors: u64,
    /// いまモデルが読み込まれているか
    pub loaded: bool,
}

impl LmStats {
    /// 時間切れの割合（`rank()` の呼び出し数に対して。0〜1）。呼び出しがなければ 0
    pub fn timeout_rate(&self) -> f64 {
        if self.calls == 0 {
            0.0
        } else {
            self.timeouts as f64 / self.calls as f64
        }
    }
}

pub struct ZenzRanker {
    cfg: ZenzConfig,
    shared: Arc<Shared>,
    tx: Mutex<Option<Sender<Request>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // ロックを持ったままパニックしても、保持しているのは単純な値なのでそのまま使う
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl ZenzRanker {
    /// ワーカースレッドを起動する。モデルは最初の要求で（`loader` により）読み込む。
    pub fn new(loader: LmLoader, cfg: ZenzConfig) -> Self {
        let shared = Arc::new(Shared {
            counters: Counters::default(),
            loaded: AtomicBool::new(false),
            late: Mutex::new(None),
            inflight: Mutex::new(None),
        });
        let (tx, rx) = mpsc::channel::<Request>();
        let worker = {
            let shared = Arc::clone(&shared);
            let cfg = cfg.clone();
            thread::Builder::new()
                .name(WORKER_THREAD_NAME.to_owned())
                .spawn(move || worker_loop(rx, loader, shared, cfg))
                .ok()
        };
        // スレッドを作れなければ、要求を送る先がないので常に何もしない Ranker になる
        let tx = worker.as_ref().map(|_| tx);
        Self {
            cfg,
            shared,
            tx: Mutex::new(tx),
            worker: Mutex::new(worker),
        }
    }

    pub fn config(&self) -> &ZenzConfig {
        &self.cfg
    }

    pub fn stats(&self) -> LmStats {
        let c = &self.shared.counters;
        LmStats {
            calls: c.calls.load(Ordering::Relaxed),
            requests: c.requests.load(Ordering::Relaxed),
            completed: c.completed.load(Ordering::Relaxed),
            timeouts: c.timeouts.load(Ordering::Relaxed),
            expired: c.expired.load(Ordering::Relaxed),
            declined: c.declined.load(Ordering::Relaxed),
            warm_ups: c.warm_ups.load(Ordering::Relaxed),
            late_hits: c.late_hits.load(Ordering::Relaxed),
            superseded: c.superseded.load(Ordering::Relaxed),
            loads: c.loads.load(Ordering::Relaxed),
            load_failures: c.load_failures.load(Ordering::Relaxed),
            unloads: c.unloads.load(Ordering::Relaxed),
            inference_errors: c.inference_errors.load(Ordering::Relaxed),
            loaded: self.shared.loaded.load(Ordering::Acquire),
        }
    }

    /// 採点する候補の表記（重複なし。Rewriter の候補は除く）
    fn scored_surfaces<'a>(&self, cands: &'a [Candidate]) -> Vec<&'a str> {
        let mut seen = std::collections::HashSet::new();
        cands
            .iter()
            .filter(|c| !matches!(c.origin, CandidateOrigin::Rewriter { .. }))
            .map(|c| c.surface.as_str())
            .filter(|s| seen.insert(*s))
            .take(self.cfg.max_candidates)
            .collect()
    }

    fn build_request_parts(&self, input: &RankInput<'_>) -> (RequestKey, String) {
        let tail = tail_chars(&input.context.left_text, self.cfg.context_chars);
        // edge は話題とプロフィールを学習していないので、作らない
        let uses_conditions = self.cfg.model_kind == ModelKind::ZenzV3;
        let topic = if uses_conditions {
            join_topic(&topic_words(&input.analysis.tokens, self.cfg.topic_words))
        } else {
            String::new()
        };
        let profile = input
            .context
            .profile
            .as_ref()
            .filter(|_| uses_conditions)
            .map(|p| {
                p.domains
                    .iter()
                    .map(|d| d.name.as_str())
                    .collect::<Vec<_>>()
                    .join(PROFILE_NAME_SEPARATOR)
            });
        let right = head_chars(&input.context.right_text, self.cfg.right_context_chars);
        let prompt = build_prompt_for(
            self.cfg.model_kind,
            &PromptInput {
                profile: profile.as_deref(),
                topic: Some(topic.as_str()),
                left_context: tail,
                right_context: right,
                reading: input.reading,
            },
        );
        // プロンプトが同じなら同じ変換として扱う（話題やプロフィールの違いも区別される）
        (
            RequestKey {
                reading: input.reading.to_owned(),
                context_tail: prompt.clone(),
            },
            prompt,
        )
    }

    fn apply(&self, scored: &HashMap<String, f32>, cands: &mut [Candidate]) {
        let cfg = &self.cfg;
        let finite: Vec<f32> = scored.values().copied().filter(|v| v.is_finite()).collect();
        let Some(min) = finite.iter().copied().reduce(f32::min) else {
            return;
        };
        let delta_of = |nll: f32| -> i32 {
            if !nll.is_finite() {
                return cfg.max_total;
            }
            let d = cfg.alpha * (nll - min) * COST_PER_NAT;
            (d.round() as i32).clamp(0, cfg.max_total)
        };
        // 採点した候補のうち base_cost が最小のもの（Rewriter の候補の基準にする）
        let mut anchor: Option<(i32, i32)> = None; // (base_cost, delta)
        let mut worst = 0;
        for c in cands.iter() {
            if matches!(c.origin, CandidateOrigin::Rewriter { .. }) {
                continue;
            }
            if let Some(&nll) = scored.get(&c.surface) {
                let d = delta_of(nll);
                worst = worst.max(d);
                if anchor.is_none_or(|(b, _)| c.base_cost < b) {
                    anchor = Some((c.base_cost, d));
                }
            }
        }
        let anchor_delta = anchor.map_or(0, |(_, d)| d);
        for c in cands.iter_mut() {
            let d = if matches!(c.origin, CandidateOrigin::Rewriter { .. }) {
                // 辞書の1位と同じ扱いにして、辞書の候補との相対的な位置を保つ
                anchor_delta
            } else {
                // 採点の対象外（上限を超えた候補）は、採点した中で最悪の加算にする
                scored.get(&c.surface).map_or(worst, |&nll| delta_of(nll))
            };
            c.score = c.score.saturating_add(d);
        }
    }
}

impl ZenzRanker {
    /// ダミーの短い採点（「きょう」）を1回実行し、モデルの読み込みと Metal の初期化を済ませる。
    /// 終わるまで呼び出しのスレッドを止める（`WARM_UP_TIMEOUT` が上限）ので、別スレッドで呼ぶ。
    /// アイドル解放（NFR-11）のあとも、読み直しを早めたいときに呼べる。
    /// 時間切れの統計と、遅れて届いた結果の保持には影響しない。
    /// 読み込みに失敗した、またはワーカーがいないときは何もしない。
    pub fn warm_up(&self) {
        let prompt = build_prompt_for(
            self.cfg.model_kind,
            &PromptInput {
                profile: None,
                topic: Some(""),
                left_context: "",
                right_context: "",
                reading: WARM_UP_READING,
            },
        );
        let (reply_tx, reply_rx) = mpsc::sync_channel::<Scored>(1);
        let request = Request {
            key: RequestKey {
                reading: WARM_UP_READING.to_owned(),
                context_tail: prompt.clone(),
            },
            prompt,
            candidates: WARM_UP_CANDIDATES.iter().map(|s| (*s).to_owned()).collect(),
            reply: reply_tx,
        };
        let sent = {
            let tx = lock(&self.tx);
            tx.as_ref().is_some_and(|tx| tx.send(request).is_ok())
        };
        if !sent {
            return;
        }
        if reply_rx.recv_timeout(WARM_UP_TIMEOUT).is_ok() {
            self.shared
                .counters
                .warm_ups
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Ranker for ZenzRanker {
    fn rank(&self, input: &RankInput<'_>, cands: &mut [Candidate]) {
        if input.context.secure || cands.is_empty() {
            return;
        }
        let surfaces = self.scored_surfaces(cands);
        if surfaces.is_empty() {
            return;
        }
        let counters = &self.shared.counters;
        // 締め切りがない呼び出しは、LM を使わない呼び出し（時間切れ率の分母に入れない）
        let Some(deadline) = input.deadline else {
            counters.declined.fetch_add(1, Ordering::Relaxed);
            return;
        };
        counters.calls.fetch_add(1, Ordering::Relaxed);
        let (key, prompt) = self.build_request_parts(input);

        // 遅れて届いた結果が、同じ変換のものならそのまま使う
        {
            let late = lock(&self.shared.late);
            if let Some((k, scored)) = late.as_ref()
                && *k == key
                && surfaces.iter().all(|s| scored.contains_key(*s))
            {
                let scored = Arc::clone(scored);
                drop(late);
                self.shared
                    .counters
                    .late_hits
                    .fetch_add(1, Ordering::Relaxed);
                self.apply(&scored, cands);
                return;
            }
        }

        // 締め切りを過ぎていれば要求を送らない（変換1回の中の2回目以降の rank が、待ちを重ねないため）
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .min(self.cfg.budget);
        if wait.is_zero() {
            counters.expired.fetch_add(1, Ordering::Relaxed);
            counters.timeouts.fetch_add(1, Ordering::Relaxed);
            return;
        }
        // 同じ要求の処理がまだ続いているなら、重ねて送らずに時間切れとして戻る
        {
            let mut inflight = lock(&self.shared.inflight);
            if inflight.as_ref() == Some(&key) {
                counters.timeouts.fetch_add(1, Ordering::Relaxed);
                return;
            }
            *inflight = Some(key.clone());
        }

        let (reply_tx, reply_rx) = mpsc::sync_channel::<Scored>(1);
        let request = Request {
            key: key.clone(),
            prompt,
            candidates: surfaces.iter().map(|s| (*s).to_owned()).collect(),
            reply: reply_tx,
        };
        let sent = {
            let tx = lock(&self.tx);
            match tx.as_ref() {
                // キューは無制限。ワーカーは溜まった要求のうち最新だけを処理する（読み込み中に溜まっても軽い）
                Some(tx) => tx.send(request).is_ok(),
                None => false,
            }
        };
        if !sent {
            clear_inflight(&self.shared, &key);
            counters.timeouts.fetch_add(1, Ordering::Relaxed);
            return;
        }
        counters.requests.fetch_add(1, Ordering::Relaxed);

        match reply_rx.recv_timeout(wait) {
            Ok(scored) => {
                clear_inflight(&self.shared, &key);
                self.apply(&scored, cands);
            }
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                counters.timeouts.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn name(&self) -> &str {
        RANKER_NAME
    }
}

impl Drop for ZenzRanker {
    fn drop(&mut self) {
        // 送信側を閉じるとワーカーが終わる。モデルの解放までスレッドの中で済ませてから戻る
        drop(lock(&self.tx).take());
        if let Some(handle) = lock(&self.worker).take() {
            let _ = handle.join();
        }
    }
}

fn clear_inflight(shared: &Shared, key: &RequestKey) {
    let mut inflight = lock(&shared.inflight);
    if inflight.as_ref() == Some(key) {
        *inflight = None;
    }
}

fn worker_loop(rx: Receiver<Request>, loader: LmLoader, shared: Arc<Shared>, cfg: ZenzConfig) {
    let counters = &shared.counters;
    let mut scorer: Option<Box<dyn LmScorer>> = None;
    let mut last_load_failure: Option<Instant> = None;

    loop {
        let first = if scorer.is_some() {
            match rx.recv_timeout(cfg.idle_unload) {
                Ok(r) => r,
                Err(RecvTimeoutError::Timeout) => {
                    // NFR-11: アイドルが続いたのでモデルを解放する
                    scorer = None;
                    shared.loaded.store(false, Ordering::Release);
                    counters.unloads.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(r) => r,
                Err(_) => break,
            }
        };

        // 溜まっている要求は最新のものだけを処理する（古い要求の送り主は、もう待っていない）
        let mut req = first;
        while let Ok(newer) = rx.try_recv() {
            clear_inflight_if_stale(&shared, &req.key, &newer.key);
            counters.superseded.fetch_add(1, Ordering::Relaxed);
            req = newer;
        }

        if scorer.is_none() {
            let blocked = last_load_failure.is_some_and(|t| t.elapsed() < cfg.load_retry);
            if !blocked {
                match loader() {
                    Ok(s) => {
                        scorer = Some(s);
                        last_load_failure = None;
                        shared.loaded.store(true, Ordering::Release);
                        counters.loads.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        last_load_failure = Some(Instant::now());
                        counters.load_failures.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
        let Some(s) = scorer.as_mut() else {
            clear_inflight(&shared, &req.key);
            continue;
        };

        match s.nll(&req.prompt, &req.candidates) {
            Ok(nlls) if nlls.len() == req.candidates.len() => {
                counters.completed.fetch_add(1, Ordering::Relaxed);
                let scored: Scored = Arc::new(req.candidates.iter().cloned().zip(nlls).collect());
                // 待っている人がいれば渡す。もういなければ（時間切れ）、次の同じ変換のために残す
                if req.reply.send(Arc::clone(&scored)).is_err() {
                    *lock(&shared.late) = Some((req.key.clone(), scored));
                }
            }
            Ok(_) | Err(_) => {
                counters.inference_errors.fetch_add(1, Ordering::Relaxed);
            }
        }
        clear_inflight(&shared, &req.key);
    }
    // ワーカーの終了時に、モデルはこのスレッドの中で解放される
    drop(scorer);
    shared.loaded.store(false, Ordering::Release);
}

/// 置き換えられた古い要求のキーを inflight から外す（新しい要求のキーが入っていれば残す）
fn clear_inflight_if_stale(shared: &Shared, old: &RequestKey, newer: &RequestKey) {
    if old != newer {
        clear_inflight(shared, old);
    }
}

#[cfg(test)]
mod tests;
