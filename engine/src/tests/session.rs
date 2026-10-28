//! セッション、確定、学習の保存、確定履歴

use super::*;

#[test]
fn commit_learns_and_end_session_saves() {
    let dir = temp_dir("learn");
    let mock = MockLm::new(Duration::ZERO);
    let path = dir.join("learning.tsv");
    let e = engine_with(&mock, Some(&path));
    begin(&e, 1, false);
    let r = convert(&e, 1, READING, 0);
    let index = surfaces(&r).iter().position(|s| *s == BOOSTED).unwrap();
    let ack = send(
        &e,
        envelope::Body::Commit(pb::Commit {
            conversion_id: r.conversion_id,
            candidate_index: index as u32,
        }),
    );
    assert!(matches!(ack, envelope::Body::Ack(_)));
    assert!(!e.inner.stack().learning.as_ref().unwrap().is_empty());
    assert!(
        !path.exists(),
        "saving is deferred to the 5 second timer or EndSession"
    );
    // 確定した変換はもう引けない
    assert_eq!(
        error_code(&page(&e, r.conversion_id, 0)),
        ErrorCode::UnknownConversion
    );
    // 確定は履歴になる
    assert_eq!(lock_mutex(&e.inner.sessions)[&1].history.len(), 1);
    // 範囲外の候補
    let r2 = convert(&e, 1, READING, 1);
    let bad = send(
        &e,
        envelope::Body::Commit(pb::Commit {
            conversion_id: r2.conversion_id,
            candidate_index: 9999,
        }),
    );
    assert_eq!(error_code(&bad), ErrorCode::BadRequest);

    send(
        &e,
        envelope::Body::EndSession(pb::EndSession { session_id: 1 }),
    );
    assert!(path.exists(), "EndSession saves");
    let reopened = LearningStore::open(&path).unwrap();
    assert!(!reopened.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dropping_the_engine_saves_pending_learning() {
    let dir = temp_dir("drop-save");
    let mock = MockLm::new(Duration::ZERO);
    let path = dir.join("learning.tsv");
    let e = engine_with(&mock, Some(&path));
    begin(&e, 1, false);
    let r = convert(&e, 1, READING, 0);
    send(
        &e,
        envelope::Body::Commit(pb::Commit {
            conversion_id: r.conversion_id,
            candidate_index: 0,
        }),
    );
    assert!(!path.exists());
    drop(e);
    assert!(path.exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_save_timer_writes_after_the_interval() {
    let dir = temp_dir("timer");
    let mock = MockLm::new(Duration::ZERO);
    let path = dir.join("learning.tsv");
    let e = engine_with(&mock, Some(&path));
    begin(&e, 1, false);
    let r = convert(&e, 1, READING, 0);
    send(
        &e,
        envelope::Body::Commit(pb::Commit {
            conversion_id: r.conversion_id,
            candidate_index: 0,
        }),
    );
    // 5 秒待たずに、最後の保存を過去にして後始末を 1 回回す
    lock_mutex(&e.inner.save).last_save = Instant::now() - SAVE_INTERVAL - Duration::from_secs(1);
    e.inner.tick();
    assert!(path.exists());
    assert!(!lock_mutex(&e.inner.save).dirty);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn end_session_drops_its_conversions_and_cache() {
    let (e, _mock) = engine(0);
    begin(&e, 1, false);
    prefetch(&e, 1, READING, 0);
    let r = convert(&e, 1, READING, 0);
    send(
        &e,
        envelope::Body::EndSession(pb::EndSession { session_id: 1 }),
    );
    assert_eq!(lock_mutex(&e.inner.cache).len(), 0);
    assert_eq!(
        error_code(&page(&e, r.conversion_id, 0)),
        ErrorCode::UnknownConversion
    );
    assert_eq!(
        error_code(&send(&e, convert_req(1, READING, 0, 0))),
        ErrorCode::UnknownSession
    );
}

#[test]
fn cancel_forgets_the_conversion() {
    let (e, _mock) = engine(0);
    begin(&e, 1, false);
    let r = convert(&e, 1, READING, 0);
    send(
        &e,
        envelope::Body::Cancel(pb::Cancel {
            conversion_id: r.conversion_id,
        }),
    );
    assert_eq!(
        error_code(&page(&e, r.conversion_id, 0)),
        ErrorCode::UnknownConversion
    );
}

#[test]
fn commit_text_feeds_the_history_up_to_its_char_limit() {
    let (e, _mock) = engine(0);
    *e.inner.limits.write().unwrap() = crate::context_limits::ContextLimits {
        history_max_chars: 30,
        ..Default::default()
    };
    begin(&e, 1, false);
    for i in 0..40 {
        send(
            &e,
            envelope::Body::CommitText(pb::CommitText {
                session_id: 1,
                reading: "あ".into(),
                surface: format!("文{i}"),
            }),
        );
    }
    let sessions = lock_mutex(&e.inner.sessions);
    let history = &sessions[&1].history;
    let total: usize = history.iter().map(|c| c.surface.chars().count()).sum();
    assert!(total <= 30 && history.len() < 40, "{total}");
    assert_eq!(history.last().unwrap().surface, "文39");
    drop(sessions);
    let too_long = send(
        &e,
        envelope::Body::CommitText(pb::CommitText {
            session_id: 1,
            reading: "あ".repeat(MAX_READING_CHARS + 1),
            surface: "x".into(),
        }),
    );
    assert_eq!(error_code(&too_long), ErrorCode::BadRequest);
}

#[test]
fn history_is_the_left_context_when_the_surrounding_text_is_unavailable() {
    let stack = mock_stack(&MockLm::new(Duration::ZERO), None);
    let history = vec![
        ikari_core::Commit {
            reading: "あ".into(),
            surface: "今日は".into(),
        },
        ikari_core::Commit {
            reading: "い".into(),
            surface: "晴れ".into(),
        },
    ];
    let unavailable = pb::SurroundingText {
        left: "ignored".into(),
        right: String::new(),
        available: false,
    };
    let ctx = Inner::build_context(&stack, false, history.clone(), Some(&unavailable));
    assert_eq!(ctx.left_text, "今日は晴れ");
    assert_eq!(ctx.history.len(), 2);
    let available = pb::SurroundingText {
        left: "本文".into(),
        right: "後ろ".into(),
        available: true,
    };
    let ctx = Inner::build_context(&stack, false, history.clone(), Some(&available));
    assert_eq!(
        (ctx.left_text.as_str(), ctx.right_text.as_str()),
        ("本文", "後ろ")
    );
    let ctx = Inner::build_context(&stack, true, history, Some(&available));
    assert!(ctx.secure && ctx.left_text.is_empty() && ctx.history.is_empty());
}

#[test]
fn char_helpers_cut_on_char_boundaries() {
    assert_eq!(tail_chars("あいうえお", 2), "えお");
    assert_eq!(tail_chars("あい", 5), "あい");
    assert_eq!(tail_chars("", 3), "");
    assert_eq!(head_chars("あいうえお", 2), "あい");
    assert_eq!(head_chars("あい", 5), "あい");
    assert!(char_count_exceeds("あい", 1));
    assert!(!char_count_exceeds("あい", 2));
}

#[test]
fn user_data_change_rebuilds_the_stack_and_keeps_the_learning_store() {
    let dir = temp_dir("reload");
    let mock = MockLm::new(Duration::ZERO);
    let learning_path = dir.join("learning.tsv");
    let rebuilds = Arc::new(AtomicUsize::new(0));
    let rebuild: StackBuilder = {
        let (mock, rebuilds, learning_path) =
            (mock.clone(), rebuilds.clone(), learning_path.clone());
        Box::new(move |reuse| {
            rebuilds.fetch_add(1, Ordering::Relaxed);
            let mut stack = mock_stack(&mock, Some(&learning_path));
            if let Some(store) = &reuse.learning {
                stack.learning = Some(Arc::clone(store));
            }
            Ok(stack)
        })
    };
    let e = Engine::from_parts(EngineParts {
        stack: mock_stack(&mock, Some(&learning_path)),
        rebuild: Some(rebuild),
        data_dir: Some(dir.clone()),
        lm_requested: true,
        lm_budget: LmBudget::default(),
        convert_wait: DEFAULT_CONVERT_WAIT,
        lock: None,
    });
    begin(&e, 1, false);
    let before = e.inner.stack().learning.clone().unwrap();
    let force_check = |e: &Engine| {
        let watch = e.inner.watch.as_ref().unwrap();
        lock_mutex(watch).last_check = Instant::now() - DATA_CHECK_INTERVAL * 2;
    };
    // 変更がなければ組み立て直さない
    force_check(&e);
    e.inner.tick();
    assert_eq!(rebuilds.load(Ordering::Relaxed), 0);
    // 1 秒に 1 回まで。変換の経路は確認も組み立て直しもしない（見回りの `tick` だけが見る）
    convert(&e, 1, READING, 0);
    assert_eq!(rebuilds.load(Ordering::Relaxed), 0);
    std::fs::write(UserDir::new(&dir).user_dict_tsv(), "ひんし\t品詞\t名詞\n").unwrap();
    e.inner.tick();
    assert_eq!(
        rebuilds.load(Ordering::Relaxed),
        0,
        "checked within the last second"
    );
    force_check(&e);
    e.inner.tick();
    assert_eq!(rebuilds.load(Ordering::Relaxed), 1);
    assert!(Arc::ptr_eq(
        &before,
        e.inner.stack().learning.as_ref().unwrap()
    ));
    // 同じ内容のまま確認しても、組み立て直さない
    force_check(&e);
    e.inner.tick();
    assert_eq!(rebuilds.load(Ordering::Relaxed), 1);
    // プロファイルの変更も検出する
    std::fs::write(UserDir::new(&dir).profile_tsv(), "it\toften\n").unwrap();
    force_check(&e);
    e.inner.tick();
    assert_eq!(rebuilds.load(Ordering::Relaxed), 2);
    drop(e);
    let _ = std::fs::remove_dir_all(dir);
}
