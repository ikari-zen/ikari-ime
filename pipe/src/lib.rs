//! ikari-pipe: TIP とエンジンをつなぐ、枠つきの双方向の流れ（ADR 0040 §2.2、§5）。**契約**。
//!
//! - 枠（`frame`）: `u32`（LE。以降のバイト数）、`u8` の種類、`u64`（LE）の通し番号、本体。
//!   本体は `Envelope` のバイト列で、この crate は中を見ない。
//! - 流れ（`duplex`）: `Duplex`（読み書きの双方向）と `Listener`（接続の受け口）。名前付きパイプの OS 層
//!   （W1-b。`cfg(windows)`）はこの 2 つを実装して差し込む。それ以外は OS に依存しない。
//! - `Client`: 要求を送って返事を通し番号で対応付ける（多重化。複数のスレッドから同時に呼んでよい）。
//! - `Server`: 受け付けを繰り返し、接続ごとにスレッドを持って `FnMut(&[u8]) -> Option<Vec<u8>>` に渡す。
//! - `name`: パイプの名前（ユーザーの SID とセッションを含める）。SID を取るのは OS 層なので、文字列で受ける。
//!   止める口（名前つきのイベント）の名前もここで作る（ADR 0068 §6.2）。
//! - `peer`: 口の相手が、インストール先の決まった実行ファイルかの照合（3 つの口で共有する規則。ADR 0068 §2 の 2）。
//! - `limits`: 口ごとの上限と締め切りの表（接続の数、枠の大きさ、枠の途中の締め切り。値は仮置き。ADR 0068 §7）。
//! - `mem`: メモリ上の `Duplex` と `Listener`（テスト用。Mac で全部試せる）。
//! - `os`: 名前付きパイプの OS 層（`os::windows`。`cfg(windows)`。W1-b）。
//!
//! `shell-core` には依存しない（依存の向きは ADR 0040 §2.2）。`unsafe` は `os::windows` の中だけに置く。

#![deny(unsafe_code)]

pub mod client;
pub mod duplex;
pub mod elevation;
pub mod frame;
pub mod limits;
pub mod mem;
mod mux;
pub mod name;
pub mod os;
pub mod peer;
pub mod server;

pub use client::{Client, ClientError};
pub use duplex::{Duplex, Listener, Shutdown};
pub use frame::{Frame, FrameError, FrameKind, MAX_FRAME_LEN};
pub use limits::{Port, PortLimits, Refusal, limits};
pub use name::{
    CANDIDATE_NAME_PREFIX, PipeNameError, SETTINGS_NAME_PREFIX, STOP_EVENT_NAME_PREFIX,
    candidate_pipe_name, pipe_name, settings_pipe_name, stop_event_name,
};
pub use server::{Server, serve_connection, serve_connection_with};
