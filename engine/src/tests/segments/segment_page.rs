//! `SegmentPage` と、上限の検証（ADR 0006 §5.2）。

use super::*;

pub(super) fn choice(index: u32) -> pb::SegmentChoice {
    pb::SegmentChoice {
        candidate_index: index,
        literal: String::new(),
    }
}

pub(super) fn literal(text: &str) -> pb::SegmentChoice {
    pb::SegmentChoice {
        candidate_index: 0,
        literal: text.to_owned(),
    }
}

pub(super) fn seg_page_body(
    id: u64,
    rev: u32,
    segment: u32,
    index: u32,
    left: Vec<pb::SegmentChoice>,
) -> envelope::Body {
    envelope::Body::SegmentPage(pb::SegmentPageRequest {
        conversion_id: id,
        layout_rev: rev,
        segment,
        index,
        left,
    })
}

pub(super) fn seg_page(e: &Engine, id: u64, segment: u32, index: u32) -> pb::SegmentPageReply {
    let left = vec![choice(0); segment as usize];
    match send(e, seg_page_body(id, 0, segment, index, left)) {
        envelope::Body::SegmentPageReply(r) => r,
        other => panic!("{other:?}"),
    }
}

/// 2 文節（ひん｜し）の変換を始める
pub(super) fn two_segments(e: &Engine, seam: &MockSeam) -> pb::ConvertReply {
    seam.split(READING, &[2]);
    begin(e, 1, false);
    convert(e, 1, READING, 0)
}

#[test]
fn segment_page_pages_through_a_segment_and_never_changes_a_returned_page() {
    let (e, seam, _mock) = seg_engine();
    let r = two_segments(&e, &seam);
    let id = r.conversion_id;
    let first = seg_page(&e, id, 1, 0);
    assert_eq!((first.segment, first.page_index), (1, 0));
    assert_eq!(first.candidates.len(), 9);
    assert!(first.has_next);
    assert_eq!(first.candidates[0].surface, mock_surface("し", 0));
    assert!(!first.lm_applied);
    let second = seg_page(&e, id, 1, 1);
    assert_eq!(second.candidates.len(), MOCK_LIST_LEN - 9);
    assert!(!second.has_next);
    assert_eq!(second.candidates[0].surface, mock_surface("し", 9));
    // もう一度引いても同じ
    assert_eq!(seg_page(&e, id, 1, 0).candidates, first.candidates);
    // 範囲外のページ
    let r = send(&e, seg_page_body(id, 0, 1, 2, vec![choice(0)]));
    assert_eq!(error_code(&r), ErrorCode::BadRequest);
}

/// 先読みのない 2 文節の変換（secure のセッションは先読みしない）。この要求が最初に文節 1 を作る
pub(super) fn two_segments_without_lookahead(e: &Engine, seam: &MockSeam) -> pb::ConvertReply {
    seam.split(READING, &[2]);
    begin(e, 1, true);
    convert(e, 1, READING, 0)
}

#[test]
fn segment_page_uses_the_left_choices_only_when_a_segment_is_first_made() {
    let (e, seam, _mock) = seg_engine();
    let r = two_segments_without_lookahead(&e, &seam);
    let id = r.conversion_id;
    let _ = send(&e, seg_page_body(id, 0, 1, 0, vec![choice(3)]));
    let _ = seg_page(&e, id, 1, 1);
    let made: Vec<_> = seam.calls().into_iter().filter(|c| c.seg == 1).collect();
    assert_eq!(made[0].left, [ikari_core::SegmentChoice::Candidate(3)]);
    assert_eq!(made[0].left_len, 1);
}

#[test]
fn segment_page_does_not_wait_for_the_lm_on_the_first_page() {
    let (e, seam, _mock) = seg_engine();
    let r = two_segments_without_lookahead(&e, &seam);
    let _ = seg_page(&e, r.conversion_id, 1, 0);
    let call = seam.calls().into_iter().find(|c| c.seg == 1).unwrap();
    assert!(call.without_waiting(), "{call:?}");
}

#[test]
fn segment_page_rejects_stale_layouts_unknown_conversions_and_bad_segments() {
    let (e, seam, _mock) = seg_engine();
    let r = two_segments(&e, &seam);
    let id = r.conversion_id;
    let stale = send(&e, seg_page_body(id, 7, 1, 0, vec![choice(0)]));
    assert_eq!(error_code(&stale), ErrorCode::StaleLayout);
    let unknown = send(&e, seg_page_body(id + 100, 0, 0, 0, vec![]));
    assert_eq!(error_code(&unknown), ErrorCode::UnknownConversion);
    for segment in [2, u32::MAX] {
        let bad = send(&e, seg_page_body(id, 0, segment, 0, vec![]));
        assert_eq!(error_code(&bad), ErrorCode::BadRequest, "segment {segment}");
    }
    let far = send(&e, seg_page_body(id, 0, 0, 1_000_000, vec![]));
    assert_eq!(error_code(&far), ErrorCode::BadRequest);
}

#[test]
fn segment_choices_have_limits() {
    let (e, seam, _mock) = seg_engine();
    let r = two_segments(&e, &seam);
    let id = r.conversion_id;
    let too_many = vec![choice(0); MAX_READING_CHARS + 1];
    let bad = send(&e, seg_page_body(id, 0, 1, 0, too_many));
    assert_eq!(error_code(&bad), ErrorCode::BadRequest);
    let long = "あ".repeat(256 + 1);
    let bad = send(&e, seg_page_body(id, 0, 1, 0, vec![literal(&long)]));
    assert_eq!(error_code(&bad), ErrorCode::BadRequest);
    // ちょうど上限は通る
    let edge = "あ".repeat(256);
    let ok = send(&e, seg_page_body(id, 0, 1, 0, vec![literal(&edge)]));
    assert!(matches!(ok, envelope::Body::SegmentPageReply(_)), "{ok:?}");
    let edge_count = vec![choice(0); MAX_READING_CHARS];
    let ok = send(&e, seg_page_body(id, 0, 1, 0, edge_count));
    assert!(matches!(ok, envelope::Body::SegmentPageReply(_)), "{ok:?}");
}

#[test]
fn a_one_segment_conversion_serves_the_whole_list_as_segment_zero() {
    let (e, _seam, _mock) = seg_engine();
    begin(&e, 1, false);
    let r = convert(&e, 1, READING, 0);
    let seg0 = seg_page(&e, r.conversion_id, 0, 0);
    assert_eq!(seg0.candidates, r.candidates);
    let whole1 = match page(&e, r.conversion_id, 1) {
        envelope::Body::PageReply(p) => p,
        other => panic!("{other:?}"),
    };
    let seg1 = seg_page(&e, r.conversion_id, 0, 1);
    assert_eq!(seg1.candidates, whole1.candidates);
}
