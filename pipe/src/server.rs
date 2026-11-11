//! サーバー（常駐のエンジンの側）。受け付けを繰り返し、接続ごとにスレッドを持つ。

use std::io;

use crate::duplex::{Duplex, Listener};

/// 受け付けの繰り返し。
pub struct Server<L: Listener> {
    _listener: L,
}

impl<L: Listener> Server<L> {
    pub fn new(listener: L) -> Server<L> {
        Server {
            _listener: listener,
        }
    }

    /// 受け口が閉じられる（`accept` が `Ok(None)`）まで受け付ける。接続ごとに `make_handler` で
    /// 取り出したハンドラ（`FnMut(&[u8]) -> Option<Vec<u8>>`）を、専用のスレッドで回す。
    /// 戻る前に、残っている接続を閉じてスレッドを待つ。
    pub fn serve<H, F>(self, _make_handler: F) -> io::Result<()>
    where
        F: FnMut() -> H,
        H: FnMut(&[u8]) -> Option<Vec<u8>> + Send + 'static,
    {
        todo!()
    }
}

/// 接続 1 本を処理する（`Server` が接続ごとのスレッドで呼ぶ）。
///
/// `Request` はハンドラの返事を `Reply` で、`None` なら `Error`（本体は空）で返す。`Post` は返事を捨てる。
/// 壊れた枠、上限超過、サーバーが受けない種類の枠が来たら閉じる。
pub fn serve_connection<D, H>(_duplex: D, _handler: H)
where
    D: Duplex,
    H: FnMut(&[u8]) -> Option<Vec<u8>>,
{
    todo!()
}
