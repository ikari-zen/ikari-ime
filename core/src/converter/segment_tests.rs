//! 文節の区切り、文節ごとの候補、区切り直し、確定の学習（ADR 0006 §10.1）のテスト。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::analysis::LatticeContextAnalyzer;
use crate::candidate::{Candidate, CandidateOrigin, Segment};
use crate::dict::{DenseGrammar, Dictionary, Grammar, InMemoryDictionary, UnknownParams, WordId};
use crate::ranker::{CostRanker, RankInput, Ranker};
use crate::rewriter::{RewriteCandidate, RewriteInput, Rewriter};
use crate::user::LearningStore;

const ID_BOS: u16 = 0;
const ID_NOUN: u16 = 1;
const ID_PARTICLE: u16 = 2;
const ID_SURU: u16 = 3;
const ID_UNK: u16 = 4;
const ID_VERB: u16 = 5;
const ID_AUX: u16 = 6;
const ID_ADVERB: u16 = 7;
/// 右ID だけが違う名詞（右の文節への連接の違いを作る）
const ID_NOUN_B: u16 = 8;
const ID_NOUN_C: u16 = 9;
const ID_COUNT: u16 = 10;

const POS_NOUN: u16 = 0;
const POS_PARTICLE: u16 = 1;
const POS_SURU: u16 = 2;
const POS_VERB: u16 = 3;
const POS_AUX: u16 = 4;
const POS_ADVERB: u16 = 5;

const UNK_COST: i16 = 6000;
const DEFAULT_CONNECTION: i16 = 400;
const REWRITER_FORMAT: u16 = 7;

fn grammar() -> DenseGrammar {
    let mut g = DenseGrammar::new(
        ID_COUNT,
        ID_COUNT,
        DEFAULT_CONNECTION,
        UnknownParams {
            left_id: ID_UNK,
            right_id: ID_UNK,
            pos_id: POS_NOUN,
            cost_per_char: UNK_COST,
        },
    );
    g.add_pos("名詞,普通名詞,一般,*,*,*");
    g.add_pos("助詞,格助詞,*,*,*,*");
    g.add_pos("動詞,非自立可能,*,*,サ行変格,終止形-一般");
    g.add_pos("動詞,一般,*,*,五段-ラ行,終止形-一般");
    g.add_pos("助動詞,*,*,*,助動詞-タ,終止形-一般");
    g.add_pos("副詞,*,*,*,*,*");
    let free = [
        (ID_BOS, ID_NOUN),
        (ID_BOS, ID_ADVERB),
        (ID_NOUN, ID_PARTICLE),
        (ID_PARTICLE, ID_NOUN),
        (ID_NOUN, ID_SURU),
        (ID_SURU, ID_BOS),
        (ID_ADVERB, ID_VERB),
        (ID_VERB, ID_AUX),
        (ID_AUX, ID_BOS),
        (ID_NOUN, ID_VERB),
        (ID_NOUN_B, ID_BOS),
        (ID_NOUN_C, ID_BOS),
        (ID_NOUN, ID_BOS),
        (ID_PARTICLE, ID_BOS),
        (ID_NOUN, ID_NOUN),
    ];
    for (prev, next) in free {
        g.set(prev, next, 0);
    }
    // 右ID だけが違う名詞 -> 動詞 の連接
    g.set(ID_NOUN_B, ID_VERB, 300);
    g.set(ID_NOUN_C, ID_VERB, 600);
    g
}

fn dictionary(g: &DenseGrammar) -> InMemoryDictionary {
    let mut d = InMemoryDictionary::new(g.grammar_id());
    let n = |d: &mut InMemoryDictionary, r: &str, s: &str, c: i16| {
        d.insert(r, s, ID_NOUN, ID_NOUN, c, POS_NOUN)
    };
    n(&mut d, "ゆーざー", "ユーザー", 1000);
    n(&mut d, "じしょ", "辞書", 1000);
    n(&mut d, "たんご", "単語", 1000);
    n(&mut d, "とうろく", "登録", 1000);
    n(&mut d, "りぽじとり", "リポジトリ", 1000);
    n(&mut d, "めい", "名", 1500);
    n(&mut d, "めい", "銘", 3000);
    n(&mut d, "めい", "姪", 3500);
    n(&mut d, "はじめ", "初め", 3000);
    n(&mut d, "ひんし", "品詞", 1000);
    n(&mut d, "ひんし", "瀕死", 3000);
    n(&mut d, "きょう", "今日", 1000);
    n(&mut d, "こんにち", "今日", 1500);
    d.insert("に", "に", ID_PARTICLE, ID_PARTICLE, 500, POS_PARTICLE);
    d.insert("を", "を", ID_PARTICLE, ID_PARTICLE, 500, POS_PARTICLE);
    d.insert("て", "て", ID_PARTICLE, ID_PARTICLE, 800, POS_PARTICLE);
    d.insert("する", "する", ID_SURU, ID_SURU, 800, POS_SURU);
    d.insert("はじめて", "初めて", ID_ADVERB, ID_ADVERB, 1000, POS_ADVERB);
    d.insert("しっ", "知っ", ID_VERB, ID_VERB, 1000, POS_VERB);
    d.insert("た", "た", ID_AUX, ID_AUX, 500, POS_AUX);
    d.insert("たべる", "食べる", ID_VERB, ID_VERB, 1000, POS_VERB);
    // 右ID だけが違う「はし」
    d.insert("はし", "橋", ID_NOUN, ID_NOUN, 1000, POS_NOUN);
    d.insert("はし", "箸", ID_NOUN, ID_NOUN_B, 1000, POS_NOUN);
    d.insert("はし", "端", ID_NOUN, ID_NOUN_C, 1000, POS_NOUN);
    d
}

struct MeiRewriter;

impl Rewriter for MeiRewriter {
    fn name(&self) -> &'static str {
        "test-mei"
    }

    fn rewrite(&self, input: &RewriteInput<'_>, out: &mut Vec<RewriteCandidate>) {
        if input.reading == "めい" {
            out.push(RewriteCandidate::new("MEI".to_owned(), REWRITER_FORMAT));
        }
    }
}

struct FrontRewriter;

impl Rewriter for FrontRewriter {
    fn name(&self) -> &'static str {
        "test-front"
    }

    fn rewrite(&self, input: &RewriteInput<'_>, out: &mut Vec<RewriteCandidate>) {
        if input.reading == "めい" {
            out.push(RewriteCandidate::at_front("F1".to_owned(), 1));
            out.push(RewriteCandidate::at_front("F2".to_owned(), 2));
        }
    }
}

pub(super) fn converter_with(
    ranker: Arc<dyn Ranker>,
    rewriters: Vec<Arc<dyn Rewriter>>,
) -> Converter {
    let g = grammar();
    let d = dictionary(&g);
    let (g, d): (Arc<dyn Grammar>, Arc<dyn Dictionary>) = (Arc::new(g), Arc::new(d));
    Converter::builder(Arc::clone(&d), Arc::clone(&g))
        .ranker(ranker)
        .analyzer(Arc::new(LatticeContextAnalyzer::new(d, g)))
        .rewriters(rewriters)
        .build()
        .unwrap()
}

pub(super) fn converter() -> Converter {
    converter_with(Arc::new(CostRanker), Vec::new())
}

pub(super) fn segmented(conv: &Converter, reading: &str) -> Segmented {
    segmented_in(conv, reading, &Context::new())
}

pub(super) fn segmented_in(conv: &Converter, reading: &str, ctx: &Context) -> Segmented {
    conv.segment(conv.convert_until(reading, ctx, None))
}

pub(super) fn spans(seg: &Segmented) -> Vec<(String, String)> {
    seg.layout()
        .spans
        .iter()
        .map(|s| (s.reading.clone(), s.surface.clone()))
        .collect()
}

pub(super) fn pair(reading: &str, surface: &str) -> (String, String) {
    (reading.to_owned(), surface.to_owned())
}

/// 文節 `seg` の `page` ページ目の表記
pub(super) fn page_surfaces(seg: &mut Segmented, index: usize, page: usize) -> Vec<String> {
    seg.page_until(index, page, &[], None)
        .unwrap()
        .map(|p| p.candidates.iter().map(|c| c.surface.clone()).collect())
        .unwrap_or_default()
}

pub(super) fn all_candidates(seg: &mut Segmented, index: usize) -> Vec<Candidate> {
    let mut page = 0;
    while seg.page_until(index, page, &[], None).unwrap().is_some() {
        page += 1;
    }
    match &seg.lists[index] {
        Some(list) => list.generated().to_vec(),
        None => seg.whole.generated().to_vec(),
    }
}

fn word_cost(d: &dyn Dictionary, word: &Segment) -> i32 {
    match word.word {
        Some(id) => {
            let e = d.entry(id).unwrap();
            i32::from(e.cost) + i32::from(e.penalty)
        }
        None => i32::from(UNK_COST) * word.reading.chars().count() as i32,
    }
}

/// 候補の語から計算し直したコスト（BOS = `bos_right`、EOS = `end_left` の連接を含む）
fn path_cost(
    g: &dyn Grammar,
    d: &dyn Dictionary,
    cand: &Candidate,
    bos_right: u16,
    end_left: u16,
) -> i32 {
    let mut cost = 0;
    let mut prev = bos_right;
    for word in &cand.segments {
        cost += i32::from(g.connection_cost(prev, word.left_id)) + word_cost(d, word);
        prev = word.right_id;
    }
    cost + i32::from(g.connection_cost(prev, end_left))
}

// ---------------------------------------------------------------------------
// 最初の区切り（§2）
// ---------------------------------------------------------------------------

#[test]
fn first_segmentation_follows_the_pos_rules() {
    let conv = converter();
    let seg = segmented(&conv, "ゆーざーじしょにたんごをとうろくする");
    assert_eq!(
        spans(&seg),
        [
            pair("ゆーざーじしょに", "ユーザー辞書に"),
            pair("たんごを", "単語を"),
            pair("とうろくする", "登録する"),
        ]
    );
    assert_eq!(seg.origin(), SegmentedOrigin::Convert);
    assert_eq!(seg.layout().rev, 0);
    // 位置は読みの文字位置
    let positions: Vec<(usize, usize)> = seg
        .layout()
        .spans
        .iter()
        .map(|s| (s.start, s.end))
        .collect();
    assert_eq!(positions, [(0, 8), (8, 12), (12, 18)]);
}

#[test]
fn adverb_and_verb_with_auxiliary() {
    let conv = converter();
    let seg = segmented(&conv, "はじめてしった");
    assert_eq!(
        spans(&seg),
        [pair("はじめて", "初めて"), pair("しった", "知った")]
    );
}

#[test]
fn compound_noun_is_one_clause() {
    let conv = converter();
    let seg = segmented(&conv, "りぽじとりめい");
    assert_eq!(spans(&seg), [pair("りぽじとりめい", "リポジトリ名")]);
}

#[test]
fn the_concatenated_first_view_is_the_top_candidate_of_the_whole() {
    let conv = converter();
    for reading in [
        "ゆーざーじしょにたんごをとうろくする",
        "はじめてしった",
        "りぽじとりめい",
        "はしたべる",
        "めい",
        "あいうえお",
        "",
    ] {
        let whole = conv.convert_until(reading, &Context::new(), None);
        let top = whole.generated().first().map(|c| c.surface.clone());
        let seg = conv.segment(whole);
        let joined: String = seg
            .layout()
            .spans
            .iter()
            .map(|s| s.surface.as_str())
            .collect();
        assert_eq!(Some(joined), top.or(Some(String::new())), "{reading}");
        let readings: String = seg
            .layout()
            .spans
            .iter()
            .map(|s| s.reading.as_str())
            .collect();
        assert_eq!(readings, reading);
    }
}

#[test]
fn non_dictionary_top_candidates_are_one_clause() {
    let conv = converter();
    // 辞書にない読みは、かなの候補（未知語）が 1 位
    let seg = segmented(&conv, "ふぁふぁ");
    assert_eq!(seg.layout().spans.len(), 1);
    assert_eq!(seg.layout().spans[0].reading, "ふぁふぁ");
    // Rewriter の候補が 1 位でも 1 文節
    struct RewriteFirst;
    impl Ranker for RewriteFirst {
        fn rank(&self, _: &RankInput<'_>, cands: &mut [Candidate]) {
            for c in cands {
                if matches!(c.origin, CandidateOrigin::Rewriter { .. }) {
                    c.score = i32::MIN / 2;
                }
            }
        }
        fn name(&self) -> &str {
            "rewrite-first"
        }
    }
    let conv = converter_with(Arc::new(RewriteFirst), vec![Arc::new(MeiRewriter)]);
    let seg = segmented(&conv, "めい");
    assert_eq!(spans(&seg), [pair("めい", "MEI")]);
}

#[test]
fn empty_reading_has_no_clauses() {
    let conv = converter();
    let mut seg = segmented(&conv, "");
    assert!(seg.layout().spans.is_empty());
    assert_eq!(
        seg.page_until(0, 0, &[], None).err(),
        Some(SegmentError::OutOfRange)
    );
    let commit = seg.commit(&[]).unwrap();
    assert_eq!(commit.surface, "");
    assert!(commit.events.is_empty());
}

// ---------------------------------------------------------------------------
// 1文節は全体の変換と同一（§2.3）
// ---------------------------------------------------------------------------

#[test]
fn single_clause_uses_the_whole_conversion_list() {
    let conv = converter();
    let reading = "りぽじとりめい";
    let mut whole = conv.convert_until(reading, &Context::new(), None);
    let mut seg = segmented(&conv, reading);
    assert_eq!(seg.layout().spans.len(), 1);
    let mut page = 0;
    loop {
        let a: Option<Vec<(String, i32)>> = whole.page_until(page, None).map(|p| {
            p.candidates
                .iter()
                .map(|c| (c.surface.clone(), c.score))
                .collect()
        });
        let b: Option<Vec<(String, i32)>> = seg.page_until(0, page, &[], None).unwrap().map(|p| {
            p.candidates
                .iter()
                .map(|c| (c.surface.clone(), c.score))
                .collect()
        });
        assert_eq!(a, b, "page {page}");
        if a.is_none() {
            break;
        }
        page += 1;
    }
    // 読み全体の変換そのものなので、`whole()` と同じ列
    assert_eq!(
        seg.whole()
            .generated()
            .iter()
            .map(|c| c.surface.clone())
            .collect::<Vec<_>>(),
        whole
            .generated()
            .iter()
            .map(|c| c.surface.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn single_clause_commit_equals_the_plain_commit() {
    let conv = converter();
    let reading = "りぽじとりめい";
    let mut whole = conv.convert_until(reading, &Context::new(), None);
    let index = whole
        .page_until(0, None)
        .unwrap()
        .candidates
        .iter()
        .position(|c| c.surface == "リポジトリ銘")
        .unwrap();
    let expected = whole.commit(index).unwrap();
    let mut seg = segmented(&conv, reading);
    let _ = seg.page_until(0, 0, &[], None);
    let got = seg.commit(&[SegmentChoice::Candidate(index)]).unwrap();
    assert_eq!(got.surface, "リポジトリ銘");
    assert_eq!(got.events.len(), 1);
    assert_eq!(got.events[0].reading, expected.reading);
    assert_eq!(got.events[0].candidate.surface, expected.candidate.surface);
    assert_eq!(got.history.len(), 1);
    assert_eq!(got.history[0].reading, reading);
    assert_eq!(got.history[0].surface, "リポジトリ銘");
}

// ---------------------------------------------------------------------------
// 文節ごとの候補（§3）
// ---------------------------------------------------------------------------

#[test]
fn first_candidate_of_every_clause_is_the_displayed_surface() {
    let conv = converter();
    let mut seg = segmented(&conv, "ゆーざーじしょにたんごをとうろくする");
    let displayed: Vec<String> = seg
        .layout()
        .spans
        .iter()
        .map(|s| s.surface.clone())
        .collect();
    for (i, surface) in displayed.iter().enumerate() {
        let first = page_surfaces(&mut seg, i, 0);
        assert_eq!(&first[0], surface, "clause {i}");
    }
}

#[test]
fn every_candidate_reads_exactly_the_clause_reading() {
    let conv = converter_with(Arc::new(CostRanker), vec![Arc::new(MeiRewriter)]);
    let mut seg = segmented(&conv, "ゆーざーじしょにたんごをとうろくする");
    for i in 0..seg.layout().spans.len() {
        let reading = seg.layout().spans[i].reading.clone();
        for cand in all_candidates(&mut seg, i) {
            let joined: String = cand.segments.iter().map(|s| s.reading.as_str()).collect();
            assert_eq!(joined, reading, "clause {i} candidate {}", cand.surface);
        }
    }
}

#[test]
fn candidates_are_unique_and_the_page_is_stable() {
    let conv = converter();
    let mut seg = segmented(&conv, "りぽじとりめいにたんご");
    let first = page_surfaces(&mut seg, 0, 0);
    let all = all_candidates(&mut seg, 0);
    let mut surfaces: Vec<&str> = all.iter().map(|c| c.surface.as_str()).collect();
    let count = surfaces.len();
    surfaces.sort_unstable();
    surfaces.dedup();
    assert_eq!(surfaces.len(), count, "duplicates");
    // 一度返したページは変わらない
    assert_eq!(page_surfaces(&mut seg, 0, 0), first);
}

#[test]
fn right_clause_left_id_is_the_eos_of_the_clause() {
    let conv = converter();
    let g = grammar();
    let d = dictionary(&g);
    // 橋 | 食べる。「はし」の右ID の違いが、動詞への連接として順位に入る
    let mut seg = segmented(&conv, "はしたべる");
    assert_eq!(spans(&seg), [pair("はし", "橋"), pair("たべる", "食べる")]);
    let all = all_candidates(&mut seg, 0);
    let by_surface = |s: &str| all.iter().find(|c| c.surface == s).unwrap();
    let verb_left = ID_VERB;
    for surface in ["橋", "箸", "端"] {
        let cand = by_surface(surface);
        assert_eq!(
            cand.base_cost,
            path_cost(&g, &d, cand, ID_BOS, verb_left),
            "{surface}"
        );
    }
    assert_eq!(by_surface("箸").base_cost - by_surface("橋").base_cost, 300);
    assert_eq!(by_surface("端").base_cost - by_surface("橋").base_cost, 600);
    let order: Vec<&str> = all.iter().map(|c| c.surface.as_str()).take(3).collect();
    assert_eq!(order, ["橋", "箸", "端"]);
    // 右の文節がないなら EOS。右ID の違いは順位に出ない
    let alone = segmented(&conv, "はし");
    let whole = alone.whole().generated();
    let bridge = whole.iter().find(|c| c.surface == "橋").unwrap();
    let chopsticks = whole.iter().find(|c| c.surface == "箸").unwrap();
    assert_eq!(bridge.base_cost, chopsticks.base_cost);
}

#[test]
fn left_clause_last_word_is_the_bos_of_the_clause() {
    let conv = converter();
    let g = grammar();
    let d = dictionary(&g);
    // 初めて | 知った。2 つ目の文節の BOS は「初めて」の右ID
    let mut seg = segmented(&conv, "はじめてしった");
    let all = all_candidates(&mut seg, 1);
    for cand in all
        .iter()
        .filter(|c| c.origin == CandidateOrigin::Dictionary)
    {
        assert_eq!(
            cand.base_cost,
            path_cost(&g, &d, cand, ID_ADVERB, ID_BOS),
            "{}",
            cand.surface
        );
    }
    // 左の選択を変えると BOS が変わる。「はし」の右ID で連接が違う名詞を使う
    let mut seg = segmented(&conv, "はしたべる");
    let all_left = all_candidates(&mut seg, 0);
    let chopsticks = all_left.iter().position(|c| c.surface == "箸").unwrap();
    let mut seg = segmented(&conv, "はしたべる");
    let _ = seg.page_until(0, 0, &[], None).unwrap();
    // 「箸」を選んだ状態で文節 1 を初めて開く
    let _ = seg
        .page_until(1, 0, &[SegmentChoice::Candidate(chopsticks)], None)
        .unwrap()
        .unwrap();
    let list = seg.lists[1].as_ref().unwrap();
    let eaten = list
        .generated()
        .iter()
        .find(|c| c.surface == "食べる")
        .unwrap();
    assert_eq!(eaten.base_cost, path_cost(&g, &d, eaten, ID_NOUN_B, ID_BOS));
    let mut seg = segmented(&conv, "はしたべる");
    let _ = seg.page_until(1, 0, &[], None).unwrap().unwrap();
    let eaten = seg.lists[1]
        .as_ref()
        .unwrap()
        .generated()
        .iter()
        .find(|c| c.surface == "食べる")
        .unwrap();
    assert_eq!(eaten.base_cost, path_cost(&g, &d, eaten, ID_NOUN, ID_BOS));
}

#[test]
fn a_built_list_is_not_rebuilt_when_the_left_choice_changes() {
    let conv = converter();
    let mut seg = segmented(&conv, "はしたべる");
    let before = page_surfaces(&mut seg, 1, 0);
    let left = [SegmentChoice::Candidate(1)];
    let after = seg
        .page_until(1, 0, &left, None)
        .unwrap()
        .unwrap()
        .candidates
        .iter()
        .map(|c| c.surface.clone())
        .collect::<Vec<_>>();
    assert_eq!(before, after);
}

pub(super) struct Recorder {
    pub(super) seen: Mutex<Vec<(String, String, u16)>>,
    pub(super) deadlines: Mutex<Vec<Option<Instant>>>,
}

impl Ranker for Recorder {
    fn rank(&self, input: &RankInput<'_>, _: &mut [Candidate]) {
        self.seen.lock().unwrap().push((
            input.reading.to_owned(),
            input.context.left_text.clone(),
            input.analysis.bos_right_id,
        ));
        self.deadlines.lock().unwrap().push(input.deadline);
    }

    fn name(&self) -> &str {
        "recorder"
    }
}

pub(super) fn recorder() -> Arc<Recorder> {
    Arc::new(Recorder {
        seen: Mutex::new(Vec::new()),
        deadlines: Mutex::new(Vec::new()),
    })
}

#[test]
fn ranker_gets_the_clause_reading_and_the_left_clause_as_context() {
    let rec = recorder();
    let conv = converter_with(rec.clone(), Vec::new());
    let ctx = Context::new().with_left_text("前置き");
    let mut seg = segmented_in(&conv, "はじめてしった", &ctx);
    rec.seen.lock().unwrap().clear();
    let _ = seg.page_until(0, 0, &[], None).unwrap();
    let _ = seg.page_until(1, 0, &[], None).unwrap();
    let seen = rec.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].0, "はじめて");
    assert_eq!(seen[0].1, "前置き");
    assert_eq!(seen[1].0, "しった");
    assert_eq!(seen[1].1, "前置き初めて");
    assert_eq!(seen[1].2, ID_ADVERB);
}

#[test]
fn literal_left_choice_becomes_context_text() {
    let rec = recorder();
    let conv = converter_with(rec.clone(), Vec::new());
    let mut seg = segmented(&conv, "はじめてしった");
    rec.seen.lock().unwrap().clear();
    let left = [SegmentChoice::Literal("ハジメテ".to_owned())];
    let _ = seg.page_until(1, 0, &left, None).unwrap();
    let seen = rec.seen.lock().unwrap().clone();
    assert_eq!(seen[0].1, "ハジメテ");
}

#[test]
fn secure_keeps_the_context_empty() {
    let rec = recorder();
    let conv = converter_with(rec.clone(), Vec::new());
    let ctx = Context::new().with_left_text("秘密").with_secure(true);
    let mut seg = segmented_in(&conv, "はじめてしった", &ctx);
    rec.seen.lock().unwrap().clear();
    let _ = seg.page_until(1, 0, &[], None).unwrap();
    let seen = rec.seen.lock().unwrap().clone();
    assert_eq!(seen[0].1, "");
    assert!(
        seg.commit(&vec![SegmentChoice::Candidate(0); 2])
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn deadline_reaches_the_clause_ranker() {
    let rec = recorder();
    let conv = converter_with(rec.clone(), Vec::new());
    let mut seg = segmented(&conv, "はじめてしった");
    rec.deadlines.lock().unwrap().clear();
    let d = Instant::now() + Duration::from_secs(1);
    let _ = seg.page_until(1, 0, &[], Some(d)).unwrap();
    assert_eq!(*rec.deadlines.lock().unwrap(), vec![Some(d)]);
}

#[test]
fn page_arguments_are_checked() {
    let conv = converter();
    let mut seg = segmented(&conv, "はじめてしった");
    assert_eq!(
        seg.page_until(2, 0, &[], None).err(),
        Some(SegmentError::OutOfRange)
    );
    // left が長すぎる、選択が範囲外
    let too_long = [SegmentChoice::Candidate(0), SegmentChoice::Candidate(0)];
    assert_eq!(
        seg.page_until(1, 0, &too_long, None).err(),
        Some(SegmentError::OutOfRange)
    );
    let far = [SegmentChoice::Candidate(9999)];
    assert_eq!(
        seg.page_until(1, 0, &far, None).err(),
        Some(SegmentError::OutOfRange)
    );
    assert_eq!(
        seg.page_until(1, 0, &[SegmentChoice::Literal(String::new())], None)
            .err(),
        Some(SegmentError::OutOfRange)
    );
    // 失敗しても列は作られない（有効な left でやり直せる）
    assert!(seg.lists[1].is_none());
    // 範囲外のページは None
    assert!(seg.page_until(1, 99, &[], None).unwrap().is_none());
    assert!(seg.page_until(1, 0, &[], None).unwrap().is_some());
}

#[test]
fn head_is_pinned_even_when_the_pool_does_not_contain_it() {
    let conv = converter();
    let mut seg = segmented(&conv, "はじめてしった");
    // 表示中の表記を、辞書の候補にない語に差し替える（読みは同じ）
    let unk = grammar().unknown();
    seg.heads[1] = Candidate {
        surface: "シッタ?".to_owned(),
        segments: vec![Segment {
            reading: "しった".to_owned(),
            surface: "シッタ?".to_owned(),
            word: None,
            left_id: unk.left_id,
            right_id: unk.right_id,
            pos_id: unk.pos_id,
        }],
        base_cost: 0,
        score: 0,
        origin: CandidateOrigin::Dictionary,
    };
    seg.layout.spans[1].surface = "シッタ?".to_owned();
    let all = all_candidates(&mut seg, 1);
    assert_eq!(all[0].surface, "シッタ?");
    assert_eq!(all.iter().filter(|c| c.surface == "シッタ?").count(), 1);
    assert!(all.iter().any(|c| c.surface == "知った"));
}

#[test]
fn head_is_pinned_when_the_ranker_disagrees() {
    // 読み全体のときだけ「箸」を押し上げる Ranker。区切りの元（全体の 1 位）は「箸 | 食べる」で、
    // 文節「はし」の並べ替えでは「橋」が先頭になるが、0 番は表示中の「箸」のまま
    struct ChopsticksInSentences;
    impl Ranker for ChopsticksInSentences {
        fn rank(&self, input: &RankInput<'_>, cands: &mut [Candidate]) {
            if input.reading.chars().count() <= 2 {
                return;
            }
            for c in cands {
                if c.surface.contains('箸') {
                    c.score -= 100_000;
                }
            }
        }
        fn name(&self) -> &str {
            "chopsticks"
        }
    }
    let conv = converter_with(Arc::new(ChopsticksInSentences), Vec::new());
    let mut seg = segmented(&conv, "はしたべる");
    assert_eq!(spans(&seg), [pair("はし", "箸"), pair("たべる", "食べる")]);
    let all = all_candidates(&mut seg, 0);
    assert_eq!(all[0].surface, "箸");
    assert_eq!(all[1].surface, "橋");
    assert_eq!(all.iter().filter(|c| c.surface == "箸").count(), 1);
    // 全体の 1 位はそのまま
    assert_eq!(seg.whole().generated()[0].surface, "箸食べる");
}

// ---------------------------------------------------------------------------
// 確定と学習（§6）
// ---------------------------------------------------------------------------

#[test]
fn commit_checks_the_choices() {
    let conv = converter();
    let mut seg = segmented(&conv, "はじめてしった");
    let _ = seg.page_until(0, 0, &[], None);
    assert_eq!(
        seg.commit(&[SegmentChoice::Candidate(0)]).err(),
        Some(SegmentError::OutOfRange)
    );
    assert_eq!(
        seg.commit(&[
            SegmentChoice::Candidate(0),
            SegmentChoice::Candidate(0),
            SegmentChoice::Candidate(0)
        ])
        .err(),
        Some(SegmentError::OutOfRange)
    );
    // まだ開いていない文節の 0 番以外は選べない
    assert_eq!(
        seg.commit(&[SegmentChoice::Candidate(0), SegmentChoice::Candidate(1)])
            .err(),
        Some(SegmentError::OutOfRange)
    );
    assert_eq!(
        seg.commit(&[SegmentChoice::Candidate(9999), SegmentChoice::Candidate(0)])
            .err(),
        Some(SegmentError::OutOfRange)
    );
}

#[test]
fn commit_without_changes_records_only_the_whole() {
    let conv = converter();
    let seg = segmented(&conv, "ゆーざーじしょにたんごをとうろくする");
    let choices = vec![SegmentChoice::Candidate(0); 3];
    let commit = seg.commit(&choices).unwrap();
    assert_eq!(commit.surface, "ユーザー辞書に単語を登録する");
    // 読み全体だけ。変えていない文節は個別に記録しない
    assert_eq!(commit.events.len(), 1);
    let whole = &commit.events[0];
    assert_eq!(whole.reading, "ゆーざーじしょにたんごをとうろくする");
    assert_eq!(whole.candidate.surface, "ユーザー辞書に単語を登録する");
    let words: Vec<&str> = whole
        .candidate
        .segments
        .iter()
        .map(|s| s.surface.as_str())
        .collect();
    assert_eq!(
        words,
        ["ユーザー", "辞書", "に", "単語", "を", "登録", "する"]
    );
    assert_eq!(commit.history.len(), 1);
    assert_eq!(
        commit.history[0].reading,
        "ゆーざーじしょにたんごをとうろくする"
    );
}

#[test]
fn commit_records_changed_clauses_with_the_left_clause_as_context() {
    let conv = converter();
    let mut seg = segmented(&conv, "ゆーざーじしょにたんごをとうろくする");
    let pages = all_candidates(&mut seg, 1);
    let other = pages.iter().position(|c| c.surface == "単語を").unwrap();
    assert_eq!(other, 0);
    // 文節 2 を開いて別の候補（平仮名）を選ぶ
    let list = all_candidates(&mut seg, 2);
    let hiragana = list
        .iter()
        .position(|c| c.surface == "とうろくする")
        .unwrap();
    assert!(hiragana > 0);
    let commit = seg
        .commit(&[
            SegmentChoice::Candidate(0),
            SegmentChoice::Candidate(0),
            SegmentChoice::Candidate(hiragana),
        ])
        .unwrap();
    assert_eq!(commit.surface, "ユーザー辞書に単語をとうろくする");
    assert_eq!(commit.events.len(), 2);
    assert_eq!(
        commit.events[0].reading,
        "ゆーざーじしょにたんごをとうろくする"
    );
    let changed = &commit.events[1];
    assert_eq!(changed.reading, "とうろくする");
    assert_eq!(changed.candidate.surface, "とうろくする");
    // 左文脈 = 文節 0, 1 の語。末尾が「を」
    let tokens: Vec<&str> = changed
        .analysis
        .tokens
        .iter()
        .map(|t| t.surface.as_str())
        .collect();
    assert_eq!(tokens, ["ユーザー", "辞書", "に", "単語", "を"]);
    assert_eq!(changed.analysis.bos_right_id, ID_PARTICLE);
}

#[test]
fn a_rewriter_or_literal_clause_keeps_the_whole_out_of_learning() {
    let conv = converter_with(Arc::new(CostRanker), vec![Arc::new(MeiRewriter)]);
    let mut seg = segmented(&conv, "ゆーざーじしょにたんごをとうろくする");
    let _ = seg.page_until(0, 0, &[], None);
    let literal = seg
        .commit(&[
            SegmentChoice::Candidate(0),
            SegmentChoice::Literal("タンゴヲ".to_owned()),
            SegmentChoice::Candidate(0),
        ])
        .unwrap();
    assert_eq!(literal.surface, "ユーザー辞書にタンゴヲ登録する");
    // Literal は学習しない。読み全体も覚えない（表記が殻のもの）
    assert!(literal.events.is_empty());
    assert_eq!(literal.history.len(), 1);

    // Rewriter の候補がある文節
    let mut seg = segmented(&conv, "ゆーざーじしょにめい");
    assert_eq!(seg.layout().spans.len(), 2);
    let list = all_candidates(&mut seg, 1);
    let rewriter = list
        .iter()
        .position(|c| matches!(c.origin, CandidateOrigin::Rewriter { .. }))
        .unwrap();
    let commit = seg
        .commit(&[
            SegmentChoice::Candidate(0),
            SegmentChoice::Candidate(rewriter),
        ])
        .unwrap();
    assert_eq!(commit.surface, "ユーザー辞書にMEI");
    // 読み全体は記録しない。変えた文節だけを書式で記録する
    assert_eq!(commit.events.len(), 1);
    assert_eq!(commit.events[0].reading, "めい");
    assert!(matches!(
        commit.events[0].candidate.origin,
        CandidateOrigin::Rewriter {
            format: REWRITER_FORMAT,
            ..
        }
    ));
}

#[test]
fn single_clause_rewriter_is_recorded_like_the_plain_commit() {
    let conv = converter_with(Arc::new(CostRanker), vec![Arc::new(MeiRewriter)]);
    let mut seg = segmented(&conv, "めい");
    let list = all_candidates(&mut seg, 0);
    let rewriter = list
        .iter()
        .position(|c| matches!(c.origin, CandidateOrigin::Rewriter { .. }))
        .unwrap();
    let commit = seg.commit(&[SegmentChoice::Candidate(rewriter)]).unwrap();
    assert_eq!(commit.events.len(), 1);
    assert!(matches!(
        commit.events[0].candidate.origin,
        CandidateOrigin::Rewriter { .. }
    ));
}

#[test]
fn secure_commit_returns_the_surface_only() {
    let conv = converter();
    let ctx = Context::new().with_secure(true);
    let seg = segmented_in(&conv, "はじめてしった", &ctx);
    let commit = seg.commit(&vec![SegmentChoice::Candidate(0); 2]).unwrap();
    assert_eq!(commit.surface, "初めて知った");
    assert!(commit.events.is_empty());
    assert!(commit.history.is_empty());
}

fn learned_rows(store: &LearningStore) -> Vec<(String, String, u64, u32)> {
    store
        .records()
        .into_iter()
        .map(|r| (r.reading, r.surface, r.feature, r.count))
        .collect()
}

/// 変えた文節の記録は、左の文節を先に確定してからその文節を変換して確定したときと同じ行になる
#[test]
fn changed_clause_is_learned_like_a_frequent_commit() {
    let conv = converter();
    let g: Arc<dyn Grammar> = Arc::new(grammar());
    let dir = tempfile::tempdir().unwrap();

    // 文節の UI: 「りぽじとり」「めい」に区切り直して、めいを「銘」にする
    let mut seg = segmented(&conv, "りぽじとりめい");
    conv.resize(&mut seg, 0, -1, &[], None).unwrap();
    conv.resize(&mut seg, 0, -1, &[], None).unwrap();
    assert_eq!(
        spans(&seg),
        [pair("りぽじとり", "リポジトリ"), pair("めい", "名")]
    );
    let list = all_candidates(&mut seg, 1);
    let mei = list.iter().position(|c| c.surface == "銘").unwrap();
    let commit = seg
        .commit(&[SegmentChoice::Candidate(0), SegmentChoice::Candidate(mei)])
        .unwrap();
    assert_eq!(commit.surface, "リポジトリ銘");
    let span_event = commit
        .events
        .iter()
        .find(|e| e.reading == "めい")
        .expect("changed clause event");
    let ui_store = LearningStore::open(&dir.path().join("ui.tsv")).unwrap();
    ui_store.attach_grammar(Arc::clone(&g));
    ui_store.record(span_event, None, 1);

    // こまめな確定: 「リポジトリ」を確定した後に「めい」を変換して「銘」を選ぶ
    let ctx = Context::new().with_left_text("リポジトリ");
    let mut plain = conv.convert_until("めい", &ctx, None);
    let _ = plain.page_until(0, None);
    let index = plain
        .generated()
        .iter()
        .position(|c| c.surface == "銘")
        .unwrap();
    // 特徴量のハッシュは salt に依るので、同じ salt の store（保存したファイルを開き直し、行を消したもの）を使う
    ui_store.save().unwrap();
    let plain_store = LearningStore::open(&dir.path().join("ui.tsv")).unwrap();
    plain_store.attach_grammar(Arc::clone(&g));
    plain_store.reset(crate::user::LayerSel::All);
    assert!(plain_store.is_empty());
    plain_store.record(&plain.commit(index).unwrap(), None, 1);

    let ui = learned_rows(&ui_store);
    assert!(!ui.is_empty());
    assert_eq!(ui, learned_rows(&plain_store));
}

// ---------------------------------------------------------------------------
// 区切り直し（§4）
// ---------------------------------------------------------------------------

#[test]
fn shrinking_twice_splits_the_compound_noun() {
    let conv = converter();
    let mut seg = segmented(&conv, "りぽじとりめい");
    conv.resize(&mut seg, 0, -1, &[], None).unwrap();
    assert_eq!(seg.layout().rev, 1);
    // りぽじとりめ | い。1文字ずつの未知語になる
    assert_eq!(seg.layout().spans.len(), 2);
    assert_eq!(seg.layout().spans[0].reading, "りぽじとりめ");
    assert_eq!(seg.layout().spans[1].reading, "い");
    conv.resize(&mut seg, 0, -1, &[], None).unwrap();
    assert_eq!(seg.layout().rev, 2);
    assert_eq!(
        spans(&seg),
        [pair("りぽじとり", "リポジトリ"), pair("めい", "名")]
    );
    // フォーカスの文節の 1 ページ目は、この呼び出しで作ってある
    assert!(seg.lists[0].is_some());
    assert!(seg.lists[1].is_none());
    let names = page_surfaces(&mut seg, 1, 0);
    assert_eq!(&names[..4], ["名", "銘", "姪", "めい"]);
    // 伸ばして戻せる
    conv.resize(&mut seg, 0, 1, &[], None).unwrap();
    assert_eq!(seg.layout().rev, 3);
    assert_eq!(seg.layout().spans[0].reading, "りぽじとりめ");
    conv.resize(&mut seg, 0, 1, &[], None).unwrap();
    assert_eq!(spans(&seg), [pair("りぽじとりめい", "リポジトリ名")]);
}

#[test]
fn resized_clause_candidates_follow_the_clause_rules() {
    let conv = converter();
    let mut seg = segmented(&conv, "りぽじとりめい");
    conv.resize(&mut seg, 0, -1, &[], None).unwrap();
    conv.resize(&mut seg, 0, -1, &[], None).unwrap();
    for i in 0..seg.layout().spans.len() {
        let reading = seg.layout().spans[i].reading.clone();
        let surface = seg.layout().spans[i].surface.clone();
        let all = all_candidates(&mut seg, i);
        assert_eq!(all[0].surface, surface);
        for cand in &all {
            let joined: String = cand.segments.iter().map(|s| s.reading.as_str()).collect();
            assert_eq!(joined, reading);
        }
    }
}

#[test]
fn no_word_crosses_the_forced_boundary() {
    // 「ぼうしだ」という長い語がある辞書で、境界をまたがない
    let mut g = grammar();
    g.add_pos("名詞,固有名詞,一般,*,*,*");
    let mut d = dictionary(&g);
    d.insert("ぼうしだ", "帽子田", ID_NOUN, ID_NOUN, 500, POS_NOUN);
    d.insert("ぼう", "棒", ID_NOUN, ID_NOUN, 1500, POS_NOUN);
    d.insert("しだ", "志田", ID_NOUN, ID_NOUN, 1500, POS_NOUN);
    d.insert("し", "市", ID_NOUN, ID_NOUN, 1500, POS_NOUN);
    let (g, d): (Arc<dyn Grammar>, Arc<dyn Dictionary>) = (Arc::new(g), Arc::new(d));
    let conv = Converter::builder(d, g).build().unwrap();
    let mut seg = segmented(&conv, "ぼうしだ");
    assert_eq!(spans(&seg), [pair("ぼうしだ", "帽子田")]);
    conv.resize(&mut seg, 0, -1, &[], None).unwrap();
    // 3 文字の「ぼうし」と「だ」。「ぼうしだ」は使えない
    assert_eq!(seg.layout().spans[0].reading, "ぼうし");
    assert_eq!(seg.layout().spans[1].reading, "だ");
    conv.resize(&mut seg, 0, -1, &[], None).unwrap();
    assert_eq!(seg.layout().spans[0].reading, "ぼう");
    assert_eq!(seg.layout().spans[0].surface, "棒");
    assert_eq!(seg.layout().spans[1].reading, "しだ");
    assert_eq!(seg.layout().spans[1].surface, "志田");
}

#[test]
fn spans_before_the_focus_are_untouched() {
    let conv = converter();
    let mut seg = segmented(&conv, "ゆーざーじしょにたんごをとうろくする");
    let first_page = page_surfaces(&mut seg, 0, 0);
    let left = [SegmentChoice::Candidate(0)];
    // 文節 1（たんごを）を 1 文字縮める
    conv.resize(&mut seg, 1, -1, &left, None).unwrap();
    assert_eq!(seg.layout().rev, 1);
    assert_eq!(seg.layout().spans[0].surface, "ユーザー辞書に");
    assert_eq!(seg.layout().spans[1].reading, "たんご");
    assert_eq!(
        seg.layout()
            .spans
            .iter()
            .map(|s| s.reading.as_str())
            .collect::<String>(),
        "ゆーざーじしょにたんごをとうろくする"
    );
    // 文節 0 の候補の列は変わらない
    assert_eq!(page_surfaces(&mut seg, 0, 0), first_page);
    // 位置がつながっている
    for pair in seg.layout().spans.windows(2) {
        assert_eq!(pair[0].end, pair[1].start);
    }
    assert_eq!(seg.layout().spans.last().unwrap().end, 18);
}

#[test]
fn resize_does_nothing_when_it_cannot_move() {
    let conv = converter();
    let mut seg = segmented(&conv, "はじめてしった");
    // 最後の文節は伸ばせない
    conv.resize(&mut seg, 1, 1, &[], None).unwrap();
    assert_eq!(seg.layout().rev, 0);
    // 0 の変化量
    conv.resize(&mut seg, 0, 0, &[], None).unwrap();
    assert_eq!(seg.layout().rev, 0);
    // 1 文字の文節は縮められない
    let mut seg = segmented(&conv, "はじめてしった");
    conv.resize(&mut seg, 0, -3, &[], None).unwrap();
    assert_eq!(seg.layout().rev, 1);
    conv.resize(&mut seg, 0, -2, &[], None).unwrap();
    assert_eq!(seg.layout().rev, 1);
    assert_eq!(seg.layout().spans[0].reading, "は");
    conv.resize(&mut seg, 0, -1, &[], None).unwrap();
    assert_eq!(seg.layout().rev, 1);
    // 範囲外の文節
    assert_eq!(
        conv.resize(&mut seg, 9, 1, &[], None).err(),
        Some(SegmentError::OutOfRange)
    );
    // left が長すぎる（変えない）
    let rev = seg.layout().rev;
    let long = vec![SegmentChoice::Candidate(0); 3];
    assert_eq!(
        conv.resize(&mut seg, 0, 1, &long, None).err(),
        Some(SegmentError::OutOfRange)
    );
    assert_eq!(seg.layout().rev, rev);
}

#[test]
fn expanding_takes_a_character_from_the_next_clause() {
    let conv = converter();
    let mut seg = segmented(&conv, "はじめてしった");
    conv.resize(&mut seg, 0, 1, &[], None).unwrap();
    assert_eq!(seg.layout().spans[0].reading, "はじめてし");
    assert_eq!(seg.layout().spans[0].end, 5);
    assert_eq!(seg.layout().spans[1].start, 5);
    let readings: String = seg
        .layout()
        .spans
        .iter()
        .map(|s| s.reading.as_str())
        .collect();
    assert_eq!(readings, "はじめてしった");
}

#[test]
fn resize_is_computed_without_waiting_for_the_lm() {
    let rec = recorder();
    let conv = converter_with(rec.clone(), Vec::new());
    let mut seg = segmented(&conv, "りぽじとりめい");
    rec.deadlines.lock().unwrap().clear();
    conv.resize(&mut seg, 0, -1, &[], None).unwrap();
    let deadlines = rec.deadlines.lock().unwrap().clone();
    assert!(!deadlines.is_empty());
    assert!(deadlines.iter().all(Option::is_none));
}

#[test]
fn resized_commit_after_the_old_choices() {
    // 区切り直しのあと、確定の選択は新しい列を指す
    let conv = converter();
    let mut seg = segmented(&conv, "りぽじとりめい");
    conv.resize(&mut seg, 0, -1, &[], None).unwrap();
    conv.resize(&mut seg, 0, -1, &[], None).unwrap();
    let names = all_candidates(&mut seg, 1);
    let mei = names.iter().position(|c| c.surface == "姪").unwrap();
    let commit = seg
        .commit(&[SegmentChoice::Candidate(0), SegmentChoice::Candidate(mei)])
        .unwrap();
    assert_eq!(commit.surface, "リポジトリ姪");
}

#[test]
fn word_lookup_of_heads_matches_the_dictionary() {
    // 0 番の語は辞書の語IDを持ち、表記が辞書と一致する
    let conv = converter();
    let g = grammar();
    let d = dictionary(&g);
    let seg = segmented(&conv, "ゆーざーじしょにたんごをとうろくする");
    for head in &seg.heads {
        for word in &head.segments {
            let id: WordId = word.word.unwrap();
            assert_eq!(d.entry(id).unwrap().surface, word.surface);
        }
    }
}

/// 副詞 そう、名詞 時期、それを覆う名詞 掃除機 を足した変換器（ADR 0008）。
/// そう＋時期 = 1000 + 400 + 1000 = 2400、掃除機 = 4000
fn adverb_converter(rule: AdverbNounRule) -> Converter {
    let g = grammar();
    let mut d = dictionary(&g);
    d.insert("そう", "そう", ID_ADVERB, ID_ADVERB, 1000, POS_ADVERB);
    d.insert("じき", "時期", ID_NOUN, ID_NOUN, 1000, POS_NOUN);
    d.insert("そうじき", "掃除機", ID_NOUN, ID_NOUN, 4000, POS_NOUN);
    let (g, d): (Arc<dyn Grammar>, Arc<dyn Dictionary>) = (Arc::new(g), Arc::new(d));
    Converter::builder(Arc::clone(&d), Arc::clone(&g))
        .analyzer(Arc::new(LatticeContextAnalyzer::new(d, g)))
        .options(ConvertOptions {
            adverb_noun: rule,
            ..ConvertOptions::default()
        })
        .build()
        .unwrap()
}

fn joined(seg: &Segmented) -> String {
    seg.layout()
        .spans
        .iter()
        .map(|s| s.surface.as_str())
        .collect()
}

#[test]
fn adverb_noun_rule_makes_one_clause_and_keeps_the_split_as_a_candidate() {
    let conv = adverb_converter(AdverbNounRule::default());
    let mut seg = segmented(&conv, "そうじき");
    assert_eq!(spans(&seg), [pair("そうじき", "掃除機")]);
    // 文節をつないだもの = 全体の1位（ADR 0006 の不変条件）
    assert_eq!(joined(&seg), seg.whole().generated()[0].surface);
    // 加算した経路は候補から消えない
    let all = all_candidates(&mut seg, 0);
    assert!(all.iter().any(|c| c.surface == "そう時期"), "{all:?}");

    // 規則を止めれば、今までどおり そう｜時期 の2文節
    let off = adverb_converter(AdverbNounRule::disabled());
    let seg = segmented(&off, "そうじき");
    assert_eq!(spans(&seg), [pair("そう", "そう"), pair("じき", "時期")]);
    assert_eq!(joined(&seg), seg.whole().generated()[0].surface);
}

#[test]
fn user_resize_inside_the_covering_noun_gives_adverb_plus_noun() {
    // ユーザーが そう｜じき に区切れば、掃除機 は覆う名詞にならず、そう＋時期 が出る（ADR 0008 §4.3）
    let conv = adverb_converter(AdverbNounRule::default());
    let mut seg = segmented(&conv, "そうじき");
    conv.resize(&mut seg, 0, -2, &[], None).unwrap();
    assert_eq!(spans(&seg), [pair("そう", "そう"), pair("じき", "時期")]);
    // 右の文節 じき の候補は、左の文節 そう を左文脈（BOS）にして作る。BOS の辺には足さない
    assert_eq!(page_surfaces(&mut seg, 1, 0)[0], "時期");
}

#[test]
fn adverb_followed_by_a_noun_in_the_left_context_is_not_penalized() {
    // こまめに確定した そう のあとの じき は 時期 のまま（左文脈は BOS の辺。ADR 0008 §2.1）
    let conv = adverb_converter(AdverbNounRule::default());
    let ctx = Context::new().with_left_text("そう");
    let conv_result = conv.convert_until("じき", &ctx, None);
    assert_eq!(conv_result.generated()[0].surface, "時期");
}

#[test]
fn front_rewriter_candidates_come_first_in_order() {
    let conv = converter_with(Arc::new(CostRanker), vec![Arc::new(FrontRewriter)]);
    let mut seg = segmented(&conv, "めい");
    let list = all_candidates(&mut seg, 0);
    assert_eq!(list[0].surface, "F1");
    assert_eq!(list[1].surface, "F2");
}

// ---- Segmented::remove（ADR 0050 §5.3）----

const THREE: &str = "ゆーざーじしょにたんごをとうろくする";

#[test]
fn remove_takes_the_tail_clause_and_keeps_the_rest() {
    let conv = converter();
    let mut seg = segmented(&conv, THREE);
    let first = page_surfaces(&mut seg, 0, 0);
    seg.remove(2, 1).unwrap();
    assert_eq!(
        spans(&seg),
        [
            pair("ゆーざーじしょに", "ユーザー辞書に"),
            pair("たんごを", "単語を")
        ]
    );
    assert_eq!(seg.layout().rev, 1);
    assert_eq!(seg.lists.len(), 2);
    assert_eq!(seg.heads.len(), 2);
    // 残りの候補の列は変わらない
    assert_eq!(page_surfaces(&mut seg, 0, 0), first);
}

#[test]
fn remove_from_the_middle_and_the_head_recomputes_positions() {
    let conv = converter();
    let mut seg = segmented(&conv, THREE);
    seg.remove(1, 1).unwrap();
    assert_eq!(
        spans(&seg),
        [
            pair("ゆーざーじしょに", "ユーザー辞書に"),
            pair("とうろくする", "登録する")
        ]
    );
    let positions: Vec<(usize, usize)> = seg
        .layout()
        .spans
        .iter()
        .map(|s| (s.start, s.end))
        .collect();
    assert_eq!(positions, [(0, 8), (8, 14)]);
    seg.remove(0, 1).unwrap();
    assert_eq!(spans(&seg), [pair("とうろくする", "登録する")]);
    assert_eq!(seg.layout().spans[0].start, 0);
    assert_eq!(seg.layout().rev, 2);
}

#[test]
fn remove_several_clauses_at_once() {
    let conv = converter();
    let mut seg = segmented(&conv, THREE);
    seg.remove(1, 2).unwrap();
    assert_eq!(spans(&seg), [pair("ゆーざーじしょに", "ユーザー辞書に")]);
    assert_eq!(seg.layout().rev, 1);
}

#[test]
fn remove_keeps_a_built_list_and_builds_the_unbuilt_ones_after_the_removal() {
    let conv = converter();
    let mut seg = segmented(&conv, THREE);
    let tail_before = page_surfaces(&mut seg, 2, 0);
    assert!(seg.lists[2].is_some());
    seg.remove(1, 1).unwrap();
    // 作ってあった列は、位置が詰まってもそのまま
    assert!(seg.lists[0].is_none());
    assert_eq!(page_surfaces(&mut seg, 1, 0), tail_before);
    // 作っていない列は、取り除いたあとの並びで作れる（0 番は表示中の表記）
    assert_eq!(page_surfaces(&mut seg, 0, 0)[0], "ユーザー辞書に");
}

#[test]
fn remove_leaves_a_single_clause_that_no_longer_uses_the_whole_list() {
    let conv = converter();
    let mut seg = segmented(&conv, THREE);
    seg.remove(1, 2).unwrap();
    // 版が上がっているので、読み全体の列を流用しない
    let surfaces = page_surfaces(&mut seg, 0, 0);
    assert_eq!(surfaces[0], "ユーザー辞書に");
    let commit = seg.commit(&[SegmentChoice::Candidate(0)]).unwrap();
    assert_eq!(commit.surface, "ユーザー辞書に");
    assert_eq!(commit.history.len(), 1);
    assert_eq!(commit.history[0].reading, "ゆーざーじしょに");
}

#[test]
fn commit_after_remove_covers_only_the_remaining_clauses() {
    let conv = converter();
    let mut seg = segmented(&conv, THREE);
    seg.remove(2, 1).unwrap();
    let commit = seg
        .commit(&[SegmentChoice::Candidate(0), SegmentChoice::Candidate(0)])
        .unwrap();
    assert_eq!(commit.surface, "ユーザー辞書に単語を");
    assert_eq!(commit.history[0].reading, "ゆーざーじしょにたんごを");
    // 取り除く前の数の選択は範囲外
    assert_eq!(
        seg.commit(&vec![SegmentChoice::Candidate(0); 3])
            .unwrap_err(),
        SegmentError::OutOfRange
    );
}

#[test]
fn remove_rejects_bad_ranges_and_changes_nothing() {
    let conv = converter();
    let mut seg = segmented(&conv, THREE);
    for (first, count) in [(3, 1), (0, 0), (2, 2), (usize::MAX, 1), (1, usize::MAX)] {
        assert_eq!(
            seg.remove(first, count).unwrap_err(),
            SegmentError::OutOfRange,
            "{first} {count}"
        );
    }
    // 全部は Empty
    assert_eq!(seg.remove(0, 3).unwrap_err(), SegmentError::Empty);
    assert_eq!(seg.layout().spans.len(), 3);
    assert_eq!(seg.layout().rev, 0);
    assert_eq!(seg.lists.len(), 3);
}

#[test]
fn remove_on_an_empty_or_single_clause_is_empty_or_out_of_range() {
    let conv = converter();
    let mut one = segmented(&conv, "ゆーざーじしょに");
    assert_eq!(one.remove(0, 1).unwrap_err(), SegmentError::Empty);
    assert_eq!(one.layout().rev, 0);
    let mut none = segmented(&conv, "");
    assert_eq!(none.remove(0, 1).unwrap_err(), SegmentError::OutOfRange);
}

#[test]
fn remove_after_resize_works_on_the_resized_layout() {
    let conv = converter();
    let mut seg = segmented(&conv, THREE);
    conv.resize(&mut seg, 1, -1, &[SegmentChoice::Candidate(0)], None)
        .unwrap();
    let rev = seg.layout().rev;
    let last = seg.layout().spans.len() - 1;
    seg.remove(last, 1).unwrap();
    assert_eq!(seg.layout().rev, rev + 1);
    assert_eq!(seg.layout().spans.len(), last);
}
