//! `CommitSegments` と学習（ADR 0006 §5.3、§6）。

use super::segment_page::{choice, literal, seg_page_body, two_segments};
use super::*;

pub(super) fn commit_body(id: u64, rev: u32, choices: Vec<pb::SegmentChoice>) -> envelope::Body {
    envelope::Body::CommitSegments(pb::CommitSegments {
        conversion_id: id,
        layout_rev: rev,
        choices,
    })
}

fn learned(e: &Engine) -> Vec<(String, String)> {
    let stack = e.inner.stack();
    let mut rows: Vec<(String, String)> = stack
        .learning
        .as_ref()
        .unwrap()
        .records()
        .into_iter()
        .map(|r| (r.reading, r.surface))
        .collect();
    rows.sort();
    rows
}

fn history(e: &Engine) -> Vec<(String, String)> {
    lock_mutex(&e.inner.sessions)[&sk(1)]
        .history
        .iter()
        .map(|c| (c.reading.clone(), c.surface.clone()))
        .collect()
}

#[test]
fn committing_records_the_whole_reading_and_only_the_changed_segments() {
    let dir = temp_dir("seg-learn");
    let path = dir.join("learning.tsv");
    let (e, seam, _mock) = seg_engine_with(0, Some(&path));
    let r = two_segments(&e, &seam);
    let ack = send(
        &e,
        commit_body(r.conversion_id, 0, vec![choice(0), choice(4)]),
    );
    assert!(matches!(ack, envelope::Body::Ack(_)), "{ack:?}");
    let whole_surface = format!("{}{}", mock_surface("ひん", 0), mock_surface("し", 4));
    // 読みの順に並べて比べる（記録の順ではない）
    assert_eq!(
        learned(&e),
        [
            ("し".to_owned(), mock_surface("し", 4)),
            ("ひんし".to_owned(), whole_surface.clone()),
        ]
    );
    // 確定は履歴になる（読み全体と、つないだ表記）
    assert_eq!(history(&e), [("ひんし".to_owned(), whole_surface)]);
    assert!(lock_mutex(&e.inner.save).dirty);
    // 確定した変換はもう引けない
    let gone = send(&e, seg_page_body(r.conversion_id, 0, 0, 0, vec![]));
    assert_eq!(error_code(&gone), ErrorCode::UnknownConversion);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unchanged_segments_are_not_recorded_one_by_one() {
    let dir = temp_dir("seg-learn-unchanged");
    let path = dir.join("learning.tsv");
    let (e, seam, _mock) = seg_engine_with(0, Some(&path));
    let r = two_segments(&e, &seam);
    send(
        &e,
        commit_body(r.conversion_id, 0, vec![choice(0), choice(0)]),
    );
    assert_eq!(learned(&e).len(), 1, "only the whole reading");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn literal_choices_are_not_learned_and_keep_the_whole_reading_out() {
    let dir = temp_dir("seg-learn-literal");
    let path = dir.join("learning.tsv");
    let (e, seam, _mock) = seg_engine_with(0, Some(&path));
    let r = two_segments(&e, &seam);
    send(
        &e,
        commit_body(r.conversion_id, 0, vec![choice(0), literal("シ")]),
    );
    assert!(learned(&e).is_empty());
    // 確定した文字列は履歴に残る（直前の文脈として使うため）
    let expected = format!("{}シ", mock_surface("ひん", 0));
    assert_eq!(history(&e), [("ひんし".to_owned(), expected)]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_one_segment_commit_learns_like_the_old_commit() {
    let dir = temp_dir("seg-learn-one");
    let path = dir.join("learning.tsv");
    let (e, _seam, _mock) = seg_engine_with(0, Some(&path));
    begin(&e, 1, false);
    let r = convert(&e, 1, READING, 0);
    let index = surfaces(&r).iter().position(|s| *s == BOOSTED).unwrap();
    send(
        &e,
        commit_body(
            r.conversion_id,
            0,
            vec![choice(u32::try_from(index).unwrap())],
        ),
    );
    assert_eq!(learned(&e), [(READING.to_owned(), BOOSTED.to_owned())]);
    assert_eq!(history(&e), [(READING.to_owned(), BOOSTED.to_owned())]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn secure_sessions_learn_nothing_from_segment_commits() {
    let dir = temp_dir("seg-secure");
    let path = dir.join("learning.tsv");
    let (e, seam, _mock) = seg_engine_with(0, Some(&path));
    seam.split(READING, &[2]);
    begin(&e, 1, true);
    let r = convert(&e, 1, READING, 0);
    assert_eq!(readings(layout_of(&r)), ["ひん", "し"]);
    let ack = send(
        &e,
        commit_body(r.conversion_id, 0, vec![choice(2), choice(4)]),
    );
    assert!(matches!(ack, envelope::Body::Ack(_)));
    assert!(learned(&e).is_empty());
    assert!(history(&e).is_empty());
    assert!(!lock_mutex(&e.inner.save).dirty);
    drop(e);
    assert!(!path.exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bad_commits_are_rejected_and_keep_the_conversion() {
    let (e, seam, _mock) = seg_engine();
    let r = two_segments(&e, &seam);
    let id = r.conversion_id;
    let stale = send(&e, commit_body(id, 3, vec![choice(0), choice(0)]));
    assert_eq!(error_code(&stale), ErrorCode::StaleLayout);
    let cases = [
        commit_body(id, 0, vec![choice(0)]),
        commit_body(id, 0, vec![choice(0), choice(0), choice(0)]),
        commit_body(id, 0, vec![choice(0), choice(9999)]),
        commit_body(id, 0, vec![choice(u32::MAX), choice(0)]),
        commit_body(id, 0, vec![choice(0); MAX_READING_CHARS + 1]),
        commit_body(id, 0, vec![choice(0), literal(&"あ".repeat(257))]),
    ];
    for body in cases {
        assert_eq!(error_code(&send(&e, body)), ErrorCode::BadRequest);
    }
    let unknown = send(&e, commit_body(id + 5, 0, vec![]));
    assert_eq!(error_code(&unknown), ErrorCode::UnknownConversion);
    // どれも失敗なので、変換は残っていて確定できる
    let ack = send(&e, commit_body(id, 0, vec![choice(1), choice(0)]));
    assert!(matches!(ack, envelope::Body::Ack(_)), "{ack:?}");
}
