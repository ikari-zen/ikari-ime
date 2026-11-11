//! クライアント（TIP の側）。要求と返事を通し番号で対応付ける（多重化）。

use std::time::Duration;

use crate::duplex::Duplex;

/// `Client` の誤り。`shell-core` の `TransportError` への対応は、使う側（W2）が決める。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientError {
    /// 待ちの上限を超えた（接続は生きている。遅れて届いた返事は捨てる）
    Timeout,
    /// つながりが切れた、または閉じた（壊れた枠、上限超過を含む）
    Disconnected,
    /// 本体が枠の上限を超える（何も送っていない。接続は生きている）
    TooLarge,
    /// サーバーが返事を作れなかった（`FrameKind::Error`）
    Rejected,
}

/// 接続 1 本のクライアント。すべて `&self` で、複数のスレッドから同時に呼んでよい。
/// `Drop` で閉じる。切れたら作り直す（再接続は使う側の仕事）。
pub struct Client {
    _private: (),
}

impl Client {
    /// 流れから作る。`server_pid` は OS 層が相手のプロセスの番号を取れたときに渡す。
    pub fn new<D: Duplex>(_duplex: D, _server_pid: Option<u32>) -> Client {
        todo!()
    }

    /// 要求を送り、返事を `timeout` まで待つ。
    pub fn request(&self, _bytes: &[u8], _timeout: Duration) -> Result<Vec<u8>, ClientError> {
        todo!()
    }

    /// 返事を待たずに送る。
    pub fn post(&self, _bytes: &[u8]) -> Result<(), ClientError> {
        todo!()
    }

    /// 閉じる。待っている `request` は `Disconnected` で戻る。
    pub fn close(&self) {
        todo!()
    }

    /// 相手のプロセスの番号。分からなければ `None`。
    pub fn server_pid(&self) -> Option<u32> {
        todo!()
    }
}
