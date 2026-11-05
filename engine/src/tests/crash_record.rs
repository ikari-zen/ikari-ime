//! 異常終了の記録（ADR 0051 §3.4、§5.1）。C0 で先書きし、R2 で実装して `#[ignore]` を外した。
//!
//! 前の印は `write_mark` で `logs/engine.run` に置く。

use super::*;

fn mark_path(dir: &Path) -> PathBuf {
    dir.join("logs").join("engine.run")
}

fn write_mark(dir: &Path, body: &str) {
    std::fs::create_dir_all(dir.join("logs")).unwrap();
    std::fs::write(mark_path(dir), body).unwrap();
}

fn parts(data_dir: Option<PathBuf>) -> EngineParts {
    EngineParts {
        stack: mock_stack(&MockLm::new(Duration::ZERO), None),
        rebuild: None,
        data_dir,
        lm_requested: false,
        lm_budget: LmBudget::default(),
        convert_wait: DEFAULT_CONVERT_WAIT,
        lock: None,
    }
}

fn first_log_line(e: &Engine, dir: &Path) -> String {
    e.inner.log.flush_for_test();
    std::fs::read_to_string(dir.join("logs").join("engine.log"))
        .unwrap_or_default()
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn a_crashed_previous_run_is_the_first_log_line() {
    let dir = temp_dir("crash-record-prev");
    write_mark(&dir, "v=1 start_ms=1793000000123\nexit=crash sig=11\n");
    let e = Engine::from_parts(parts(Some(dir.clone())));
    let line = first_log_line(&e, &dir);
    assert_eq!(line.split_whitespace().nth(1), Some("prev_exit"), "{line}");
    assert!(line.contains("exit=crash sig=11"), "{line}");
}

#[test]
fn no_previous_mark_writes_no_prev_exit_line() {
    let dir = temp_dir("crash-record-none");
    let e = Engine::from_parts(parts(Some(dir.clone())));
    assert!(!first_log_line(&e, &dir).contains("prev_exit"));
    assert!(mark_path(&dir).exists());
}

#[test]
fn drop_ends_the_mark_with_exit_clean() {
    let dir = temp_dir("crash-record-clean");
    drop(Engine::from_parts(parts(Some(dir.clone()))));
    let mark = std::fs::read_to_string(mark_path(&dir)).unwrap();
    assert!(mark.ends_with("exit=clean\n"), "{mark:?}");
}

#[test]
fn an_engine_without_the_lock_does_not_touch_the_mark() {
    // ロックを取れなかったエンジンは、印を読まない・作り直さない（2 つのエンジンが同じ印を扱わない）
    let dir = temp_dir("crash-record-locked");
    let body = "v=1 start_ms=1793000000123\nexit=crash sig=11\n";
    write_mark(&dir, body);
    let _held = EngineLock::acquire(&dir).unwrap();
    let cfg = EngineConfig {
        data_dir: dir.to_string_lossy().into_owned(),
        ..EngineConfig::default()
    };
    assert!(matches!(Engine::open(&cfg), Err(EngineError::Open(_))));
    assert_eq!(std::fs::read_to_string(mark_path(&dir)).unwrap(), body);
}

#[test]
fn a_disabled_log_creates_no_mark() {
    let dir = temp_dir("crash-record-disabled");
    let _e = Engine::from_parts(parts(None));
    assert!(!mark_path(&dir).exists());
}

#[test]
fn panic_lines_carry_a_listed_thread_and_a_narrowed_location() {
    let dir = temp_dir("crash-record-panic");
    let log = EngineLog::open(&dir);
    crate::diag::write_panic_line(&log, "ikari-prefetch", "engine/src/lib.rs:12");
    crate::diag::write_panic_line(&log, "worker-of-someone", "/Users/someone/.cargo/x.rs:1");
    log.flush_for_test();
    let text = std::fs::read_to_string(dir.join("logs").join("engine.log")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert_eq!(lines[0].split_whitespace().nth(1), Some("panic"));
    assert!(lines[0].contains("thread=prefetch loc=engine/src/lib.rs:12"));
    assert!(
        lines[1].ends_with("thread=other loc=external"),
        "{}",
        lines[1]
    );
}

// 回した世代の後始末が `*.run` に触らないテストは `log.rs` の `tidy_leaves_the_run_marks_alone`
