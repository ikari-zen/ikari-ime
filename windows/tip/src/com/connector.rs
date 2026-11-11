//! 名前付きパイプへの接続（`transport::Connector` の Windows の実装）。
//!
//! パイプの名前は `pipe::pipe_name(ユーザーの SID, セッション番号)`。SID とセッション番号の取得、重ね合わせの I/O の
//! `Duplex` は名前付きパイプの OS 層（W1-b。`pipe/src/os/windows.rs`。#315）が持つ。この層は、それを呼ぶだけにする。
//!
//! つながらなければ `None`（`TransportError::Unavailable`。キーは失わず、漢字に変換できないだけ）。
//!
//! 相手が本物のエンジンかどうか（インストール先のパスとの一致）の確認は、判断なのでここではしない
//! （`pid` を返すところまで。W5 で確認を足す）。エンジンの起動（常駐の `engine-host` がないとき）も W5（S3）。

use ikari_pipe::os::windows::{NamedPipeDuplex, connect_duplex, current_pipe_name};

use crate::transport::Connector;

pub struct PipeConnector;

impl Connector for PipeConnector {
    type Conn = NamedPipeDuplex;

    fn open(&self) -> Option<(NamedPipeDuplex, Option<u32>)> {
        let name = current_pipe_name().ok()?;
        connect_duplex(&name).ok()
    }
}
