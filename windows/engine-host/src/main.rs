//! ikari-engine-host: Windows の常駐のエンジン（ADR 0040 §2.2）。
//!
//! 受け付けの繰り返しは `ikari_engine_host::serve`（OS に依存しない。Mac でテストする）。
//! 名前付きパイプの `Listener` と起動の手順（エンジンを開く、ロック、終了）は W1-b 以降で `main` に足す。

fn main() {}
