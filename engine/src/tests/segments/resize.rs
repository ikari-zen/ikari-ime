//! `ResizeSegment`（ADR 0006 §4、§5.3）。

use super::segment_page::{choice, literal, seg_page, seg_page_body, two_segments};
use super::*;

pub(super) fn resize_body(
    id: u64,
    rev: u32,
    segment: u32,
    delta: i32,
    left: Vec<pb::SegmentChoice>,
) -> envelope::Body {
    envelope::Body::ResizeSegment(pb::ResizeSegment {
        conversion_id: id,
        layout_rev: rev,
        segment,
        delta,
        left,
    })
}

fn resize(
    e: &Engine,
    id: u64,
    rev: u32,
    segment: u32,
    delta: i32,
    left: Vec<pb::SegmentChoice>,
) -> pb::SegmentLayout {
    match send(e, resize_body(id, rev, segment, delta, left)) {
        envelope::Body::SegmentLayout(l) => l,
        other => panic!("{other:?}"),
    }
}

#[test]
fn shrinking_the_first_segment_redraws_from_it_and_bumps_the_revision() {
    let (e, seam, _mock) = seg_engine();
    let r = two_segments(&e, &seam);
    let id = r.conversion_id;
    let layout = resize(&e, id, 0, 0, -1, vec![]);
    assert_eq!(layout.conversion_id, id);
    assert_eq!(layout.layout_rev, 1);
    assert_eq!(layout.focused, 0);
    assert_eq!(readings(&layout), ["ひ", "んし"]);
    assert_eq!(layout.segments[0].surface, mock_surface("ひ", 0));
    let page0 = layout.focused_page.as_ref().unwrap();
    assert_eq!(page0.segment, 0);
    assert_eq!(page0.candidates[0].surface, mock_surface("ひ", 0));
    // 区切り直しは LM を待たない
    let call = seam
        .calls()
        .into_iter()
        .find(|c| c.what == "resize")
        .unwrap();
    assert!(call.without_waiting(), "{call:?}");
    // 古い版の要求は STALE_LAYOUT。新しい版なら通る
    let stale = send(&e, seg_page_body(id, 0, 1, 0, vec![choice(0)]));
    assert_eq!(error_code(&stale), ErrorCode::StaleLayout);
    let stale = send(&e, resize_body(id, 0, 0, 1, vec![]));
    assert_eq!(error_code(&stale), ErrorCode::StaleLayout);
    let ok = send(&e, seg_page_body(id, 1, 1, 0, vec![choice(0)]));
    let envelope::Body::SegmentPageReply(ok) = ok else {
        panic!("{ok:?}")
    };
    assert_eq!(ok.candidates[0].surface, mock_surface("んし", 0));
}

#[test]
fn expanding_works_both_ways_and_the_response_has_the_new_page_for_the_focus() {
    let (e, seam, _mock) = seg_engine();
    seam.split(READING, &[1, 2]);
    begin(&e, 1, false);
    let r = convert(&e, 1, READING, 0);
    let id = r.conversion_id;
    assert_eq!(readings(layout_of(&r)), ["ひ", "ん", "し"]);
    let layout = resize(&e, id, 0, 1, 1, vec![choice(4)]);
    assert_eq!(layout.focused, 1);
    assert_eq!(readings(&layout), ["ひ", "んし"]);
    assert_eq!(layout.layout_rev, 1);
    assert_eq!(
        layout.focused_page.unwrap().candidates[0].surface,
        mock_surface("んし", 0)
    );
    // 縮めて戻す
    let layout = resize(&e, id, 1, 1, -1, vec![choice(4)]);
    assert_eq!(readings(&layout), ["ひ", "ん", "し"]);
    assert_eq!(layout.layout_rev, 2);
}

#[test]
fn segments_before_the_resized_one_keep_their_lists() {
    let (e, seam, _mock) = seg_engine();
    seam.split(READING, &[1, 2]);
    begin(&e, 1, false);
    let id = convert(&e, 1, READING, 0).conversion_id;
    let before = seg_page(&e, id, 0, 0);
    let made = seam.calls().iter().filter(|c| c.seg == 0).count();
    let _ = resize(&e, id, 0, 1, 1, vec![choice(2)]);
    let send_page = |rev| send(&e, seg_page_body(id, rev, 0, 0, vec![]));
    let envelope::Body::SegmentPageReply(after) = send_page(1) else {
        panic!()
    };
    assert_eq!(after.candidates, before.candidates);
    // 文節 0 の塊は作り直していない（エンジンが列を持っている）
    let chunk_calls = seam
        .calls()
        .iter()
        .filter(|c| c.seg == 0 && c.what == "chunk")
        .count();
    assert_eq!(chunk_calls, made);
}

#[test]
fn a_move_that_cannot_happen_changes_nothing() {
    let (e, seam, _mock) = seg_engine();
    seam.split(READING, &[1]);
    begin(&e, 1, false);
    let id = convert(&e, 1, READING, 0).conversion_id;
    // 1 文字の文節は縮まない、最後の文節は伸びない
    for (segment, delta) in [(0, -1), (1, 1)] {
        let layout = resize(&e, id, 0, segment, delta, vec![choice(0); segment as usize]);
        assert_eq!(layout.layout_rev, 0, "segment {segment} delta {delta}");
        assert_eq!(readings(&layout), ["ひ", "んし"]);
        assert_eq!(layout.focused, segment);
        assert!(layout.focused_page.is_some());
    }
}

#[test]
fn a_one_segment_conversion_can_be_shrunk_into_two() {
    let (e, _seam, _mock) = seg_engine();
    begin(&e, 1, false);
    let r = convert(&e, 1, READING, 0);
    let layout = resize(&e, r.conversion_id, 0, 0, -1, vec![]);
    assert_eq!(readings(&layout), ["ひん", "し"]);
    // 伸ばすのは何もしない
    let same = resize(&e, r.conversion_id, 1, 1, 1, vec![choice(0)]);
    assert_eq!(same.layout_rev, 1);
}

#[test]
fn resize_rejects_bad_requests() {
    let (e, seam, _mock) = seg_engine();
    let r = two_segments(&e, &seam);
    let id = r.conversion_id;
    let cases = [
        resize_body(id, 0, 2, 1, vec![]),
        resize_body(id, 0, u32::MAX, -1, vec![]),
        resize_body(id, 0, 0, i32::MAX, vec![]),
        resize_body(id, 0, 0, i32::MIN, vec![]),
        resize_body(id, 0, 0, 1, vec![choice(0); MAX_READING_CHARS + 1]),
        resize_body(id, 0, 0, 1, vec![literal(&"あ".repeat(257))]),
    ];
    for body in cases {
        assert_eq!(error_code(&send(&e, body)), ErrorCode::BadRequest);
    }
    let unknown = send(&e, resize_body(id + 9, 0, 0, 1, vec![]));
    assert_eq!(error_code(&unknown), ErrorCode::UnknownConversion);
    // 失敗しても変換は壊れない
    assert_eq!(resize(&e, id, 0, 0, -1, vec![]).layout_rev, 1);
}

fn remove_body(id: u64, rev: u32, first: u32, count: u32) -> envelope::Body {
    envelope::Body::RemoveSegments(pb::RemoveSegments {
        conversion_id: id,
        layout_rev: rev,
        first,
        count,
    })
}

fn remove(e: &Engine, id: u64, rev: u32, first: u32, count: u32) -> pb::SegmentLayout {
    match send(e, remove_body(id, rev, first, count)) {
        envelope::Body::SegmentLayout(l) => l,
        other => panic!("{other:?}"),
    }
}

/// 3 文節（ひ｜ん｜し）。先読みなし（secure）なので、呼び出しの記録が決まった形になる
fn three_segments_without_lookahead(e: &Engine, seam: &MockSeam) -> u64 {
    seam.split(READING, &[1, 2]);
    begin(e, 1, true);
    convert(e, 1, READING, 0).conversion_id
}

#[test]
fn remove_segments_drops_the_tail_and_focuses_the_one_on_its_left() {
    let (e, seam, _mock) = seg_engine();
    let id = three_segments_without_lookahead(&e, &seam);
    let layout = remove(&e, id, 0, 2, 1);
    assert_eq!(layout.conversion_id, id);
    assert_eq!(layout.layout_rev, 1);
    assert_eq!(readings(&layout), ["ひ", "ん"]);
    assert_eq!(layout.focused, 1);
    // 残りの文節の表記は 0 番のまま
    assert_eq!(layout.segments[1].surface, mock_surface("ん", 0));
    // 古い版の要求は STALE_LAYOUT。新しい版なら通る
    let stale = send(&e, seg_page_body(id, 0, 1, 0, vec![choice(0)]));
    assert_eq!(error_code(&stale), ErrorCode::StaleLayout);
    let ok = send(&e, seg_page_body(id, 1, 1, 0, vec![choice(0)]));
    let envelope::Body::SegmentPageReply(ok) = ok else {
        panic!("{ok:?}")
    };
    assert_eq!(ok.candidates[0].surface, mock_surface("ん", 0));
    // 取り除いた文節の番号はもう範囲外
    let gone = send(&e, seg_page_body(id, 1, 2, 0, vec![choice(0); 2]));
    assert_eq!(error_code(&gone), ErrorCode::BadRequest);
}

#[test]
fn remove_segments_from_the_head_focuses_zero_and_moves_the_rest_up() {
    let (e, seam, _mock) = seg_engine();
    let id = three_segments_without_lookahead(&e, &seam);
    let layout = remove(&e, id, 0, 0, 2);
    assert_eq!(readings(&layout), ["し"]);
    assert_eq!(layout.focused, 0);
    assert_eq!(layout.layout_rev, 1);
    // 1 文節になっても、読み全体の列ではなく文節の列を返す（0 番は表示中の表記）
    let page = send(&e, seg_page_body(id, 1, 0, 0, vec![]));
    let envelope::Body::SegmentPageReply(page) = page else {
        panic!("{page:?}")
    };
    assert_eq!(page.candidates[0].surface, mock_surface("し", 0));
}

#[test]
fn remove_segments_keeps_the_lists_already_made_for_the_rest() {
    let (e, seam, _mock) = seg_engine();
    let id = three_segments_without_lookahead(&e, &seam);
    let before = seg_page(&e, id, 2, 0);
    let made = seam.calls().iter().filter(|c| c.what == "chunk").count();
    // 文節 1 を取り除くと、文節 2 は 1 番になる。作ってあった列をそのまま返す
    let layout = remove(&e, id, 0, 1, 1);
    assert_eq!(readings(&layout), ["ひ", "し"]);
    assert_eq!(layout.focused, 0);
    let page = send(&e, seg_page_body(id, 1, 1, 0, vec![choice(0)]));
    let envelope::Body::SegmentPageReply(page) = page else {
        panic!("{page:?}")
    };
    assert_eq!(page.candidates, before.candidates);
    assert_eq!(
        seam.calls().iter().filter(|c| c.what == "chunk").count(),
        made,
        "no list is made again"
    );
    // 取り除きは LM を使う計算を呼ばない
    let call = seam
        .calls()
        .into_iter()
        .find(|c| c.what == "remove")
        .unwrap();
    assert_eq!((call.seg, call.index), (1, 1));
}

#[test]
fn remove_segments_works_after_resize_and_again_after_remove() {
    let (e, seam, _mock) = seg_engine();
    let r = two_segments(&e, &seam);
    let id = r.conversion_id;
    let layout = resize(&e, id, 0, 0, -1, vec![]);
    assert_eq!(readings(&layout), ["ひ", "んし"]);
    let layout = remove(&e, id, layout.layout_rev, 1, 1);
    assert_eq!(readings(&layout), ["ひ"]);
    assert_eq!(layout.layout_rev, 2);
    // 1 文節だけになったので、これ以上は取り除けない（殻は Cancel する）
    let last = send(&e, remove_body(id, 2, 0, 1));
    assert_eq!(error_code(&last), ErrorCode::BadRequest);
}

#[test]
fn remove_segments_is_not_confused_by_the_lookahead() {
    let (e, seam, _mock) = seg_engine();
    seam.split(READING, &[1, 2]);
    begin(&e, 1, false);
    let id = convert(&e, 1, READING, 0).conversion_id;
    // 先読みが終わってから取り除く
    wait_until(|| {
        seam.calls()
            .iter()
            .any(|c| c.what == "chunk" && c.seg == 2 && c.index == 1)
    });
    let layout = remove(&e, id, 0, 2, 1);
    assert_eq!(readings(&layout), ["ひ", "ん"]);
    // 先読みの続きが、取り除いた文節を作り直さない（版が変わると止まる）
    let after = seam.calls().len();
    let page = send(&e, seg_page_body(id, 1, 1, 0, vec![choice(0)]));
    assert!(matches!(page, envelope::Body::SegmentPageReply(_)));
    assert_eq!(seam.calls().len(), after);
}

#[test]
fn remove_segments_rejects_bad_requests_and_keeps_the_conversion() {
    let (e, seam, _mock) = seg_engine();
    let id = three_segments_without_lookahead(&e, &seam);
    let cases = [
        remove_body(id, 0, 3, 1),
        remove_body(id, 0, 0, 0),
        remove_body(id, 0, 2, 2),
        remove_body(id, 0, u32::MAX, 1),
        remove_body(id, 0, 1, u32::MAX),
        // 全部
        remove_body(id, 0, 0, 3),
    ];
    for body in cases {
        assert_eq!(error_code(&send(&e, body)), ErrorCode::BadRequest);
    }
    let unknown = send(&e, remove_body(id + 9, 0, 0, 1));
    assert_eq!(error_code(&unknown), ErrorCode::UnknownConversion);
    let stale = send(&e, remove_body(id, 7, 2, 1));
    assert_eq!(error_code(&stale), ErrorCode::StaleLayout);
    // 失敗しても変換は壊れない（版は変わらない）
    let layout = remove(&e, id, 0, 2, 1);
    assert_eq!(layout.layout_rev, 1);
}

#[test]
fn commit_after_remove_commits_only_the_remaining_segments() {
    let (e, seam, _mock) = seg_engine();
    let id = three_segments_without_lookahead(&e, &seam);
    let _ = remove(&e, id, 0, 2, 1);
    let body = envelope::Body::CommitSegments(pb::CommitSegments {
        conversion_id: id,
        layout_rev: 1,
        choices: vec![choice(0), choice(0)],
    });
    assert!(matches!(send(&e, body), envelope::Body::Ack(_)));
}
