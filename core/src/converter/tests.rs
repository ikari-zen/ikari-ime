//! `Converter` と `Conversion` の単体テスト。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::analysis::{AnalyzedContext, ContextAnalyzer};
use crate::candidate::{Candidate, CandidateOrigin};
use crate::dict::{Dictionary, Grammar};
use crate::ranker::{RankInput, Ranker};
use crate::rewriter::Rewriter;

#[test]
fn lm_budget_is_base_per_char_and_capped() {
    let b = LmBudget::default();
    assert_eq!(b.for_reading(""), Duration::from_millis(30));
    assert_eq!(b.for_reading("きょう"), Duration::from_millis(30));
    assert_eq!(b.for_reading("ひんしをえらぶ"), Duration::from_millis(42));
    assert_eq!(b.for_reading(&"あ".repeat(500)), Duration::from_millis(300));
}

use crate::dict::{DenseGrammar, InMemoryDictionary};
use crate::lattice::testutil::*;
use crate::ranker::CostRanker;

fn conv_with(
    g: DenseGrammar,
    d: InMemoryDictionary,
    ranker: Arc<dyn Ranker>,
    opts: ConvertOptions,
) -> Converter {
    Converter::new(Arc::new(d), Arc::new(g), ranker, opts).unwrap()
}

struct DeadlineRecorder(Mutex<Vec<Option<Instant>>>);
impl Ranker for DeadlineRecorder {
    fn rank(&self, input: &RankInput<'_>, _: &mut [Candidate]) {
        self.0.lock().unwrap().push(input.deadline);
    }
    fn name(&self) -> &str {
        "deadline-recorder"
    }
}

#[test]
fn deadline_reaches_every_rank_call_of_one_invocation() {
    let rec = Arc::new(DeadlineRecorder(Mutex::new(Vec::new())));
    let opts = ConvertOptions {
        pool_size: 2,
        chunk_size: 1,
        ..ConvertOptions::default()
    };
    let conv = henkan_converter(rec.clone(), opts);
    let d1 = Instant::now() + Duration::from_secs(1);
    let mut c = conv.convert_until("へんかん", &Context::new(), Some(d1));
    assert_eq!(*rec.0.lock().unwrap(), vec![Some(d1)]);
    let d2 = Instant::now() + Duration::from_secs(2);
    rec.0.lock().unwrap().clear();
    let _ = c.page_until(1, Some(d2));
    let seen = rec.0.lock().unwrap().clone();
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|d| *d == Some(d2)));
    rec.0.lock().unwrap().clear();
    let _ = conv.convert_until("へんかん", &Context::new(), None);
    assert_eq!(*rec.0.lock().unwrap(), vec![None]);
    rec.0.lock().unwrap().clear();
    let _ = conv.convert("へんかん", &Context::new());
    assert!(rec.0.lock().unwrap()[0].is_some());
}

/// へんか／へんかん／する の小さな辞書
fn henkan_converter(ranker: Arc<dyn Ranker>, opts: ConvertOptions) -> Converter {
    let mut g = grammar();
    g.set(ID_NOUN, ID_VERB_SURU, 50);
    g.set(ID_NOUN, ID_NOUN, 800);
    let mut d = InMemoryDictionary::new(g.grammar_id());
    d.insert("へんかん", "変換", ID_NOUN, ID_NOUN, 1000, 0);
    d.insert("へんか", "変化", ID_NOUN, ID_NOUN, 1100, 0);
    d.insert("へん", "変", ID_NOUN, ID_NOUN, 2000, 0);
    d.insert("か", "課", ID_NOUN, ID_NOUN, 2500, 0);
    d.insert("する", "する", ID_VERB_SURU, ID_VERB_SURU, 500, 1);
    conv_with(g, d, ranker, opts)
}

fn all_surfaces(conv: &mut Conversion) -> Vec<String> {
    let mut i = 0;
    while conv.page(i).is_some() {
        i += 1;
    }
    conv.generated().iter().map(|c| c.surface.clone()).collect()
}

fn pool_of(pool_size: usize) -> ConvertOptions {
    ConvertOptions {
        pool_size,
        ..ConvertOptions::default()
    }
}

fn cost_ranker() -> Arc<dyn Ranker> {
    Arc::new(CostRanker)
}

#[test]
fn fr06_reading_must_match_exactly() {
    let c = henkan_converter(cost_ranker(), ConvertOptions::default());
    let mut conv = c.convert("へんか", &Context::new());
    let s = all_surfaces(&mut conv);
    assert!(s.contains(&"変化".to_owned()));
    assert!(!s.contains(&"変換".to_owned()), "{s:?}");
    // 変+課 のような分割は出てよい（読み全体を覆っている）
    assert!(s.contains(&"変課".to_owned()));
}

#[test]
fn fr06_split_into_multiple_words_is_allowed() {
    // 試験用の 変換 は 名詞,一般 なので、規則 S（ADR 0016）を止めて基本の分割だけを確かめる
    let opts = ConvertOptions {
        noun_suru: NounSuruRule::disabled(),
        ..ConvertOptions::default()
    };
    let c = henkan_converter(cost_ranker(), opts);
    let mut conv = c.convert("へんかんする", &Context::new());
    let page = conv.page(0).unwrap();
    let top = &page.candidates[0];
    assert_eq!(top.surface, "変換する");
    assert_eq!(top.origin, CandidateOrigin::Dictionary);
    let segs: Vec<(&str, &str)> = top
        .segments
        .iter()
        .map(|s| (s.reading.as_str(), s.surface.as_str()))
        .collect();
    assert_eq!(segs, [("へんかん", "変換"), ("する", "する")]);
    assert!(top.segments.iter().all(|s| s.word.is_some()));
    assert_eq!(top.base_cost, 1000 + 50 + 500);
    assert_eq!(top.score, top.base_cost);
}

#[test]
fn candidates_are_unique_and_cover_whole_reading() {
    let c = henkan_converter(cost_ranker(), ConvertOptions::default());
    let mut conv = c.convert("へんかんする", &Context::new());
    let s = all_surfaces(&mut conv);
    let set: HashSet<_> = s.iter().collect();
    assert_eq!(set.len(), s.len(), "duplicates: {s:?}");
    for cand in conv.generated() {
        let joined: String = cand.segments.iter().map(|x| x.reading.as_str()).collect();
        assert_eq!(joined, "へんかんする");
    }
}

#[test]
fn hiragana_and_katasingle_segment_candidates_follow_pool() {
    // プールを小さくして、読み全体が未知語のパス（表記が同じ）がまだ出ていない状態にする
    let c = henkan_converter(cost_ranker(), pool_of(2));
    let mut conv = c.convert("へんかん", &Context::new());
    // ひらがな／カタカナの base_cost は最初のプール内の最大値 + 1
    let pool = conv.generated().to_vec();
    all_surfaces(&mut conv);
    let gen_ = conv.generated().to_vec();
    let by_origin = |o: CandidateOrigin| gen_.iter().find(|c| c.origin == o);
    let h = by_origin(CandidateOrigin::Hiragana).expect("hiragana");
    let k = by_origin(CandidateOrigin::Katakana).expect("katakana");
    assert_eq!(h.surface, "へんかん");
    assert_eq!(k.surface, "ヘンカン");
    let max_dict = pool
        .iter()
        .filter(|c| c.origin == CandidateOrigin::Dictionary)
        .map(|c| c.base_cost)
        .max()
        .unwrap();
    assert_eq!(h.base_cost, max_dict + 1);
    assert_eq!(k.base_cost, max_dict + 1);
}

#[test]
fn katakana_input_is_normalized() {
    let c = henkan_converter(cost_ranker(), ConvertOptions::default());
    let conv = c.convert("ヘンカン", &Context::new());
    assert_eq!(conv.reading(), "へんかん");
    assert_eq!(conv.generated()[0].surface, "変換");
}

#[test]
fn unknown_characters_still_yield_candidates() {
    let g = grammar();
    let d = InMemoryDictionary::new(g.grammar_id());
    let c = conv_with(g, d, cost_ranker(), ConvertOptions::default());
    for reading in ["ぷ←あ", "xyz", "ぷ"] {
        let mut conv = c.convert(reading, &Context::new());
        let page = conv.page(0).expect(reading);
        assert!(!page.candidates.is_empty());
        assert!(
            page.candidates.iter().any(|x| x.surface == reading),
            "{reading}: {:?}",
            page.candidates
                .iter()
                .map(|x| &x.surface)
                .collect::<Vec<_>>()
        );
    }
    let mut conv = c.convert("ぷ←", &Context::new());
    let s = all_surfaces(&mut conv);
    assert!(s.contains(&"プ←".to_owned()), "{s:?}");
}

#[test]
fn empty_reading_yields_no_candidates() {
    let c = henkan_converter(cost_ranker(), ConvertOptions::default());
    let mut conv = c.convert("", &Context::new());
    assert!(conv.page(0).is_none());
    assert!(conv.generated().is_empty());
    assert_eq!(conv.reading(), "");
}

/// 同じ読み「あ」に12件の同音語を持つ。ページングのテスト用
fn many_homophones(ranker: Arc<dyn Ranker>, opts: ConvertOptions) -> Converter {
    let g = grammar();
    let mut d = InMemoryDictionary::new(g.grammar_id());
    for i in 0..12i16 {
        d.insert(
            "あ",
            &format!("w{i:02}"),
            ID_NOUN,
            ID_NOUN,
            100 * (i + 1),
            0,
        );
    }
    conv_with(g, d, ranker, opts)
}

fn small_opts() -> ConvertOptions {
    ConvertOptions {
        page_size: 4,
        pool_size: 3,
        chunk_size: 2,
        max_astar_pops: 20_000,
        ..ConvertOptions::default()
    }
}

#[test]
fn pages_respect_size_and_has_next() {
    let c = many_homophones(cost_ranker(), small_opts());
    let mut conv = c.convert("あ", &Context::new());
    let mut sizes = Vec::new();
    let mut i = 0;
    while let Some(p) = conv.page(i) {
        assert_eq!(p.index, i);
        sizes.push((p.candidates.len(), p.has_next));
        i += 1;
    }
    // 12語 + ひらがな「あ」(未知語経路と同じ表記) + カタカナ「ア」 = 14件、4件ずつ
    assert_eq!(conv.generated().len(), 14);
    assert_eq!(sizes, [(4, true), (4, true), (4, true), (2, false)]);
    assert!(conv.page(4).is_none());
    assert!(conv.page(usize::MAX).is_none());
}

#[test]
fn displayed_pages_never_change() {
    // 後の塊ほど score を小さくする Ranker でも、確定済みの順位は動かない
    struct Reverser;
    impl Ranker for Reverser {
        fn rank(&self, _: &RankInput<'_>, cands: &mut [Candidate]) {
            for c in cands {
                c.score = -c.base_cost;
            }
        }
        fn name(&self) -> &str {
            "reverser"
        }
    }
    let c = many_homophones(Arc::new(Reverser), small_opts());
    let mut conv = c.convert("あ", &Context::new());
    let first: Vec<String> = conv
        .page(0)
        .unwrap()
        .candidates
        .iter()
        .map(|c| c.surface.clone())
        .collect();
    let snapshot_after_p0 = conv.generated().to_vec();
    let second: Vec<String> = conv
        .page(1)
        .unwrap()
        .candidates
        .iter()
        .map(|c| c.surface.clone())
        .collect();
    conv.page(2).unwrap();
    conv.page(3).unwrap();
    let again: Vec<String> = conv
        .page(0)
        .unwrap()
        .candidates
        .iter()
        .map(|c| c.surface.clone())
        .collect();
    assert_eq!(first, again);
    // 先に生成された部分は、後からの追加で並びも内容も変わらない
    for (a, b) in snapshot_after_p0.iter().zip(conv.generated()) {
        assert_eq!(a.surface, b.surface);
        assert_eq!(a.score, b.score);
    }
    assert_eq!(
        second
            .iter()
            .collect::<HashSet<_>>()
            .intersection(&first.iter().collect())
            .count(),
        0
    );
}

#[test]
fn pages_are_generated_lazily() {
    let c = many_homophones(cost_ranker(), small_opts());
    let mut conv = c.convert("あ", &Context::new());
    // プール: 辞書3件 + ひらがな + カタカナ
    assert_eq!(conv.generated().len(), 5);
    conv.page(0).unwrap();
    assert_eq!(conv.generated().len(), 5);
    conv.page(1).unwrap();
    // 2ページ目の次の1件が要る(9件)ので chunk_size=2 を2回足す
    assert_eq!(conv.generated().len(), 5 + 2 + 2);
}

#[test]
fn ranker_is_called_per_pool_and_chunk() {
    struct Spy(Mutex<Vec<usize>>);
    impl Ranker for Arc<Spy> {
        fn rank(&self, _: &RankInput<'_>, cands: &mut [Candidate]) {
            self.0.lock().unwrap().push(cands.len());
        }
        fn name(&self) -> &str {
            "spy"
        }
    }
    let spy = Arc::new(Spy(Mutex::new(Vec::new())));
    let c = many_homophones(Arc::new(Arc::clone(&spy)), small_opts());
    let mut conv = c.convert("あ", &Context::new());
    assert_eq!(*spy.0.lock().unwrap(), [5]);
    let mut i = 0;
    while conv.page(i).is_some() {
        i += 1;
    }
    let calls = spy.0.lock().unwrap().clone();
    assert_eq!(calls[0], 5);
    assert!(
        calls[1..].iter().all(|&n| (1..=2).contains(&n)),
        "{calls:?}"
    );
    // どの候補も1回だけ rank される
    assert_eq!(calls.iter().sum::<usize>(), conv.generated().len());
}

#[test]
fn secure_context_is_empty_for_ranker() {
    struct Capture(Mutex<Vec<(String, usize, String, bool)>>);
    impl Ranker for Arc<Capture> {
        fn rank(&self, input: &RankInput<'_>, _: &mut [Candidate]) {
            let c = input.context;
            self.0.lock().unwrap().push((
                c.left_text.clone(),
                c.history.len(),
                c.right_text.clone(),
                c.secure,
            ));
        }
        fn name(&self) -> &str {
            "capture"
        }
    }
    let cap = Arc::new(Capture(Mutex::new(Vec::new())));
    let c = many_homophones(Arc::new(Arc::clone(&cap)), small_opts());
    let history = vec![crate::Commit {
        reading: "あ".into(),
        surface: "亜".into(),
    }];
    let mk = |secure| {
        let mut ctx = Context::new()
            .with_left_text("secret")
            .with_history(history.clone())
            .with_secure(secure);
        ctx.right_text = "right".into();
        ctx
    };

    let mut conv = c.convert("あ", &mk(true));
    conv.page(0);
    conv.page(2);
    for (left, hist, right, secure) in cap.0.lock().unwrap().iter() {
        assert_eq!((left.as_str(), *hist, right.as_str()), ("", 0, ""));
        assert!(*secure);
    }
    assert!(!cap.0.lock().unwrap().is_empty());

    cap.0.lock().unwrap().clear();
    let _ = c.convert("あ", &mk(false));
    let calls = cap.0.lock().unwrap();
    assert_eq!(
        calls[0],
        ("secret".to_owned(), 1, "right".to_owned(), false)
    );
}

#[test]
fn ranker_can_change_the_order() {
    struct Prefer(&'static str);
    impl Ranker for Prefer {
        fn rank(&self, _: &RankInput<'_>, cands: &mut [Candidate]) {
            for c in cands {
                if c.surface == self.0 {
                    c.score -= 10_000;
                }
            }
        }
        fn name(&self) -> &str {
            "prefer"
        }
    }
    let base = henkan_converter(cost_ranker(), ConvertOptions::default());
    let mut a = base.convert("へんか", &Context::new());
    assert_eq!(a.page(0).unwrap().candidates[0].surface, "変化");

    let c = henkan_converter(Arc::new(Prefer("へんか")), pool_of(2));
    let mut b = c.convert("へんか", &Context::new());
    let top = &b.page(0).unwrap().candidates[0];
    assert_eq!(top.surface, "へんか");
    assert_eq!(top.origin, CandidateOrigin::Hiragana);
    assert_eq!(top.base_cost - 10_000, top.score);
}

#[test]
fn max_astar_pops_bounds_one_generation() {
    // 9文字の読みは、1パスに最低9回の展開が要る。上限5では辞書の候補が1件も出ず、
    // ひらがなとカタカナだけで打ち切られる（遅延の保護）
    let g = grammar();
    let mut d = InMemoryDictionary::new(g.grammar_id());
    for r in ["あ", "い", "う"] {
        d.insert(r, &r.to_uppercase(), ID_NOUN, ID_NOUN, 100, 0);
    }
    let reading = "あいうあいうあいう";
    let tiny = ConvertOptions {
        max_astar_pops: 5,
        ..small_opts()
    };
    let c = conv_with(g, d, cost_ranker(), tiny);
    let mut conv = c.convert(reading, &Context::new());
    let page = conv.page(0).unwrap();
    assert!(!page.has_next);
    assert!(
        page.candidates
            .iter()
            .all(|c| c.origin != CandidateOrigin::Dictionary)
    );
    assert_eq!(page.candidates.len(), 2);
}

#[test]
fn max_astar_pops_is_per_generation_not_total() {
    // 予算は1回の生成ごと。十分な予算なら、続けて取り出すと候補が増え続ける
    let g = grammar();
    let mut d = InMemoryDictionary::new(g.grammar_id());
    for r in ["あ", "い", "う"] {
        for i in 0..4i16 {
            d.insert(r, &format!("{r}{i}"), ID_NOUN, ID_NOUN, 100 + i, 0);
        }
    }
    let opts = ConvertOptions {
        max_astar_pops: 5_000,
        ..small_opts()
    };
    let c = conv_with(g, d, cost_ranker(), opts);
    let mut conv = c.convert("あいうあいう", &Context::new());
    for i in 0..50 {
        assert!(conv.page(i).is_some_and(|p| p.has_next), "page {i}");
    }
    assert!(conv.generated().len() > 50 * 4);
}

#[test]
fn grammar_mismatch_is_an_error() {
    let g = grammar();
    let d = InMemoryDictionary::new(g.grammar_id() ^ 1);
    let r = Converter::new(
        Arc::new(d),
        Arc::new(g),
        cost_ranker(),
        ConvertOptions::default(),
    );
    assert!(matches!(r, Err(ConvertError::GrammarMismatch { .. })));
}

#[test]
fn options_are_sanitized() {
    let opts = ConvertOptions {
        page_size: 0,
        pool_size: 0,
        chunk_size: 0,
        max_astar_pops: 20_000,
        ..ConvertOptions::default()
    };
    let c = many_homophones(cost_ranker(), opts);
    let mut conv = c.convert("あ", &Context::new());
    assert_eq!(conv.page(0).unwrap().candidates.len(), 1);
    let big = ConvertOptions {
        page_size: 100,
        ..ConvertOptions::default()
    };
    let c = many_homophones(cost_ranker(), big);
    let mut conv = c.convert("あ", &Context::new());
    assert_eq!(conv.page(0).unwrap().candidates.len(), 9);
}

#[test]
fn conversion_is_static_and_send() {
    fn assert_static_send<T: Send + 'static>() {}
    fn assert_send_sync<T: Send + Sync>() {}
    assert_static_send::<Conversion>();
    assert_send_sync::<Converter>();
    let c = henkan_converter(cost_ranker(), ConvertOptions::default());
    let mut conv = c.convert("へんかん", &Context::new());
    drop(c);
    let n = std::thread::spawn(move || conv.page(0).map_or(0, |p| p.candidates.len()))
        .join()
        .unwrap();
    assert!(n > 0);
}

/// `cargo test --release -p ikari-core -- --ignored --nocapture perf_smoke`
#[test]
#[ignore = "manual timing check; run in release"]
fn perf_smoke() {
    let g = grammar();
    let mut d = InMemoryDictionary::new(g.grammar_id());
    let reading: Vec<char> = "ほんじつはせいてんなりへんかんする"
        .chars()
        .chain("あいうえお".chars())
        .collect();
    let reading: String = reading.iter().take(20).collect();
    // 全部分文字列（1..=6文字）に3エントリずつ
    let chars: Vec<char> = reading.chars().collect();
    for s in 0..chars.len() {
        for l in 1..=6.min(chars.len() - s) {
            let r: String = chars[s..s + l].iter().collect();
            for k in 0..3i16 {
                d.insert(
                    &r,
                    &format!("{r}{k}"),
                    ID_NOUN,
                    ID_NOUN,
                    500 * l as i16 + k * 100,
                    0,
                );
            }
        }
    }
    let c = conv_with(g, d, cost_ranker(), ConvertOptions::default());
    let mut times = Vec::new();
    for _ in 0..200 {
        let t = std::time::Instant::now();
        let mut conv = c.convert(&reading, &Context::new());
        let n = conv.page(0).map_or(0, |p| p.candidates.len());
        times.push(t.elapsed());
        assert_eq!(n, 9);
    }
    times.sort();
    eprintln!(
        "perf_smoke: {} chars, p50={:?} p99={:?} max={:?}",
        reading.chars().count(),
        times[times.len() / 2],
        times[times.len() * 99 / 100],
        times[times.len() - 1]
    );
}

// ---- フェーズ1: 解析器、penalty、Rewriter、commit、secure ----

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::analysis::{ContextToken, LatticeContextAnalyzer};
use crate::candidate::CandidateOrigin as Origin;
use crate::dict::{IME_EXCLUDED, UnknownParams, WordEntry, WordId};
use crate::lattice::testutil::TestDict;
use crate::rewriter::{FixedClock, LocalDateTime, RewriteCandidate};

/// `bos_eos_id` を変えられる文法（フェーズ1の BOS の置き換えの確認用）
struct BosGrammar {
    inner: DenseGrammar,
    bos: u16,
}

impl Grammar for BosGrammar {
    fn connection_cost(&self, prev_right_id: u16, next_left_id: u16) -> i16 {
        self.inner.connection_cost(prev_right_id, next_left_id)
    }
    fn bos_eos_id(&self) -> u16 {
        self.bos
    }
    fn unknown(&self) -> UnknownParams {
        self.inner.unknown()
    }
    fn pos_name(&self, pos_id: u16) -> Option<&str> {
        self.inner.pos_name(pos_id)
    }
    fn grammar_id(&self) -> u64 {
        self.inner.grammar_id()
    }
}

/// 解析結果を固定で返し、呼ばれた回数を数える解析器
struct FixedAnalyzer {
    bos_right_id: u16,
    tokens: usize,
    calls: AtomicUsize,
    seen: Mutex<Vec<String>>,
}

impl FixedAnalyzer {
    fn new(bos_right_id: u16, tokens: usize) -> Arc<Self> {
        Arc::new(Self {
            bos_right_id,
            tokens,
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        })
    }
}

impl ContextAnalyzer for FixedAnalyzer {
    fn analyze(&self, left_text: &str, _history: &[crate::Commit]) -> Arc<AnalyzedContext> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(left_text.to_owned());
        let token = ContextToken {
            surface: "語".into(),
            lemma: "語".into(),
            lemma_id: None,
            word: None,
            left_id: 0,
            right_id: self.bos_right_id,
            pos_id: 0,
            is_content: true,
            char_end: 0,
        };
        Arc::new(AnalyzedContext {
            tokens: vec![token; self.tokens],
            bos_right_id: self.bos_right_id,
            bos_alternatives: Vec::new(),
            content_count: self.tokens,
            carried_from: None,
        })
    }
}

fn builder_for(g: DenseGrammar, d: TestDict) -> ConverterBuilder {
    Converter::builder(Arc::new(d), Arc::new(g))
}

/// なった（成った／ナッタ）の組。助詞の右ID の後ろでは「成った」が安い。
fn natta_setup() -> (DenseGrammar, TestDict) {
    let mut g = grammar();
    g.set(ID_PARTICLE, ID_VERB_SURU, -3000);
    g.set(ID_PARTICLE, ID_NOUN, 600);
    g.set(0, ID_NOUN, 1);
    g.set(0, ID_VERB_SURU, 300);
    let mut d = TestDict::new(g.grammar_id());
    d.add("なった", "成った", ID_VERB_SURU, ID_VERB_SURU, 3000, 1);
    d.add("なった", "ナッタ", ID_NOUN, ID_NOUN, 2900, 0);
    (g, d)
}

#[test]
fn analyzer_bos_changes_the_connection_to_the_first_word() {
    let (g, d) = natta_setup();
    let plain = builder_for(g, d).build().unwrap();
    let mut conv = plain.convert("なった", &Context::new());
    assert_eq!(conv.page(0).unwrap().candidates[0].surface, "ナッタ");

    let (g, d) = natta_setup();
    let analyzer = FixedAnalyzer::new(ID_PARTICLE, 1);
    let c = builder_for(g, d)
        .analyzer(analyzer.clone())
        .build()
        .unwrap();
    let mut conv = c.convert("なった", &Context::new().with_left_text("春に"));
    let top = &conv.page(0).unwrap().candidates[0];
    assert_eq!(top.surface, "成った");
    assert_eq!(top.base_cost, 3000 - 3000);
    assert_eq!(*analyzer.seen.lock().unwrap(), ["春に"]);
}

#[test]
fn lattice_analyzer_end_to_end_makes_the_grammatical_candidate_win() {
    let (g, mut d) = natta_setup();
    d.add("に", "に", ID_PARTICLE, ID_PARTICLE, 100, 2);
    d.add("さくら", "桜", ID_NOUN, ID_NOUN, 100, 0);
    // 同じ辞書と文法を解析にも変換にも使う（ID 空間が同じ）
    let (dict, grammar): (Arc<dyn Dictionary>, Arc<dyn Grammar>) = (Arc::new(d), Arc::new(g));
    let analyzer = Arc::new(LatticeContextAnalyzer::new(dict.clone(), grammar.clone()));
    let c = Converter::builder(dict, grammar)
        .analyzer(analyzer)
        .build()
        .unwrap();
    let mut conv = c.convert("なった", &Context::new().with_left_text("桜に"));
    assert_eq!(conv.page(0).unwrap().candidates[0].surface, "成った");
    // 文末の後ろでは BOS に戻る
    let mut conv = c.convert("なった", &Context::new().with_left_text("桜に。"));
    assert_eq!(conv.page(0).unwrap().candidates[0].surface, "ナッタ");
}

#[test]
fn null_analyzer_uses_grammar_bos_even_when_it_is_not_zero() {
    const BOS: u16 = 3;
    let mut inner = grammar();
    inner.set(BOS, ID_NOUN, -700);
    inner.set(0, ID_NOUN, 900);
    let g = BosGrammar { inner, bos: BOS };
    let mut d = TestDict::new(g.grammar_id());
    d.add("あ", "亜", ID_NOUN, ID_NOUN, 1000, 0);
    let c = Converter::builder(Arc::new(d), Arc::new(g))
        .build()
        .unwrap();
    let mut conv = c.convert("あ", &Context::new());
    // 0 ではなく bos_eos_id (=3) から接続される
    assert_eq!(conv.page(0).unwrap().candidates[0].base_cost, 1000 - 700);
    let event = conv.commit(0).unwrap();
    assert_eq!(event.analysis.bos_right_id, BOS);
}

#[test]
fn secure_skips_the_analyzer_and_uses_bos_eos() {
    let (g, d) = natta_setup();
    let analyzer = FixedAnalyzer::new(ID_PARTICLE, 3);
    let c = builder_for(g, d)
        .analyzer(analyzer.clone())
        .build()
        .unwrap();
    let ctx = Context::new().with_left_text("春に").with_secure(true);
    let mut conv = c.convert("なった", &ctx);
    assert_eq!(analyzer.calls.load(Ordering::SeqCst), 0);
    // 文脈なしと同じ（ナッタが1位）
    assert_eq!(conv.page(0).unwrap().candidates[0].surface, "ナッタ");
    assert!(conv.commit(0).is_none());
    assert!(conv.commit(1).is_none());

    let mut conv = c.convert("なった", &Context::new().with_left_text("春に"));
    assert_eq!(analyzer.calls.load(Ordering::SeqCst), 1);
    assert!(conv.page(0).is_some());
}

#[test]
fn ranker_receives_the_analysis_and_the_dictionary() {
    struct Probe(Mutex<Vec<(usize, u16, bool)>>);
    impl Ranker for Arc<Probe> {
        fn rank(&self, input: &RankInput<'_>, _: &mut [Candidate]) {
            self.0.lock().unwrap().push((
                input.analysis.tokens.len(),
                input.analysis.bos_right_id,
                input.dict.entry(WordId { dict: 0, index: 0 }).is_some(),
            ));
        }
        fn name(&self) -> &str {
            "probe"
        }
    }
    let probe = Arc::new(Probe(Mutex::new(Vec::new())));
    let (g, d) = natta_setup();
    let c = builder_for(g, d)
        .analyzer(FixedAnalyzer::new(ID_PARTICLE, 4))
        .ranker(Arc::new(Arc::clone(&probe)))
        .build()
        .unwrap();
    let mut conv = c.convert("なった", &Context::new().with_left_text("春に"));
    conv.page(0);
    assert_eq!(*probe.0.lock().unwrap(), [(4, ID_PARTICLE, true)]);
}

#[test]
fn penalty_is_added_to_the_cost_in_conversion() {
    let g = grammar();
    let mut d = TestDict::new(g.grammar_id());
    d.add("あ", "亜", ID_NOUN, ID_NOUN, 1000, 0);
    let pen = d.add("あ", "阿", ID_NOUN, ID_NOUN, 500, 0);
    d.set_penalty(pen, 3000);
    let c = builder_for(g, d).build().unwrap();
    let mut conv = c.convert("あ", &Context::new());
    let page = conv.page(0).unwrap();
    let by: Vec<(&str, i32)> = page
        .candidates
        .iter()
        .map(|c| (c.surface.as_str(), c.base_cost))
        .collect();
    // 阿は 500 + 3000 = 3500 になり、亜 (1000) より後ろ
    assert_eq!(&by[..2], [("亜", 1000), ("阿", 3500)]);
}

#[test]
fn excluded_entries_never_appear_in_conversion() {
    let g = grammar();
    let mut d = TestDict::new(g.grammar_id());
    d.add("あ", "亜", ID_NOUN, ID_NOUN, 1000, 0);
    let ex = d.add("あ", "a", ID_NOUN, ID_NOUN, 10, 0);
    d.set_penalty(ex, IME_EXCLUDED);
    let c = builder_for(g, d).build().unwrap();
    let mut conv = c.convert("あ", &Context::new());
    let mut i = 0;
    while conv.page(i).is_some() {
        i += 1;
    }
    let surfaces: Vec<&str> = conv
        .generated()
        .iter()
        .map(|c| c.surface.as_str())
        .collect();
    assert_eq!(surfaces, ["亜", "あ", "ア"]);
}

/// 決まった表記を返す Rewriter
struct Fixed {
    name: &'static str,
    out: Vec<(&'static str, u16)>,
    seen: Mutex<Vec<(String, LocalDateTime)>>,
}

impl Fixed {
    fn new(name: &'static str, out: &[(&'static str, u16)]) -> Arc<Self> {
        Arc::new(Self {
            name,
            out: out.to_vec(),
            seen: Mutex::new(Vec::new()),
        })
    }
}

impl Rewriter for Fixed {
    fn name(&self) -> &'static str {
        self.name
    }
    fn rewrite(&self, input: &crate::rewriter::RewriteInput<'_>, out: &mut Vec<RewriteCandidate>) {
        self.seen
            .lock()
            .unwrap()
            .push((input.reading.to_owned(), input.now));
        out.extend(
            self.out
                .iter()
                .map(|&(surface, format)| RewriteCandidate::new(surface.to_owned(), format)),
        );
    }
}

/// `AfterWords` の位置で候補を 1 つ出す Rewriter（日付と同じ位置の指定）
struct FixedAfterWords;

impl Rewriter for FixedAfterWords {
    fn name(&self) -> &'static str {
        "after-words"
    }
    fn rewrite(&self, _: &crate::rewriter::RewriteInput<'_>, out: &mut Vec<RewriteCandidate>) {
        out.push(RewriteCandidate::after_words("D".to_owned(), 1));
    }
}

const NOW: LocalDateTime = LocalDateTime {
    year: 2026,
    month: 10,
    day: 11,
    hour: 14,
    minute: 5,
    weekday: 6,
};

/// 辞書の候補が 1000 / 1100 / 1200 の「あ」
fn rewriter_converter(rewriters: Vec<Arc<dyn Rewriter>>, anchor: usize) -> Converter {
    let g = grammar();
    let mut d = TestDict::new(g.grammar_id());
    d.add("あ", "亜", ID_NOUN, ID_NOUN, 1000, 0);
    d.add("あ", "阿", ID_NOUN, ID_NOUN, 1100, 0);
    d.add("あ", "吾", ID_NOUN, ID_NOUN, 1200, 0);
    builder_for(g, d)
        .rewriters(rewriters)
        .clock(Arc::new(FixedClock(NOW)))
        .options(ConvertOptions {
            rewriter_anchor: anchor,
            ..ConvertOptions::default()
        })
        .build()
        .unwrap()
}

fn all_surfaces_of(c: &Converter, reading: &str, ctx: &Context) -> Vec<String> {
    let mut conv = c.convert(reading, ctx);
    all_surfaces(&mut conv)
}

#[test]
fn rewriter_candidates_are_inserted_after_the_anchor() {
    for (anchor, expected) in [
        (1, ["亜", "R", "阿", "吾"]),
        (2, ["亜", "阿", "R", "吾"]),
        (3, ["亜", "阿", "吾", "R"]),
        // 0 は 1 として扱う
        (0, ["亜", "R", "阿", "吾"]),
    ] {
        let c = rewriter_converter(vec![Fixed::new("fixed", &[("R", 7)])], anchor);
        let s = all_surfaces_of(&c, "あ", &Context::new());
        assert_eq!(s[..4], expected, "anchor={anchor}");
    }
}

#[test]
fn after_words_candidates_follow_date_anchor_and_not_rewriter_anchor() {
    // 既定（date_anchor = 1）は Anchor と同じ。k を上げると D が後ろへ下がる。
    // Anchor の R は rewriter_anchor = 1 のまま動かない
    for (date_anchor, expected) in [
        (1, ["亜", "R", "D", "阿", "吾"]),
        (2, ["亜", "R", "阿", "D", "吾"]),
        (3, ["亜", "R", "阿", "吾", "D"]),
        (0, ["亜", "R", "D", "阿", "吾"]),
    ] {
        let g = grammar();
        let mut d = TestDict::new(g.grammar_id());
        d.add("あ", "亜", ID_NOUN, ID_NOUN, 1000, 0);
        d.add("あ", "阿", ID_NOUN, ID_NOUN, 1100, 0);
        d.add("あ", "吾", ID_NOUN, ID_NOUN, 1200, 0);
        let c = builder_for(g, d)
            .rewriters(vec![
                Fixed::new("fixed", &[("R", 7)]) as Arc<dyn Rewriter>,
                Arc::new(FixedAfterWords),
            ])
            .clock(Arc::new(FixedClock(NOW)))
            .options(ConvertOptions {
                date_anchor,
                ..ConvertOptions::default()
            })
            .build()
            .unwrap();
        let s = all_surfaces_of(&c, "あ", &Context::new());
        assert_eq!(s[..5], expected, "date_anchor={date_anchor}");
    }
}

#[test]
fn date_anchor_is_read_from_ranker_config() {
    let kv = |v: &str| {
        std::collections::BTreeMap::from([("rewriter.date_anchor".to_owned(), v.to_owned())])
    };
    let opts = ConvertOptions::default();
    assert_eq!(opts.date_anchor, 3);
    let four = opts.clone().with_date_anchor_kv(&kv("4")).unwrap();
    assert_eq!(four.date_anchor, 4);
    assert!(opts.clone().with_date_anchor_kv(&kv("x")).is_err());
    let unknown = std::collections::BTreeMap::from([("rewriter.nope".to_owned(), "1".to_owned())]);
    assert!(opts.with_date_anchor_kv(&unknown).is_err());
}

#[test]
fn rewriter_anchor_beyond_the_pool_goes_after_the_last_dictionary_candidate() {
    // 辞書の候補は 亜 阿 吾 と、未知語だけの経路 あ（コスト 5000）の4件
    let c = rewriter_converter(vec![Fixed::new("fixed", &[("R", 7)])], 99);
    let s = all_surfaces_of(&c, "あ", &Context::new());
    assert_eq!(s[..5], ["亜", "阿", "吾", "あ", "R"]);
}

#[test]
fn rewriter_candidate_has_origin_format_and_one_unknown_segment() {
    let c = rewriter_converter(vec![Fixed::new("fixed", &[("R", 7)])], 1);
    let mut conv = c.convert("あ", &Context::new());
    let page = conv.page(0).unwrap();
    let r = &page.candidates[1];
    assert_eq!(r.surface, "R");
    assert_eq!(
        r.origin,
        Origin::Rewriter {
            name: "fixed",
            format: 7
        }
    );
    assert_eq!(r.base_cost, 1001);
    assert_eq!(r.segments.len(), 1);
    assert!(r.segments[0].word.is_none());
    assert_eq!(r.segments[0].surface, "R");
    assert_eq!(r.segments[0].reading, "あ");
}

#[test]
fn rewriter_receives_normalized_reading_and_clock_time() {
    let fixed = Fixed::new("fixed", &[("R", 1)]);
    let c = rewriter_converter(vec![fixed.clone()], 1);
    let _ = c.convert("ア", &Context::new());
    assert_eq!(*fixed.seen.lock().unwrap(), [("あ".to_owned(), NOW)]);
}

#[test]
fn rewriter_duplicates_and_empty_surfaces_are_dropped() {
    let a = Fixed::new("a", &[("亜", 1), ("X", 2), ("", 3)]);
    let b = Fixed::new("b", &[("X", 9), ("Y", 1)]);
    let c = rewriter_converter(vec![a, b], 1);
    let s = all_surfaces_of(&c, "あ", &Context::new());
    assert_eq!(s.iter().filter(|x| *x == "亜").count(), 1);
    assert_eq!(s.iter().filter(|x| *x == "X").count(), 1);
    assert!(s.contains(&"Y".to_owned()));
    assert!(!s.contains(&String::new()));
    // 先に出た Rewriter の書式が残る
    let mut conv = c.convert("あ", &Context::new());
    let x = conv
        .page(0)
        .unwrap()
        .candidates
        .iter()
        .find(|c| c.surface == "X")
        .unwrap()
        .origin;
    assert_eq!(
        x,
        Origin::Rewriter {
            name: "a",
            format: 2
        }
    );
}

#[test]
fn rewriter_candidates_go_through_the_ranker() {
    struct Boost;
    impl Ranker for Boost {
        fn rank(&self, _: &RankInput<'_>, cands: &mut [Candidate]) {
            for c in cands {
                if matches!(c.origin, Origin::Rewriter { .. }) {
                    c.score -= 5000;
                }
            }
        }
        fn name(&self) -> &str {
            "boost"
        }
    }
    let g = grammar();
    let mut d = TestDict::new(g.grammar_id());
    d.add("あ", "亜", ID_NOUN, ID_NOUN, 1000, 0);
    let c = builder_for(g, d)
        .rewriters(vec![Fixed::new("fixed", &[("R", 1)])])
        .ranker(Arc::new(Boost))
        .build()
        .unwrap();
    let mut conv = c.convert("あ", &Context::new());
    assert_eq!(conv.page(0).unwrap().candidates[0].surface, "R");
}

#[test]
fn rewriters_do_not_run_for_empty_reading_or_later_chunks() {
    let fixed = Fixed::new("fixed", &[("R", 1)]);
    let c = rewriter_converter(vec![fixed.clone()], 1);
    let conv = c.convert("", &Context::new());
    assert!(conv.generated().is_empty());
    assert!(fixed.seen.lock().unwrap().is_empty());
    let mut conv = c.convert("あ", &Context::new());
    conv.page(0);
    conv.page(1);
    assert_eq!(fixed.seen.lock().unwrap().len(), 1);
}

#[test]
fn rewriter_with_empty_pool_still_adds_candidates() {
    let g = grammar();
    let d = TestDict::new(g.grammar_id());
    let c = builder_for(g, d)
        .rewriters(vec![Fixed::new("fixed", &[("R", 1)])])
        .build()
        .unwrap();
    let s = all_surfaces_of(&c, "ぬ", &Context::new());
    assert!(s.contains(&"R".to_owned()));
    assert!(s.contains(&"ぬ".to_owned()));
}

#[test]
fn commit_returns_the_event_for_the_chosen_candidate() {
    let g = grammar();
    let mut d = TestDict::new(g.grammar_id());
    d.add("あ", "亜", ID_NOUN, ID_NOUN, 1000, 0);
    let c = builder_for(g, d)
        .analyzer(FixedAnalyzer::new(ID_PARTICLE, 2))
        .rewriters(vec![Fixed::new("fixed", &[("R", 7)])])
        .build()
        .unwrap();
    let mut conv = c.convert("ア", &Context::new().with_left_text("春に"));
    conv.page(0);
    let event = conv.commit(1).unwrap();
    assert_eq!(event.reading, "あ");
    assert_eq!(event.candidate.surface, "R");
    assert_eq!(
        event.candidate.origin,
        Origin::Rewriter {
            name: "fixed",
            format: 7
        }
    );
    assert_eq!(event.analysis.bos_right_id, ID_PARTICLE);
    assert_eq!(event.analysis.tokens.len(), 2);
    assert_eq!(event.candidate.score, conv.generated()[1].score);
    assert!(conv.commit(conv.generated().len()).is_none());
    assert!(conv.commit(usize::MAX).is_none());
}

#[test]
fn commit_on_empty_conversion_is_none() {
    let c = henkan_converter(cost_ranker(), ConvertOptions::default());
    let conv = c.convert("", &Context::new());
    assert!(conv.commit(0).is_none());
}

#[test]
fn word_entry_exposes_penalty() {
    let g = grammar();
    let mut d = TestDict::new(g.grammar_id());
    let id = d.add("あ", "亜", ID_NOUN, ID_NOUN, 1, 0);
    d.set_penalty(id, 5);
    let mut seen = None;
    d.lookup_exact("あ", &mut |e: WordEntry<'_>| seen = Some(e.penalty));
    assert_eq!(seen, Some(5));
}

/// 英字の1位をカタカナにする規則（ADR 0020）の、変換としての確かめ
fn english_converter() -> Converter {
    let g = grammar();
    let mut d = InMemoryDictionary::new(g.grammar_id());
    d.insert(
        "はるしねーしょん",
        "Hallucination",
        ID_NOUN,
        ID_NOUN,
        1000,
        0,
    );
    d.insert(
        "はるしねーしょん",
        "ハルシネーション",
        ID_NOUN,
        ID_NOUN,
        3000,
        0,
    );
    d.insert("が", "が", ID_PARTICLE, ID_PARTICLE, 100, 2);
    d.insert("おきる", "起きる", ID_NOUN, ID_NOUN, 100, 0);
    d.insert("えすきゅーえる", "SQL", ID_NOUN, ID_NOUN, 1000, 0);
    let ranker: Arc<dyn Ranker> =
        Arc::new(crate::ranker::CompositeRanker::new(vec![cost_ranker()]));
    conv_with(g, d, ranker, ConvertOptions::default())
}

fn first_two(c: &Converter, reading: &str) -> Vec<String> {
    let mut conv = c.convert(reading, &Context::new());
    conv.page(0)
        .unwrap()
        .candidates
        .iter()
        .take(2)
        .map(|c| c.surface.clone())
        .collect()
}

#[test]
fn english_word_alone_gets_katakana_first_and_latin_second() {
    let c = english_converter();
    assert_eq!(
        first_two(&c, "はるしねーしょん"),
        ["ハルシネーション", "Hallucination"]
    );
}

#[test]
fn latin_without_a_katakana_entry_gets_the_katakana_of_the_reading_first() {
    let c = english_converter();
    assert_eq!(first_two(&c, "えすきゅーえる"), ["エスキューエル", "SQL"]);
}

#[test]
fn english_word_inside_a_sentence_is_replaced_too() {
    let c = english_converter();
    assert_eq!(
        first_two(&c, "はるしねーしょんがおきる"),
        ["ハルシネーションが起きる", "Hallucinationが起きる"]
    );
}

#[test]
fn kanji_with_small_kana_is_shown_only_when_reading_has_it() {
    let g = grammar();
    let mut d = TestDict::new(g.grammar_id());
    d.add("め", "目", ID_NOUN, ID_NOUN, 1000, 0);
    d.add("め", "目ぇ", ID_NOUN, ID_NOUN, 500, 0);
    d.add("わりぃ", "悪ぃ", ID_NOUN, ID_NOUN, 500, 0);
    let c = builder_for(g, d).build().unwrap();
    let surfaces = |reading: &str| {
        let mut conv = c.convert(reading, &Context::new());
        let mut i = 0;
        while conv.page(i).is_some() {
            i += 1;
        }
        conv.generated()
            .iter()
            .map(|c| c.surface.clone())
            .collect::<Vec<_>>()
    };
    // 入力していない「ぇ」を足す候補は出さない
    assert!(!surfaces("め").contains(&"目ぇ".to_owned()));
    // 「めぇ」には「目」+「ぇ」の経路で出る。辞書の「目ぇ」+「ぇ」（目ぇぇ）は出ない
    let with_small = surfaces("めぇ");
    assert!(with_small.contains(&"目ぇ".to_owned()));
    assert!(!with_small.contains(&"目ぇぇ".to_owned()));
    // 読みに小書き仮名があれば辞書の表記が出る
    assert!(surfaces("わりぃ").contains(&"悪ぃ".to_owned()));
}

/// K1（ADR 0037）の統合: 未知語のカタカナだけに上限が掛かる。ひらがなと、辞書に同じ表記の 1 語があるカタカナは動かない。
#[test]
fn k1_caps_only_unknown_katakana() {
    let k1 = |base: i32, per_char: i32| ConvertOptions {
        pool_size: 2,
        katakana_like: KatakanaLikeRule {
            base,
            per_char,
            ..KatakanaLikeRule::default()
        },
        ..ConvertOptions::default()
    };
    let cost_of = |opts: ConvertOptions, reading: &str, origin: CandidateOrigin| {
        let mut g = grammar();
        g.set(ID_NOUN, ID_NOUN, 800);
        let mut d = InMemoryDictionary::new(g.grammar_id());
        d.insert("へんかん", "変換", ID_NOUN, ID_NOUN, 1000, 0);
        d.insert("へんか", "変化", ID_NOUN, ID_NOUN, 1100, 0);
        d.insert("かたかな", "カタカナ", ID_NOUN, ID_NOUN, 3000, 0);
        let c = conv_with(g, d, cost_ranker(), opts);
        let mut conv = c.convert(reading, &Context::new());
        all_surfaces(&mut conv);
        conv.generated()
            .iter()
            .find(|c| c.origin == origin)
            .map(|c| c.base_cost)
    };
    // 無効（0/0）: 今と同じ（辞書の最大 + 1。ひらがなと同じ値）
    let off = cost_of(k1(0, 0), "へんかん", CandidateOrigin::Katakana).unwrap();
    assert!(off > 400);
    // 有効: 100 × 4 文字 = 400。ひらがなは動かない
    assert_eq!(
        cost_of(k1(0, 100), "へんかん", CandidateOrigin::Katakana),
        Some(400)
    );
    assert_eq!(
        cost_of(k1(0, 100), "へんかん", CandidateOrigin::Hiragana),
        Some(off)
    );
    // 上限が最大 + 1 より高ければ変わらない
    assert_eq!(
        cost_of(k1(0, 100_000), "へんかん", CandidateOrigin::Katakana),
        Some(off)
    );
    // 辞書に同じ表記の 1 語（かたかな → カタカナ）は辞書の候補のまま（K1 の影響を受けない）
    let dict_off = cost_of(k1(0, 0), "かたかな", CandidateOrigin::Dictionary);
    let dict_on = cost_of(k1(0, 1), "かたかな", CandidateOrigin::Dictionary);
    assert!(dict_off.is_some());
    assert_eq!(dict_off, dict_on);
}

// ---- 語の予約枠と、かな・アンカーの基準（ADR 0048、#185） ----

const COMPOSITE_COST: i16 = 1000;
const WORD_COST: i16 = 6000;

fn reserve_opts(pool_size: usize, word_reserve: usize) -> ConvertOptions {
    ConvertOptions {
        pool_size,
        word_reserve,
        ..ConvertOptions::default()
    }
}

/// いき（動詞 1000）+ ち（名詞 1000）の合成が、いきち（名詞 6000）の 1 語より先に並ぶ（#185 の型）
fn ikichi_dict() -> (DenseGrammar, InMemoryDictionary) {
    let g = grammar();
    let mut d = InMemoryDictionary::new(g.grammar_id());
    d.insert(
        "いき",
        "行き",
        ID_VERB_SURU,
        ID_VERB_SURU,
        COMPOSITE_COST,
        1,
    );
    d.insert("ち", "値", ID_NOUN, ID_NOUN, COMPOSITE_COST, 0);
    d.insert("いきち", "閾値", ID_NOUN, ID_NOUN, WORD_COST, 0);
    (g, d)
}

fn first_pool(opts: ConvertOptions) -> Vec<Candidate> {
    let (g, d) = ikichi_dict();
    let c = conv_with(g, d, cost_ranker(), opts);
    c.convert("いきち", &Context::new()).generated().to_vec()
}

#[test]
fn reserve_adds_the_single_word_covering_a_composite_span() {
    let off = first_pool(reserve_opts(1, 0));
    assert!(off.iter().all(|c| c.surface != "閾値"), "{off:?}");
    let on = first_pool(reserve_opts(1, 1));
    let word = on.iter().find(|c| c.surface == "閾値").expect("reserved");
    // 経路のコストのまま（加点しない）。辞書の経路として足される
    assert_eq!(word.origin, CandidateOrigin::Dictionary);
    assert_eq!(word.base_cost, i32::from(WORD_COST));
    assert_eq!(word.score, word.base_cost);
    assert_eq!(word.segments.len(), 1);
    // 上位は変わらない
    assert_eq!(on[0].surface, "行き値");
    assert_eq!(on[0].surface, off[0].surface);
}

#[test]
fn reserve_zero_changes_nothing() {
    let surfaces = |o: ConvertOptions| -> Vec<(String, i32)> {
        first_pool(o)
            .into_iter()
            .map(|c| (c.surface, c.base_cost))
            .collect()
    };
    assert_eq!(surfaces(reserve_opts(1, 0)), surfaces(pool_of(1)));
    assert_eq!(
        surfaces(reserve_opts(30, 0)),
        surfaces(ConvertOptions::default())
    );
}

#[test]
fn reserve_skips_words_already_in_the_pool() {
    // プールに 閾値 が既にあれば、予約枠は何も足さない
    let off = first_pool(reserve_opts(30, 0));
    let on = first_pool(reserve_opts(30, 5));
    assert_eq!(
        off.iter().map(|c| &c.surface).collect::<Vec<_>>(),
        on.iter().map(|c| &c.surface).collect::<Vec<_>>()
    );
    assert_eq!(on.iter().filter(|c| c.surface == "閾値").count(), 1);
}

#[test]
fn reserve_ignores_spans_with_a_particle() {
    // きょう + は（助詞）の区間は合成とみなさない。きょうは（教派）の 1 語があっても足さない
    let g = grammar();
    let mut d = InMemoryDictionary::new(g.grammar_id());
    d.insert("きょう", "今日", ID_NOUN, ID_NOUN, COMPOSITE_COST, 0);
    d.insert("は", "は", ID_PARTICLE, ID_PARTICLE, COMPOSITE_COST, 2);
    d.insert("きょうは", "教派", ID_NOUN, ID_NOUN, WORD_COST, 0);
    let c = conv_with(g, d, cost_ranker(), reserve_opts(1, 5));
    let conv = c.convert("きょうは", &Context::new());
    let s: Vec<&str> = conv
        .generated()
        .iter()
        .map(|c| c.surface.as_str())
        .collect();
    assert_eq!(s[0], "今日は");
    assert!(!s.contains(&"教派"), "{s:?}");
}

/// ち・い・き（名詞）の 3 語の合成が、ちい（地位）といき（息）の 1 語で覆える
fn chiiki_converter(opts: ConvertOptions) -> Converter {
    let g = grammar();
    let mut d = InMemoryDictionary::new(g.grammar_id());
    d.insert("ち", "値", ID_NOUN, ID_NOUN, COMPOSITE_COST, 0);
    d.insert("い", "胃", ID_NOUN, ID_NOUN, COMPOSITE_COST, 0);
    d.insert("き", "木", ID_NOUN, ID_NOUN, COMPOSITE_COST, 0);
    d.insert("ちい", "地位", ID_NOUN, ID_NOUN, WORD_COST, 0);
    d.insert("いき", "息", ID_NOUN, ID_NOUN, WORD_COST, 0);
    conv_with(g, d, cost_ranker(), opts)
}

#[test]
fn reserve_stops_at_the_limit() {
    let dictionary_count = |word_reserve: usize| {
        let c = chiiki_converter(reserve_opts(1, word_reserve));
        let conv = c.convert("ちいき", &Context::new());
        conv.generated()
            .iter()
            .filter(|c| c.origin == CandidateOrigin::Dictionary)
            .count()
    };
    // A* の 1 件 + 予約枠
    assert_eq!(dictionary_count(0), 1);
    assert_eq!(dictionary_count(1), 2);
    assert_eq!(dictionary_count(2), 3);
    assert_eq!(dictionary_count(9), 3);
}

/// 同音語のコストの刻み（60 件で 3000。未知語の 5000 より下）
const HOMOPHONE_COST_STEP: i16 = 50;

fn many_homophones_n(n: i16, ranker: Arc<dyn Ranker>, opts: ConvertOptions) -> Converter {
    let g = grammar();
    let mut d = InMemoryDictionary::new(g.grammar_id());
    for i in 0..n {
        d.insert(
            "あ",
            &format!("w{i:02}"),
            ID_NOUN,
            ID_NOUN,
            HOMOPHONE_COST_STEP * (i + 1),
            0,
        );
    }
    conv_with(g, d, ranker, opts)
}

#[test]
fn kana_cost_does_not_depend_on_pool_size() {
    let kana_costs = |pool_size: usize| {
        let c = many_homophones_n(60, cost_ranker(), pool_of(pool_size));
        let conv = c.convert("あ", &Context::new());
        let by = |o: CandidateOrigin| {
            conv.generated()
                .iter()
                .find(|c| c.origin == o)
                .map(|c| c.base_cost)
        };
        (by(CandidateOrigin::Hiragana), by(CandidateOrigin::Katakana))
    };
    // 先頭 30 件の最大（1500）+ 1
    assert_eq!(kana_costs(30), (Some(1501), Some(1501)));
    assert_eq!(kana_costs(45), kana_costs(30));
    assert_eq!(kana_costs(35), kana_costs(30));
}

#[test]
fn rewriter_anchor_ignores_reserved_candidates() {
    let r_cost = |opts: ConvertOptions| {
        let (g, d) = ikichi_dict();
        let c = Converter::builder(Arc::new(d), Arc::new(g))
            .rewriters(vec![Fixed::new("fixed", &[("R", 7)]) as Arc<dyn Rewriter>])
            .clock(Arc::new(FixedClock(NOW)))
            .options(ConvertOptions {
                rewriter_anchor: 2,
                ..opts
            })
            .build()
            .unwrap();
        let conv = c.convert("いきち", &Context::new());
        conv.generated()
            .iter()
            .find(|c| c.surface == "R")
            .map(|c| c.base_cost)
    };
    // A* の候補だけで基準を作る。予約枠（閾値 6000）は入らない
    let without = r_cost(reserve_opts(1, 0));
    assert_eq!(r_cost(reserve_opts(1, 3)), without);
    assert!(without.is_some_and(|c| c < i32::from(WORD_COST)));
}
