//! 設定アプリとの連絡（移行の前の Swift の `FrameCodec`、`RelayPolicy`、`BridgeProtocol`、`BridgeServer` の判断。
//! ADR 0005 §6.3、ADR 0041 §4.3。レーン L-B）。
//!
//! OS 側に残すもの: ソケットの待ち受け、接続元の確認（同じユーザー、署名）、接続ごとの読み書きのスレッド、
//! 書き込みの時間の上限（`SO_SNDTIMEO`）。ここは、枠の切り出しと組み立て、最初の枠の照合、中継の許可リスト、
//! 中継の同時数、枠ごとの振り分けを持つ。

pub mod frame_codec;
pub mod proto;
pub mod protocol;
pub mod relay_gate;
pub mod relay_policy;
pub mod server;

pub use frame_codec::{FrameDecoder, FrameError};
pub use proto::{BridgeErrorCode, Frame};
pub use protocol::{BRIDGE_VERSION, Handshake, MAX_FRAME_BYTES, SOCKET_RELATIVE_PATH, handshake};
pub use relay_gate::{RelayGate, RelayPermit};
pub use server::{RelayJob, Route, route};
