//! メモリ上の `Duplex` と `Listener`（テスト用。OS に依存しない）。

use std::io;
use std::sync::mpsc::{Receiver, Sender};

use crate::duplex::{Duplex, Halves, Listener};

/// メモリ上の流れの片側。
pub struct MemDuplex {
    _private: (),
}

impl Duplex for MemDuplex {
    fn split(self) -> Halves {
        todo!()
    }
}

/// つながった 2 つの流れ（片方の書きがもう片方の読みになる）。
pub fn duplex_pair() -> (MemDuplex, MemDuplex) {
    todo!()
}

/// メモリ上の受け口。`MemConnector::connect` で作った接続を `accept` が返す。
pub struct MemListener {
    _incoming: Receiver<MemDuplex>,
}

/// `MemListener` に接続する側。複製できる。全部落とすと、受け口は閉じる。
#[derive(Clone)]
pub struct MemConnector {
    _tx: Sender<MemDuplex>,
}

pub fn listener() -> (MemListener, MemConnector) {
    todo!()
}

impl MemConnector {
    /// 接続し、クライアント側の流れを返す。受け口が落ちていれば `None`。
    pub fn connect(&self) -> Option<MemDuplex> {
        todo!()
    }
}

impl Listener for MemListener {
    type Conn = MemDuplex;

    fn accept(&mut self) -> io::Result<Option<MemDuplex>> {
        todo!()
    }
}
