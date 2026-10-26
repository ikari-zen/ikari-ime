//! 先行計算が文節 0 まで作ること、`Convert` のあとの文節の先読み（ADR 0006 §3.4、§5.3）。

use super::segment_page::{seg_page, two_segments};
use super::*;
use crate::segments::{LookJob, LookaheadQueue};

fn chunk_calls(seam: &MockSeam, seg: usize) -> Vec<crate::mock_seam::Call> {
    seam.calls()
        .into_iter()
        .filter(|c| c.what == "chunk" && c.seg == seg)
        .collect()
}

#[test]
fn the_prefetch_job_builds_segment_zero_with_the_lm_deadline() {
    let (e, seam, _mock) = seg_engine();
    seam.split(READING, &[2]);
    begin(&e, 1, false);
    prefetch(&e, 1, READING, 0);
    wait_until(|| !lock_mutex(&e.inner.cache).any_computing());
    let made = chunk_calls(&seam, 0);
    assert!(!made.is_empty(), "segment 0 was made by the prefetch job");
    assert!(
        made.iter().all(|c| !c.without_waiting()),
        "the job waits for the LM like the whole conversion: {made:?}"
    );
    // Convert はキャッシュに当たり、文節 0 を作り直さない
    let r = convert(&e, 1, READING, 0);
    let page0 = layout_of(&r).focused_page.as_ref().unwrap();
    assert_eq!(page0.candidates[0].surface, mock_surface("ひん", 0));
    assert_eq!(chunk_calls(&seam, 0).len(), made.len());
}

#[test]
fn a_one_segment_prefetch_does_not_build_segment_pages_ahead() {
    let (e, seam, _mock) = seg_engine();
    begin(&e, 1, false);
    prefetch(&e, 1, READING, 0);
    wait_until(|| !lock_mutex(&e.inner.cache).any_computing());
    assert!(seam.calls().is_empty());
}

#[test]
fn convert_looks_ahead_at_the_first_page_of_the_later_segments() {
    let (e, seam, _mock) = seg_engine();
    seam.split(READING, &[1, 2]);
    begin(&e, 1, false);
    let r = convert(&e, 1, READING, 0);
    // 文節 2 の列が 10 件まで揃うと、先読みは終わる
    wait_until(|| chunk_calls(&seam, 2).iter().any(|c| c.index == 1));
    let ahead = chunk_calls(&seam, 1);
    assert_eq!(ahead[0].index, 0);
    assert!(!ahead[0].without_waiting(), "the lookahead uses the LM");
    assert_eq!(ahead[0].left, [ikari_core::SegmentChoice::Candidate(0)]);
    let third = chunk_calls(&seam, 2);
    assert_eq!(third[0].left.len(), 2);
    // 移動の要求は、できているものをそのまま返す（作り直さない）
    let before = seam.calls().len();
    let page = seg_page(&e, r.conversion_id, 1, 0);
    assert_eq!(page.candidates[0].surface, mock_surface("ん", 0));
    assert_eq!(seam.calls().len(), before);
}

#[test]
fn a_request_during_the_lookahead_waits_for_it_instead_of_making_it_twice() {
    let (e, seam, _mock) = seg_engine();
    seam.set_delay(Duration::from_millis(120));
    seam.split(READING, &[2]);
    begin(&e, 1, false);
    let r = convert(&e, 1, READING, 0);
    let page = seg_page(&e, r.conversion_id, 1, 0);
    assert_eq!(page.candidates[0].surface, mock_surface("し", 0));
    assert_eq!(
        chunk_calls(&seam, 1)
            .iter()
            .filter(|c| c.index == 0)
            .count(),
        1,
        "made once"
    );
}

#[test]
fn a_new_convert_cancels_the_old_lookahead() {
    let queue = LookaheadQueue::default();
    let (e, seam, _mock) = seg_engine();
    seam.split(READING, &[2]);
    begin(&e, 1, false);
    let r = convert(&e, 1, READING, 0);
    let (conv, page_size, _) = lock_mutex(&e.inner.conversions)
        .get(r.conversion_id)
        .unwrap();
    let first = LookJob::new(1, Arc::clone(&conv), page_size, 0);
    let first_cancel = Arc::clone(&first.cancel);
    queue.submit(first);
    let second = LookJob::new(1, conv, page_size, 0);
    let second_cancel = Arc::clone(&second.cancel);
    queue.submit(second);
    assert!(first_cancel.load(Ordering::Acquire));
    assert!(!second_cancel.load(Ordering::Acquire));
    let next = queue.next().unwrap();
    assert!(Arc::ptr_eq(&next.cancel, &second_cancel));
    // 処理中でも、同じセッションの新しい依頼や forget で止まる
    queue.forget_session(1);
    assert!(second_cancel.load(Ordering::Acquire));
    queue.close();
    assert!(queue.next().is_none());
}

#[test]
fn secure_sessions_do_not_look_ahead() {
    let (e, seam, _mock) = seg_engine();
    seam.split(READING, &[1, 2]);
    begin(&e, 1, true);
    let _ = convert(&e, 1, READING, 0);
    std::thread::sleep(Duration::from_millis(60));
    assert!(chunk_calls(&seam, 1).is_empty());
    assert!(chunk_calls(&seam, 2).is_empty());
}

#[test]
fn a_one_segment_convert_has_no_lookahead() {
    let (e, seam, _mock) = seg_engine();
    begin(&e, 1, false);
    let _ = convert(&e, 1, READING, 0);
    std::thread::sleep(Duration::from_millis(40));
    assert!(chunk_calls(&seam, 1).is_empty());
}

#[test]
fn ending_the_session_and_dropping_the_engine_stop_the_lookahead_thread() {
    let (e, seam, _mock) = seg_engine();
    seam.set_delay(Duration::from_millis(30));
    seam.split(READING, &[1, 2]);
    let _ = two_segments(&e, &seam);
    send(
        &e,
        envelope::Body::EndSession(pb::EndSession { session_id: 1 }),
    );
    // Drop は処理中の先読みを待ってからスレッドを閉じる。ここで固まらなければよい
    drop(e);
}
