//! 本物のコア（`RealSeam`、fixture の辞書）で通すテスト。モックの文字列に頼らず、
//! 候補の番号と区切りの構造（ADR 0006 §2〜§4、§6、§9）で確かめる。

use super::commit::commit_body;
use super::resize::resize_body;
use super::segment_page::{choice, seg_page_body};
use super::*;

/// 2 文節以上に分かれる読み（fixture の辞書で）
const MULTI_CANDIDATES: [&str; 4] = [
    "このしよう",
    "ひんしのしよう",
    "このひんしはしよう",
    "かきをする",
];

fn multi_segment(r: &Real) -> pb::ConvertReply {
    for reading in MULTI_CANDIDATES {
        begin(r.e(), 1, false);
        let reply = convert(r.e(), 1, reading, 0);
        if layout_of(&reply).segments.len() >= 2 {
            return reply;
        }
        send(
            r.e(),
            envelope::Body::Cancel(pb::Cancel {
                conversion_id: reply.conversion_id,
            }),
        );
    }
    panic!("no reading is split into two segments by the fixture dictionary");
}

fn surface_concat(l: &pb::SegmentLayout) -> String {
    l.segments.iter().map(|s| s.surface.as_str()).collect()
}

fn reading_concat(l: &pb::SegmentLayout) -> String {
    l.segments.iter().map(|s| s.reading.as_str()).collect()
}

fn learned_rows(r: &Real) -> usize {
    r.e().inner.stack().learning.as_ref().unwrap().len()
}

#[test]
fn the_first_display_is_the_whole_top_candidate_split_into_segments() {
    let r = Real::new();
    let reply = multi_segment(&r);
    let layout = layout_of(&reply);
    assert_eq!(surface_concat(layout), surfaces(&reply)[0]);
    assert_eq!(layout.layout_rev, 0);
    assert_eq!(layout.focused, 0);
    // 各文節の 0 番は、表示している表記
    for (i, span) in layout.segments.iter().enumerate() {
        let page = match send(
            r.e(),
            seg_page_body(reply.conversion_id, 0, i as u32, 0, vec![choice(0); i]),
        ) {
            envelope::Body::SegmentPageReply(p) => p,
            other => panic!("{other:?}"),
        };
        assert_eq!(page.candidates[0].surface, span.surface, "segment {i}");
    }
}

#[test]
fn resizing_keeps_the_reading_and_the_segments_before_the_focus() {
    let r = Real::new();
    let reply = multi_segment(&r);
    let id = reply.conversion_id;
    let before = layout_of(&reply).clone();
    let last = before.segments.len() as u32 - 1;
    // 最後の文節は伸ばせない（何もしない）
    let same = match send(
        r.e(),
        resize_body(id, 0, last, 1, vec![choice(0); last as usize]),
    ) {
        envelope::Body::SegmentLayout(l) => l,
        other => panic!("{other:?}"),
    };
    assert_eq!(same.layout_rev, 0);
    // 先頭を縮める（1 文字なら動かない）
    let first_len = before.segments[0].reading.chars().count();
    let resized = match send(r.e(), resize_body(id, 0, 0, -1, vec![])) {
        envelope::Body::SegmentLayout(l) => l,
        other => panic!("{other:?}"),
    };
    assert_eq!(reading_concat(&resized), reading_concat(&before));
    if first_len > 1 {
        assert_eq!(resized.layout_rev, 1);
        assert_eq!(resized.segments[0].reading.chars().count(), first_len - 1);
        // 古い版の要求は STALE_LAYOUT
        let stale = send(r.e(), seg_page_body(id, 0, 0, 0, vec![]));
        assert_eq!(error_code(&stale), ErrorCode::StaleLayout);
    } else {
        assert_eq!(resized.layout_rev, 0);
    }
    // 3 文節以上なら、フォーカスより前は変わらない
    if before.segments.len() >= 3 {
        let moved = match send(
            r.e(),
            resize_body(id, resized.layout_rev, 1, -1, vec![choice(0)]),
        ) {
            envelope::Body::SegmentLayout(l) => l,
            other => panic!("{other:?}"),
        };
        assert_eq!(moved.segments[0], resized.segments[0]);
        assert_eq!(reading_concat(&moved), reading_concat(&before));
    }
    let page = resized.focused_page.as_ref().unwrap();
    assert_eq!(page.candidates[0].surface, resized.segments[0].surface);
}

#[test]
fn committing_all_zero_learns_the_whole_reading_once() {
    let r = Real::new();
    let reply = multi_segment(&r);
    let n = layout_of(&reply).segments.len();
    let ack = send(
        r.e(),
        commit_body(reply.conversion_id, 0, vec![choice(0); n]),
    );
    assert!(matches!(ack, envelope::Body::Ack(_)), "{ack:?}");
    assert!(learned_rows(&r) >= 1);
    let history = lock_mutex(&r.e().inner.sessions)[&sk(1)].history.len();
    assert!(history >= 1);
}

#[test]
fn a_changed_segment_adds_rows_beyond_the_whole_reading() {
    let r = Real::new();
    let reply = multi_segment(&r);
    let id = reply.conversion_id;
    let n = layout_of(&reply).segments.len();
    // 変えられる文節（候補が 2 件以上）を探す
    for seg in 0..n {
        let page = match send(
            r.e(),
            seg_page_body(id, 0, seg as u32, 0, vec![choice(0); seg]),
        ) {
            envelope::Body::SegmentPageReply(p) => p,
            other => panic!("{other:?}"),
        };
        if page.candidates.len() < 2 {
            continue;
        }
        let mut choices = vec![choice(0); n];
        choices[seg] = choice(1);
        send(r.e(), commit_body(id, 0, choices));
        let rows = r.e().inner.stack().learning.as_ref().unwrap().records();
        let seg_reading = &layout_of(&reply).segments[seg].reading;
        assert!(
            rows.iter().any(|row| &row.reading == seg_reading),
            "the changed segment is recorded under its own reading"
        );
        return;
    }
    panic!("no segment had a second candidate");
}

#[test]
fn reconversion_keeps_the_original_surface_and_is_not_learned_untouched() {
    let r = Real::new();
    begin(r.e(), 1, false);
    let layout = match send(
        r.e(),
        envelope::Body::Reconvert(pb::Reconvert {
            session_id: 1,
            surface: "ひんし".into(),
            context: None,
            page_size: 0,
        }),
    ) {
        envelope::Body::SegmentLayout(l) => l,
        other => panic!("{other:?}"),
    };
    assert_eq!(surface_concat(&layout), "ひんし");
    let page = layout.focused_page.as_ref().unwrap();
    assert_eq!(page.candidates[0].surface, layout.segments[0].surface);
    let n = layout.segments.len();
    send(
        r.e(),
        commit_body(layout.conversion_id, 0, vec![choice(0); n]),
    );
    assert_eq!(learned_rows(&r), 0);
    assert!(lock_mutex(&r.e().inner.sessions)[&sk(1)].history.is_empty());
}

#[test]
fn reconversion_refuses_secure_sessions_and_unreadable_surfaces() {
    let r = Real::new();
    begin(r.e(), 1, false);
    begin(r.e(), 2, true);
    let body = |session, surface: &str| {
        envelope::Body::Reconvert(pb::Reconvert {
            session_id: session,
            surface: surface.into(),
            context: None,
            page_size: 0,
        })
    };
    assert_eq!(
        error_code(&send(r.e(), body(2, "ひんし"))),
        ErrorCode::BadRequest
    );
    assert_eq!(
        error_code(&send(r.e(), body(1, "𠮟𠮟"))),
        ErrorCode::BadRequest
    );
}

#[test]
fn a_one_segment_conversion_serves_the_whole_list_as_segment_zero_with_the_real_core() {
    let r = Real::new();
    begin(r.e(), 1, false);
    let reply = convert(r.e(), 1, "ひんし", 0);
    let layout = layout_of(&reply);
    // 本物のコアで 1 文節になる読みを使う（なっていなければ、このテストの前提が崩れたので気づけるように止める）
    assert_eq!(layout.segments.len(), 1, "{layout:?}");
    let page = layout.focused_page.as_ref().unwrap();
    assert_eq!(page.candidates, reply.candidates);
}
