//! Windows の TIP（ADR 0040 §2.2）。**この crate の純粋な部分は OS に依存しない**（`cfg(windows)` なし。Mac でテストが回る）。
//!
//! - `keys`: Windows の仮想キー（VK）と修飾、OS が返した文字 → `ShellEvent`
//! - `ops`: `ShellAction` → `TsfOp` の列
//! - `tsf_op`: 純粋な部分と COM の層の間の契約。COM の層は `TsfOp` を順に実行するだけで、判断を持たない
//! - `transport`: `pipe::Client` を `shell-core` の `EngineTransport` に適合させる（落ちた通知を含む）
//! - `com`（`cfg(windows)`。W2-b）: TSF の COM の層（登録、キーイベント、編集セッション）。`unsafe` はここだけに置く
//!
//! 判断は `shell-core` と、`keys`・`ops` の表にだけ置く。TSF の用語（`ITf…`）は `com` の外に持ち込まない。

#![deny(unsafe_code)]

pub mod keys;
pub mod ops;
pub mod transport;
pub mod tsf_op;

#[cfg(windows)]
#[allow(unsafe_code)]
mod com;

pub use tsf_op::TsfOp;
