//! 接続ごとのセッション（ADR 0064 D1・D2、§4.4 の 1）。2 つの `Connection` が同じ `session_id` を使っても、
//! 確定履歴、secure、先行計算、変換の表が混ざらないこと。接続を落とすと、その接続の状態が消えること。

use super::*;

/// 同じ番号を使う 2 つの殻（Windows の TIP の 2 つのアプリ）
const SAME_ID: u64 = 1;

fn send_on(c: &Connection, body: envelope::Body) -> envelope::Body {
    let req = Envelope {
        schema_version: SCHEMA_VERSION,
        request_id: 7,
        body: Some(body),
    }
    .encode_to_vec();
    let reply = Envelope::decode(c.handle(&req).as_slice()).expect("reply must decode");
    assert_eq!(reply.request_id, 7, "request_id is echoed");
    reply.body.expect("reply body")
}

fn ack(body: &envelope::Body) {
    assert!(matches!(body, envelope::Body::Ack(_)), "{body:?}");
}

fn begin_on(c: &Connection, session: u64, secure: bool) {
    ack(&send_on(
        c,
        envelope::Body::BeginSession(pb::BeginSession {
            session_id: session,
            client_bundle_id: "test".into(),
            secure,
        }),
    ));
}

fn commit_text_on(c: &Connection, session: u64, surface: &str) {
    ack(&send_on(
        c,
        envelope::Body::CommitText(pb::CommitText {
            session_id: session,
            reading: "よみ".into(),
            surface: surface.into(),
        }),
    ));
}

fn convert_on(c: &Connection, session: u64) -> pb::ConvertReply {
    match send_on(c, convert_req(session, READING, 0, 0)) {
        envelope::Body::ConvertReply(r) => r,
        other => panic!("unexpected: {other:?}"),
    }
}

/// 周辺テキストなしで変換し、LM が受けた左文脈（= そのセッションの確定履歴）を返す
fn left_context_on(c: &Connection, mock: &Arc<MockLm>, session: u64) -> String {
    *mock.last_context.lock().unwrap() = None;
    convert_on(c, session);
    mock.last_context
        .lock()
        .unwrap()
        .clone()
        .expect("the ranker saw a context")
}

fn page_on(c: &Connection, conversion_id: u64) -> envelope::Body {
    send_on(
        c,
        envelope::Body::Page(pb::PageRequest {
            conversion_id,
            index: 0,
        }),
    )
}

fn sessions_of(e: &Engine, conn: ConnectionId) -> usize {
    lock_mutex(&e.inner.sessions)
        .keys()
        .filter(|k| k.conn == conn)
        .count()
}

#[test]
fn connections_get_their_own_numbers() {
    let (e, _mock) = engine(0);
    let a = e.connect();
    let b = e.connect();
    assert_ne!(a.id(), b.id());
    assert_ne!(a.id(), ConnectionId::DEFAULT);
    assert_ne!(b.id(), ConnectionId::DEFAULT);
}

#[test]
fn history_does_not_cross_connections_with_the_same_session_id() {
    let (e, mock) = engine(0);
    let a = e.connect();
    let b = e.connect();
    begin_on(&a, SAME_ID, false);
    begin_on(&b, SAME_ID, false);
    commit_text_on(&a, SAME_ID, "今日は");
    // B の左文脈に A の確定は入らない（ADR 0046 E5）
    assert_eq!(left_context_on(&b, &mock, SAME_ID), "");
    assert_eq!(left_context_on(&a, &mock, SAME_ID), "今日は");
    // 既定の接続（macOS の経路）の同じ番号とも別
    begin(&e, SAME_ID, false);
    assert!(
        lock_mutex(&e.inner.sessions)[&sk(SAME_ID)]
            .history
            .is_empty()
    );
    assert_eq!(lock_mutex(&e.inner.sessions).len(), 3);
}

#[test]
fn end_session_on_one_connection_leaves_the_other_running() {
    let (e, _mock) = engine(0);
    let a = e.connect();
    let b = e.connect();
    begin_on(&a, SAME_ID, false);
    begin_on(&b, SAME_ID, false);
    ack(&send_on(
        &a,
        envelope::Body::EndSession(pb::EndSession {
            session_id: SAME_ID,
        }),
    ));
    assert!(!convert_on(&b, SAME_ID).candidates.is_empty());
    assert_eq!(
        error_code(&send_on(&a, convert_req(SAME_ID, READING, 0, 0))),
        ErrorCode::UnknownSession
    );
}

#[test]
fn secure_on_one_connection_keeps_the_others_history() {
    let (e, mock) = engine(0);
    let a = e.connect();
    let b = e.connect();
    begin_on(&a, SAME_ID, false);
    begin_on(&b, SAME_ID, false);
    commit_text_on(&a, SAME_ID, "今日は");
    commit_text_on(&b, SAME_ID, "晴れ");
    // A がパスワード欄に入る（同じ番号で secure の BeginSession）
    begin_on(&a, SAME_ID, true);
    assert_eq!(left_context_on(&b, &mock, SAME_ID), "晴れ");
}

#[test]
fn prefetch_does_not_cross_connections() {
    let (e, _mock) = engine(0);
    let a = e.connect();
    let b = e.connect();
    begin_on(&a, SAME_ID, false);
    begin_on(&b, SAME_ID, false);
    ack(&send_on(
        &a,
        envelope::Body::Prefetch(pb::Prefetch {
            session_id: SAME_ID,
            reading: READING.into(),
            context: None,
            context_rev: 0,
        }),
    ));
    wait_until(|| !lock_mutex(&e.inner.cache).any_computing());
    // B は A の先行計算（LM つき）に当たらず、その場で LM なしに変換する
    let rb = convert_on(&b, SAME_ID);
    assert!(!rb.lm_applied, "B must not use A's prefetch");
    let ra = convert_on(&a, SAME_ID);
    assert!(ra.lm_applied, "A uses its own prefetch");
}

#[test]
fn conversions_answer_only_the_connection_that_made_them() {
    let (e, _mock) = engine(0);
    let a = e.connect();
    let b = e.connect();
    begin_on(&a, SAME_ID, false);
    begin_on(&b, SAME_ID, false);
    let r = convert_on(&a, SAME_ID);
    assert_eq!(
        error_code(&page_on(&b, r.conversion_id)),
        ErrorCode::UnknownConversion
    );
    assert_eq!(
        error_code(&send_on(
            &b,
            envelope::Body::Commit(pb::Commit {
                conversion_id: r.conversion_id,
                candidate_index: 0,
            }),
        )),
        ErrorCode::UnknownConversion
    );
    // B の取り消しは A の変換に触れない
    ack(&send_on(
        &b,
        envelope::Body::Cancel(pb::Cancel {
            conversion_id: r.conversion_id,
        }),
    ));
    assert!(matches!(
        page_on(&a, r.conversion_id),
        envelope::Body::PageReply(_)
    ));
    // 既定の接続からも引けない
    assert_eq!(
        error_code(&page(&e, r.conversion_id, 0)),
        ErrorCode::UnknownConversion
    );
}

#[test]
fn dropping_a_connection_drops_its_sessions_and_state() {
    let (e, mock) = engine(0);
    let a = e.connect();
    let b = e.connect();
    let a_id = a.id();
    begin_on(&a, SAME_ID, false);
    begin_on(&a, SAME_ID + 1, false);
    begin_on(&b, SAME_ID, false);
    commit_text_on(&a, SAME_ID, "今日は");
    commit_text_on(&b, SAME_ID, "晴れ");
    ack(&send_on(
        &a,
        envelope::Body::Prefetch(pb::Prefetch {
            session_id: SAME_ID,
            reading: READING.into(),
            context: None,
            context_rev: 0,
        }),
    ));
    wait_until(|| !lock_mutex(&e.inner.cache).any_computing());
    // 休止中のセッションも捨てる（D2。休止にしない）
    ack(&send_on(
        &a,
        envelope::Body::EndSession(pb::EndSession {
            session_id: SAME_ID + 1,
        }),
    ));
    let r = convert_on(&a, SAME_ID);
    assert_eq!(sessions_of(&e, a_id), 2);
    assert_eq!(lock_mutex(&e.inner.conversions).len(), 1);

    drop(a);

    assert_eq!(sessions_of(&e, a_id), 0);
    assert_eq!(lock_mutex(&e.inner.cache).len(), 0, "A's prefetch is gone");
    assert_eq!(lock_mutex(&e.inner.conversions).len(), 0);
    assert!(lock_mutex(&e.inner.ctx_trace).is_empty());
    assert_eq!(
        error_code(&page(&e, r.conversion_id, 0)),
        ErrorCode::UnknownConversion
    );
    // B は残る
    assert_eq!(left_context_on(&b, &mock, SAME_ID), "晴れ");
}

/// `Engine` を先に落としても、残った `Connection` は panic せず `ENGINE_NOT_READY` を返す（`connection.rs` の注記）
#[test]
fn a_connection_that_outlives_the_engine_answers_not_ready() {
    let (e, _mock) = engine(0);
    let a = e.connect();
    begin_on(&a, SAME_ID, false);
    drop(e);
    assert_eq!(
        error_code(&send_on(&a, convert_req(SAME_ID, READING, 0, 0))),
        ErrorCode::EngineNotReady
    );
    // 読めない要求でも返事を返す
    let reply = Envelope::decode(a.handle(&[0xff; 4]).as_slice()).unwrap();
    assert!(matches!(reply.body, Some(envelope::Body::Error(_))));
    drop(a);
}
