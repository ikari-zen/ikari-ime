//! hello と要求の検査（壊れた入力、上限、スキーマ）

use super::*;

#[test]
fn warm_up_runs_on_another_thread_at_start_and_on_the_first_prefetch_after_unload() {
    let (e, mock) = engine(0);
    begin(&e, 1, false);
    wait_until(|| mock.warm_ups.load(Ordering::Relaxed) == 1);
    let caller = std::thread::current().id();
    assert_ne!(*mock.warm_thread.lock().unwrap(), Some(caller));
    // アイドル解放の再現: 読み込み済みでなくなる。次の Prefetch で warm_up が再び呼ばれる
    wait_until(|| mock.loaded.load(Ordering::Acquire));
    mock.loaded.store(false, Ordering::Release);
    let mut rev = 0;
    wait_until(|| {
        rev += 1;
        prefetch(&e, 1, READING, rev);
        mock.warm_ups.load(Ordering::Relaxed) >= 2
    });
    assert_ne!(*mock.warm_thread.lock().unwrap(), Some(caller));
}

#[test]
fn lm_state_is_disabled_or_failed_without_a_handle() {
    let mock = MockLm::new(Duration::ZERO);
    for (requested, expected) in [(false, LmState::LmDisabled), (true, LmState::LmFailed)] {
        let mut stack = mock_stack(&mock, None);
        stack.lm = None;
        let e = Engine::from_parts(EngineParts {
            stack,
            rebuild: None,
            data_dir: None,
            lm_requested: requested,
            lm_budget: LmBudget::default(),
            lock: None,
        });
        let envelope::Body::HelloReply(h) = send(&e, envelope::Body::Hello(pb::Hello::default()))
        else {
            panic!();
        };
        assert_eq!(h.lm, expected as i32);
    }
}

#[test]
fn broken_bytes_never_panic_and_always_return_an_envelope() {
    let (e, _mock) = engine(0);
    begin(&e, 1, false);
    let valid = Envelope {
        schema_version: SCHEMA_VERSION,
        request_id: 9,
        body: Some(convert_req(1, READING, 0, 0)),
    }
    .encode_to_vec();
    let mut inputs: Vec<Vec<u8>> = vec![
        vec![],
        vec![0xff; 4],
        vec![0x0a; 1000],
        // 巨大な長さを名乗る
        vec![0x82, 0x01, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x00],
        vec![0u8; MAX_REQUEST_BYTES + 1],
    ];
    for cut in 0..valid.len() {
        inputs.push(valid[..cut].to_vec());
    }
    for i in 0..valid.len() {
        for flip in [0xffu8, 0x01, 0x80] {
            let mut v = valid.clone();
            v[i] ^= flip;
            inputs.push(v);
        }
    }
    for input in inputs {
        let reply = e.handle(&input);
        Envelope::decode(reply.as_slice()).expect("reply must decode");
    }
    // 壊れた要求のあとも普通に動く
    assert!(!convert(&e, 1, READING, 0).candidates.is_empty());
}

#[test]
fn oversized_and_empty_requests_are_bad_requests() {
    let (e, _mock) = engine(0);
    let reply = Envelope::decode(e.handle(&vec![0u8; MAX_REQUEST_BYTES + 1]).as_slice()).unwrap();
    assert_eq!(error_code(&reply.body.unwrap()), ErrorCode::BadRequest);
    let empty = Envelope {
        schema_version: SCHEMA_VERSION,
        request_id: 4,
        body: None,
    }
    .encode_to_vec();
    let reply = Envelope::decode(e.handle(&empty).as_slice()).unwrap();
    assert_eq!(reply.request_id, 4);
    assert_eq!(error_code(&reply.body.unwrap()), ErrorCode::BadRequest);
    // 返事の種類を要求として送る
    let r = send(&e, envelope::Body::Ack(pb::Ack {}));
    assert_eq!(error_code(&r), ErrorCode::BadRequest);
}

#[test]
fn schema_mismatch_is_reported_but_shutdown_is_still_accepted() {
    let (e, _mock) = engine(0);
    let r = send_raw(
        &e,
        SCHEMA_VERSION + 1,
        5,
        envelope::Body::Hello(pb::Hello::default()),
    );
    assert_eq!(error_code(&r.body.unwrap()), ErrorCode::SchemaMismatch);
    let r = send_raw(
        &e,
        SCHEMA_VERSION + 1,
        6,
        envelope::Body::Shutdown(pb::Shutdown::default()),
    );
    assert!(matches!(r.body, Some(envelope::Body::Ack(_))));
}

#[test]
fn shutdown_with_a_reason_is_acknowledged() {
    for reason in [
        pb::ShutdownReason::Unspecified,
        pb::ShutdownReason::VersionMismatch,
        pb::ShutdownReason::SettingsChanged,
    ] {
        let (e, _mock) = engine(0);
        let r = send(
            &e,
            envelope::Body::Shutdown(pb::Shutdown {
                reason: reason as i32,
            }),
        );
        assert!(matches!(r, envelope::Body::Ack(_)), "{reason:?}");
    }
}

/// 管理の要求は、データのディレクトリがないエンジン（`from_parts` で渡さない）では受けない
#[test]
fn admin_requests_need_a_data_directory() {
    let (e, _mock) = engine(0);
    let requests = [
        envelope::Body::ListUserDict(pb::UserDictList {}),
        envelope::Body::AddUserDict(pb::UserDictAdd::default()),
        envelope::Body::RemoveUserDict(pb::UserDictRemove::default()),
        envelope::Body::LearningStatus(pb::LearningStatus {}),
        envelope::Body::ResetLearning(pb::LearningReset::default()),
        envelope::Body::ExportData(pb::ExportData {}),
        envelope::Body::ImportData(pb::ImportData::default()),
        envelope::Body::GetProfile(pb::ProfileGet {}),
        envelope::Body::SetProfile(pb::ProfileSet::default()),
    ];
    for request in requests {
        let reply = send(&e, request.clone());
        assert_eq!(error_code(&reply), ErrorCode::EngineNotReady, "{request:?}");
    }
}

#[test]
fn unknown_session_and_conversion_are_reported() {
    let (e, _mock) = engine(0);
    let r = send(&e, convert_req(99, READING, 0, 0));
    assert_eq!(error_code(&r), ErrorCode::UnknownSession);
    let r = send(
        &e,
        envelope::Body::Prefetch(pb::Prefetch {
            session_id: 99,
            reading: READING.into(),
            ..pb::Prefetch::default()
        }),
    );
    assert_eq!(error_code(&r), ErrorCode::UnknownSession);
    assert_eq!(
        error_code(&page(&e, 12345, 0)),
        ErrorCode::UnknownConversion
    );
    let r = send(
        &e,
        envelope::Body::Commit(pb::Commit {
            conversion_id: 12345,
            candidate_index: 0,
        }),
    );
    assert_eq!(error_code(&r), ErrorCode::UnknownConversion);
    // 終わったセッションの EndSession と、知らない変換の Cancel は Ack
    assert!(matches!(
        send(
            &e,
            envelope::Body::EndSession(pb::EndSession { session_id: 99 })
        ),
        envelope::Body::Ack(_)
    ));
    assert!(matches!(
        send(
            &e,
            envelope::Body::Cancel(pb::Cancel {
                conversion_id: 12345
            })
        ),
        envelope::Body::Ack(_)
    ));
}

#[test]
fn input_limits_are_enforced() {
    let (e, _mock) = engine(0);
    begin(&e, 1, false);
    let long = "あ".repeat(MAX_READING_CHARS + 1);
    assert_eq!(
        error_code(&send(&e, convert_req(1, &long, 0, 0))),
        ErrorCode::BadRequest
    );
    assert_eq!(
        error_code(&send(&e, convert_req(1, "", 0, 0))),
        ErrorCode::BadRequest
    );
    assert_eq!(
        error_code(&send(&e, convert_req(1, READING, 0, MAX_PAGE_SIZE + 1))),
        ErrorCode::BadRequest
    );
    let exact = "あ".repeat(MAX_READING_CHARS);
    assert!(matches!(
        send(&e, convert_req(1, &exact, 0, 0)),
        envelope::Body::ConvertReply(_)
    ));
    // 長すぎる文脈は古い側を切って受ける
    let r = send(
        &e,
        envelope::Body::Convert(pb::Convert {
            session_id: 1,
            reading: READING.into(),
            context: Some(pb::SurroundingText {
                left: "あ".repeat(MAX_CONTEXT_CHARS * 3),
                right: "い".repeat(MAX_CONTEXT_CHARS * 3),
                available: true,
            }),
            context_rev: 0,
            page_size: 0,
        }),
    );
    assert!(matches!(r, envelope::Body::ConvertReply(_)), "{r:?}");
    let r = page(&e, 1, 1_000_000);
    assert_eq!(error_code(&r), ErrorCode::BadRequest);
}

#[test]
fn convert_without_prefetch_skips_the_lm() {
    let (e, mock) = engine(0);
    begin(&e, 1, false);
    let r = convert(&e, 1, READING, 0);
    assert_eq!(surfaces(&r)[0], BASE_TOP);
    assert!(!r.lm_applied);
    assert!(
        r.has_next,
        "16 dictionary candidates do not fit in one page"
    );
    assert_eq!(r.candidates.len(), 9);
    assert_eq!(r.page_index, 0);
    assert!(r.conversion_id > 0);
    // 締め切りは「今」なので、モックは時間切れ扱い
    assert!(mock.timeouts.load(Ordering::Relaxed) > 0);
    assert!(!mock.boost.load(Ordering::Relaxed) || surfaces(&r)[0] == BASE_TOP);
}

#[test]
fn diag_is_acked_without_side_effects() {
    let (e, _) = engine(0);
    let reply = send(
        &e,
        envelope::Body::Diag(pb::DiagEvent {
            session_id: 1,
            kind: "sh_episode".into(),
            nums: vec![pb::DiagNum {
                key: "keys".into(),
                value: 3,
            }],
            tags: vec![pb::DiagTag {
                key: "mode".into(),
                value: "romaji".into(),
            }],
        }),
    );
    assert!(matches!(reply, envelope::Body::Ack(_)), "{reply:?}");
}
