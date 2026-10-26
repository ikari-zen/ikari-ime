//! kanakanji-edge-33m の実モデルのテスト（`#[ignore]`）。次のように手動で実行する。
//!
//! ```text
//! IKARI_EDGE_MODEL=data/raw/kanakanji-edge/kanakanji-edge-33m-Q8_0.gguf \
//!   cargo test -p ikari-lm --features metal --release --test real_model_edge -- --ignored --nocapture
//! ```
//!
//! `IKARI_EDGE_MODEL` が未設定なら、各テストは失敗する（`#[ignore]` を明示して流したときに黙って成功させない）。
//! 読み込み時の事実（終端トークン、制御トークン、BOS、tokenizer.ggml.pre）もここで確かめる。
#![cfg(feature = "llama")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ikari_lm::{LlamaScorer, LmScorer, ModelKind, PromptInput, build_prompt_for, tail_chars};

const MODEL_ENV: &str = "IKARI_EDGE_MODEL";
const CONTEXT_CHARS: usize = 40;
const CONTEXT_TAG_ID: i32 = 6000;
const INPUT_TAG_ID: i32 = 6001;
const OUTPUT_TAG_ID: i32 = 6002;
const END_TOKEN_ID: i32 = 3;
const VOCAB_SIZE: i32 = 6003;
/// 正解の候補の NLL（終端込み）の上限。終端トークンを取り違えると跳ね上がる
const MAX_CORRECT_NLL: f32 = 4.0;
/// 時間を測る繰り返し回数
const TIMING_RUNS: usize = 30;

fn model_path() -> PathBuf {
    let p = std::env::var_os(MODEL_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{MODEL_ENV} is not set (required by #[ignore] tests)"));
    assert!(p.exists(), "{MODEL_ENV}={} does not exist", p.display());
    p
}

fn prompt(context: &str, reading: &str) -> String {
    build_prompt_for(
        ModelKind::KanakanjiEdge,
        &PromptInput {
            left_context: tail_chars(context, CONTEXT_CHARS),
            reading,
            ..Default::default()
        },
    )
}

#[test]
#[ignore = "requires kanakanji-edge GGUF (IKARI_EDGE_MODEL)"]
fn model_facts_that_the_prompt_depends_on() {
    let path = model_path();
    let scorer = LlamaScorer::load(&path).expect("load model");
    let info = scorer.info();
    eprintln!("[edge] {info:?}");
    assert_eq!(info.n_vocab, VOCAB_SIZE);
    // 終端は語彙の `</s>`。メタデータの EOS も同じ id を指す（zenz は食い違う）
    assert_eq!(info.end_token, END_TOKEN_ID);
    assert_eq!(info.end_token_text, "</s>");
    assert_eq!(info.metadata_eos, END_TOKEN_ID);
    // 学習時の入力は EE02 から始まる。BOS を付けてはいけない
    assert!(!info.adds_bos);
    // tokenizer.ggml.pre の読み替えは要らない
    assert!(!info.pre_tokenizer_fallback);

    // 制御トークンは 1 トークンとして扱われる（parse_special=true）
    let ids: Vec<i32> = scorer
        .debug_tokens(&prompt("", "にほんご"))
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(ids.first(), Some(&CONTEXT_TAG_ID), "{ids:?}");
    assert_eq!(ids.get(1), Some(&INPUT_TAG_ID), "{ids:?}");
    assert_eq!(ids.last(), Some(&OUTPUT_TAG_ID), "{ids:?}");
    // ニホンゴ の 4 文字は 4 トークン
    assert_eq!(ids.len(), 3 + 4, "{ids:?}");
}

/// (左文脈, 読み, 正解, 他の候補)
const CASES: &[(&str, &str, &str, &[&str])] = &[
    ("", "にほんご", "日本語", &["二本後", "ニホンゴ"]),
    (
        "きっかけで、漫画の仕事",
        "のいらいがくるように",
        "の依頼が来るように",
        &["の以来が来るように", "の依頼が刳るように"],
    ),
    (
        "",
        "きょうはいいてんきですね",
        "今日はいい天気ですね",
        &["京は良い転機ですね"],
    ),
    (
        "原稿の誤字を",
        "こうせい",
        "校正",
        &["構成", "更生", "コウセイ"],
    ),
    (
        "システムの",
        "こうせい",
        "構成",
        &["校正", "更生", "コウセイ"],
    ),
    (
        "敵の攻撃を受けて兵士は",
        "ひんし",
        "瀕死",
        &["品詞", "貧死"],
    ),
    (
        "ユーザー辞書に単語を登録するときは",
        "ひんし",
        "品詞",
        &["瀕死", "貧死"],
    ),
    ("今日はここまでと", "しよう", "しよう", &["使用", "仕様"]),
    ("やっと春に", "なった", "なった", &["鳴った", "成った"]),
];

#[test]
#[ignore = "requires kanakanji-edge GGUF (IKARI_EDGE_MODEL)"]
fn scorer_prefers_the_correct_candidate() {
    let path = model_path();
    let mut scorer = LlamaScorer::load(&path).expect("load model");
    let mut correct = 0;
    for (ctx, reading, expected, others) in CASES {
        let mut cands = vec![(*expected).to_owned()];
        cands.extend(others.iter().map(|s| (*s).to_owned()));
        let nll = scorer.nll(&prompt(ctx, reading), &cands).expect("score");
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
        eprintln!("[edge] {ctx} / {reading}: {}", line.join(" "));
        assert!(nll.iter().all(|n| n.is_finite() && *n >= 0.0), "{nll:?}");
        assert!(
            nll[0] < MAX_CORRECT_NLL,
            "{ctx}/{reading}/{expected}: {}",
            nll[0]
        );
        if best == 0 {
            correct += 1;
        }
    }
    eprintln!("[edge] top1 {correct}/{}", CASES.len());
    assert!(correct * 4 >= CASES.len() * 3, "only {correct} correct");
}

#[test]
#[ignore = "requires kanakanji-edge GGUF (IKARI_EDGE_MODEL)"]
fn scores_are_independent_of_batch_composition() {
    let path = model_path();
    let mut scorer = LlamaScorer::load(&path).expect("load model");
    let p = prompt("原稿の誤字を", "こうせい");
    let alone = scorer.nll(&p, &["校正".to_owned()]).unwrap()[0];
    let together = scorer
        .nll(
            &p,
            &["構成".to_owned(), "校正".to_owned(), "更生".to_owned()],
        )
        .unwrap()[1];
    assert!((alone - together).abs() < 1e-2, "{alone} vs {together}");
}

/// 実際のプールに近い: 30 個の漢字かな混じりの候補（各 2〜6 文字）。`real_model.rs` と同じ候補
const POOL: [&str; 30] = [
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
];

#[test]
#[ignore = "requires kanakanji-edge GGUF (IKARI_EDGE_MODEL)"]
fn inference_time_for_a_typical_pool() {
    let path = model_path();
    let t = Instant::now();
    let mut scorer = LlamaScorer::load(&path).expect("load model");
    eprintln!("[edge] model load: {:?}", t.elapsed());
    let pool: Vec<String> = POOL.iter().map(|s| (*s).to_owned()).collect();
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
    eprintln!(
        "[edge] pool of {} candidates: first {first:?}, warm p50 {:?}, p99 {:?}, min {:?}",
        pool.len(),
        times[times.len() / 2],
        times[(times.len() * 99).div_ceil(100) - 1],
        times[0]
    );

    let mut small: Vec<Duration> = Vec::new();
    for _ in 0..TIMING_RUNS {
        let t = Instant::now();
        scorer.nll(&p, &pool[..9]).expect("run");
        small.push(t.elapsed());
    }
    small.sort();
    eprintln!(
        "[edge] chunk of 9 candidates: warm p50 {:?}, p99 {:?}",
        small[small.len() / 2],
        small[(small.len() * 99).div_ceil(100) - 1]
    );
}

#[test]
#[ignore = "requires kanakanji-edge GGUF (IKARI_EDGE_MODEL)"]
fn greedy_generation_reproduces_the_readme_examples() {
    let path = model_path();
    let mut scorer = LlamaScorer::load(&path).expect("load model");
    // README のとおり: ニホンゴ → 日本語、文脈つき、今日はいい天気ですね
    for (ctx, reading, expected) in [
        ("", "にほんご", "日本語"),
        (
            "きっかけで、漫画の仕事",
            "のいらいがくるように",
            "の依頼が来るように",
        ),
        ("", "きょうはいいてんきですね", "今日はいい天気ですね"),
    ] {
        let out = scorer
            .generate(&prompt(ctx, reading), 64)
            .expect("generate");
        assert_eq!(out, expected, "{ctx}/{reading}");
    }
}
