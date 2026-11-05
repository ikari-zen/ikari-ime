//! 異常終了の記録（ADR 0051 §3.4、§5.1）。C0 の先書きのテスト。R2 で中身を入れて `#[ignore]` を外す。
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
#[ignore = "R2 で印を扱う実装を入れたら外す（ADR 0051 §8）"]
fn a_crashed_previous_run_is_the_first_log_line() {
    let dir = temp_dir("crash-record-prev");
    write_mark(&dir, "v=1 start_ms=1793000000123\nexit=crash sig=11\n");
    let e = Engine::from_parts(parts(Some(dir.clone())));
    let line = first_log_line(&e, &dir);
    assert_eq!(line.split_whitespace().nth(1), Some("prev_exit"), "{line}");
    assert!(line.contains("exit=crash sig=11"), "{line}");
}

#[test]
#[ignore = "R2 で印を扱う実装を入れたら外す（ADR 0051 §8）"]
fn no_previous_mark_writes_no_prev_exit_line() {
    let dir = temp_dir("crash-record-none");
    let e = Engine::from_parts(parts(Some(dir.clone())));
    assert!(!first_log_line(&e, &dir).contains("prev_exit"));
    assert!(mark_path(&dir).exists());
}

#[test]
#[ignore = "R2 で印を扱う実装を入れたら外す（ADR 0051 §8）"]
fn drop_ends_the_mark_with_exit_clean() {
    let dir = temp_dir("crash-record-clean");
    drop(Engine::from_parts(parts(Some(dir.clone()))));
    let mark = std::fs::read_to_string(mark_path(&dir)).unwrap();
    assert!(mark.ends_with("exit=clean\n"), "{mark:?}");
}

#[test]
#[ignore = "R2 で印を扱う実装を入れたら外す（ADR 0051 §8）"]
fn an_engine_without_the_lock_does_not_touch_the_mark() {
    // ロックを取れなかったエンジンは、印を読まない・作り直さない（2 つのエンジンが同じ印を扱わない）
    todo!("R2: ロックを取れない Engine::open の作り方を決めてから書く");
}

#[test]
#[ignore = "R2 で印を扱う実装を入れたら外す（ADR 0051 §8）"]
fn a_disabled_log_creates_no_mark() {
    let dir = temp_dir("crash-record-disabled");
    let _e = Engine::from_parts(parts(None));
    assert!(!mark_path(&dir).exists());
}

#[test]
#[ignore = "R2 で印を扱う実装を入れたら外す（ADR 0051 §8）"]
fn log_cleanup_keeps_the_run_marks() {
    // 回した世代の後始末（`engine-*` と `engine.log.*` を探す）が `engine.run` と `shell.run` を消さない
    todo!("R2: log.rs の後始末を呼ぶ小さな入口を決めてから書く");
}
