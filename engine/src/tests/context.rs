//! 文脈の読み取り（ADR 0015、0046）: 休止と再開、履歴の文字数、ほかのアプリから借りないこと、左文脈の組み立て、secure

use super::*;

use crate::context_limits::ContextLimits;

fn begin_with(e: &Engine, session: u64, secure: bool) {
    let r = send(
        e,
        envelope::Body::BeginSession(pb::BeginSession {
            session_id: session,
            client_bundle_id: "test".into(),
            secure,
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

/// 周辺テキストなしで変換し、LM が受けた左文脈を返す
fn convert_context(e: &Engine, mock: &Arc<MockLm>, session: u64) -> String {
    *mock.last_context.lock().unwrap() = None;
    convert(e, session, READING, 0);
    mock.last_context
        .lock()
        .unwrap()
        .clone()
        .expect("the ranker saw a context")
}

/// 周辺テキストつきで変換し、LM が受けた左文脈を返す
fn convert_with_surrounding(
    e: &Engine,
    mock: &Arc<MockLm>,
    session: u64,
    left: &str,
    available: bool,
) -> String {
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

#[test]
fn begin_with_the_same_id_keeps_the_history() {
    let (e, _mock) = engine(0);
    begin_with(&e, 1, false);
    commit_text(&e, 1, "今日は");
    begin_with(&e, 1, false);
    assert_eq!(history_len(&e, 1), 1);
    // secure が変わったら切る
    begin_with(&e, 1, true);
    assert_eq!(history_len(&e, 1), 0);
    begin_with(&e, 1, false);
    commit_text(&e, 1, "晴れ");
    assert_eq!(history_len(&e, 1), 1);
}

#[test]
fn end_session_pauses_and_begin_resumes_with_the_history() {
    let (e, mock) = engine(0);
    begin_with(&e, 1, false);
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
    begin_with(&e, 1, false);
    assert!(lock_mutex(&e.inner.sessions)[&1].paused_at.is_none());
    assert_eq!(convert_context(&e, &mock, 1), "今日は");
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
    begin_with(&e, 1, false);
    begin_with(&e, 2, false);
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
    begin_with(&e, 1, false);
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
    begin_with(&e, 1, false);
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
    begin_with(&e, 2, false);
    for _ in 0..100 {
        commit_text(&e, 2, "あ");
    }
    assert_eq!(history_len(&e, 2), 100);
    let left = convert_context(&e, &mock, 2);
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
    begin_with(&e, 1, false);
    commit_text(&e, 1, "あいうえお");
    assert_eq!(history_len(&e, 1), 1);
}

#[test]
fn another_apps_commit_never_enters_the_left_context() {
    // ADR 0046 E5: ほかのアプリ（ほかのセッション）の確定は、空の欄の文脈にならない
    let (e, mock) = engine(0);
    begin_with(&e, 1, false);
    begin_with(&e, 2, false);
    commit_text(&e, 1, "明日のリリース");
    // 周辺テキストが取れない欄
    assert_eq!(convert_context(&e, &mock, 2), "");
    // 取れていて左が空の欄、空白だけの欄
    assert_eq!(convert_with_surrounding(&e, &mock, 2, "", true), "");
    assert_eq!(convert_with_surrounding(&e, &mock, 2, " ", true), " ");
    // 自分の履歴は今までどおり
    assert_eq!(convert_context(&e, &mock, 1), "明日のリリース");
    // 休止したセッションの確定も貸さない
    end(&e, 1);
    assert_eq!(convert_context(&e, &mock, 2), "");
}

#[test]
fn coming_back_to_an_app_continues_its_history_within_the_pause_ttl() {
    // ADR 0046 A1: shell-core が同じアプリに同じ ID を使い回す。アプリ A(1) → B(2) → A(1)
    let (e, mock) = engine(0);
    begin_with(&e, 1, false);
    commit_text(&e, 1, "Aの文。");
    end(&e, 1);
    begin_with(&e, 2, false);
    commit_text(&e, 2, "Bの文。");
    end(&e, 2);
    // 29 分後の見回りでは残り、戻ると A の履歴だけが続く（B の文は入らない）
    e.inner
        .expire_context(Instant::now() + Duration::from_secs(29 * 60));
    begin_with(&e, 1, false);
    assert_eq!(convert_context(&e, &mock, 1), "Aの文。");
    commit_text(&e, 1, "続き");
    assert_eq!(convert_context(&e, &mock, 1), "Aの文。続き");
}

#[test]
fn coming_back_after_the_pause_ttl_starts_without_context() {
    let (e, mock) = engine(0);
    begin_with(&e, 1, false);
    commit_text(&e, 1, "Aの文。");
    end(&e, 1);
    begin_with(&e, 2, false);
    e.inner
        .expire_context(Instant::now() + Duration::from_secs(31 * 60));
    assert!(!lock_mutex(&e.inner.sessions).contains_key(&1));
    begin_with(&e, 1, false);
    assert_eq!(convert_context(&e, &mock, 1), "");
}

#[test]
fn carry_fill_adds_the_same_sessions_history_to_a_short_left() {
    // ADR 0046 E1・E2: 左が履歴の接尾辞なら、足りないぶんを履歴から足す。fill = 0 では足さない（既定は 40。ADR 0049）
    let (e, mock) = engine(0);
    begin_with(&e, 1, false);
    commit_text(&e, 1, "品詞の話をした");
    commit_text(&e, 1, "次は");
    let mut off = ContextLimits::DEFAULT;
    off.carry.fill_chars = 0;
    set_limits(&e, off);
    assert_eq!(convert_with_surrounding(&e, &mock, 1, "次は", true), "次は");
    let mut limits = ContextLimits::DEFAULT;
    limits.carry.fill_chars = 40;
    limits.carry.joint = ikari_core::CarryJoint::Period;
    set_limits(&e, limits);
    assert_eq!(
        convert_with_surrounding(&e, &mock, 1, "次は", true),
        "品詞の話をした。次は"
    );
    // 空になった入力欄（送信のあと）。履歴のあとに区切りが入る
    assert_eq!(
        convert_with_surrounding(&e, &mock, 1, "", true),
        "品詞の話をした次は。"
    );
    // 左が履歴の接尾辞でなければ足さない
    assert_eq!(convert_with_surrounding(&e, &mock, 1, "別の", true), "別の");
}

#[test]
fn left_carried_chars_counts_the_part_added_from_history() {
    // ADR 0049 K2: 足した前文の字数（区切りを含む）を LM の文脈に渡す
    let (e, mock) = engine(0);
    begin_with(&e, 1, false);
    commit_text(&e, 1, "品詞の話をした");
    commit_text(&e, 1, "次は");
    let carried = |mock: &Arc<MockLm>| mock.last_carried.lock().unwrap().expect("ranked");
    // fill = 0（既定は 40。ADR 0049）: 足さない
    let mut off = ContextLimits::DEFAULT;
    off.carry.fill_chars = 0;
    set_limits(&e, off);
    convert_with_surrounding(&e, &mock, 1, "次は", true);
    assert_eq!(carried(&mock), 0);
    let mut limits = ContextLimits::DEFAULT;
    limits.carry.fill_chars = 40;
    limits.carry.joint = ikari_core::CarryJoint::Period;
    set_limits(&e, limits);
    // 前文 + 区切り（「品詞の話をした。」の 8 字）
    assert_eq!(
        convert_with_surrounding(&e, &mock, 1, "次は", true),
        "品詞の話をした。次は"
    );
    assert_eq!(carried(&mock), 8);
    // 空の欄: 履歴 + 区切りの全体が足した前文（元の左は 0 字）
    let text = convert_with_surrounding(&e, &mock, 1, "", true);
    assert_eq!(carried(&mock), text.chars().count());
    // 周辺テキストなし（Unavailable）: 履歴の全体
    let text = convert_context(&e, &mock, 1);
    assert_eq!(carried(&mock), text.chars().count());
    assert!(carried(&mock) > 0);
    // 足さない（履歴の接尾辞でない）: 0
    convert_with_surrounding(&e, &mock, 1, "別の", true);
    assert_eq!(carried(&mock), 0);
}

#[test]
fn secure_commits_go_nowhere() {
    let (e, mock) = engine(0);
    begin_with(&e, 1, true);
    begin_with(&e, 2, false);
    commit_text(&e, 1, "パスワード");
    commit_text(&e, 2, "ふつうの文");
    assert_eq!(history_len(&e, 1), 0);
    // secure の変換は何も入れない
    *mock.last_context.lock().unwrap() = None;
    convert(&e, 1, READING, 0);
    let seen = mock.last_context.lock().unwrap().clone();
    assert!(seen.is_none_or(|left| left.is_empty()));
}
