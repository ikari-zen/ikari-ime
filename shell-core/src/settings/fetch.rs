//! 取得（`ikari-data`）の OS に依存しない核の置き場所（ADR 0079 §3.6）。
//!
//! 契約コミットでは宣言だけを置く。`windows/settings/src/fetch.rs`（`FetchRequest`、`model_fetch_args`、
//! `Snapshot`、`Phase`、`Running`、`Launcher`、`FetchTask`、行の読み）と `text.rs` の取得の文
//! （`fetch_error`、`fetch_snapshot` など）を、ここへ移す（レーン C。中身は変えない）。
//! 移すときに `ikari-resource` の依存を `shell-core` に足し、`windows/settings` はここを使う。
//!
//! 判断の置き場所: Rust（macOS と Windows で 1 つ）。
