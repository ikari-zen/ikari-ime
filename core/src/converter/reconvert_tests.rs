//! 再変換（ADR 0006 §9、§10.1）のテスト。fixture は `segment_tests` と共有する。

use std::sync::Arc;

use super::segment_tests::{
    all_candidates, converter, converter_with, page_surfaces, pair, recorder, spans,
};
use super::*;
use crate::candidate::{Candidate, CandidateOrigin};
use crate::context::{Commit, Context};
use crate::ranker::{RankInput, Ranker};

fn reconverted(conv: &Converter, surface: &str, hint: &[Commit]) -> Segmented {
    conv.reconvert(surface, &Context::new(), hint, None)
        .unwrap()
}

fn commit_item(reading: &str, surface: &str) -> Commit {
    Commit {
        reading: reading.to_owned(),
        surface: surface.to_owned(),
    }
}

#[test]
fn reads_the_surface_through_the_dictionary() {
    let conv = converter();
    let mut seg = reconverted(&conv, "品詞", &[]);
    assert_eq!(seg.origin(), SegmentedOrigin::Reconvert);
    assert_eq!(spans(&seg), [pair("ひんし", "品詞")]);
    let all = all_candidates(&mut seg, 0);
    // 0 番は元の表記。瀕死も選べる
    assert_eq!(all[0].surface, "品詞");
    assert!(all.iter().any(|c| c.surface == "瀕死"));
    for cand in &all {
        let reading: String = cand.segments.iter().map(|s| s.reading.as_str()).collect();
        assert_eq!(reading, "ひんし");
    }
    // 読み全体の変換は空
    assert!(seg.whole().generated().is_empty());
}

#[test]
fn picks_the_best_reading_for_a_homograph() {
    let conv = converter();
    let seg = reconverted(&conv, "今日", &[]);
    assert_eq!(spans(&seg), [pair("きょう", "今日")]);
}

#[test]
fn keeps_the_original_surface_first_even_if_the_ranker_disagrees() {
    struct PreferAlternatives;
    impl Ranker for PreferAlternatives {
        fn rank(&self, _: &RankInput<'_>, cands: &mut [Candidate]) {
            for c in cands {
                if c.surface == "瀕死" {
                    c.score -= 100_000;
                }
            }
        }
        fn name(&self) -> &str {
            "prefer-alternatives"
        }
    }
    let conv = converter_with(Arc::new(PreferAlternatives), Vec::new());
    let mut seg = reconverted(&conv, "品詞", &[]);
    let all = all_candidates(&mut seg, 0);
    assert_eq!(all[0].surface, "品詞");
    assert_eq!(all[1].surface, "瀕死");
}

#[test]
fn splits_the_surface_into_clauses_like_a_conversion() {
    let conv = converter();
    let seg = reconverted(&conv, "ユーザー辞書に単語を", &[]);
    assert_eq!(
        spans(&seg),
        [
            pair("ゆーざーじしょに", "ユーザー辞書に"),
            pair("たんごを", "単語を")
        ]
    );
    let positions: Vec<(usize, usize)> = seg
        .layout()
        .spans
        .iter()
        .map(|s| (s.start, s.end))
        .collect();
    assert_eq!(positions, [(0, 8), (8, 12)]);
}

#[test]
fn contiguous_history_items_are_clause_boundaries() {
    let conv = converter();
    // 新しい順。「リポジトリ」→「名」と確定した。履歴がなければ複合名詞 1 文節になる
    let hint = [
        commit_item("めい", "名"),
        commit_item("りぽじとり", "リポジトリ"),
    ];
    let seg = reconverted(&conv, "リポジトリ名", &hint);
    assert_eq!(
        spans(&seg),
        [pair("りぽじとり", "リポジトリ"), pair("めい", "名")]
    );
    let plain = reconverted(&conv, "リポジトリ名", &[]);
    assert_eq!(spans(&plain), [pair("りぽじとりめい", "リポジトリ名")]);
}

#[test]
fn history_that_skips_an_item_is_not_used() {
    let conv = converter();
    let hint = [
        commit_item("めい", "名"),
        commit_item("に", "に"),
        commit_item("りぽじとり", "リポジトリ"),
    ];
    // 「名」の前の項目は「に」で、「リポジトリ」とは連続していない
    let seg = reconverted(&conv, "リポジトリ名", &hint);
    assert_eq!(spans(&seg), [pair("りぽじとりめい", "リポジトリ名")]);
}

#[test]
fn history_item_without_a_reading_is_skipped() {
    // 英語のモードの文脈のたまり（ADR 0026 §5.5）は `reading` が空の `CommitText` として入る。
    // 履歴の読みを使う部品は「読みなし」として飛ばし、辞書の読みに戻る（つなぎも切る）
    let conv = converter();
    let hint = [commit_item("", "品詞"), commit_item("ひんし", "瀕死")];
    let seg = reconverted(&conv, "品詞", &hint);
    assert_eq!(spans(&seg), [pair("ひんし", "品詞")]);
    let joined = [
        commit_item("りぽじとり", "リポジトリ"),
        commit_item("", "名"),
    ];
    let seg = reconverted(&conv, "リポジトリ名", &joined);
    assert_eq!(spans(&seg), [pair("りぽじとりめい", "リポジトリ名")]);
}

#[test]
fn history_matches_a_single_item_anywhere_in_the_list() {
    let conv = converter();
    let hint = [commit_item("ひんし", "瀕死"), commit_item("ひんし", "品詞")];
    let seg = reconverted(&conv, "品詞", &hint);
    assert_eq!(spans(&seg), [pair("ひんし", "品詞")]);
    let seg = reconverted(&conv, "瀕死", &hint);
    assert_eq!(spans(&seg), [pair("ひんし", "瀕死")]);
}

#[test]
fn history_reading_wins_over_the_dictionary() {
    // 「ひんし」と確定した「品詞」でも、履歴の読みが優先される（辞書の読みは「ひんし」だが、
    // ここでは別の読みで確定した表記を使う）
    let conv = converter();
    let hint = [commit_item("ひん", "品詞")];
    let seg = reconverted(&conv, "品詞", &hint);
    assert_eq!(spans(&seg), [pair("ひん", "品詞")]);
}

#[test]
fn history_item_the_dictionary_cannot_produce_is_one_clause() {
    let conv = converter();
    let hint = [commit_item("ふぁふぁ", "ホゲ")];
    let mut seg = reconverted(&conv, "ホゲ", &hint);
    assert_eq!(spans(&seg), [pair("ふぁふぁ", "ホゲ")]);
    let all = all_candidates(&mut seg, 0);
    assert_eq!(all[0].surface, "ホゲ");
    assert!(all.iter().any(|c| c.surface == "ふぁふぁ"));
}

#[test]
fn reads_unknown_kana_and_ascii() {
    let conv = converter();
    let seg = reconverted(&conv, "プラグイン", &[]);
    assert_eq!(spans(&seg), [pair("ぷらぐいん", "プラグイン")]);
    assert_eq!(seg.heads[0].origin, CandidateOrigin::Katakana);
    let seg = reconverted(&conv, "ふぁふぁ", &[]);
    assert_eq!(spans(&seg), [pair("ふぁふぁ", "ふぁふぁ")]);
    assert_eq!(seg.heads[0].origin, CandidateOrigin::Hiragana);
    let seg = reconverted(&conv, "abc", &[]);
    assert_eq!(spans(&seg), [pair("abc", "abc")]);
}

#[test]
fn keeps_unreadable_parts_as_literal_clauses() {
    let conv = converter();
    let mut seg = reconverted(&conv, "ユーザー辞書に鰯を単語", &[]);
    assert_eq!(
        spans(&seg),
        [
            pair("ゆーざーじしょに", "ユーザー辞書に"),
            pair("", "鰯"),
            pair("を", "を"),
            pair("たんご", "単語"),
        ]
    );
    // 位置がつながり、読めない部分は幅 0
    let positions: Vec<(usize, usize)> = seg
        .layout()
        .spans
        .iter()
        .map(|s| (s.start, s.end))
        .collect();
    assert_eq!(positions, [(0, 8), (8, 8), (8, 9), (9, 12)]);
    // 読めない部分の候補は元の表記だけ
    let page = seg.page_until(1, 0, &[], None).unwrap().unwrap();
    assert_eq!(page.candidates.len(), 1);
    assert_eq!(page.candidates[0].surface, "鰯");
    assert_eq!(page.candidates[0].origin, CandidateOrigin::Literal);
    assert!(!page.has_next);
    // 読める文節の 0 番も元の表記
    let first = page_surfaces(&mut seg, 0, 0);
    assert_eq!(first[0], "ユーザー辞書に");
}

#[test]
fn literal_clause_is_part_of_the_left_context() {
    let rec = recorder();
    let conv = converter_with(rec.clone(), Vec::new());
    let mut seg = reconverted(&conv, "ユーザー辞書に鰯を単語", &[]);
    rec.seen.lock().unwrap().clear();
    let left = [SegmentChoice::Candidate(0), SegmentChoice::Candidate(0)];
    let _ = seg.page_until(2, 0, &left, None).unwrap();
    assert_eq!(rec.seen.lock().unwrap()[0].1, "ユーザー辞書に鰯");
}

#[test]
fn refuses_what_it_cannot_read() {
    let conv = converter();
    for surface in ["", "鰯", "鰯、鰯", " ", "漢字"] {
        assert_eq!(
            conv.reconvert(surface, &Context::new(), &[], None).err(),
            Some(SegmentError::Empty),
            "{surface:?}"
        );
    }
    // secure では始めない
    let secure = Context::new().with_secure(true);
    assert_eq!(
        conv.reconvert("品詞", &secure, &[], None).err(),
        Some(SegmentError::Empty)
    );
}

#[test]
fn uses_only_the_left_text_as_context() {
    let rec = recorder();
    let conv = converter_with(rec.clone(), Vec::new());
    let ctx = Context::new()
        .with_left_text("前置き")
        .with_history(vec![commit_item("あ", "亜")]);
    let mut seg = conv.reconvert("品詞", &ctx, &[], None).unwrap();
    assert!(seg.whole().ctx.history.is_empty());
    rec.seen.lock().unwrap().clear();
    let _ = seg.page_until(0, 0, &[], None).unwrap();
    assert_eq!(rec.seen.lock().unwrap()[0].1, "前置き");
}

#[test]
fn does_not_evict_the_left_context_analysis_cache() {
    let conv = converter();
    let ctx = Context::new().with_left_text("ユーザー辞書に");
    let before = conv.analyze(&ctx);
    let _ = conv.reconvert("品詞", &ctx, &[], None).unwrap();
    let _ = conv
        .reconvert("ユーザー辞書に単語を", &ctx, &[], None)
        .unwrap();
    let after = conv.analyze(&ctx);
    assert!(Arc::ptr_eq(&before, &after));
}

#[test]
fn commit_learns_only_when_something_changed() {
    let conv = converter();
    let mut seg = reconverted(&conv, "品詞", &[]);
    let all = all_candidates(&mut seg, 0);
    // 何も変えなければ学習しない。確定履歴にも積まない
    let same = seg.commit(&[SegmentChoice::Candidate(0)]).unwrap();
    assert_eq!(same.surface, "品詞");
    assert!(same.events.is_empty());
    assert!(same.history.is_empty());
    // 別の表記を選ぶと、通常の確定と同じ記録。履歴には積まない
    let dead = all.iter().position(|c| c.surface == "瀕死").unwrap();
    let changed = seg.commit(&[SegmentChoice::Candidate(dead)]).unwrap();
    assert_eq!(changed.surface, "瀕死");
    assert_eq!(changed.events.len(), 1);
    assert_eq!(changed.events[0].reading, "ひんし");
    assert_eq!(changed.events[0].candidate.surface, "瀕死");
    assert!(changed.history.is_empty());
}

#[test]
fn literal_clause_is_never_learned() {
    let conv = converter();
    let mut seg = reconverted(&conv, "ユーザー辞書に鰯を単語", &[]);
    let list = all_candidates(&mut seg, 3);
    let hiragana = list.iter().position(|c| c.surface == "たんご").unwrap();
    let commit = seg
        .commit(&[
            SegmentChoice::Candidate(0),
            SegmentChoice::Candidate(0),
            SegmentChoice::Candidate(0),
            SegmentChoice::Candidate(hiragana),
        ])
        .unwrap();
    assert_eq!(commit.surface, "ユーザー辞書に鰯をたんご");
    // 読み全体は覚えない（読めない部分を含む）。変えた文節だけ
    assert_eq!(commit.events.len(), 1);
    assert_eq!(commit.events[0].reading, "たんご");
}

#[test]
fn a_literal_clause_is_a_wall_for_resizing() {
    let conv = converter();
    let mut seg = reconverted(&conv, "ユーザー辞書に鰯を単語", &[]);
    // 文節 0 は壁（鰯）に接しているので伸ばせない
    conv.resize(&mut seg, 0, 1, &[], None).unwrap();
    assert_eq!(seg.layout().rev, 0);
    // Literal の文節は動かせない
    conv.resize(&mut seg, 1, 1, &[], None).unwrap();
    conv.resize(&mut seg, 1, -1, &[], None).unwrap();
    assert_eq!(seg.layout().rev, 0);
    // 縮めると、壁の手前で区切り直す。壁の右はそのまま
    let right_before: Vec<(String, String)> = seg.layout().spans[2..]
        .iter()
        .map(|s| (s.reading.clone(), s.surface.clone()))
        .collect();
    conv.resize(&mut seg, 0, -1, &[], None).unwrap();
    assert_eq!(seg.layout().rev, 1);
    assert_eq!(seg.layout().spans[0].reading, "ゆーざーじしょ");
    let literal = seg
        .layout()
        .spans
        .iter()
        .position(|s| s.surface == "鰯")
        .unwrap();
    assert_eq!(seg.layout().spans[literal].reading, "");
    let readings: String = seg
        .layout()
        .spans
        .iter()
        .map(|s| s.reading.as_str())
        .collect();
    assert_eq!(readings, "ゆーざーじしょにをたんご");
    let after_wall: Vec<(String, String)> = seg.layout().spans[literal + 1..]
        .iter()
        .map(|s| (s.reading.clone(), s.surface.clone()))
        .collect();
    assert_eq!(after_wall, right_before);
    for pair in seg.layout().spans.windows(2) {
        assert_eq!(pair[0].end, pair[1].start);
    }
}

#[test]
fn resizing_a_reconversion_counts_as_a_change() {
    let conv = converter();
    let mut seg = reconverted(&conv, "リポジトリ名", &[]);
    assert_eq!(seg.layout().spans.len(), 1);
    conv.resize(&mut seg, 0, -1, &[], None).unwrap();
    conv.resize(&mut seg, 0, -1, &[], None).unwrap();
    assert_eq!(
        spans(&seg),
        [pair("りぽじとり", "リポジトリ"), pair("めい", "名")]
    );
    // 区切り直しで作った文節は変換の 1 位になる。確定すると読み全体を記録する
    let commit = seg
        .commit(&[SegmentChoice::Candidate(0), SegmentChoice::Candidate(0)])
        .unwrap();
    assert_eq!(commit.events.len(), 1);
    assert_eq!(commit.events[0].reading, "りぽじとりめい");
}
