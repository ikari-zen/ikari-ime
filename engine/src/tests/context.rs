//! 文脈の読み取り（ADR 0015）: 休止と再開、履歴の文字数、最近の確定と借りる文脈、share_context、secure

use super::*;

use crate::context_limits::ContextLimits;

fn begin_with(e: &Engine, session: u64, secure: bool, share: bool) {
    let r = send(
        e,
        envelope::Body::BeginSession(pb::BeginSession {
            session_id: session,
            client_bundle_id: "test".into(),
            secure,
            share_context: share,
        }),
    );
    assert!(matches!(r, envelope::Body::Ack(_)), "{r:?}");
}

fn end(e: &Engine, session: u64) {
    let r = send(
        e,
        envelope::Body::EndSession(pb::EndSession {
            session_id: session,
        }),
    );
    assert!(matches!(r, envelope::Body::Ack(_)), "{r:?}");
}

fn commit_text_req(session: u64, surface: &str) -> envelope::Body {
    envelope::Body::CommitText(pb::CommitText {
        session_id: session,
        reading: "よみ".into(),
        surface: surface.into(),
    })
}

fn commit_text(e: &Engine, session: u64, surface: &str) {
    let r = send(e, commit_text_req(session, surface));
    assert!(matches!(r, envelope::Body::Ack(_)), "{r:?}");
}

fn set_limits(e: &Engine, limits: ContextLimits) {
    *e.inner.limits.write().unwrap() = limits;
}

fn history_len(e: &Engine, session: u64) -> usize {
    lock_mutex(&e.inner.sessions)[&session].history.len()
}

/// 周辺テキストなしで変換し、LM が受けた (左文脈, 借りたか) を返す
fn convert_context(e: &Engine, mock: &Arc<MockLm>, session: u64) -> (String, bool) {
    *mock.last_context.lock().unwrap() = None;
    convert(e, session, READING, 0);
    mock.last_context
        .lock()
        .unwrap()
        .clone()
        .expect("the ranker saw a context")
}

/// 周辺テキストつきで変換し、LM が受けた (左文脈, 借りたか) を返す
fn convert_with_surrounding(
    e: &Engine,
    mock: &Arc<MockLm>,
    session: u64,
    left: &str,
    available: bool,
) -> (String, bool) {
    *mock.last_context.lock().unwrap() = None;
    send(
        e,
        envelope::Body::Convert(pb::Convert {
            session_id: session,
            reading: READING.into(),
            context: Some(pb::SurroundingText {
                left: left.into(),
                right: String::new(),
                available,
            }),
            context_rev: 0,
            page_size: 0,
        }),
    );
    mock.last_context
        .lock()
        .unwrap()
        .clone()
        .expect("the ranker saw a context")
}

fn short_limits() -> ContextLimits {
    ContextLimits {
        borrow_window: Duration::from_millis(150),
        recent_commits_ttl: Duration::from_millis(400),
        ..ContextLimits::DEFAULT
    }
}

#[test]
fn begin_with_the_same_id_keeps_the_history() {
    let (e, _mock) = engine(0);
    begin_with(&e, 1, false, false);
    commit_text(&e, 1, "今日は");
    begin_with(&e, 1, false, false);
    assert_eq!(history_len(&e, 1), 1);
    // secure が変わったら切る
    begin_with(&e, 1, true, false);
    assert_eq!(history_len(&e, 1), 0);
    begin_with(&e, 1, false, false);
    commit_text(&e, 1, "晴れ");
    assert_eq!(history_len(&e, 1), 1);
}

#[test]
fn end_session_pauses_and_begin_resumes_with_the_history() {
    let (e, mock) = engine(0);
    begin_with(&e, 1, false, false);
    commit_text(&e, 1, "今日は");
    end(&e, 1);
    {
        let sessions = lock_mutex(&e.inner.sessions);
        assert!(sessions[&1].paused_at.is_some(), "paused, not removed");
        assert_eq!(sessions[&1].history.len(), 1);
    }
    // 休止中は要求に答えない（今までと同じく UNKNOWN_SESSION）
    assert_eq!(
        error_code(&send(&e, convert_req(1, READING, 0, 0))),
        ErrorCode::UnknownSession
    );
    assert_eq!(
        error_code(&send(&e, commit_text_req(1, "x"))),
        ErrorCode::UnknownSession
    );
    begin_with(&e, 1, false, false);
    assert!(lock_mutex(&e.inner.sessions)[&1].paused_at.is_none());
    assert_eq!(convert_context(&e, &mock, 1), ("今日は".to_owned(), false));
}

#[test]
fn a_paused_session_is_dropped_after_the_ttl() {
    let (e, _mock) = engine(0);
    set_limits(
        &e,
        ContextLimits {
            paused_session_ttl: Duration::from_millis(100),
            ..ContextLimits::DEFAULT
        },
    );
    begin_with(&e, 1, false, false);
    begin_with(&e, 2, false, false);
    commit_text(&e, 1, "今日は");
    end(&e, 1);
    // TTL の前は残る。動いている 2 は消えない
    e.inner.expire_context(Instant::now());
    assert!(lock_mutex(&e.inner.sessions).contains_key(&1));
    e.inner
        .expire_context(Instant::now() + Duration::from_millis(150));
    let sessions = lock_mutex(&e.inner.sessions);
    assert!(!sessions.contains_key(&1), "expired");
    assert!(sessions.contains_key(&2), "active sessions stay");
}

#[test]
fn the_default_ttl_is_thirty_minutes() {
    let (e, _mock) = engine(0);
    begin_with(&e, 1, false, false);
    end(&e, 1);
    e.inner
        .expire_context(Instant::now() + Duration::from_secs(29 * 60));
    assert!(lock_mutex(&e.inner.sessions).contains_key(&1));
    e.inner
        .expire_context(Instant::now() + Duration::from_secs(31 * 60));
    assert!(!lock_mutex(&e.inner.sessions).contains_key(&1));
}

#[test]
fn history_is_capped_by_characters_not_count() {
    let (e, mock) = engine(0);
    set_limits(
        &e,
        ContextLimits {
            history_max_chars: 10,
            ..ContextLimits::DEFAULT
        },
    );
    begin_with(&e, 1, false, false);
    // 1 文字ずつ 40 回。件数の上限（32）では切れず、文字数（10）で切れる
    for _ in 0..40 {
        commit_text(&e, 1, "あ");
    }
    assert_eq!(history_len(&e, 1), 10);
    commit_text(&e, 1, "いうえお");
    {
        let sessions = lock_mutex(&e.inner.sessions);
        let total: usize = sessions[&1]
            .history
            .iter()
            .map(|c| c.surface.chars().count())
            .sum();
        assert!(total <= 10, "{total}");
        assert_eq!(sessions[&1].history.last().unwrap().surface, "いうえお");
    }
    // 件数 32 を超えても、文字数の上限までは持つ
    set_limits(&e, ContextLimits::DEFAULT);
    begin_with(&e, 2, false, false);
    for _ in 0..100 {
        commit_text(&e, 2, "あ");
    }
    assert_eq!(history_len(&e, 2), 100);
    let (left, _) = convert_context(&e, &mock, 2);
    assert_eq!(left.chars().count(), 100);
}

#[test]
fn a_single_long_commit_is_kept() {
    let (e, _mock) = engine(0);
    set_limits(
        &e,
        ContextLimits {
            history_max_chars: 3,
            ..ContextLimits::DEFAULT
        },
    );
    begin_with(&e, 1, false, false);
    commit_text(&e, 1, "あいうえお");
    assert_eq!(history_len(&e, 1), 1);
}

#[test]
fn an_empty_field_borrows_the_other_apps_recent_commit() {
    let (e, mock) = engine(0);
    begin_with(&e, 1, false, true);
    begin_with(&e, 2, false, true);
    commit_text(&e, 1, "明日のリリース");
    assert_eq!(
        convert_context(&e, &mock, 2),
        ("明日のリリース".to_owned(), true)
    );
    // 自分の履歴があれば、それが左文脈で、借りたことにはならない
    assert_eq!(
        convert_context(&e, &mock, 1),
        ("明日のリリース".to_owned(), false)
    );
}

#[test]
fn it_does_not_borrow_when_the_field_has_its_own_context() {
    let (e, mock) = engine(0);
    begin_with(&e, 1, false, true);
    begin_with(&e, 2, false, true);
    commit_text(&e, 1, "別のアプリの文");
    // 同じ入力欄の履歴がある
    commit_text(&e, 2, "こちらの文");
    assert_eq!(
        convert_context(&e, &mock, 2),
        ("こちらの文".to_owned(), false)
    );
    // 周辺テキストが取れている（空でも available なら文頭として扱い、借りない）
    begin_with(&e, 3, false, true);
    assert_eq!(
        convert_with_surrounding(&e, &mock, 3, "", true),
        (String::new(), false)
    );
    // 周辺テキストが取れない (available=false) なら借りる
    begin_with(&e, 4, false, true);
    let (left, borrowed) = convert_with_surrounding(&e, &mock, 4, "", false);
    assert!(borrowed && left.contains("別のアプリの文"), "{left}");
}

#[test]
fn borrowed_commits_are_joined_in_time_order() {
    let (e, mock) = engine(0);
    begin_with(&e, 1, false, true);
    begin_with(&e, 2, false, true);
    begin_with(&e, 3, false, true);
    commit_text(&e, 1, "一つ目。");
    commit_text(&e, 2, "二つ目。");
    assert_eq!(
        convert_context(&e, &mock, 3),
        ("一つ目。二つ目。".to_owned(), true)
    );
}

#[test]
fn the_borrow_window_and_the_recent_ttl_expire() {
    let (e, mock) = engine(0);
    set_limits(&e, short_limits());
    begin_with(&e, 1, false, true);
    begin_with(&e, 2, false, true);
    commit_text(&e, 1, "借りる文");
    assert!(convert_context(&e, &mock, 2).1, "inside the window");
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        convert_context(&e, &mock, 2),
        (String::new(), false),
        "outside the borrow window"
    );
    // 10 分（ここでは 400ms）を過ぎたら「最近の確定」からも消える
    assert_eq!(lock_mutex(&e.inner.recent).len(), 1);
    e.inner
        .expire_context(Instant::now() + Duration::from_millis(500));
    assert!(lock_mutex(&e.inner.recent).is_empty());
}

#[test]
fn recent_commits_are_capped_by_characters() {
    let (e, _mock) = engine(0);
    set_limits(
        &e,
        ContextLimits {
            recent_commits_max_chars: 6,
            ..ContextLimits::DEFAULT
        },
    );
    begin_with(&e, 1, false, true);
    for s in ["あああ", "いいい", "ううう"] {
        commit_text(&e, 1, s);
    }
    let recent = lock_mutex(&e.inner.recent);
    let all: Vec<&str> = recent.iter().map(|r| r.surface.as_str()).collect();
    assert_eq!(all, ["いいい", "ううう"]);
}

#[test]
fn share_context_false_neither_lends_nor_borrows() {
    let (e, mock) = engine(0);
    begin_with(&e, 1, false, false); // 除外されたアプリ
    begin_with(&e, 2, false, true);
    begin_with(&e, 3, false, true);
    commit_text(&e, 1, "秘密の文");
    commit_text(&e, 3, "共有の文");
    assert!(
        lock_mutex(&e.inner.recent)
            .iter()
            .all(|r| r.session_id != 1),
        "not recorded"
    );
    // 2 は共有の文だけを借りる
    assert_eq!(convert_context(&e, &mock, 2), ("共有の文".to_owned(), true));
    // 除外のセッションは借りない（自分の履歴だけ）
    begin_with(&e, 4, false, false);
    assert_eq!(convert_context(&e, &mock, 4), (String::new(), false));
    assert_eq!(
        convert_context(&e, &mock, 1),
        ("秘密の文".to_owned(), false)
    );
}

#[test]
fn secure_commits_go_nowhere_and_secure_sessions_do_not_borrow() {
    let (e, mock) = engine(0);
    begin_with(&e, 1, true, true);
    begin_with(&e, 2, false, true);
    begin_with(&e, 3, false, true);
    commit_text(&e, 1, "パスワード");
    commit_text(&e, 3, "ふつうの文");
    assert!(
        lock_mutex(&e.inner.recent)
            .iter()
            .all(|r| r.session_id != 1)
    );
    assert_eq!(history_len(&e, 1), 0);
    assert_eq!(
        convert_context(&e, &mock, 2),
        ("ふつうの文".to_owned(), true)
    );
    // secure の変換は何も借りない
    *mock.last_context.lock().unwrap() = None;
    convert(&e, 1, READING, 0);
    let seen = mock.last_context.lock().unwrap().clone();
    assert!(seen.is_none_or(|(left, borrowed)| left.is_empty() && !borrowed));
}

#[test]
fn a_commit_from_a_paused_session_is_still_lent() {
    let (e, mock) = engine(0);
    begin_with(&e, 1, false, true);
    begin_with(&e, 2, false, true);
    commit_text(&e, 1, "移る前の文");
    end(&e, 1);
    assert_eq!(
        convert_context(&e, &mock, 2),
        ("移る前の文".to_owned(), true)
    );
}

#[test]
fn the_share_flag_follows_the_latest_begin() {
    let (e, mock) = engine(0);
    begin_with(&e, 1, false, true);
    begin_with(&e, 2, false, true);
    commit_text(&e, 1, "文");
    begin_with(&e, 2, false, false);
    assert_eq!(convert_context(&e, &mock, 2), (String::new(), false));
}
