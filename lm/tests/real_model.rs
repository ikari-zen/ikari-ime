//! 実モデルのテスト（`#[ignore]`）。次のように手動で実行する。
//!
//! ```text
//! IKARI_ZENZ_MODEL=data/raw/zenz/zenz-v3.2-small-Q5_K_M.gguf \
//!   cargo test -p ikari-lm --features metal --release -- --ignored --nocapture
//! ```
//!
//! `IKARI_ZENZ_MODEL` が未設定なら、各テストは失敗する（`#[ignore]` を明示して流したときに黙って成功させない）。
//! 時間は標準エラーに出す。ADR 0002 §13.2 の「1回の推論時間」の実測に使う。
#![cfg(feature = "llama")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ikari_core::{
    AnalyzedContext, Candidate, CandidateOrigin, Context, DenseGrammar, InMemoryDictionary,
    RankInput, Ranker, UnknownParams,
};
use ikari_lm::{
    LlamaScorer, LmScorer, PromptInput, ZenzConfig, ZenzRanker, build_prompt, tail_chars,
};

const MODEL_ENV: &str = "IKARI_ZENZ_MODEL";
const CONTEXT_CHARS: usize = 40;
/// 時間を測る繰り返し回数（最初の1回は GPU のウォームアップなので別に出す）
const TIMING_RUNS: usize = 30;

fn model_path() -> PathBuf {
    let p = std::env::var_os(MODEL_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{MODEL_ENV} is not set (required by #[ignore] tests)"));
    assert!(p.exists(), "{MODEL_ENV}={} does not exist", p.display());
    p
}

fn prompt(context: &str, reading: &str) -> String {
    build_prompt(&PromptInput {
        left_context: tail_chars(context, CONTEXT_CHARS),
        reading,
        ..Default::default()
    })
}

/// (左文脈, 読み, 正解, 他の候補)
const CASES: &[(&str, &str, &str, &[&str])] = &[
    (
        "ユーザー辞書に単語を登録するときは",
        "ひんし",
        "品詞",
        &["瀕死", "貧死", "ヒンシ"],
    ),
    (
        "敵の攻撃を受けて兵士は",
        "ひんし",
        "瀕死",
        &["品詞", "貧死", "ヒンシ"],
    ),
    (
        "システムの",
        "こうせい",
        "構成",
        &["校正", "更生", "後生", "コウセイ"],
    ),
    (
        "原稿の誤字を",
        "こうせい",
        "校正",
        &["構成", "更生", "後生", "コウセイ"],
    ),
    (
        "質問への",
        "かいとう",
        "回答",
        &["解答", "解凍", "快刀", "カイトウ"],
    ),
    (
        "冷凍した肉を",
        "かいとう",
        "解凍",
        &["回答", "解答", "快刀", "カイトウ"],
    ),
    (
        "やっと春に",
        "なった",
        "なった",
        &["鳴った", "成った", "ナッタ"],
    ),
    (
        "今日はここまでと",
        "しよう",
        "しよう",
        &["仕様", "使用", "シヨウ"],
    ),
    ("外部APIの", "しよう", "仕様", &["使用", "しよう", "シヨウ"]),
];

fn to_strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
#[ignore = "requires a zenz GGUF model (IKARI_ZENZ_MODEL)"]
fn scorer_prefers_the_contextually_correct_candidate() {
    let path = model_path();
    let t = Instant::now();
    let mut scorer = LlamaScorer::load(&path).expect("load model");
    eprintln!("[lm] model load: {:?}", t.elapsed());

    let mut correct = 0;
    for (ctx, reading, expected, others) in CASES {
        let mut cands = vec![(*expected).to_owned()];
        cands.extend(to_strings(others));
        let nll = scorer.nll(&prompt(ctx, reading), &cands).expect("score");
        assert_eq!(nll.len(), cands.len());
        let best = nll
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap();
        let line: Vec<String> = cands
            .iter()
            .zip(&nll)
            .map(|(c, n)| format!("{c}={n:.2}"))
            .collect();
        eprintln!("[lm] {ctx} / {reading}: {}", line.join(" "));
        assert!(nll.iter().all(|n| n.is_finite() && *n >= 0.0), "{nll:?}");
        // 正解の候補は、終端（</s>）まで含めても数 nat 以内に収まる。終端のトークンを取り違えると
        // ここが 10 nat 以上に跳ね上がる
        assert!(nll[0] < 4.0, "{ctx}/{reading}/{expected}: {}", nll[0]);
        if best == 0 {
            correct += 1;
        }
    }
    eprintln!("[lm] top1 {correct}/{}", CASES.len());
    // zenz は文脈を使える。候補が4〜5個の中から、半数以上で正解を最小の NLL にすること
    // （small は 9/9、xsmall は 6/9 だった。2026-10-11、v3.2 Q5_K_M）
    assert!(correct * 2 >= CASES.len(), "only {correct} correct");
}

#[test]
#[ignore = "requires a zenz GGUF model (IKARI_ZENZ_MODEL)"]
fn scores_are_independent_of_batch_composition() {
    let path = model_path();
    let mut scorer = LlamaScorer::load(&path).expect("load model");
    let p = prompt("原稿の誤字を", "こうせい");
    let alone = scorer.nll(&p, &to_strings(&["校正"])).unwrap()[0];
    let together = scorer
        .nll(&p, &to_strings(&["構成", "校正", "更生"]))
        .unwrap()[1];
    assert!(
        (alone - together).abs() < 0.05,
        "alone={alone} together={together}"
    );
    // 評価できない候補（空）は無限大で、ほかの候補に影響しない
    let with_empty = scorer.nll(&p, &to_strings(&["", "校正"])).unwrap();
    assert!(with_empty[0].is_infinite());
    assert!((with_empty[1] - alone).abs() < 0.05);
}

#[test]
#[ignore = "requires a zenz GGUF model (IKARI_ZENZ_MODEL)"]
fn inference_time_for_a_typical_pool() {
    let path = model_path();
    let mut scorer = LlamaScorer::load(&path).expect("load model");
    // 実際のプールに近い: 30 個の漢字かな混じりの候補（各 2〜6 文字）
    let pool: Vec<String> = [
        "構成",
        "校正",
        "更生",
        "後生",
        "コウセイ",
        "こうせい",
        "公正",
        "厚生",
        "攻性",
        "恒星",
        "広聖",
        "港西",
        "行政",
        "高精",
        "好晴",
        "航星",
        "硬性",
        "溝清",
        "鋼製",
        "甲制",
        "香西",
        "抗性",
        "皇政",
        "考正",
        "紅星",
        "荒政",
        "稿成",
        "交星",
        "光生",
        "功成",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let p = prompt("原稿の誤字を", "こうせい");

    let t = Instant::now();
    let nll = scorer.nll(&p, &pool).expect("first run");
    let first = t.elapsed();
    assert_eq!(nll.len(), pool.len());

    let mut times: Vec<Duration> = Vec::new();
    for _ in 0..TIMING_RUNS {
        let t = Instant::now();
        scorer.nll(&p, &pool).expect("run");
        times.push(t.elapsed());
    }
    times.sort();
    let p50 = times[times.len() / 2];
    let p99 = times[(times.len() * 99).div_ceil(100) - 1];
    eprintln!(
        "[lm] pool of {} candidates: first {first:?}, warm p50 {p50:?}, p99 {p99:?}, min {:?}",
        pool.len(),
        times[0]
    );

    // 1候補だけ（先行計算が効いたあとの小さな塊に近い）
    let mut small: Vec<Duration> = Vec::new();
    for _ in 0..TIMING_RUNS {
        let t = Instant::now();
        scorer.nll(&p, &pool[..9]).expect("run");
        small.push(t.elapsed());
    }
    small.sort();
    eprintln!(
        "[lm] chunk of 9 candidates: warm p50 {:?}, p99 {:?}",
        small[small.len() / 2],
        small[(small.len() * 99).div_ceil(100) - 1]
    );
}

fn dict_cand(surface: &str, base: i32) -> Candidate {
    Candidate {
        surface: surface.to_owned(),
        segments: Vec::new(),
        base_cost: base,
        score: base,
        origin: CandidateOrigin::Dictionary,
    }
}

#[test]
#[ignore = "requires a zenz GGUF model (IKARI_ZENZ_MODEL)"]
fn zenz_ranker_end_to_end_reorders_homophones_within_budget() {
    let path = model_path();
    // 待ちの上限と締め切りを 30ms にそろえる（NFR-04。既定の上限は ADR 0004 の 300ms）
    let budget = Duration::from_millis(30);
    let cfg = ZenzConfig {
        budget,
        ..ZenzConfig::default()
    };
    let ranker = ZenzRanker::new(LlamaScorer::loader(&path), cfg);
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
    let ctx = Context::new().with_left_text("原稿の誤字を");
    let make = || {
        vec![
            dict_cand("構成", 5400),
            dict_cand("コウセイ", 5450),
            dict_cand("校正", 5500),
            dict_cand("更生", 5400),
        ]
    };
    let run = |cands: &mut Vec<Candidate>| -> Duration {
        let input = RankInput {
            reading: "こうせい",
            context: &ctx,
            grammar: &grammar,
            analysis: &analysis,
            dict: &dict,
            deadline: Some(Instant::now() + budget),
        };
        let t = Instant::now();
        ranker.rank(&input, cands);
        t.elapsed()
    };

    // 最初の要求はモデルの読み込みが入るので、時間切れになる（NFR-15）。結果は遅れて届く
    let mut first = make();
    let waited = run(&mut first);
    eprintln!("[lm] first rank() (cold, model not loaded): {waited:?}");
    assert!(waited < budget + Duration::from_millis(30), "{waited:?}");
    let deadline = Instant::now() + Duration::from_secs(30);
    while ranker.stats().completed == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(ranker.stats().completed, 1);
    eprintln!("[lm] stats after cold start: {:?}", ranker.stats());

    // 遅れて届いた結果を同じ変換で使う
    let mut second = make();
    run(&mut second);
    eprintln!(
        "[lm] scores: {:?}",
        second
            .iter()
            .map(|c| (&c.surface, c.score))
            .collect::<Vec<_>>()
    );
    // 文脈（誤字を）に合う「校正」への加算は、合わない「更生」「コウセイ」より小さい
    let delta = |s: &str| {
        second
            .iter()
            .find(|c| c.surface == s)
            .map(|c| c.score - c.base_cost)
    };
    assert!(delta("校正") < delta("更生"), "{second:?}");
    assert!(delta("校正") < delta("コウセイ"), "{second:?}");

    // ウォーム状態: 文脈を変えながら繰り返し、時間切れ率と待ち時間を測る
    let mut waits = Vec::new();
    for i in 0..TIMING_RUNS {
        let ctx_i = Context::new().with_left_text(format!("原稿の誤字を{i}"));
        let input = RankInput {
            reading: "こうせい",
            context: &ctx_i,
            grammar: &grammar,
            analysis: &analysis,
            dict: &dict,
            deadline: Some(Instant::now() + budget),
        };
        let mut c = make();
        let t = Instant::now();
        ranker.rank(&input, &mut c);
        waits.push(t.elapsed());
    }
    waits.sort();
    let stats = ranker.stats();
    eprintln!(
        "[lm] warm rank(): p50 {:?}, max {:?}; stats {stats:?}; timeout rate {:.1}%",
        waits[waits.len() / 2],
        waits[waits.len() - 1],
        stats.timeout_rate() * 100.0
    );
    assert!(*waits.last().unwrap() < budget + Duration::from_millis(30));
}

#[test]
#[ignore = "requires a zenz GGUF model (IKARI_ZENZ_MODEL)"]
fn model_is_released_after_idle_and_reloaded() {
    let path = model_path();
    let cfg = ZenzConfig {
        idle_unload: Duration::from_millis(500),
        budget: Duration::from_secs(10),
        ..ZenzConfig::default()
    };
    let ranker = ZenzRanker::new(LlamaScorer::loader(&path), cfg);
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
    let ctx = Context::new().with_left_text("システムの");
    let input = RankInput {
        reading: "こうせい",
        context: &ctx,
        grammar: &grammar,
        analysis: &analysis,
        dict: &dict,
        deadline: Some(Instant::now() + Duration::from_secs(10)),
    };
    let mut c = vec![dict_cand("構成", 0), dict_cand("校正", 0)];
    ranker.rank(&input, &mut c);
    assert!(ranker.stats().loaded);

    let deadline = Instant::now() + Duration::from_secs(10);
    while ranker.stats().loaded && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!ranker.stats().loaded, "model was not unloaded");
    assert_eq!(ranker.stats().unloads, 1);

    let ctx2 = Context::new().with_left_text("システムの設定");
    let input2 = RankInput {
        context: &ctx2,
        deadline: Some(Instant::now() + Duration::from_secs(10)),
        ..input
    };
    let mut c = vec![dict_cand("構成", 0), dict_cand("校正", 0)];
    ranker.rank(&input2, &mut c);
    assert_eq!(ranker.stats().loads, 2);
}

#[test]
#[ignore = "requires a zenz GGUF model (IKARI_ZENZ_MODEL)"]
fn tokenizer_is_character_level() {
    let path = model_path();
    let scorer = LlamaScorer::load(&path).expect("load model");
    // 漢字・カタカナ・かなは1文字が1トークン
    assert_eq!(scorer.debug_tokens("構成").len(), 2);
    assert_eq!(scorer.debug_tokens("ヒンシ").len(), 3);
    // タグ（私用領域の文字）は UTF-8 の3バイトに分かれる。学習時も同じ扱い
    assert_eq!(scorer.debug_tokens("\u{EE00}").len(), 3);
}

#[test]
#[ignore = "requires a zenz GGUF model (IKARI_ZENZ_MODEL)"]
fn warm_up_makes_the_first_rank_meet_the_deadline() {
    let path = model_path();
    let budget = Duration::from_millis(30);
    let cfg = ZenzConfig {
        budget,
        ..ZenzConfig::default()
    };
    let ranker = ZenzRanker::new(LlamaScorer::loader(&path), cfg);
    let t = Instant::now();
    ranker.warm_up();
    eprintln!("[lm] warm_up(): {:?}", t.elapsed());
    assert!(ranker.stats().loaded);

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
    // 読み込み直後の最初の変換から、締め切り（30ms）の中で LM の結果が使われる
    let mut waits = Vec::new();
    for i in 0..10 {
        let ctx = Context::new().with_left_text(format!("原稿の誤字を{i}"));
        let input = RankInput {
            reading: "こうせい",
            context: &ctx,
            grammar: &grammar,
            analysis: &analysis,
            dict: &dict,
            deadline: Some(Instant::now() + budget),
        };
        let mut c = vec![dict_cand("構成", 5400), dict_cand("校正", 5500)];
        let t = Instant::now();
        ranker.rank(&input, &mut c);
        waits.push(t.elapsed());
        assert!(
            c[0].score != 5400 || c[1].score != 5500,
            "LM was not applied at run {i}: {c:?}"
        );
    }
    let stats = ranker.stats();
    eprintln!("[lm] first-ranks after warm_up: {waits:?}; stats {stats:?}");
    assert_eq!(stats.timeouts, 0, "{stats:?}");
    assert!(
        waits.iter().all(|w| *w < budget + Duration::from_millis(5)),
        "{waits:?}"
    );
}
