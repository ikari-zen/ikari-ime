//! データの更新、ロック、ログ、並行

use super::*;

#[test]
fn convert_prefetch_and_page_are_not_blocked_while_the_stack_is_rebuilt() {
    use std::sync::mpsc;
    let dir = temp_dir("rebuild-nonblocking");
    let mock = MockLm::new(Duration::ZERO);
    let learning_path = dir.join("learning.tsv");
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let rebuild: StackBuilder = {
        let (mock, release, learning_path) = (mock.clone(), release.clone(), learning_path.clone());
        let entered_tx = Mutex::new(entered_tx);
        Box::new(move |_| {
            let _ = lock_mutex(&entered_tx).send(());
            // 組み立て直しが長引いている状態を作る
            let (flag, cv) = &*release;
            let mut open = lock_mutex(flag);
            while !*open {
                open = cv.wait(open).unwrap();
            }
            Ok(mock_stack(&mock, Some(&learning_path)))
        })
    };
    let e = Arc::new(Engine::from_parts(EngineParts {
        stack: mock_stack(&mock, Some(&learning_path)),
        rebuild: Some(rebuild),
        data_dir: Some(dir.clone()),
        lm_requested: true,
        lm_budget: LmBudget::default(),
        convert_wait: DEFAULT_CONVERT_WAIT,
        lock: None,
    }));
    begin(&e, 1, false);
    let open_gate = || {
        let (flag, cv) = &*release;
        *lock_mutex(flag) = true;
        cv.notify_all();
    };
    // ファイルを書き換えて、確認の間隔を過ぎた状態にする
    std::fs::write(UserDir::new(&dir).user_dict_tsv(), "ひんし\t品詞\t名詞\n").unwrap();
    lock_mutex(e.inner.watch.as_ref().unwrap()).last_check =
        Instant::now() - DATA_CHECK_INTERVAL * 2;

    // 変換の経路が組み立て直しを自分で始めて待たされていないこと（別スレッドで測り、詰まっても止まらないようにする）
    let (done_tx, done_rx) = mpsc::channel::<Duration>();
    let worker = {
        let e = Arc::clone(&e);
        std::thread::spawn(move || {
            let started = Instant::now();
            prefetch(&e, 1, READING, 0);
            let conv = convert(&e, 1, READING, 0);
            let _ = page(&e, conv.conversion_id, 1);
            let _ = done_tx.send(started.elapsed());
        })
    };
    let elapsed = done_rx.recv_timeout(Duration::from_secs(5));
    // 見回りのスレッドが組み立て直しに入る（入ったあとも変換は待たされない）
    let entered = entered_rx.recv_timeout(Duration::from_secs(5)).is_ok();
    let during = if entered {
        let started = Instant::now();
        prefetch(&e, 1, READING, 1);
        let conv = convert(&e, 1, READING, 1);
        let _ = page(&e, conv.conversion_id, 1);
        Some(started.elapsed())
    } else {
        None
    };
    open_gate();
    let _ = worker.join();
    let elapsed = elapsed.expect("convert path was blocked by the rebuild");
    assert!(elapsed < Duration::from_secs(1), "{elapsed:?}");
    assert!(entered, "the janitor never started the rebuild");
    assert!(during.unwrap() < Duration::from_secs(1), "{during:?}");
    drop(e);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn open_rejects_bad_config_and_releases_the_lock_on_failure() {
    let empty = EngineConfig::default();
    assert!(matches!(Engine::open(&empty), Err(EngineError::Config(_))));
    let missing = EngineConfig {
        data_dir: "/nonexistent/ikari-data".into(),
        ..EngineConfig::default()
    };
    assert!(matches!(
        Engine::open(&missing),
        Err(EngineError::Config(_))
    ));

    // 辞書がない
    let dir = temp_dir("open-fail");
    let cfg = EngineConfig {
        data_dir: dir.to_string_lossy().into_owned(),
        enable_lm: false,
        idle_unload_secs: 300,
    };
    assert!(matches!(Engine::open(&cfg), Err(EngineError::Open(_))));
    assert!(
        EngineLock::acquire(&dir).is_ok(),
        "the lock is released after a failed open"
    );

    // 別のエンジンがロックを持っている
    let held = EngineLock::acquire(&dir).unwrap();
    match Engine::open(&cfg) {
        Err(EngineError::Open(m)) => assert!(m.contains("lock"), "{m}"),
        other => panic!("{other:?}"),
    }
    drop(held);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn engine_holds_the_data_lock_while_alive() {
    let dir = temp_dir("lock");
    let mock = MockLm::new(Duration::ZERO);
    let lock = EngineLock::acquire(&dir).unwrap();
    let e = Engine::from_parts(EngineParts {
        stack: mock_stack(&mock, None),
        rebuild: None,
        data_dir: Some(dir.clone()),
        lm_requested: true,
        lm_budget: LmBudget::default(),
        convert_wait: DEFAULT_CONVERT_WAIT,
        lock: Some(lock),
    });
    assert!(matches!(EngineLock::acquire(&dir), Err(LockError::Held)));
    drop(e);
    assert!(EngineLock::acquire(&dir).is_ok());
    // ログは文字列を含まない
    let log = std::fs::read_to_string(dir.join("logs").join("engine.log")).unwrap_or_default();
    assert!(!log.contains(READING));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn log_lines_never_contain_the_reading() {
    let dir = temp_dir("log-privacy");
    let mock = MockLm::new(Duration::ZERO);
    let e = Engine::from_parts(EngineParts {
        stack: mock_stack(&mock, None),
        rebuild: None,
        data_dir: Some(dir.clone()),
        lm_requested: true,
        lm_budget: LmBudget::default(),
        convert_wait: DEFAULT_CONVERT_WAIT,
        lock: None,
    });
    begin(&e, 1, false);
    let r = convert(&e, 1, READING, 0);
    send(
        &e,
        envelope::Body::CommitText(pb::CommitText {
            session_id: 1,
            reading: READING.into(),
            surface: BOOSTED.into(),
        }),
    );
    let _ = page(&e, r.conversion_id + 100, 0);
    drop(e);
    let log = std::fs::read_to_string(dir.join("logs").join("engine.log")).unwrap();
    assert!(log.contains(" convert "), "{log}");
    assert!(log.contains("err=UNKNOWN_CONVERSION"), "{log}");
    for secret in [READING, BOOSTED, BASE_TOP] {
        assert!(!log.contains(secret), "{secret} leaked into the log");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn concurrent_requests_are_safe() {
    let (e, _mock) = engine(1);
    let e = Arc::new(e);
    for s in 1..=4 {
        begin(&e, s, false);
    }
    let threads: Vec<_> = (1..=4u64)
        .map(|s| {
            let e = Arc::clone(&e);
            std::thread::spawn(move || {
                for rev in 0..30 {
                    prefetch(&e, s, READING, rev);
                    let r = convert(&e, s, READING, rev);
                    assert!(!r.candidates.is_empty());
                    if rev % 3 == 0 {
                        let _ = page(&e, r.conversion_id, 1);
                    }
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
}

// ---- engine.lm_deadline_ms / engine.convert_wait_ms は Engine::open のときだけ読む ----

#[test]
fn open_reads_the_timeout_settings_and_clamps_them() {
    let mut real = Real::new();
    // 設定がなければ今の定数のまま
    assert_eq!(real.e().inner.lm_budget, LmBudget::default());
    assert_eq!(real.e().inner.convert_wait, DEFAULT_CONVERT_WAIT);

    let write = |real: &Real, text: &str| {
        std::fs::write(real.dir.join(SETTINGS_FILE_NAME), text).unwrap();
    };
    real.engine = None;
    write(
        &real,
        "engine.lm_deadline_ms\t700\nengine.convert_wait_ms\t0\n",
    );
    real.open();
    assert_eq!(real.e().inner.lm_budget.max, Duration::from_millis(700));
    // max 以外（base、per_char）は変えない
    assert_eq!(real.e().inner.lm_budget.base, LmBudget::default().base);
    assert_eq!(real.e().inner.convert_wait, Duration::ZERO);

    real.engine = None;
    write(
        &real,
        "engine.lm_deadline_ms\t9\nengine.convert_wait_ms\t99999\n",
    );
    real.open();
    assert_eq!(real.e().inner.lm_budget.max, Duration::from_millis(50));
    assert_eq!(real.e().inner.convert_wait, Duration::from_millis(200));

    real.engine = None;
    write(
        &real,
        "engine.lm_deadline_ms\t-3\nengine.convert_wait_ms\tx\n",
    );
    real.open();
    assert_eq!(real.e().inner.lm_budget, LmBudget::default());
    assert_eq!(real.e().inner.convert_wait, DEFAULT_CONVERT_WAIT);
}
