//! Windows の TIP（ADR 0040 §2.2）。**この crate の純粋な部分は OS に依存しない**（`cfg(windows)` なし。Mac でテストが回る）。
//!
//! - `keys`: Windows の仮想キー（VK）と修飾、OS が返した文字 → `ShellEvent`
//! - `latency`: 往復時間の測定（S1。既定でオフ。入力の内容は記録しない）
//! - `mode`: 入力モード（日本語・英語）の表示と切り替えの写し（W4。compartment の値の表、モードの通知）
//! - `ops`: `ShellAction` → `TsfOp` の列
//! - `secure`: secure な入力欄（パスワード、PIN）の判定の表（TSF が返した入力スコープの値 → secure か）
//! - `tsf_op`: 純粋な部分と COM の層の間の契約。COM の層は `TsfOp` を順に実行するだけで、判断を持たない
//! - `transport`: `pipe::Client` を `shell-core` の `EngineTransport` に適合させる（落ちた通知を含む）
//! - `com`（`cfg(windows)`。W2-b）: TSF の COM の層（登録、キーイベント、編集セッション）。`unsafe` はここだけに置く
//!
//! 判断は `shell-core` と、`keys`・`ops` の表にだけ置く。TSF の用語（`ITf…`）は `com` の外に持ち込まない。

#![deny(unsafe_code)]

pub mod keys;
pub mod latency;
pub mod mode;
pub mod ops;
pub mod secure;
pub mod transport;
pub mod tsf_op;

#[cfg(windows)]
#[allow(unsafe_code)]
mod com;

pub use tsf_op::TsfOp;
