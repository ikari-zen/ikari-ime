//! ikari-engine-host: Windows の常駐のエンジン（ADR 0040 §2.2）。
//!
//! 受け付けの繰り返しは `ikari_engine_host::serve`（OS に依存しない。Mac でテストする）。
//! 名前付きパイプの `Listener` と起動の手順（エンジンを開く、ロック、終了）は W5 以降で `main` に足す。
//! 設定画面の口は `ikari_engine_host::settings_pipe`（`bind_settings_listener` で受け口を作り、`serve_settings` を
//! エンジンの `serve` とは別のスレッドで回す。画面の接続が切れても、エンジンの本線は止まらない）。

fn main() {}
