//! クライアント（TIP の側）。要求と返事を通し番号で対応付ける（多重化）。
//!
//! 読みは専用のスレッドが 1 本で行い、枠の通し番号で、待っている `request` に返事を渡す。
//! 書きはロックで 1 枠ずつ直列にする。

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::duplex::{Duplex, Shutdown};
use crate::frame::{Frame, FrameError, FrameKind, read_frame, write_frame};
use crate::mux::{Outcome, Pending};

/// 読みのスレッドの名前
const READER_THREAD_NAME: &str = "ikari-pipe-client-reader";
/// 通し番号の最初の値（0 は使わない。ログで「番号なし」と区別するため）
const FIRST_SEQ: u64 = 1;

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

struct Inner {
    pending: Pending,
    writer: Mutex<Box<dyn Write + Send>>,
    shutdown: Arc<dyn Shutdown>,
    next_seq: AtomicU64,
}

impl Inner {
    fn writer(&self) -> MutexGuard<'_, Box<dyn Write + Send>> {
        self.writer.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 流れを閉じ、待っているすべての `request` を `Disconnected` で戻す。
    fn disconnect(&self) {
        self.pending.close_all();
        self.shutdown.shutdown();
    }

    fn send(&self, kind: FrameKind, seq: u64, body: &[u8]) -> Result<(), ClientError> {
        let frame = Frame {
            kind,
            seq,
            body: body.to_vec(),
        };
        let result = write_frame(&mut *self.writer(), &frame);
        match result {
            Ok(()) => Ok(()),
            // 何も書いていないので、接続は生きている
            Err(FrameError::TooLarge(_)) => Err(ClientError::TooLarge),
            // 途中まで書いたかもしれない。流れが壊れたので切る（締め切りを超えた `TimedOut` もここ。
            // 送れなかったものは捨てる。再接続は使う側の仕事）
            Err(_) => {
                self.disconnect();
                Err(ClientError::Disconnected)
            }
        }
    }
}

/// 接続 1 本のクライアント。すべて `&self` で、複数のスレッドから同時に呼んでよい。
/// `Drop` で閉じる。切れたら作り直す（再接続は使う側の仕事）。
pub struct Client {
    inner: Arc<Inner>,
    reader: Mutex<Option<JoinHandle<()>>>,
    server_pid: Option<u32>,
}

impl Client {
    /// 流れから作る。`server_pid` は OS 層が相手のプロセスの番号を取れたときに渡す。
    ///
    /// 読みのスレッドを起こせなかったときは、最初から切れた状態（`request` は `Disconnected`）になる。
    pub fn new<D: Duplex>(duplex: D, server_pid: Option<u32>) -> Client {
        let (mut reader, writer, shutdown) = duplex.split();
        let inner = Arc::new(Inner {
            pending: Pending::default(),
            writer: Mutex::new(writer),
            shutdown,
            next_seq: AtomicU64::new(FIRST_SEQ),
        });
        let thread_inner = Arc::clone(&inner);
        let spawned = std::thread::Builder::new()
            .name(READER_THREAD_NAME.to_owned())
            .spawn(move || {
                loop {
                    match read_frame(&mut reader) {
                        Ok(Some(Frame {
                            kind: FrameKind::Reply,
                            seq,
                            body,
                        })) => {
                            thread_inner.pending.complete(seq, Outcome::Reply(body));
                        }
                        Ok(Some(Frame {
                            kind: FrameKind::Error,
                            seq,
                            ..
                        })) => {
                            thread_inner.pending.complete(seq, Outcome::Rejected);
                        }
                        // サーバーが送らない種類（Request、Post）、枠の境目の EOF、壊れた枠、上限超過
                        Ok(Some(_)) | Ok(None) | Err(_) => break,
                    }
                }
                thread_inner.disconnect();
            });
        let reader = match spawned {
            Ok(handle) => Some(handle),
            Err(_) => {
                inner.disconnect();
                None
            }
        };
        Client {
            inner,
            reader: Mutex::new(reader),
            server_pid,
        }
    }

    fn next_seq(&self) -> u64 {
        self.inner.next_seq.fetch_add(1, Ordering::Relaxed)
    }

    /// 要求を送り、返事を `timeout` まで待つ。`timeout` は返事の待ちだけにかかる。送る側の待ちは
    /// `WRITE_DEADLINE` で別に切る（超えたら接続を切って `Disconnected`）。
    pub fn request(&self, bytes: &[u8], timeout: Duration) -> Result<Vec<u8>, ClientError> {
        let seq = self.next_seq();
        let waiter = self
            .inner
            .pending
            .register(seq)
            .ok_or(ClientError::Disconnected)?;
        if let Err(e) = self.inner.send(FrameKind::Request, seq, bytes) {
            self.inner.pending.cancel(seq);
            return Err(e);
        }
        match waiter.recv_timeout(timeout) {
            Ok(Outcome::Reply(body)) => Ok(body),
            Ok(Outcome::Rejected) => Err(ClientError::Rejected),
            Ok(Outcome::Disconnected) | Err(RecvTimeoutError::Disconnected) => {
                Err(ClientError::Disconnected)
            }
            Err(RecvTimeoutError::Timeout) => {
                self.inner.pending.cancel(seq);
                Err(ClientError::Timeout)
            }
        }
    }

    /// 返事を待たずに送る。書きが締め切り（`WRITE_DEADLINE`）までに終わらなければ、接続を切って `Disconnected`。
    pub fn post(&self, bytes: &[u8]) -> Result<(), ClientError> {
        let seq = self.next_seq();
        self.inner.send(FrameKind::Post, seq, bytes)
    }

    /// 閉じる。待っている `request` は `Disconnected` で戻る。
    pub fn close(&self) {
        self.inner.disconnect();
    }

    /// 相手のプロセスの番号。分からなければ `None`。
    pub fn server_pid(&self) -> Option<u32> {
        self.server_pid
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.inner.disconnect();
        let handle = self.reader.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }
}
