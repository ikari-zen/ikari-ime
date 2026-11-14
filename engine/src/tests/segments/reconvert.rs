//! `Reconvert`（ADR 0006 §9）。

use super::commit::commit_body;
use super::segment_page::choice;
use super::*;
use crate::mock_seam::UNREADABLE;

fn reconvert_body(session: u64, surface: &str, left: Option<&str>) -> envelope::Body {
    envelope::Body::Reconvert(pb::Reconvert {
        session_id: session,
        surface: surface.to_owned(),
        context: left.map(|l| pb::SurroundingText {
            available: true,
            left: l.to_owned(),
            right: String::new(),
        }),
        page_size: 0,
    })
}

fn reconvert(e: &Engine, session: u64, surface: &str) -> pb::SegmentLayout {
    match send(e, reconvert_body(session, surface, None)) {
        envelope::Body::SegmentLayout(l) => l,
        other => panic!("{other:?}"),
    }
}

fn commit_text(e: &Engine, reading: &str, surface: &str) {
    send(
        e,
        envelope::Body::CommitText(pb::CommitText {
            session_id: 1,
            reading: reading.into(),
            surface: surface.into(),
        }),
    );
}

fn history_len(e: &Engine) -> usize {
    lock_mutex(&e.inner.sessions)[&sk(1)].history.len()
}

fn learned_len(e: &Engine) -> usize {
    e.inner.stack().learning.as_ref().unwrap().records().len()
}

#[test]
fn a_reconversion_starts_with_the_original_surface_as_candidate_zero() {
    let (e, seam, _mock) = seg_engine();
    seam.split(READING, &[2]);
    begin(&e, 1, false);
    let layout = reconvert(&e, 1, READING);
    assert!(layout.conversion_id > 0);
    assert_eq!(layout.layout_rev, 0);
    assert_eq!(layout.focused, 0);
    assert_eq!(readings(&layout), ["ひん", "し"]);
    let surfaces: Vec<&str> = layout.segments.iter().map(|s| s.surface.as_str()).collect();
    assert_eq!(surfaces, ["ひん", "し"]);
    let page0 = layout.focused_page.as_ref().unwrap();
    assert_eq!(page0.candidates[0].surface, "ひん");
    assert_eq!(page0.candidates[1].surface, mock_surface("ひん", 1));
    let call = seam
        .calls()
        .into_iter()
        .find(|c| c.what == "reconvert")
        .unwrap();
    assert!(call.without_waiting(), "no waiting for the LM: {call:?}");
    // 変換として引ける
    let more = super::segment_page::seg_page(&e, layout.conversion_id, 1, 0);
    assert_eq!(more.candidates[0].surface, "し");
}

#[test]
fn the_history_is_a_hint_newest_first_and_never_the_left_context() {
    let (e, seam, _mock) = seg_engine();
    begin(&e, 1, false);
    commit_text(&e, "あ", "亜");
    commit_text(&e, "い", "胃");
    let _ = reconvert(&e, 1, READING);
    let envelope::Body::SegmentLayout(_) = send(&e, reconvert_body(1, READING, Some("前の文")))
    else {
        panic!()
    };
    let seen = seam.reconverted();
    let hint = |i: usize| -> Vec<(String, String)> {
        seen[i]
            .2
            .iter()
            .map(|c| (c.reading.clone(), c.surface.clone()))
            .collect()
    };
    let newest_first = [
        ("い".to_owned(), "胃".to_owned()),
        ("あ".to_owned(), "亜".to_owned()),
    ];
    assert_eq!(hint(0), newest_first);
    assert_eq!(hint(1), newest_first);
    assert_eq!(
        seen[0].1, "",
        "no surrounding text: the history is not used"
    );
    assert_eq!(seen[1].1, "前の文");
}

#[test]
fn an_untouched_reconversion_is_not_learned_or_remembered() {
    let dir = temp_dir("reconvert-untouched");
    let path = dir.join("learning.tsv");
    let (e, seam, _mock) = seg_engine_with(0, Some(&path));
    seam.split(READING, &[2]);
    begin(&e, 1, false);
    let layout = reconvert(&e, 1, READING);
    let before = history_len(&e);
    let ack = send(
        &e,
        commit_body(layout.conversion_id, 0, vec![choice(0), choice(0)]),
    );
    assert!(matches!(ack, envelope::Body::Ack(_)), "{ack:?}");
    assert_eq!(learned_len(&e), 0);
    assert_eq!(history_len(&e), before);
    assert!(!lock_mutex(&e.inner.save).dirty);
    // 終わった変換は引けない
    let again = send(
        &e,
        commit_body(layout.conversion_id, 0, vec![choice(0), choice(0)]),
    );
    assert_eq!(error_code(&again), ErrorCode::UnknownConversion);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_changed_reconversion_is_learned_but_does_not_enter_the_history() {
    let dir = temp_dir("reconvert-changed");
    let path = dir.join("learning.tsv");
    let (e, seam, _mock) = seg_engine_with(0, Some(&path));
    seam.split(READING, &[2]);
    begin(&e, 1, false);
    let layout = reconvert(&e, 1, READING);
    let before = history_len(&e);
    send(
        &e,
        commit_body(layout.conversion_id, 0, vec![choice(0), choice(2)]),
    );
    assert!(learned_len(&e) >= 1);
    assert!(lock_mutex(&e.inner.save).dirty);
    assert_eq!(history_len(&e), before, "the position is mid-document");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn moving_a_boundary_counts_as_a_change_even_with_choice_zero() {
    let dir = temp_dir("reconvert-resized");
    let path = dir.join("learning.tsv");
    let (e, _seam, _mock) = seg_engine_with(0, Some(&path));
    begin(&e, 1, false);
    let layout = reconvert(&e, 1, READING);
    assert_eq!(readings(&layout), [READING]);
    let resized = match send(
        &e,
        super::resize::resize_body(layout.conversion_id, 0, 0, -1, vec![]),
    ) {
        envelope::Body::SegmentLayout(l) => l,
        other => panic!("{other:?}"),
    };
    send(
        &e,
        commit_body(resized.conversion_id, 1, vec![choice(0), choice(0)]),
    );
    assert!(learned_len(&e) >= 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reconvert_validates_the_surface_and_the_session() {
    let (e, _seam, _mock) = seg_engine();
    begin(&e, 1, false);
    begin(&e, 2, true);
    let sixty_four = "あ".repeat(64);
    let ok = send(&e, reconvert_body(1, &sixty_four, None));
    assert!(matches!(ok, envelope::Body::SegmentLayout(_)), "{ok:?}");
    let bad = [
        reconvert_body(1, "", None),
        reconvert_body(1, &"あ".repeat(65), None),
        reconvert_body(1, "あ\nい", None),
        reconvert_body(1, "あ\rい", None),
        reconvert_body(1, "あ\u{2028}い", None),
        reconvert_body(1, UNREADABLE, None),
        reconvert_body(2, READING, None),
        envelope::Body::Reconvert(pb::Reconvert {
            session_id: 1,
            surface: READING.into(),
            context: None,
            page_size: 10,
        }),
    ];
    for body in bad {
        assert_eq!(error_code(&send(&e, body)), ErrorCode::BadRequest);
    }
    let unknown = send(&e, reconvert_body(99, READING, None));
    assert_eq!(error_code(&unknown), ErrorCode::UnknownSession);
    // 長い表記を許す側は MAX_RECONVERT_CHARS だけ。バイト数ではなく文字数で数える
    let multibyte = "漢".repeat(64);
    let ok = send(&e, reconvert_body(1, &multibyte, None));
    assert!(matches!(ok, envelope::Body::SegmentLayout(_)), "{ok:?}");
}
