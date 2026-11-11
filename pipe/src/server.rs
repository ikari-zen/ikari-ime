//! サーバー（常駐のエンジンの側）。受け付けを繰り返し、接続ごとにスレッドを持つ。
//!
//! 1 本の接続の要求は、届いた順に 1 つずつハンドラへ渡す（ハンドラは `FnMut`）。別の接続とは並行に動く。

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::duplex::{Duplex, Listener, Shutdown};
use crate::frame::{Frame, FrameError, FrameKind, read_frame, write_frame};

/// 受け付けの失敗を続けてやり直す回数（これを超えたら受け付けをやめてエラーで戻る）
const MAX_CONSECUTIVE_ACCEPT_ERRORS: u32 = 8;
/// 受け付けの失敗のあと、やり直すまでの待ち
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(50);
/// 接続のスレッドの名前
const CONNECTION_THREAD_NAME: &str = "ikari-pipe-conn";

/// 受け付けの繰り返し。
pub struct Server<L: Listener> {
    listener: L,
}

struct Connection {
    thread: JoinHandle<()>,
    shutdown: Arc<dyn Shutdown>,
}

impl<L: Listener> Server<L> {
    pub fn new(listener: L) -> Server<L> {
        Server { listener }
    }

    /// 受け口が閉じられる（`accept` が `Ok(None)`）まで受け付ける。接続ごとに `make_handler` で
    /// 取り出したハンドラ（`FnMut(&[u8]) -> Option<Vec<u8>>`）を、専用のスレッドで回す。
    /// 戻る前に、残っている接続を閉じてスレッドを待つ。
    ///
    /// `accept` の失敗は `MAX_CONSECUTIVE_ACCEPT_ERRORS` 回まで続けてやり直し、超えたら最後のエラーで戻る。
    pub fn serve<H, F>(mut self, mut make_handler: F) -> io::Result<()>
    where
        F: FnMut() -> H,
        H: FnMut(&[u8]) -> Option<Vec<u8>> + Send + 'static,
    {
        let mut connections: Vec<Connection> = Vec::new();
        let mut errors = 0u32;
        let outcome = loop {
            match self.listener.accept() {
                Ok(Some(conn)) => {
                    errors = 0;
                    reap_finished(&mut connections);
                    let (reader, writer, shutdown) = conn.split();
                    let mut handler = make_handler();
                    let conn_shutdown = Arc::clone(&shutdown);
                    let spawned = std::thread::Builder::new()
                        .name(CONNECTION_THREAD_NAME.to_owned())
                        .spawn(move || {
                            run_connection(reader, writer, &conn_shutdown, &mut handler)
                        });
                    match spawned {
                        Ok(thread) => connections.push(Connection { thread, shutdown }),
                        // スレッドを作れない。この接続だけ閉じて続ける
                        Err(_) => shutdown.shutdown(),
                    }
                }
                Ok(None) => break Ok(()),
                Err(e) => {
                    errors += 1;
                    if errors > MAX_CONSECUTIVE_ACCEPT_ERRORS {
                        break Err(e);
                    }
                    std::thread::sleep(ACCEPT_RETRY_DELAY);
                }
            }
        };
        for conn in &connections {
            conn.shutdown.shutdown();
        }
        for conn in connections {
            let _ = conn.thread.join();
        }
        outcome
    }
}

fn reap_finished(connections: &mut Vec<Connection>) {
    let mut live = Vec::with_capacity(connections.len());
    for conn in connections.drain(..) {
        if conn.thread.is_finished() {
            let _ = conn.thread.join();
        } else {
            live.push(conn);
        }
    }
    *connections = live;
}

/// 接続 1 本を処理する（`Server` が接続ごとのスレッドで呼ぶ）。
///
/// `Request` はハンドラの返事を `Reply` で、`None` なら `Error`（本体は空）で返す。`Post` は返事を捨てる。
/// 壊れた枠、上限超過、サーバーが受けない種類の枠が来たら閉じる。
pub fn serve_connection<D, H>(duplex: D, mut handler: H)
where
    D: Duplex,
    H: FnMut(&[u8]) -> Option<Vec<u8>>,
{
    let (reader, writer, shutdown) = duplex.split();
    run_connection(reader, writer, &shutdown, &mut handler);
}

fn run_connection(
    mut reader: Box<dyn Read + Send>,
    mut writer: Box<dyn Write + Send>,
    shutdown: &Arc<dyn Shutdown>,
    handler: &mut dyn FnMut(&[u8]) -> Option<Vec<u8>>,
) {
    loop {
        let frame = match read_frame(&mut reader) {
            Ok(Some(frame)) => frame,
            // 相手が閉じた、壊れた枠、上限超過
            Ok(None) | Err(_) => break,
        };
        match frame.kind {
            FrameKind::Request => {
                let reply = match handler(&frame.body) {
                    Some(body) => Frame {
                        kind: FrameKind::Reply,
                        seq: frame.seq,
                        body,
                    },
                    None => error_frame(frame.seq),
                };
                match write_frame(&mut writer, &reply) {
                    Ok(()) => {}
                    // 返事が上限を超えた。接続は壊さず、待っている側には Error で知らせる
                    Err(FrameError::TooLarge(_)) => {
                        if write_frame(&mut writer, &error_frame(frame.seq)).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            FrameKind::Post => {
                let _ = handler(&frame.body);
            }
            // クライアントが送らない種類
            FrameKind::Reply | FrameKind::Error => break,
        }
    }
    shutdown.shutdown();
}

fn error_frame(seq: u64) -> Frame {
    Frame {
        kind: FrameKind::Error,
        seq,
        body: Vec::new(),
    }
}
