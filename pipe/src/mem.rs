//! メモリ上の `Duplex` と `Listener`（テスト用。OS に依存しない）。
//!
//! 片方の書きがもう片方の読みになる。名前付きパイプと同じく、書く側（または読む側）を落とすと相手に EOF
//! （書きは `BrokenPipe`）が伝わり、`Shutdown` は両方向を閉じてブロック中の読みを起こす。

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use crate::duplex::{Duplex, Halves, Listener, Shutdown};

/// 一方向のバイト列の溜め。
#[derive(Default)]
struct Chan {
    state: Mutex<ChanState>,
    readable: Condvar,
}

#[derive(Default)]
struct ChanState {
    buf: VecDeque<u8>,
    closed: bool,
}

impl Chan {
    fn lock(&self) -> MutexGuard<'_, ChanState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn close(&self) {
        self.lock().closed = true;
        self.readable.notify_all();
    }
}

struct MemReader(Arc<Chan>);

impl Read for MemReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let mut state = self.0.lock();
        loop {
            if !state.buf.is_empty() {
                let n = out.len().min(state.buf.len());
                for (slot, byte) in out.iter_mut().zip(state.buf.drain(..n)) {
                    *slot = byte;
                }
                return Ok(n);
            }
            if state.closed {
                return Ok(0);
            }
            state = self
                .0
                .readable
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
    }
}

impl Drop for MemReader {
    fn drop(&mut self) {
        self.0.close();
    }
}

struct MemWriter(Arc<Chan>);

impl Write for MemWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut state = self.0.lock();
        if state.closed {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        state.buf.extend(data);
        drop(state);
        self.0.readable.notify_all();
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for MemWriter {
    fn drop(&mut self) {
        self.0.close();
    }
}

struct MemShutdown {
    incoming: Arc<Chan>,
    outgoing: Arc<Chan>,
}

impl Shutdown for MemShutdown {
    fn shutdown(&self) {
        self.incoming.close();
        self.outgoing.close();
    }
}

/// メモリ上の流れの片側。
pub struct MemDuplex {
    incoming: Arc<Chan>,
    outgoing: Arc<Chan>,
}

impl Duplex for MemDuplex {
    fn split(self) -> Halves {
        let shutdown = MemShutdown {
            incoming: Arc::clone(&self.incoming),
            outgoing: Arc::clone(&self.outgoing),
        };
        (
            Box::new(MemReader(self.incoming)),
            Box::new(MemWriter(self.outgoing)),
            Arc::new(shutdown),
        )
    }
}

/// つながった 2 つの流れ（片方の書きがもう片方の読みになる）。
pub fn duplex_pair() -> (MemDuplex, MemDuplex) {
    let a_to_b = Arc::new(Chan::default());
    let b_to_a = Arc::new(Chan::default());
    (
        MemDuplex {
            incoming: Arc::clone(&b_to_a),
            outgoing: Arc::clone(&a_to_b),
        },
        MemDuplex {
            incoming: a_to_b,
            outgoing: b_to_a,
        },
    )
}

/// メモリ上の受け口。`MemConnector::connect` で作った接続を `accept` が返す。
/// `MemConnector` がすべて落ちると、`accept` は `Ok(None)`（受け口が閉じた）を返す。
pub struct MemListener {
    incoming: Receiver<MemDuplex>,
}

/// `MemListener` に接続する側。複製できる。全部落とすと、受け口は閉じる。
#[derive(Clone)]
pub struct MemConnector {
    tx: Sender<MemDuplex>,
}

pub fn listener() -> (MemListener, MemConnector) {
    let (tx, incoming) = mpsc::channel();
    (MemListener { incoming }, MemConnector { tx })
}

impl MemConnector {
    /// 接続し、クライアント側の流れを返す。受け口が落ちていれば `None`。
    pub fn connect(&self) -> Option<MemDuplex> {
        let (client, server) = duplex_pair();
        self.tx.send(server).ok()?;
        Some(client)
    }
}

impl Listener for MemListener {
    type Conn = MemDuplex;

    fn accept(&mut self) -> io::Result<Option<MemDuplex>> {
        Ok(self.incoming.recv().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_flow_both_ways() {
        let (a, b) = duplex_pair();
        let (mut ar, mut aw, _) = a.split();
        let (mut br, mut bw, _) = b.split();
        aw.write_all(b"ping").unwrap();
        bw.write_all(b"pong").unwrap();
        let mut buf = [0u8; 4];
        br.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"ping");
        ar.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"pong");
    }

    #[test]
    fn dropping_the_writer_gives_eof_after_the_buffered_bytes() {
        let (a, b) = duplex_pair();
        let (_ar, mut aw, _) = a.split();
        let (mut br, _bw, _) = b.split();
        aw.write_all(b"xy").unwrap();
        drop(aw);
        let mut all = Vec::new();
        br.read_to_end(&mut all).unwrap();
        assert_eq!(all, b"xy");
    }

    #[test]
    fn shutdown_wakes_a_blocked_reader_and_fails_writes() {
        let (a, b) = duplex_pair();
        let (mut ar, _aw, shutdown) = a.split();
        let (_br, mut bw, _) = b.split();
        let reader = std::thread::spawn(move || {
            let mut buf = [0u8; 1];
            ar.read(&mut buf).unwrap()
        });
        shutdown.shutdown();
        assert_eq!(reader.join().unwrap(), 0);
        assert_eq!(
            bw.write(b"x").unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn listener_closes_when_all_connectors_drop() {
        let (mut l, c) = listener();
        let client = c.connect().unwrap();
        assert!(l.accept().unwrap().is_some());
        drop(c);
        assert!(l.accept().unwrap().is_none());
        drop(client);
    }
}
