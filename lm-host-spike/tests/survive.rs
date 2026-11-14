//! ADR 0060 Q4 の確定条件のスパイク。偽の lm-host（モデル不要）を使い、`ZenzRanker` の経路で
//! 「子が落ちる・応答しない → LM なし（辞書だけの順位）で返し続ける → 連続失敗で諦める」を確かめる。
//! 数値は仮置き（テストが揺れない程度の長さ。`ikari_testtime` で伸ばせる）。

#![cfg(unix)]

use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use ikari_core::{
    AnalyzedContext, Candidate, CandidateOrigin, Context, DenseGrammar, InMemoryDictionary,
    RankInput, Ranker, UnknownParams,
};
use ikari_lm::{LmError, LmScorer, LmStats, ZenzConfig, ZenzRanker};
use ikari_lm_host_spike::{HostConfig, HostScorer};

const FAKE_HOST: &str = env!("CARGO_BIN_EXE_fake-host");
/// ranker が結果を待つ上限。偽の子の応答（ms 未満）より桁で長い
const RANK_BUDGET_MS: u64 = 1500;
const REQUEST_DEADLINE_MS: u64 = 300;
/// 良い候補（NLL 0）と悪い候補（NLL 20）。基礎コストは悪い候補のほうが良い（LM があると逆転する）
const GOOD: &str = "良";
const BAD: &str = "悪";
const BAD_BASE: i32 = 5000;
const GOOD_BASE: i32 = 5200;

/// 初めて実行する exe は、OS の走査（macOS の XProtect など）で起動が 1 秒以上かかることがある
/// （Windows の初回 901ms と同じ現象と推測）。テストが揺れないよう、最初に 1 回だけ空打ちする。
fn warm_up_exe() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = Command::new(FAKE_HOST).arg("die-on-start").status();
    });
}

fn host_cfg(mode: &str) -> HostConfig {
    warm_up_exe();
    let mut c = HostConfig::new(FAKE_HOST, vec![mode.to_owned()]);
    c.request_deadline = ikari_testtime::ms(REQUEST_DEADLINE_MS);
    c.ready_deadline = ikari_testtime::ms(3000);
    c
}

fn cand(surface: &str, base: i32) -> Candidate {
    Candidate {
        surface: surface.to_owned(),
        segments: Vec::new(),
        base_cost: base,
        score: base,
        origin: CandidateOrigin::Dictionary,
    }
}

/// 辞書だけの順位なら BAD が先、LM が効けば GOOD が先
fn first_surface(r: &ZenzRanker, reading: &str) -> String {
    let grammar = DenseGrammar::new(
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
    let analysis = AnalyzedContext::default();
    let ctx = Context::new().with_left_text("文脈");
    let mut cands = vec![cand(BAD, BAD_BASE), cand(GOOD, GOOD_BASE)];
    let input = RankInput {
        reading,
        context: &ctx,
        grammar: &grammar,
        analysis: &analysis,
        dict: &dict,
        deadline: Some(Instant::now() + ikari_testtime::ms(RANK_BUDGET_MS)),
    };
    r.rank(&input, &mut cands);
    cands.sort_by_key(|c| c.score);
    cands[0].surface.clone()
}

fn ranker_with(scorer: Arc<Mutex<Option<HostScorer>>>) -> ZenzRanker {
    // ローダーは 1 回だけ HostScorer を渡す（ワーカーは Err でも scorer を捨てないので、再読み込みは起きない）
    let loader: Arc<dyn Fn() -> Result<Box<dyn LmScorer>, LmError> + Send + Sync> =
        Arc::new(move || {
            let s = scorer
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| LmError::Load("already taken".to_owned()))?;
            Ok(Box::new(s))
        });
    let cfg = ZenzConfig {
        budget: ikari_testtime::ms(RANK_BUDGET_MS),
        ..ZenzConfig::default()
    };
    ZenzRanker::new(loader, cfg)
}

fn pid_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + ikari_testtime::scaled(Duration::from_secs(5));
    while Instant::now() < end {
        if cond() {
            return true;
        }
        thread::sleep(ikari_testtime::ms(5));
    }
    false
}

#[test]
fn baseline_lm_reorders_through_the_child() {
    let scorer = HostScorer::new(host_cfg("ok"));
    let r = ranker_with(Arc::new(Mutex::new(Some(scorer))));
    assert_eq!(first_surface(&r, "あ0"), GOOD, "LM が効くと GOOD が先");
    assert_eq!(r.stats().completed, 1);
}

// (a) 子を kill → 次の変換が LM なしで返る。そのあと再起動して LM が戻る。子は回収される（zombie なし）。
#[test]
fn killed_child_falls_back_then_recovers() {
    // 外から kill（SIGKILL）
    let mut scorer = HostScorer::new(host_cfg("ok"));
    assert!(scorer.nll("p", &[GOOD.to_owned()]).is_ok());
    let pid = scorer.child_id().expect("child");
    assert!(pid_alive(pid));
    Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .status()
        .unwrap();
    // 次の要求は失敗する（Err = ranker から見れば LM なし）。失敗時に子を回収する
    assert!(scorer.nll("p", &[GOOD.to_owned()]).is_err());
    assert!(!pid_alive(pid), "wait() で回収済み（zombie が残らない）");
    assert_eq!(scorer.consecutive_failures(), 1);
    // その次は再起動して成功し、失敗の数は 0 に戻る
    assert!(scorer.nll("p", &[GOOD.to_owned()]).is_ok());
    assert_eq!(scorer.consecutive_failures(), 0);
    assert_eq!(scorer.spawns(), 2);
}

// ranker 経由: 子が途中で（N 件のあと）死んでも、順位は辞書だけのものに戻り、パニックしない。
#[test]
fn ranker_keeps_returning_dictionary_order_after_child_dies() {
    let scorer = HostScorer::new(host_cfg("die-after=1"));
    let r = ranker_with(Arc::new(Mutex::new(Some(scorer))));
    assert_eq!(first_surface(&r, "あ1"), GOOD, "1 件目は LM が効く");
    // 2 件目: 子は何も返さず終了 → LM なし
    assert_eq!(first_surface(&r, "あ2"), BAD, "LM なしの順位");
    assert!(wait_until(|| r.stats().inference_errors >= 1));
    // 3 件目: 再起動した子が 1 件答える（die-after=1 なので）→ LM が戻る
    assert_eq!(first_surface(&r, "あ3"), GOOD, "再起動で LM が戻る");
    let s: LmStats = r.stats();
    assert!(s.inference_errors >= 1);
}

// (b) 子が応答しない（ハング）→ 締め切りで LM なしに戻る。ワーカーは塞がり続けない。
#[test]
fn hung_child_is_abandoned_at_the_deadline() {
    let mut scorer = HostScorer::new(host_cfg("hang-after=0"));
    let t = Instant::now();
    assert!(scorer.nll("p", &[GOOD.to_owned()]).is_err());
    let took = t.elapsed();
    assert!(
        took >= ikari_testtime::ms(REQUEST_DEADLINE_MS),
        "締め切りまでは待つ: {took:?}"
    );
    assert!(
        took < ikari_testtime::ms(REQUEST_DEADLINE_MS * 4),
        "締め切りを大きく超えて塞がらない: {took:?}"
    );
    assert!(
        scorer.child_id().is_none(),
        "ハングした子は kill して回収済み"
    );

    // ranker 経由でも、締め切り前に LM なしで返る（budget < 子の締め切りのとき、ranker が先に諦める）
    let scorer = HostScorer::new(host_cfg("hang-after=0"));
    let cfg = ZenzConfig {
        budget: ikari_testtime::ms(100),
        ..ZenzConfig::default()
    };
    let slot = Arc::new(Mutex::new(Some(scorer)));
    let loader: Arc<dyn Fn() -> Result<Box<dyn LmScorer>, LmError> + Send + Sync> =
        Arc::new(move || {
            let s = slot.lock().unwrap().take().unwrap();
            Ok(Box::new(s))
        });
    let r = ZenzRanker::new(loader, cfg);
    assert_eq!(first_surface_with_budget(&r, "あ4", 100), BAD);
}

fn first_surface_with_budget(r: &ZenzRanker, reading: &str, budget_ms: u64) -> String {
    let grammar = DenseGrammar::new(
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
    let analysis = AnalyzedContext::default();
    let ctx = Context::new().with_left_text("文脈");
    let mut cands = vec![cand(BAD, BAD_BASE), cand(GOOD, GOOD_BASE)];
    let input = RankInput {
        reading,
        context: &ctx,
        grammar: &grammar,
        analysis: &analysis,
        dict: &dict,
        deadline: Some(Instant::now() + ikari_testtime::ms(budget_ms)),
    };
    let t = Instant::now();
    r.rank(&input, &mut cands);
    assert!(
        t.elapsed() < ikari_testtime::ms(budget_ms * 5),
        "rank が待ちすぎ"
    );
    cands.sort_by_key(|c| c.score);
    cands[0].surface.clone()
}

// (c) 連続で落ちたら再起動をやめる。やめたあとは子を起動せず、即座に Err。
#[test]
fn gives_up_after_consecutive_failures() {
    let mut scorer = HostScorer::new(host_cfg("die-after=0"));
    for i in 1..=3 {
        assert!(scorer.nll("p", &[GOOD.to_owned()]).is_err());
        assert_eq!(scorer.consecutive_failures(), i);
    }
    assert!(scorer.gave_up());
    assert_eq!(scorer.spawns(), 3);
    let t = Instant::now();
    for _ in 0..5 {
        assert!(scorer.nll("p", &[GOOD.to_owned()]).is_err());
    }
    assert_eq!(scorer.spawns(), 3, "諦めたあとは起動しない");
    assert!(
        t.elapsed() < ikari_testtime::ms(50),
        "諦めたあとは即座に返る"
    );
}

// 起動すらできない子（READY 前に終了）も、同じ連続失敗の数え方で諦める。
#[test]
fn gives_up_when_child_never_becomes_ready() {
    let mut scorer = HostScorer::new(host_cfg("die-on-start"));
    for _ in 0..3 {
        assert!(scorer.nll("p", &[GOOD.to_owned()]).is_err());
    }
    assert!(scorer.gave_up());
    assert_eq!(scorer.spawns(), 3);
}

// 間に成功をはさめば、連続ではないので諦めない（カウントが 0 に戻る）。
#[test]
fn success_resets_the_failure_count() {
    let mut scorer = HostScorer::new(host_cfg("die-after=1"));
    // 起動 → 1 件成功 → 2 件目で死ぬ → 再起動 → 成功 → 死ぬ … の繰り返し
    for _ in 0..4 {
        let _ = scorer.nll("p", &[GOOD.to_owned()]);
        let _ = scorer.nll("p", &[GOOD.to_owned()]);
    }
    assert!(!scorer.gave_up(), "失敗は連続しないので諦めない");
}

// ranker の経路でも、諦めたあとは辞書だけの順位を返し続け、子を起動しない。
#[test]
fn ranker_survives_a_permanently_broken_child() {
    let scorer = HostScorer::new(host_cfg("die-after=0"));
    let r = ranker_with(Arc::new(Mutex::new(Some(scorer))));
    for i in 0..6 {
        assert_eq!(first_surface(&r, &format!("あ5{i}")), BAD);
        // ワーカーが 1 件ずつ処理し終えるのを待つ（要求は最新だけが残る仕組みのため）
        assert!(wait_until(|| r.stats().inference_errors > i));
    }
}
