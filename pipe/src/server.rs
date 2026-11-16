//! サーバー（常駐のエンジンの側）。受け付けを繰り返し、接続ごとにスレッドを持つ。
//!
//! 1 本の接続の要求は、届いた順に 1 つずつハンドラへ渡す（ハンドラは `FnMut`）。別の接続とは並行に動く。

use std::io::{self, Read, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::duplex::{Duplex, Listener, Shutdown};
use crate::frame::{Frame, FrameError, FrameKind, read_frame_limited, write_frame_limited};
use crate::limits::{Port, PortLimits, Refusal, limits};

/// 受け付けの失敗を続けてやり直す回数（これを超えたら受け付けをやめてエラーで戻る）
const MAX_CONSECUTIVE_ACCEPT_ERRORS: u32 = 8;
/// 受け付けの失敗のあと、やり直すまでの待ち
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(50);
/// 接続のスレッドの名前
const CONNECTION_THREAD_NAME: &str = "ikari-pipe-conn";
/// 枠の途中の締め切りを見張るスレッドの名前
const WATCHDOG_THREAD_NAME: &str = "ikari-pipe-watch";

/// 断ったことを知らせる先（理由の固定の語だけを受け取る）
type RefusalSink = Arc<dyn Fn(Refusal) + Send + Sync>;

/// 受け付けの繰り返し。
pub struct Server<L: Listener> {
    listener: L,
    limits: PortLimits,
    on_refuse: Option<RefusalSink>,
}

struct Connection {
    thread: JoinHandle<()>,
    shutdown: Arc<dyn Shutdown>,
}

impl<L: Listener> Server<L> {
    /// エンジンの口の上限で作る。ほかの口は `for_port`。
    pub fn new(listener: L) -> Server<L> {
        Server::for_port(listener, Port::Engine)
    }

    /// 口の上限（`limits`）で作る。
    pub fn for_port(listener: L, port: Port) -> Server<L> {
        Server::with_limits(listener, limits(port))
    }

    /// 上限を指定して作る（試験用。製品は `for_port`）。
    pub fn with_limits(listener: L, limits: PortLimits) -> Server<L> {
        Server {
            listener,
            limits,
            on_refuse: None,
        }
    }

    /// 上限や締め切りで断ったときに呼ぶものを付ける。渡すのは `Refusal` だけ（相手の内容は渡さない）。
    pub fn on_refuse(mut self, sink: impl Fn(Refusal) + Send + Sync + 'static) -> Server<L> {
        self.on_refuse = Some(Arc::new(sink));
        self
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
                    if connections.len() >= self.limits.max_connections {
                        // 上限を超えた接続は、ハンドラを作らず、受けてすぐ切る
                        notify(self.on_refuse.as_ref(), Refusal::TooManyConnections);
                        shutdown.shutdown();
                        continue;
                    }
                    let mut handler = make_handler();
                    let conn_shutdown = Arc::clone(&shutdown);
                    let conn_limits = self.limits;
                    let sink = self.on_refuse.clone();
                    let spawned = std::thread::Builder::new()
                        .name(CONNECTION_THREAD_NAME.to_owned())
                        .spawn(move || {
                            run_connection(
                                reader,
                                writer,
                                &conn_shutdown,
                                &mut handler,
                                &conn_limits,
                                sink.as_ref(),
                            )
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

fn notify(sink: Option<&RefusalSink>, reason: Refusal) {
    if let Some(sink) = sink {
        sink(reason);
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
/// 壊れた枠、上限超過、枠の途中の締め切り超過、サーバーが受けない種類の枠が来たら閉じる。
/// 上限は、エンジンの口のもの（`limits(Port::Engine)`）。
pub fn serve_connection<D, H>(duplex: D, handler: H)
where
    D: Duplex,
    H: FnMut(&[u8]) -> Option<Vec<u8>>,
{
    serve_connection_with(duplex, handler, &limits(Port::Engine));
}

/// `serve_connection` の、上限を指定する版。
pub fn serve_connection_with<D, H>(duplex: D, mut handler: H, limits: &PortLimits)
where
    D: Duplex,
    H: FnMut(&[u8]) -> Option<Vec<u8>>,
{
    let (reader, writer, shutdown) = duplex.split();
    run_connection(reader, writer, &shutdown, &mut handler, limits, None);
}

/// 枠の読みの進み具合（枠の途中の締め切りの見張りに渡す）。
#[derive(Default)]
struct FrameClock {
    state: Mutex<ClockState>,
    changed: Condvar,
}

#[derive(Default)]
struct ClockState {
    /// 今の枠の最初のバイトが届いた時刻。枠の境目では `None`
    started: Option<Instant>,
    /// 接続の処理が終わった（見張りも終わる）
    finished: bool,
}

impl FrameClock {
    fn lock(&self) -> MutexGuard<'_, ClockState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn mark_started(&self) {
        let mut st = self.lock();
        if st.started.is_none() {
            st.started = Some(Instant::now());
            drop(st);
            self.changed.notify_all();
        }
    }

    fn mark_frame_done(&self) {
        self.lock().started = None;
    }

    fn finish(&self) {
        self.lock().finished = true;
        self.changed.notify_all();
    }

    /// 枠の途中で `deadline` を超えたら、`shutdown` して戻る。`finish` されても戻る。
    fn watch(&self, deadline: Duration, shutdown: &dyn Shutdown, sink: Option<&RefusalSink>) {
        let mut st = self.lock();
        loop {
            if st.finished {
                return;
            }
            match st.started {
                None => {
                    st = self.changed.wait(st).unwrap_or_else(|e| e.into_inner());
                }
                Some(t) => {
                    let elapsed = t.elapsed();
                    if elapsed >= deadline {
                        drop(st);
                        // 相手が切れを知るより前に、理由を知らせる
                        notify(sink, Refusal::FrameTimeout);
                        shutdown.shutdown();
                        return;
                    }
                    st = self
                        .changed
                        .wait_timeout(st, deadline - elapsed)
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
            }
        }
    }
}

/// 読めたバイトがあれば、枠が始まったことを `FrameClock` に知らせる読み手。
struct ClockedReader<'a> {
    inner: &'a mut (dyn Read + Send),
    clock: &'a FrameClock,
}

impl Read for ClockedReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.clock.mark_started();
        }
        Ok(n)
    }
}

fn run_connection(
    mut reader: Box<dyn Read + Send>,
    mut writer: Box<dyn Write + Send>,
    shutdown: &Arc<dyn Shutdown>,
    handler: &mut dyn FnMut(&[u8]) -> Option<Vec<u8>>,
    limits: &PortLimits,
    sink: Option<&RefusalSink>,
) {
    let clock = Arc::new(FrameClock::default());
    let watcher = {
        let clock = Arc::clone(&clock);
        let shutdown = Arc::clone(shutdown);
        let deadline = limits.frame_read_deadline;
        let sink = sink.cloned();
        std::thread::Builder::new()
            .name(WATCHDOG_THREAD_NAME.to_owned())
            .spawn(move || clock.watch(deadline, shutdown.as_ref(), sink.as_ref()))
    };
    // 見張りを置けない接続は、締め切りを守れないので続けない
    let Ok(watcher) = watcher else {
        shutdown.shutdown();
        return;
    };
    serve_frames(&mut reader, &mut writer, handler, limits, sink, &clock);
    clock.finish();
    let _ = watcher.join();
    shutdown.shutdown();
}

fn serve_frames(
    reader: &mut Box<dyn Read + Send>,
    writer: &mut Box<dyn Write + Send>,
    handler: &mut dyn FnMut(&[u8]) -> Option<Vec<u8>>,
    limits: &PortLimits,
    sink: Option<&RefusalSink>,
    clock: &FrameClock,
) {
    // 相手が閉じた、壊れた枠、上限超過のどれも、ここで抜けて閉じる
    loop {
        let read = read_frame_limited(
            &mut ClockedReader {
                inner: reader.as_mut(),
                clock,
            },
            limits.max_frame_len,
        );
        clock.mark_frame_done();
        let frame = match read {
            Ok(Some(frame)) => frame,
            Err(FrameError::TooLarge(_)) => {
                notify(sink, Refusal::FrameTooLarge);
                break;
            }
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
                match write_frame_limited(writer, &reply, limits.max_frame_len) {
                    Ok(()) => {}
                    // 返事が上限を超えた。接続は壊さず、待っている側には Error で知らせる
                    Err(FrameError::TooLarge(_)) => {
                        if write_frame_limited(
                            writer,
                            &error_frame(frame.seq),
                            limits.max_frame_len,
                        )
                        .is_err()
                        {
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
}

fn error_frame(seq: u64) -> Frame {
    Frame {
        kind: FrameKind::Error,
        seq,
        body: Vec::new(),
    }
}
