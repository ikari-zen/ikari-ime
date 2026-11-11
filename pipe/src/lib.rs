//! ikari-pipe: TIP とエンジンをつなぐ、枠つきの双方向の流れ（ADR 0040 §2.2、§5）。**契約**。
//!
//! - 枠（`frame`）: `u32`（LE。以降のバイト数）、`u8` の種類、`u64`（LE）の通し番号、本体。
//!   本体は `Envelope` のバイト列で、この crate は中を見ない。
//! - 流れ（`duplex`）: `Duplex`（読み書きの双方向）と `Listener`（接続の受け口）。名前付きパイプの OS 層
//!   （W1-b。`cfg(windows)`）はこの 2 つを実装して差し込む。それ以外は OS に依存しない。
//! - `Client`: 要求を送って返事を通し番号で対応付ける（多重化。複数のスレッドから同時に呼んでよい）。
//! - `Server`: 受け付けを繰り返し、接続ごとにスレッドを持って `FnMut(&[u8]) -> Option<Vec<u8>>` に渡す。
//! - `name`: パイプの名前（ユーザーの SID とセッションを含める）。SID を取るのは OS 層なので、文字列で受ける。
//! - `mem`: メモリ上の `Duplex` と `Listener`（テスト用。Mac で全部試せる）。
//! - `os`: 名前付きパイプの OS 層（`os::windows`。`cfg(windows)`。W1-b）。
//!
//! `shell-core` には依存しない（依存の向きは ADR 0040 §2.2）。`unsafe` は `os::windows` の中だけに置く。

#![deny(unsafe_code)]

pub mod client;
pub mod duplex;
pub mod frame;
pub mod mem;
mod mux;
pub mod name;
pub mod os;
pub mod server;

pub use client::{Client, ClientError};
pub use duplex::{Duplex, Listener, Shutdown};
pub use frame::{Frame, FrameError, FrameKind, MAX_FRAME_LEN};
pub use name::{PipeNameError, pipe_name};
pub use server::{Server, serve_connection};
