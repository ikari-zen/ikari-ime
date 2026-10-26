use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use ikari_core::{AnalyzedContext, Context, DenseGrammar, InMemoryDictionary, UnknownParams};

use super::*;
use crate::error::LmError;

/// モックの挙動と観測
#[derive(Default)]
struct MockState {
    loads: AtomicUsize,
    drops: AtomicUsize,
    calls: AtomicUsize,
    prompts: Mutex<Vec<String>>,
    /// 1回の採点にかかる時間
    delay: Mutex<Duration>,
    /// 読み込みにかかる時間
    load_delay: Mutex<Duration>,
    fail_load: AtomicBool,
    /// 表記 -> NLL
    table: Mutex<HashMap<String, f32>>,
}

struct MockScorer {
    state: Arc<MockState>,
}

impl LmScorer for MockScorer {
    fn nll(&mut self, prompt: &str, candidates: &[String]) -> Result<Vec<f32>, LmError> {
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        lock(&self.state.prompts).push(prompt.to_owned());
        let delay = *lock(&self.state.delay);
        thread::sleep(delay);
        let table = lock(&self.state.table);
        Ok(candidates
            .iter()
            .map(|c| table.get(c).copied().unwrap_or(10.0))
            .collect())
    }
}

impl Drop for MockScorer {
    fn drop(&mut self) {
        self.state.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn loader(state: &Arc<MockState>) -> LmLoader {
    let state = Arc::clone(state);
    Arc::new(move || {
        thread::sleep(*lock(&state.load_delay));
        if state.fail_load.load(Ordering::SeqCst) {
            return Err(LmError::Load("mock failure".to_owned()));
        }
        state.loads.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(MockScorer {
            state: Arc::clone(&state),
        }))
    })
}

fn mock(nlls: &[(&str, f32)]) -> Arc<MockState> {
    let s = Arc::new(MockState::default());
    let mut t = lock(&s.table);
    for (k, v) in nlls {
        t.insert((*k).to_owned(), *v);
    }
    drop(t);
    s
}

/// 暖機のテストの予算（ms）。モックの遅れ（数十 ms）より桁で長い
const WARM_UP_TEST_BUDGET_MS: u64 = 10_000;

fn cfg(budget_ms: u64, idle_ms: u64) -> ZenzConfig {
    ZenzConfig {
        budget: Duration::from_millis(budget_ms),
        idle_unload: Duration::from_millis(idle_ms),
        load_retry: Duration::from_millis(50),
        ..ZenzConfig::default()
    }
}

fn cand(surface: &str, base: i32, origin: CandidateOrigin) -> Candidate {
    Candidate {
        surface: surface.to_owned(),
        segments: Vec::new(),
        base_cost: base,
        score: base,
        origin,
    }
}

fn dict_cand(surface: &str, base: i32) -> Candidate {
    cand(surface, base, CandidateOrigin::Dictionary)
}

struct Fixture {
    grammar: DenseGrammar,
    dict: InMemoryDictionary,
    analysis: AnalyzedContext,
}

impl Fixture {
    fn new() -> Self {
        let g = DenseGrammar::new(
            1,
            1,
            0,
            UnknownParams {
                left_id: 0,
                right_id: 0,
                pos_id: 0,
                cost_per_char: 1,
            },
        );
        let dict = InMemoryDictionary::new(0);
        Self {
            grammar: g,
            dict,
            analysis: AnalyzedContext::default(),
        }
    }

    /// 締め切りは「今 + ranker の budget」（budget が待ちの上限なので、従来の挙動と同じ）
    fn rank(&self, r: &ZenzRanker, ctx: &Context, reading: &str, cands: &mut [Candidate]) {
        let deadline = Some(Instant::now() + r.cfg.budget);
        self.rank_until(r, ctx, reading, cands, deadline);
    }

    fn rank_until(
        &self,
        r: &ZenzRanker,
        ctx: &Context,
        reading: &str,
        cands: &mut [Candidate],
        deadline: Option<Instant>,
    ) {
        let input = RankInput {
            reading,
            context: ctx,
            grammar: &self.grammar,
            analysis: &self.analysis,
            dict: &self.dict,
            deadline,
        };
        r.rank(&input, cands);
    }
}

fn scores(c: &[Candidate]) -> Vec<(String, i32)> {
    c.iter().map(|c| (c.surface.clone(), c.score)).collect()
}

fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        thread::sleep(Duration::from_millis(5));
    }
    false
}

#[test]
fn rate_stays_within_one_when_the_request_cannot_be_sent() {
    // ローダーがパニックするとワーカーが終わり、以降の要求は送れない（`!sent` の経路）
    let loader: LmLoader = Arc::new(|| panic!("loader panics on purpose"));
    let r = ZenzRanker::new(loader, cfg(50, 60_000));
    let fx = Fixture::new();
    let ctx = Context::new().with_left_text("文脈");
    let mut cands = vec![dict_cand("品詞", 5000)];
    fx.rank(&r, &ctx, "ひんし", &mut cands); // 送れるが、返事が来ない
    assert!(wait_until(|| {
        lock(&r.worker).as_ref().is_some_and(|h| h.is_finished())
    }));
    for i in 0..3 {
        let reading = format!("ひんし{i}");
        fx.rank(&r, &ctx, &reading, &mut cands);
    }
    let s = r.stats();
    assert_eq!(s.calls, 4);
    assert_eq!(s.timeouts, 4, "every call timed out, once each");
    assert_eq!(s.requests, 1, "only the first request reached the queue");
    assert_eq!(s.timeout_rate(), 1.0);
    assert_eq!(scores(&cands), vec![("品詞".to_owned(), 5000)]);
}

#[test]
fn rate_stays_within_one_when_many_calls_overlap() {
    let state = mock(&[("品詞", 1.0)]);
    *lock(&state.delay) = Duration::from_millis(150);
    let r = ZenzRanker::new(loader(&state), cfg(10, 60_000));
    let fx = Fixture::new();
    let ctx = Context::new().with_left_text("文脈");
    const THREADS: usize = 8;
    const CALLS_PER_THREAD: usize = 5;
    thread::scope(|scope| {
        for t in 0..THREADS {
            let (r, fx, ctx) = (&r, &fx, &ctx);
            scope.spawn(move || {
                for i in 0..CALLS_PER_THREAD {
                    // 同じ要求と別の要求を混ぜて、処理中の要求に重ねる
                    let reading = if i % 2 == 0 {
                        "ひんし".to_owned()
                    } else {
                        format!("ひんし{t}")
                    };
                    let mut cands = vec![dict_cand("品詞", 5000)];
                    fx.rank(r, ctx, &reading, &mut cands);
                }
            });
        }
    });
    let s = r.stats();
    assert_eq!(s.calls, (THREADS * CALLS_PER_THREAD) as u64);
    assert!(s.timeouts <= s.calls, "{s:?}");
    assert!(s.timeout_rate() <= 1.0, "{s:?}");
    // 遅いスコアラーなので、ほとんどが時間切れになる
    assert!(s.timeout_rate() > 0.5, "{s:?}");
}

#[test]
fn edge_prompt_is_sent_with_the_context_tag_and_without_conditions() {
    let state = mock(&[("構成", 1.0)]);
    let r = ZenzRanker::new(
        loader(&state),
        ZenzConfig {
            model_kind: ModelKind::KanakanjiEdge,
            ..cfg(2000, 60_000)
        },
    );
    let fx = Fixture::new();
    // 左文脈が空でも EE02 を置く。話題は付けない
    let mut cands = vec![dict_cand("構成", 5000)];
    fx.rank(&r, &Context::new(), "こうせい", &mut cands);
    let ctx = Context::new().with_left_text("システムの");
    let mut cands = vec![dict_cand("構成", 5000)];
    fx.rank(&r, &ctx, "こうせい", &mut cands);
    let prompts = lock(&state.prompts).clone();
    assert_eq!(
        prompts,
        vec![
            "\u{EE02}\u{EE00}コウセイ\u{EE01}".to_owned(),
            "\u{EE02}システムの\u{EE00}コウセイ\u{EE01}".to_owned(),
        ]
    );
}

#[test]
fn scores_are_added_relative_to_the_best_nll() {
    let state = mock(&[("品詞", 2.0), ("瀕死", 5.5), ("貧しい", 9.0)]);
    let r = ZenzRanker::new(loader(&state), cfg(2000, 60_000));
    let fx = Fixture::new();
    let ctx = Context::new().with_left_text("ユーザー辞書に単語を登録するときは");
    let mut cands = vec![
        dict_cand("瀕死", 5000),
        dict_cand("品詞", 5100),
        dict_cand("貧しい", 5200),
    ];
    fx.rank(&r, &ctx, "ひんし", &mut cands);
    assert_eq!(
        scores(&cands),
        vec![
            ("瀕死".to_owned(), 5000 + 3500),
            ("品詞".to_owned(), 5100),
            ("貧しい".to_owned(), 5200 + 6000), // 7000 だが max_total で頭打ち
        ]
    );
    let s = r.stats();
    assert_eq!((s.requests, s.completed, s.timeouts), (1, 1, 0));
    assert!(s.loaded);
    assert_eq!(s.loads, 1);
    // プロンプトには左文脈と読み（カタカナ）が入る
    let prompts = lock(&state.prompts);
    assert!(prompts[0].contains("ユーザー辞書に単語を登録するときは"));
    assert!(prompts[0].contains("\u{EE00}ヒンシ\u{EE01}"));
}

#[test]
fn alpha_scales_the_delta() {
    let state = mock(&[("a", 1.0), ("b", 2.0)]);
    let mut c = cfg(2000, 60_000);
    c.alpha = 0.5;
    let r = ZenzRanker::new(loader(&state), c);
    let fx = Fixture::new();
    let mut cands = vec![dict_cand("a", 100), dict_cand("b", 100)];
    fx.rank(&r, &Context::new(), "x", &mut cands);
    assert_eq!(cands[0].score, 100);
    assert_eq!(cands[1].score, 600);
}

#[test]
fn rewriter_candidates_follow_the_best_dictionary_candidate() {
    let state = mock(&[("bad", 9.0), ("good", 1.0)]);
    let r = ZenzRanker::new(loader(&state), cfg(2000, 60_000));
    let fx = Fixture::new();
    let rw = CandidateOrigin::Rewriter {
        name: "date",
        format: 1,
    };
    let mut cands = vec![
        dict_cand("good", 100),
        cand("2026/10/11", 101, rw),
        dict_cand("bad", 200),
    ];
    fx.rank(&r, &Context::new(), "きょう", &mut cands);
    // good が最小の NLL（加算 0）なので、Rewriter の候補も 0。bad だけが後ろへ下がる
    assert_eq!(cands[1].score, 101);
    assert_eq!(cands[2].score, 200 + 6000);
    // Rewriter の候補は採点の対象にならない
    let prompts = lock(&state.prompts);
    assert_eq!(prompts.len(), 1);
    drop(prompts);
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn candidates_beyond_the_limit_get_the_worst_delta() {
    let state = mock(&[("a", 1.0), ("b", 3.0)]);
    let mut c = cfg(2000, 60_000);
    c.max_candidates = 2;
    let r = ZenzRanker::new(loader(&state), c);
    let fx = Fixture::new();
    let mut cands = vec![dict_cand("a", 0), dict_cand("b", 0), dict_cand("c", 0)];
    fx.rank(&r, &Context::new(), "x", &mut cands);
    assert_eq!(
        cands.iter().map(|c| c.score).collect::<Vec<_>>(),
        vec![0, 2000, 2000]
    );
}

#[test]
fn non_finite_nll_is_penalized_not_fatal() {
    let state = mock(&[("a", 1.0), ("long", f32::INFINITY), ("nan", f32::NAN)]);
    let r = ZenzRanker::new(loader(&state), cfg(2000, 60_000));
    let fx = Fixture::new();
    let mut cands = vec![dict_cand("a", 0), dict_cand("long", 0), dict_cand("nan", 0)];
    fx.rank(&r, &Context::new(), "x", &mut cands);
    assert_eq!(
        cands.iter().map(|c| c.score).collect::<Vec<_>>(),
        vec![0, 6000, 6000]
    );
}

#[test]
fn timeout_leaves_scores_untouched_and_late_result_is_reused() {
    let state = mock(&[("品詞", 2.0), ("瀕死", 5.0)]);
    *lock(&state.delay) = Duration::from_millis(200);
    let r = ZenzRanker::new(loader(&state), cfg(30, 60_000));
    let fx = Fixture::new();
    let ctx = Context::new().with_left_text("ctx");
    let mk = || vec![dict_cand("瀕死", 5000), dict_cand("品詞", 5100)];

    let mut cands = mk();
    let t = Instant::now();
    fx.rank(&r, &ctx, "ひんし", &mut cands);
    let waited = t.elapsed();
    // NFR-04: 30ms 待って諦める（余裕を見て 150ms 未満）
    assert!(waited < Duration::from_millis(150), "{waited:?}");
    assert_eq!(scores(&cands), scores(&mk()));
    assert_eq!(r.stats().timeouts, 1);

    // 同じ変換がもう一度来て、まだ処理中なら、重ねて送らずに戻る
    let mut again = mk();
    fx.rank(&r, &ctx, "ひんし", &mut again);
    assert_eq!(scores(&again), scores(&mk()));
    assert_eq!(r.stats().requests, 1);
    assert_eq!(r.stats().timeouts, 2);
    // 重ねて送らずに戻った分も 1 回の呼び出しとして数えるので、率は 100% を超えない
    assert_eq!(r.stats().calls, 2);
    assert_eq!(r.stats().timeout_rate(), 1.0);

    // 遅れて届いた結果が保持される
    assert!(wait_until(|| r.stats().completed == 1));
    assert!(wait_until(|| lock(&r.shared.late).is_some()));
    let mut third = mk();
    fx.rank(&r, &ctx, "ひんし", &mut third);
    assert_eq!(third[0].score, 5000 + 3000);
    assert_eq!(third[1].score, 5100);
    let s = r.stats();
    assert_eq!(s.late_hits, 1);
    assert_eq!(s.requests, 1, "late hit must not send a new request");
    assert_eq!(s.calls, 3);
    assert!(s.timeouts <= s.calls && s.timeout_rate() < 1.0);
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn waits_only_until_the_deadline_even_when_the_budget_is_longer() {
    let state = mock(&[("品詞", 2.0), ("瀕死", 5.0)]);
    *lock(&state.delay) = Duration::from_millis(400);
    let r = ZenzRanker::new(loader(&state), cfg(2000, 60_000));
    let fx = Fixture::new();
    let ctx = Context::new().with_left_text("ctx");
    let mut cands = vec![dict_cand("瀕死", 5000), dict_cand("品詞", 5100)];
    let t = Instant::now();
    let deadline = Some(Instant::now() + Duration::from_millis(40));
    fx.rank_until(&r, &ctx, "ひんし", &mut cands, deadline);
    let waited = t.elapsed();
    assert!(waited < Duration::from_millis(200), "{waited:?}");
    assert_eq!(
        scores(&cands),
        vec![("瀕死".to_owned(), 5000), ("品詞".to_owned(), 5100)]
    );
    assert_eq!(r.stats().timeouts, 1);
    assert_eq!(r.stats().expired, 0);
}

#[test]
fn budget_is_still_the_ceiling_when_the_deadline_is_far() {
    let state = mock(&[("a", 1.0), ("b", 2.0)]);
    *lock(&state.delay) = Duration::from_millis(400);
    let r = ZenzRanker::new(loader(&state), cfg(30, 60_000));
    let fx = Fixture::new();
    let mut cands = vec![dict_cand("a", 0), dict_cand("b", 0)];
    let t = Instant::now();
    let deadline = Some(Instant::now() + Duration::from_secs(10));
    fx.rank_until(&r, &Context::new(), "x", &mut cands, deadline);
    assert!(
        t.elapsed() < Duration::from_millis(200),
        "{:?}",
        t.elapsed()
    );
    assert_eq!(r.stats().timeouts, 1);
}

#[test]
fn an_expired_or_missing_deadline_sends_nothing() {
    let state = mock(&[("a", 1.0), ("b", 2.0)]);
    let r = ZenzRanker::new(loader(&state), cfg(2000, 60_000));
    let fx = Fixture::new();
    let ctx = Context::new().with_left_text("ctx");

    // 過ぎた締め切り: 送らない。時間切れとして数える（calls に含む）
    let mut c = vec![dict_cand("a", 0), dict_cand("b", 0)];
    let past = Instant::now().checked_sub(Duration::from_millis(1));
    fx.rank_until(&r, &ctx, "x", &mut c, past);
    assert_eq!(c.iter().map(|c| c.score).collect::<Vec<_>>(), vec![0, 0]);
    let s = r.stats();
    assert_eq!((s.calls, s.timeouts, s.expired, s.requests), (1, 1, 1, 0));
    assert!(s.timeout_rate() <= 1.0);

    // 締め切りなし: LM を使わない。calls にも数えない
    fx.rank_until(&r, &ctx, "x", &mut c, None);
    let s = r.stats();
    assert_eq!((s.calls, s.declined, s.requests), (1, 1, 0));
    // モデルは読み込まれてもいない
    assert_eq!(state.loads.load(Ordering::SeqCst), 0);
    assert!(!s.loaded);
}

#[test]
fn all_rank_calls_of_one_conversion_share_one_deadline() {
    // 1つ目の rank が締め切りまで待って時間切れになったら、2つ目は待たずに戻る
    let state = mock(&[("a", 1.0), ("b", 2.0)]);
    *lock(&state.delay) = Duration::from_millis(300);
    let r = ZenzRanker::new(loader(&state), cfg(2000, 60_000));
    let fx = Fixture::new();
    let ctx = Context::new().with_left_text("ctx");
    let deadline = Some(Instant::now() + Duration::from_millis(50));
    let t = Instant::now();
    let mut first = vec![dict_cand("a", 0), dict_cand("b", 0)];
    fx.rank_until(&r, &ctx, "x", &mut first, deadline);
    let after_first = t.elapsed();
    let mut second = vec![dict_cand("a", 0), dict_cand("c", 0)];
    fx.rank_until(&r, &ctx, "x", &mut second, deadline);
    assert!(t.elapsed() - after_first < Duration::from_millis(20));
    let s = r.stats();
    assert_eq!((s.calls, s.timeouts, s.expired, s.requests), (2, 2, 1, 1));
}

#[test]
fn a_finished_late_result_is_used_even_after_the_deadline() {
    // 先行計算（長い締め切り）が終わったあとの、締め切りが「今」の呼び出しは、待たずに結果だけ使う
    let state = mock(&[("品詞", 2.0), ("瀕死", 5.0)]);
    *lock(&state.delay) = Duration::from_millis(100);
    let r = ZenzRanker::new(loader(&state), cfg(2000, 60_000));
    let fx = Fixture::new();
    let ctx = Context::new().with_left_text("ctx");
    let mk = || vec![dict_cand("瀕死", 5000), dict_cand("品詞", 5100)];
    let mut early = mk();
    let soon = Some(Instant::now() + Duration::from_millis(10));
    fx.rank_until(&r, &ctx, "ひんし", &mut early, soon);
    assert_eq!(r.stats().timeouts, 1);
    assert!(wait_until(|| lock(&r.shared.late).is_some()));

    let mut later = mk();
    fx.rank_until(&r, &ctx, "ひんし", &mut later, Some(Instant::now()));
    assert_eq!(later[0].score, 5000 + 3000);
    assert_eq!(later[1].score, 5100);
    let s = r.stats();
    assert_eq!((s.late_hits, s.requests, s.expired), (1, 1, 0));
}

#[test]
fn warm_up_loads_the_model_and_scores_once_without_touching_the_rates() {
    let state = mock(&[]);
    *lock(&state.load_delay) = Duration::from_millis(50);
    *lock(&state.delay) = Duration::from_millis(20);
    // 予算は採点の遅れより十分長くする（負荷が高くても、後半の本番の呼び出しが時間切れにならない）。
    // 読み込みと採点の遅れは、warm_up がそれらを待ってから返すことの確認にだけ使う
    let r = ZenzRanker::new(loader(&state), cfg(WARM_UP_TEST_BUDGET_MS, 60_000));
    assert!(!r.stats().loaded);
    r.warm_up();
    let s = r.stats();
    assert!(s.loaded);
    assert_eq!((s.loads, s.warm_ups, s.calls, s.timeouts), (1, 1, 0, 0));
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);
    // ダミーの採点は「きょう」の短い読み
    let prompts = lock(&state.prompts);
    assert!(
        prompts[0].contains("\u{EE00}キョウ\u{EE01}"),
        "{:?}",
        prompts[0]
    );
    drop(prompts);
    // 暖機のあとの本番の呼び出しは、読み込みを待たずに採点まで進む（採点の遅れは外す）
    *lock(&state.delay) = Duration::ZERO;
    let fx = Fixture::new();
    let mut c = vec![dict_cand("a", 0), dict_cand("b", 0)];
    *lock(&state.table) = [("a".to_owned(), 1.0), ("b".to_owned(), 2.0)].into();
    fx.rank(&r, &Context::new(), "x", &mut c);
    assert_eq!(c[1].score, 1000);
    assert_eq!(r.stats().loads, 1);
}

#[test]
fn warm_up_after_idle_unload_reloads_the_model() {
    let state = mock(&[("a", 1.0)]);
    let r = ZenzRanker::new(loader(&state), cfg(30, 60));
    r.warm_up();
    assert!(wait_until(|| r.stats().unloads == 1));
    r.warm_up();
    assert!(r.stats().loaded);
    assert_eq!(state.loads.load(Ordering::SeqCst), 2);
    assert_eq!(r.stats().warm_ups, 2);
}

#[test]
fn warm_up_returns_when_loading_fails_or_the_worker_is_gone() {
    let state = mock(&[]);
    state.fail_load.store(true, Ordering::SeqCst);
    let r = ZenzRanker::new(loader(&state), cfg(30, 60_000));
    r.warm_up();
    let s = r.stats();
    assert_eq!((s.loaded, s.warm_ups, s.load_failures), (false, 0, 1));

    let broken: LmLoader = Arc::new(|| panic!("loader panics on purpose"));
    let r = ZenzRanker::new(broken, cfg(30, 60_000));
    r.warm_up(); // 1回目: 送れるが、ワーカーが落ちて返事が来ない
    assert!(wait_until(|| {
        lock(&r.worker).as_ref().is_some_and(|h| h.is_finished())
    }));
    r.warm_up(); // 2回目: 送れない
    assert_eq!(r.stats().warm_ups, 0);
}

#[test]
fn late_result_is_not_used_for_a_different_conversion() {
    let state = mock(&[("a", 1.0), ("b", 2.0)]);
    *lock(&state.delay) = Duration::from_millis(120);
    let r = ZenzRanker::new(loader(&state), cfg(20, 60_000));
    let fx = Fixture::new();
    let ctx = Context::new().with_left_text("ctx");
    let mut cands = vec![dict_cand("a", 0), dict_cand("b", 0)];
    fx.rank(&r, &ctx, "あ", &mut cands);
    assert!(wait_until(|| lock(&r.shared.late).is_some()));
    *lock(&state.delay) = Duration::ZERO;

    // 読みが違う、または文脈が違えば、遅れて届いた結果は使わない
    let mut other_reading = vec![dict_cand("a", 0), dict_cand("b", 0)];
    fx.rank(&r, &ctx, "い", &mut other_reading);
    assert_eq!(r.stats().late_hits, 0);
    assert_eq!(r.stats().requests, 2);
    let ctx2 = Context::new().with_left_text("別の文脈");
    let mut other_ctx = vec![dict_cand("a", 0), dict_cand("b", 0)];
    fx.rank(&r, &ctx2, "い", &mut other_ctx);
    assert_eq!(r.stats().late_hits, 0);
    assert_eq!(r.stats().requests, 3);
}

#[test]
fn model_is_unloaded_when_idle_and_reloaded_on_demand() {
    let state = mock(&[("a", 1.0), ("b", 2.0)]);
    let r = ZenzRanker::new(loader(&state), cfg(2000, 80));
    let fx = Fixture::new();
    let mut cands = vec![dict_cand("a", 0), dict_cand("b", 0)];
    fx.rank(&r, &Context::new(), "x", &mut cands);
    assert!(r.stats().loaded);
    assert_eq!(state.drops.load(Ordering::SeqCst), 0);

    // NFR-11: アイドルが続くと解放される
    assert!(wait_until(|| r.stats().unloads == 1));
    assert!(wait_until(|| state.drops.load(Ordering::SeqCst) == 1));
    assert!(!r.stats().loaded);

    // 次の要求で読み直す
    let mut again = vec![dict_cand("a", 0), dict_cand("b", 0)];
    fx.rank(&r, &Context::new(), "y", &mut again);
    assert_eq!(state.loads.load(Ordering::SeqCst), 2);
    assert_eq!(again[1].score, 1000);
    assert!(r.stats().loaded);
}

#[test]
fn requests_keep_the_model_alive() {
    let state = mock(&[("a", 1.0)]);
    let r = ZenzRanker::new(loader(&state), cfg(2000, 300));
    let fx = Fixture::new();
    for i in 0..4 {
        let mut cands = vec![dict_cand("a", 0)];
        fx.rank(&r, &Context::new(), &format!("k{i}"), &mut cands);
        thread::sleep(Duration::from_millis(100));
    }
    // 合計 400ms 以上経っているが、各間隔は idle_unload より短いので解放されない
    assert_eq!(r.stats().unloads, 0);
    assert_eq!(state.loads.load(Ordering::SeqCst), 1);
}

#[test]
fn reload_after_unload_counts_as_timeout_while_loading() {
    let state = mock(&[("a", 1.0), ("b", 2.0)]);
    let r = ZenzRanker::new(loader(&state), cfg(30, 60));
    let fx = Fixture::new();
    let mut c = vec![dict_cand("a", 0), dict_cand("b", 0)];
    fx.rank(&r, &Context::new(), "x", &mut c);
    assert!(wait_until(|| r.stats().unloads == 1));

    // 読み込みに 200ms かかる。その間は時間切れとして、順位を変えずに戻る
    *lock(&state.load_delay) = Duration::from_millis(200);
    let mut c = vec![dict_cand("a", 0), dict_cand("b", 0)];
    let t = Instant::now();
    fx.rank(&r, &Context::new(), "y", &mut c);
    assert!(t.elapsed() < Duration::from_millis(150));
    assert_eq!(c.iter().map(|c| c.score).collect::<Vec<_>>(), vec![0, 0]);
    assert_eq!(r.stats().timeouts, 1);
    // 読み込みと採点が終わったあとは、遅れて届いた結果を使える
    assert!(wait_until(|| r.stats().completed == 2));
    assert!(wait_until(|| lock(&r.shared.late).is_some()));
    let mut c = vec![dict_cand("a", 0), dict_cand("b", 0)];
    fx.rank(&r, &Context::new(), "y", &mut c);
    assert_eq!(c[1].score, 1000);
}

#[test]
fn load_failure_keeps_ranking_unchanged_and_retries_later() {
    let state = mock(&[("a", 1.0), ("b", 2.0)]);
    state.fail_load.store(true, Ordering::SeqCst);
    let r = ZenzRanker::new(loader(&state), cfg(500, 60_000));
    let fx = Fixture::new();
    let mut c = vec![dict_cand("a", 0), dict_cand("b", 0)];
    fx.rank(&r, &Context::new(), "x", &mut c);
    assert_eq!(c.iter().map(|c| c.score).collect::<Vec<_>>(), vec![0, 0]);
    let s = r.stats();
    assert_eq!(s.load_failures, 1);
    assert!(!s.loaded);

    // 失敗の直後は読み込みを試さない（キーごとに重い読み込みを繰り返さない）
    let mut c = vec![dict_cand("a", 0), dict_cand("b", 0)];
    fx.rank(&r, &Context::new(), "y", &mut c);
    assert_eq!(r.stats().load_failures, 1);

    // 待ち時間を過ぎて、読み込みが直っていれば成功する
    state.fail_load.store(false, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(80));
    let mut c = vec![dict_cand("a", 0), dict_cand("b", 0)];
    fx.rank(&r, &Context::new(), "z", &mut c);
    assert_eq!(c[1].score, 1000);
    assert_eq!(r.stats().loads, 1);
}

#[test]
fn secure_context_never_reaches_the_model() {
    let state = mock(&[("a", 1.0)]);
    let r = ZenzRanker::new(loader(&state), cfg(500, 60_000));
    let fx = Fixture::new();
    let ctx = Context::new().with_secure(true);
    let mut c = vec![dict_cand("a", 5), dict_cand("b", 7)];
    fx.rank(&r, &ctx, "x", &mut c);
    assert_eq!(c.iter().map(|c| c.score).collect::<Vec<_>>(), vec![5, 7]);
    assert_eq!(r.stats().requests, 0);
    assert_eq!(state.loads.load(Ordering::SeqCst), 0);
}

#[test]
fn empty_and_rewriter_only_pools_do_nothing() {
    let state = mock(&[]);
    let r = ZenzRanker::new(loader(&state), cfg(500, 60_000));
    let fx = Fixture::new();
    fx.rank(&r, &Context::new(), "x", &mut []);
    let rw = CandidateOrigin::Rewriter {
        name: "n",
        format: 1,
    };
    let mut c = vec![cand("1", 3, rw)];
    fx.rank(&r, &Context::new(), "x", &mut c);
    assert_eq!(c[0].score, 3);
    assert_eq!(r.stats().requests, 0);
}

#[test]
fn newest_request_wins_when_the_worker_is_busy() {
    let state = mock(&[("a", 1.0)]);
    *lock(&state.load_delay) = Duration::from_millis(150);
    let r = Arc::new(ZenzRanker::new(loader(&state), cfg(10, 60_000)));
    let fx = Arc::new(Fixture::new());
    // 読み込み中に3つの要求が溜まる
    for reading in ["r1", "r2", "r3"] {
        let mut c = vec![dict_cand("a", 0)];
        fx.rank(&r, &Context::new(), reading, &mut c);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(wait_until(|| r.stats().completed >= 2));
    thread::sleep(Duration::from_millis(50));
    let s = r.stats();
    // r1 は読み込み中に受け取って処理する。その間に届いた r2 は、r3 に置き換えられる
    assert_eq!(s.requests, 3);
    assert_eq!(s.completed, 2, "{s:?}");
    assert_eq!(s.superseded, 1, "{s:?}");
    let prompts = lock(&state.prompts);
    assert_eq!(prompts.len(), 2);
    assert!(prompts[0].contains("r1"), "{:?}", prompts[0]);
    assert!(prompts[1].contains("r3"), "{:?}", prompts[1]);
}

#[test]
fn dropping_the_ranker_releases_the_model_and_joins_the_worker() {
    let state = mock(&[("a", 1.0)]);
    let r = ZenzRanker::new(loader(&state), cfg(500, 60_000));
    let fx = Fixture::new();
    let mut c = vec![dict_cand("a", 0)];
    fx.rank(&r, &Context::new(), "x", &mut c);
    assert_eq!(state.drops.load(Ordering::SeqCst), 0);
    drop(r);
    assert_eq!(state.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn dropping_before_any_request_never_loads_the_model() {
    let state = mock(&[]);
    let r = ZenzRanker::new(loader(&state), cfg(500, 60_000));
    drop(r);
    assert_eq!(state.loads.load(Ordering::SeqCst), 0);
}

#[test]
fn profile_and_topic_enter_the_prompt() {
    use ikari_core::{Profile, ProfileDomain, ProfileWeight};
    let state = mock(&[("a", 1.0)]);
    let r = ZenzRanker::new(loader(&state), cfg(2000, 60_000));
    let mut fx = Fixture::new();
    let tok = |lemma: &str| ikari_core::ContextToken {
        surface: lemma.to_owned(),
        lemma: lemma.to_owned(),
        lemma_id: None,
        word: None,
        left_id: 0,
        right_id: 0,
        pos_id: 0,
        is_content: true,
        char_end: 0,
    };
    fx.analysis.tokens = vec![tok("辞書"), tok("登録"), tok("辞書")];
    let profile = Profile::new(vec![ProfileDomain {
        id: "it".to_owned(),
        name: "IT".to_owned(),
        weight: ProfileWeight::OFTEN,
        seeds: Vec::new(),
    }]);
    let ctx = Context::new()
        .with_left_text("あいうえお")
        .with_profile(Some(Arc::new(profile)));
    let mut c = vec![dict_cand("a", 0)];
    fx.rank(&r, &ctx, "ひんし", &mut c);
    let prompts = lock(&state.prompts);
    assert_eq!(
        prompts[0],
        "\u{EE03}IT\u{EE04}辞書、登録\u{EE02}あいうえお\u{EE00}ヒンシ\u{EE01}"
    );
}

#[test]
fn long_left_context_is_truncated_to_the_tail() {
    let state = mock(&[("a", 1.0)]);
    let r = ZenzRanker::new(loader(&state), cfg(2000, 60_000));
    let fx = Fixture::new();
    let long: String = (0..100)
        .map(|i| char::from_u32(0x3042 + (i % 40)).unwrap())
        .collect();
    let ctx = Context::new().with_left_text(long.clone());
    let mut c = vec![dict_cand("a", 0)];
    fx.rank(&r, &ctx, "x", &mut c);
    let prompts = lock(&state.prompts);
    let expected_tail = tail_chars(&long, 40);
    assert!(prompts[0].contains(expected_tail));
    assert!(!prompts[0].contains(&long));
}
