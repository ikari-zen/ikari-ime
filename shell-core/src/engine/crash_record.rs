//! 異常終了の記録の殻側（ADR 0051 §5.2、§8）。C0 の先書きのテスト。R3 で中身と型を入れて `#[ignore]` を外す。
//!
//! 前回の判定（`sh_prev_exit`）と panic の数（`sh_panic`）を、`sh_engine` と同じ相乗りで送る。
//! 型と置き場所は R3 で決めるので、ここは名前と期待だけ。キー操作の状態機械は変えないので、
//! `shell-core/scenarios/` の JSON は足さない。

#[test]
#[ignore = "R3 で相乗りを入れたら外す（ADR 0051 §8）"]
fn a_crashed_previous_run_rides_on_the_first_non_secure_event_once() {
    todo!(
        "前回 crash なら、最初の secure でないセッションの事象に sh_prev_exit が 1 回だけ相乗りする"
    );
}

#[test]
#[ignore = "R3 で相乗りを入れたら外す（ADR 0051 §8）"]
fn prev_exit_is_not_sent_for_a_secure_session() {
    todo!("secure のセッションには相乗りしない（NFR-18）");
}

#[test]
#[ignore = "R3 で相乗りを入れたら外す（ADR 0051 §8）"]
fn prev_exit_is_carried_over_when_the_send_fails() {
    todo!("送れなかったら持ち越す");
}

#[test]
#[ignore = "R3 で相乗りを入れたら外す（ADR 0051 §8）"]
fn sh_panic_count_is_reduced_by_what_was_sent() {
    todo!("sh_panic の n は送った分だけ引く");
}
