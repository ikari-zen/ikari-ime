//! 名前付きパイプへの接続（`transport::Connector` の Windows の実装）。
//!
//! パイプの名前は `pipe::pipe_name(ユーザーの SID, セッション番号)`。SID とセッション番号の取得、重ね合わせの I/O の
//! `Duplex` は名前付きパイプの OS 層（W1-b。`pipe/src/os/windows.rs`。#315）が持つ。この層は、それを呼ぶだけにする。
//!
//! TODO(W1-b): #315 が main に入ったら、`open` を次の形にする（`pipe` の公開の口）:
//! `let name = ikari_pipe::os::windows::current_pipe_name().ok()?;`
//! `let (duplex, pid) = ikari_pipe::os::windows::connect_duplex(&name).ok()?;`
//! `Some((duplex, pid))`
//! それまでは、つながらない（`TransportError::Unavailable`。キーは失わず、漢字に変換できないだけ）。
//!
//! エンジンの起動（常駐の `engine-host` がないとき）は W5（S3）。

use ikari_pipe::mem::MemDuplex;

use crate::transport::Connector;

pub struct PipeConnector;

impl Connector for PipeConnector {
    // W1-b の `NamedPipeDuplex` に替える。型を合わせるために、いまはメモリ上の流れの型を置く（作らない）
    type Conn = MemDuplex;

    fn open(&self) -> Option<(MemDuplex, Option<u32>)> {
        None
    }
}
