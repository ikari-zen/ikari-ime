//! 実辞書での文節の不変条件と性能（ADR 0006 §7.1-4、§10.1）。`#[ignore]` なので `--ignored` で明示したときだけ動き、
//! `IKARI_DICT_DIR` がなければ失敗する（黙って成功しない）。
//!
//! `IKARI_DICT_DIR=data/build/dict cargo test --release -p ikari-core --test segment_real -- --ignored --nocapture`
//!
//! Ranker は LM なしの `full`（左文脈、かなの表記、文書内キャッシュ、学習（空）、プロファイル、話題）。
//! 評価セットは読むだけ（`eval/sets/*.tsv`）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ikari_core::{
    CalculatorRewriter, CompositeRanker, Context, Converter, DateRewriter, Dictionary,
    DocumentCacheConfig, DocumentCacheRanker, EmoticonRewriter, Grammar, KanaStyleConfig,
    KanaStyleRanker, KanaStyleTable, LatticeContextAnalyzer, LearningConfig, LearningRanker,
    LearningStore, LeftContextConfig, LeftContextRanker, NumberRewriter, ProfileConfig,
    ProfileRanker, Ranker, Rewriter, SegmentChoice, SymbolRewriter, TopicConfig, TopicRanker,
    VECTORS_FILE_NAME, open_dir, open_vectors,
};
use ikari_dict_format::reading::normalize_key;

const DICT_DIR_ENV: &str = "IKARI_DICT_DIR";
const SET_FILES: [&str; 4] = [
    "homophones.tsv",
    "homophones-ext.tsv",
    "hiragana.tsv",
    "katakana-suffix.tsv",
];
/// 性能を測る短い読みの上限（文字数）
const SHORT_READING_CHARS: usize = 10;
/// 性能の基準（エンジン内 p99、LM なし。ADR 0006 §10.1）
const P99_LIMIT: Duration = Duration::from_millis(2);
const REPEAT: usize = 3;
/// 評価セットに少ない、複数の文節になる短い読み（文節の 1 ページ目の測定に足す）
const PHRASES: [&str; 12] = [
    "ひんしをえらぶ",
    "てんきがいい",
    "はなしをきく",
    "しごとをする",
    "かんじにへんかん",
    "ほんをよんだ",
    "あしたかいぎ",
    "でんしゃにのる",
    "あめがふっている",
    "かいしゃにいく",
    "はじめてしった",
    "りぽじとりめい",
];

fn dict_dir() -> PathBuf {
    std::env::var_os(DICT_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            panic!("{DICT_DIR_ENV} is required (a real dictionary built by ikari-dict-builder)")
        })
}

struct Case {
    context: String,
    reading: String,
    expected: String,
}

fn load_cases() -> Vec<Case> {
    let sets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../eval/sets");
    let mut cases = Vec::new();
    for name in SET_FILES {
        let text = std::fs::read_to_string(sets.join(name)).unwrap();
        let mut columns: Option<BTreeMap<String, usize>> = None;
        for line in text.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            let Some(index) = &columns else {
                columns = Some(
                    fields
                        .iter()
                        .enumerate()
                        .map(|(i, f)| ((*f).to_owned(), i))
                        .collect(),
                );
                continue;
            };
            let get = |key: &str| {
                index
                    .get(key)
                    .and_then(|&i| fields.get(i))
                    .copied()
                    .unwrap_or("")
            };
            cases.push(Case {
                context: get("context").to_owned(),
                reading: get("reading").to_owned(),
                expected: get("expected").to_owned(),
            });
        }
    }
    assert!(
        cases.len() > 1000,
        "evaluation sets not found ({})",
        cases.len()
    );
    cases
}

fn converter() -> (Converter, Arc<dyn Dictionary>) {
    let dir = dict_dir();
    let loaded = open_dir(&dir).unwrap();
    let dict_id = loaded.dict.dict_id().unwrap();
    let vectors = open_vectors(&dir.join(VECTORS_FILE_NAME), dict_id).unwrap();
    let dict: Arc<dyn Dictionary> = loaded.dict;
    let grammar: Arc<dyn Grammar> = loaded.grammar;
    // 学習は空（ファイルは保存しない。実機のユーザーデータには触れない）
    let store =
        LearningStore::open(&std::env::temp_dir().join("ikari-segment-real-learning.tsv")).unwrap();
    store.attach_grammar(Arc::clone(&grammar));
    let kv = BTreeMap::new();
    let rankers: Vec<Arc<dyn Ranker>> = vec![
        Arc::new(LeftContextRanker::new(
            LeftContextConfig::from_kv(&kv).unwrap(),
        )),
        Arc::new(KanaStyleRanker::new(
            KanaStyleTable::builtin(),
            KanaStyleConfig::from_kv(&kv).unwrap(),
        )),
        Arc::new(DocumentCacheRanker::new(
            DocumentCacheConfig::from_kv(&kv).unwrap(),
        )),
        Arc::new(LearningRanker::new(
            Arc::new(store),
            Some(Arc::clone(&vectors)),
            LearningConfig::from_kv(&kv).unwrap(),
        )),
        Arc::new(ProfileRanker::new(
            Some(Arc::clone(&vectors)),
            ProfileConfig::from_kv(&kv).unwrap(),
        )),
        Arc::new(TopicRanker::new(
            Arc::clone(&vectors),
            TopicConfig::from_kv(&kv).unwrap(),
        )),
    ];
    let rewriters: Vec<Arc<dyn Rewriter>> = vec![
        Arc::new(DateRewriter::new()),
        Arc::new(NumberRewriter::new()),
        Arc::new(CalculatorRewriter::new()),
        Arc::new(SymbolRewriter::builtin()),
        Arc::new(EmoticonRewriter::builtin()),
    ];
    let converter = Converter::builder(Arc::clone(&dict), Arc::clone(&grammar))
        .ranker(Arc::new(CompositeRanker::new(rankers)))
        .analyzer(Arc::new(LatticeContextAnalyzer::new(
            Arc::clone(&dict),
            grammar,
        )))
        .rewriters(rewriters)
        .build()
        .unwrap();
    (converter, dict)
}

/// 不変条件（ADR 0006 §7.1-4）: 区切りの表示をつないだものが、今の全体の 1 位と一致する。
/// 1 文節なら候補の列は全体の変換そのもの。複数でも 0 番は表示中の表記で、読みは文節の読みと一致する
#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary built by ikari-dict-builder)"]
fn real_dictionary_segmentation_keeps_the_first_view() {
    let (conv, _) = converter();
    let cases = load_cases();
    let (mut single, mut multi, mut clauses) = (0, 0, 0);
    for case in &cases {
        let ctx = Context::new().with_left_text(case.context.clone());
        let reading = normalize_key(&case.reading);
        let mut whole = conv.convert_until(&case.reading, &ctx, None);
        let top = whole
            .page_until(0, None)
            .map(|p| p.candidates[0].surface.clone());
        let mut plain = conv.convert_until(&case.reading, &ctx, None);
        let mut seg = conv.segment(whole);
        let spans = seg.layout().spans.clone();
        let joined: String = spans.iter().map(|s| s.surface.as_str()).collect();
        let joined_reading: String = spans.iter().map(|s| s.reading.as_str()).collect();
        assert_eq!(
            top.as_deref().unwrap_or(""),
            joined,
            "first view changed: {} / {}",
            case.reading,
            case.context
        );
        assert_eq!(joined_reading, reading, "{}", case.reading);
        let zeros = vec![SegmentChoice::Candidate(0); spans.len()];
        assert_eq!(seg.commit(&zeros).unwrap().surface, joined);
        if spans.len() <= 1 {
            single += 1;
            // 1 文節: 読み全体の変換の列と同一
            for page in 0..2 {
                let a: Option<Vec<String>> = plain
                    .page_until(page, None)
                    .map(|p| p.candidates.iter().map(|c| c.surface.clone()).collect());
                let b: Option<Vec<String>> = seg
                    .page_until(0, page, &[], None)
                    .unwrap()
                    .map(|p| p.candidates.iter().map(|c| c.surface.clone()).collect());
                assert_eq!(a, b, "{} page {page}", case.reading);
            }
            continue;
        }
        multi += 1;
        for (i, span) in spans.iter().enumerate() {
            clauses += 1;
            let page = seg.page_until(i, 0, &[], None).unwrap().unwrap();
            assert_eq!(
                page.candidates[0].surface, span.surface,
                "NFR-05: {} clause {i}",
                case.reading
            );
            let mut seen = std::collections::HashSet::new();
            for cand in page.candidates {
                let candidate_reading: String =
                    cand.segments.iter().map(|s| s.reading.as_str()).collect();
                assert_eq!(candidate_reading, span.reading, "FR-06: {}", case.reading);
                assert!(
                    seen.insert(cand.surface.clone()),
                    "duplicate {}",
                    cand.surface
                );
            }
        }
    }
    eprintln!(
        "cases {}: single clause {single}, multi {multi} ({clauses} clauses)",
        cases.len()
    );
}

/// 区切り直し: 読みの総和が変わらず、位置がつながり、先頭の候補が表示中の表記になる
#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary built by ikari-dict-builder)"]
fn real_dictionary_resize_keeps_the_reading_and_the_view_of_the_left() {
    let (conv, _) = converter();
    let cases = load_cases();
    let mut resized = 0;
    for case in cases.iter().filter(|c| c.reading.chars().count() <= 20) {
        let ctx = Context::new().with_left_text(case.context.clone());
        let reading = normalize_key(&case.reading);
        let mut seg = conv.segment(conv.convert_until(&case.reading, &ctx, None));
        for (index, delta) in [(0usize, -1i32), (0, 1), (0, -1), (0, -1)] {
            let before = seg.layout().clone();
            if index >= before.spans.len() {
                continue;
            }
            conv.resize(&mut seg, index, delta, &[], None).unwrap();
            let after = seg.layout();
            let joined: String = after.spans.iter().map(|s| s.reading.as_str()).collect();
            assert_eq!(joined, reading, "{}", case.reading);
            for pair in after.spans.windows(2) {
                assert_eq!(pair[0].end, pair[1].start);
            }
            if after.rev != before.rev {
                resized += 1;
                let moved = after.spans[index].end - after.spans[index].start;
                let was = before.spans[index].end - before.spans[index].start;
                assert_eq!(
                    i64::try_from(moved).unwrap() - i64::try_from(was).unwrap(),
                    i64::from(delta)
                );
                assert_eq!(after.spans[..index], before.spans[..index]);
                let displayed = seg.layout().spans[index].surface.clone();
                let page = seg.page_until(index, 0, &[], None).unwrap().unwrap();
                assert_eq!(page.candidates[0].surface, displayed);
            }
        }
    }
    eprintln!("resized {resized} times");
    assert!(resized > 500);
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let at = ((sorted.len() as f64 * p).ceil() as usize).clamp(1, sorted.len()) - 1;
    sorted[at]
}

fn report(name: &str, mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    let p50 = percentile(&samples, 0.50);
    let p99 = percentile(&samples, 0.99);
    let max = *samples.last().unwrap();
    eprintln!(
        "{name}: n={} p50={:.3}ms p99={:.3}ms max={:.3}ms",
        samples.len(),
        p50.as_secs_f64() * 1000.0,
        p99.as_secs_f64() * 1000.0,
        max.as_secs_f64() * 1000.0
    );
    p99
}

/// 性能（ADR 0006 §10.1）: 短い読みで、文節の 1 ページ目、区切り直し、再変換のエンジン内 p99 ≤ 2ms（LM なし）
#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary built by ikari-dict-builder)"]
fn real_dictionary_clause_operations_are_fast() {
    let (conv, dict) = converter();
    let mut cases: Vec<Case> = load_cases()
        .into_iter()
        .filter(|c| c.reading.chars().count() <= SHORT_READING_CHARS)
        .collect();
    assert!(cases.len() > 300);
    cases.extend(PHRASES.iter().map(|p| Case {
        context: String::new(),
        reading: (*p).to_owned(),
        expected: (*p).to_owned(),
    }));
    // 逆引き表は起動のあとに裏で作る想定。測定に含めない
    let started = Instant::now();
    dict.warm_reading_index();
    eprintln!("reading index: {:?}", started.elapsed());

    let mut segment_time = Vec::new();
    let mut first_page = Vec::new();
    let mut later_page = Vec::new();
    let mut resize_time = Vec::new();
    let mut resize_page = Vec::new();
    let mut reconvert_time = Vec::new();
    let mut reconvert_page = Vec::new();
    for round in 0..=REPEAT {
        for case in &cases {
            let ctx = Context::new().with_left_text(case.context.clone());
            let whole = conv.convert_until(&case.reading, &ctx, None);
            let t = Instant::now();
            let mut seg = conv.segment(whole);
            let t_segment = t.elapsed();
            let clauses = seg.layout().spans.len();
            // 文節の 1 ページ目（フォーカスを動かして初めて開く文節）。1 文節は全体の変換の列なので測らない
            let mut t_pages = Vec::new();
            for i in (0..clauses).filter(|_| clauses > 1) {
                let t = Instant::now();
                let page = seg.page_until(i, 0, &[], None).unwrap();
                t_pages.push((i, t.elapsed()));
                assert!(page.is_some());
            }
            // 区切り直し（縮める）と、フォーカスの文節の 1 ページ目
            let t = Instant::now();
            conv.resize(&mut seg, 0, -1, &[], None).unwrap();
            let t_resize = t.elapsed();
            let t = Instant::now();
            let _ = seg.page_until(0, 0, &[], None).unwrap();
            let t_resize_page = t.elapsed();
            // 再変換: 正解の表記を選択したものとして
            let t = Instant::now();
            let reconverted = conv.reconvert(&case.expected, &ctx, &[], None);
            let t_reconvert = t.elapsed();
            let mut t_reconvert_page = Duration::ZERO;
            if let Ok(mut re) = reconverted {
                let t = Instant::now();
                let _ = re.page_until(0, 0, &[], None).unwrap();
                t_reconvert_page = t.elapsed();
            }
            if round == 0 {
                continue; // 最初の 1 周は温め
            }
            segment_time.push(t_segment);
            for (i, d) in t_pages {
                if i == 0 {
                    first_page.push(d);
                } else {
                    later_page.push(d);
                }
            }
            resize_time.push(t_resize);
            resize_page.push(t_resize_page);
            reconvert_time.push(t_reconvert);
            reconvert_page.push(t_reconvert_page);
        }
    }
    let worst = [
        report("segment()", segment_time),
        report("clause page 0 (first of 2+ clauses)", first_page),
        report("clause page 0 (later of 2+ clauses)", later_page),
        report("resize", resize_time),
        report("page after resize", resize_page),
        report("reconvert", reconvert_time),
        report("page after reconvert", reconvert_page),
    ];
    for p99 in worst {
        assert!(p99 <= P99_LIMIT, "p99 {p99:?} exceeds {P99_LIMIT:?}");
    }
}

/// 再変換: 最初の表示は選択した表記のまま。読みの推定の一致率を出す（基準は決めていない。ADR 0006 §16 の 12）
#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary built by ikari-dict-builder)"]
fn real_dictionary_reconvert_keeps_the_surface_and_reports_reading_accuracy() {
    let (conv, dict) = converter();
    dict.warm_reading_index();
    let cases = load_cases();
    let (mut total, mut exact, mut with_literal, mut refused) = (0, 0, 0, 0);
    for case in &cases {
        total += 1;
        let ctx = Context::new().with_left_text(case.context.clone());
        let Ok(mut seg) = conv.reconvert(&case.expected, &ctx, &[], None) else {
            refused += 1;
            continue;
        };
        let joined: String = seg
            .layout()
            .spans
            .iter()
            .map(|s| s.surface.as_str())
            .collect();
        assert_eq!(
            joined, case.expected,
            "the first view must be the selection"
        );
        let reading: String = seg
            .layout()
            .spans
            .iter()
            .map(|s| s.reading.as_str())
            .collect();
        let literal = seg.layout().spans.iter().any(|s| s.reading.is_empty());
        if literal {
            with_literal += 1;
        } else if reading == normalize_key(&case.reading) {
            exact += 1;
        }
        let count = seg.layout().spans.len();
        for i in 0..count {
            let span = seg.layout().spans[i].clone();
            let page = seg.page_until(i, 0, &[], None).unwrap().unwrap();
            assert_eq!(page.candidates[0].surface, span.surface);
            for cand in page.candidates {
                let r: String = cand.segments.iter().map(|s| s.reading.as_str()).collect();
                assert_eq!(r, span.reading);
            }
        }
        let zeros = vec![SegmentChoice::Candidate(0); count];
        let commit = seg.commit(&zeros).unwrap();
        assert_eq!(commit.surface, case.expected);
        assert!(commit.events.is_empty() && commit.history.is_empty());
    }
    eprintln!(
        "reconvert: {total} cases, reading exact {exact} ({:.1}%), with literal {with_literal}, refused {refused}",
        exact as f64 * 100.0 / total as f64
    );
    // 漢字だけの複合語（関数名、飛行機、効果的）も辞書の語に分けて読める（`analyze_text` の未知語のコスト）
    assert_eq!(refused, 0, "every selection in the sets must be readable");
}

/// 助詞のあとの用言は別の文節になる。Sudachi は「切る」「行く」「ある」「いい」を 動詞/形容詞,非自立可能 にしている
/// ので、品詞だけで補助用言として左に付けると「髪を切る」が 1 文節になっていた。全体の 1 位（表示）は変わらない
#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary built by ikari-dict-builder)"]
fn real_dictionary_verb_after_a_particle_starts_a_clause() {
    const EXPECTED: [(&str, &[&str]); 9] = [
        ("かみをきる", &["かみを", "きる"]),
        ("こうえんにいく", &["こうえんに", "いく"]),
        ("かんしんがある", &["かんしんが", "ある"]),
        ("てんきがいい", &["てんきが", "いい"]),
        ("ろくにんでいく", &["ろくにんで", "いく"]),
        (
            "ほんとうはいきたくなかった",
            &["ほんとうは", "いきたくなかった"],
        ),
        // 補助の用法は 1 文節のまま
        ("あめがふっている", &["あめが", "ふっている"]),
        ("たべてみる", &["たべてみる"]),
        ("へんかんする", &["へんかんする"]),
    ];
    let (conv, _) = converter();
    for (reading, readings) in EXPECTED {
        let seg = conv.segment(conv.convert_until(reading, &Context::new(), None));
        let got: Vec<&str> = seg
            .layout()
            .spans
            .iter()
            .map(|s| s.reading.as_str())
            .collect();
        assert_eq!(got, readings, "{reading}");
    }
}
