use std::sync::mpsc;
use std::thread;

use ikari_pipe::mem::{self, MemConnector, MemDuplex};
use ikari_pipe::{Listener, serve_connection};

use super::*;

fn long() -> Duration {
    ikari_testtime::scaled(Duration::from_secs(5))
}

fn short() -> Duration {
    ikari_testtime::scaled(Duration::from_millis(50))
}

struct MemOpen {
    connector: Mutex<Option<MemConnector>>,
    pid: Option<u32>,
}

impl Connector for MemOpen {
    type Conn = MemDuplex;

    fn open(&self) -> Option<(MemDuplex, Option<u32>)> {
        let connector = self.connector.lock().unwrap().clone()?;
        Some((connector.connect()?, self.pid))
    }
}

/// 落ちた通知を記録する
#[derive(Default)]
struct Recorder(
    Mutex<Vec<(TransportHandle, DownKind)>>,
    Mutex<Option<mpsc::Sender<()>>>,
);

impl DownTarget for Recorder {
    fn down(&self, handle: TransportHandle, kind: DownKind) {
        self.0.lock().unwrap().push((handle, kind));
        if let Some(tx) = &*self.1.lock().unwrap() {
            let _ = tx.send(());
        }
    }
}

/// 受けた要求に `b"re:" + 本体` を返すサーバー。接続ごとの流れを `servers` に渡すので、テストが切れる
struct Fixture {
    transport: PipeTransport<MemOpen>,
    recorder: Arc<Recorder>,
    down: mpsc::Receiver<()>,
    /// サーバー側の接続を切る手段（接続の順）
    kill: mpsc::Receiver<Arc<dyn ikari_pipe::Shutdown>>,
}

fn fixture(pid: Option<u32>) -> Fixture {
    let (mut listener, connector) = mem::listener();
    let (kill_tx, kill) = mpsc::channel();
    thread::spawn(move || {
        while let Ok(Some(duplex)) = listener.accept() {
            let (reader, writer, shutdown) = duplex.split();
            let _ = kill_tx.send(Arc::clone(&shutdown));
            thread::spawn(move || {
                serve_connection(Rejoined(reader, writer, shutdown), |body| {
                    let mut reply = b"re:".to_vec();
                    reply.extend_from_slice(body);
                    Some(reply)
                });
            });
        }
    });
    let recorder = Arc::new(Recorder::default());
    let (tx, down) = mpsc::channel();
    *recorder.1.lock().unwrap() = Some(tx);
    let sink = DownSink::new();
    sink.bind(&recorder);
    let transport = PipeTransport::new(
        MemOpen {
            connector: Mutex::new(Some(connector)),
            pid,
        },
        sink,
    );
    Fixture {
        transport,
        recorder,
        down,
        kill,
    }
}

/// 分けた流れを、もう一度 `Duplex` にまとめる（サーバー側の切る手段をテストが持つため）
struct Rejoined(
    Box<dyn Read + Send>,
    Box<dyn std::io::Write + Send>,
    Arc<dyn ikari_pipe::Shutdown>,
);

impl Duplex for Rejoined {
    fn split(self) -> Halves {
        (self.0, self.1, self.2)
    }
}

#[test]
fn request_round_trips_through_the_pipe_client() {
    let f = fixture(Some(4321));
    let handle = f.transport.connect().unwrap();
    assert_eq!(
        f.transport.request(handle, b"ping", long()).unwrap(),
        b"re:ping"
    );
    assert_eq!(f.transport.pid(handle), Some(4321));
    f.transport.post(handle, b"fire-and-forget");
    assert_eq!(f.transport.request(handle, b"2", long()).unwrap(), b"re:2");
}

#[test]
fn each_connect_gets_a_new_handle() {
    let f = fixture(None);
    let a = f.transport.connect().unwrap();
    let b = f.transport.connect().unwrap();
    assert_ne!(a, b);
    assert_ne!(a.0, 0);
    assert_eq!(f.transport.pid(a), None);
    assert_eq!(f.transport.request(b, b"x", long()).unwrap(), b"re:x");
}

#[test]
fn no_server_is_unavailable() {
    let sink = DownSink::new();
    let transport = PipeTransport::new(
        MemOpen {
            connector: Mutex::new(None),
            pid: None,
        },
        sink,
    );
    assert_eq!(transport.connect(), Err(TransportError::Unavailable));
}

#[test]
fn unknown_or_closed_handles_are_unavailable_and_silent() {
    let f = fixture(None);
    let handle = f.transport.connect().unwrap();
    f.transport.close(handle);
    f.transport.close(handle); // 2 回目は何もしない
    assert_eq!(
        f.transport.request(handle, b"x", short()),
        Err(TransportError::Unavailable)
    );
    f.transport.post(handle, b"x");
    assert_eq!(f.transport.pid(handle), None);
    assert_eq!(
        f.transport.request(TransportHandle(999), b"x", short()),
        Err(TransportError::Unavailable)
    );
    // 自分で閉じた接続は、落ちたと知らせない
    assert!(f.down.recv_timeout(short()).is_err());
    assert!(f.recorder.0.lock().unwrap().is_empty());
}

#[test]
fn server_going_away_notifies_once_with_the_handle() {
    let f = fixture(None);
    let first = f.transport.connect().unwrap();
    let second = f.transport.connect().unwrap();
    let kill_first = f.kill.recv_timeout(long()).unwrap();
    let _kill_second = f.kill.recv_timeout(long()).unwrap();
    kill_first.shutdown();
    f.down.recv_timeout(long()).unwrap();
    assert_eq!(
        *f.recorder.0.lock().unwrap(),
        vec![(first, DownKind::Invalidated)]
    );
    // 切れた接続を閉じても、もう一度は知らせない
    f.transport.close(first);
    assert!(f.down.recv_timeout(short()).is_err());
    assert_eq!(f.recorder.0.lock().unwrap().len(), 1);
    // 別の接続は生きている
    assert_eq!(f.transport.request(second, b"y", long()).unwrap(), b"re:y");
}

#[test]
fn notifications_before_binding_are_dropped() {
    let (mut listener, connector) = mem::listener();
    let server = thread::spawn(move || listener.accept().unwrap().unwrap());
    let transport = PipeTransport::new(
        MemOpen {
            connector: Mutex::new(Some(connector)),
            pid: None,
        },
        DownSink::new(),
    );
    let handle = transport.connect().unwrap();
    let (_reader, _writer, shutdown) = server.join().unwrap().split();
    shutdown.shutdown(); // 結ぶ前に切れても panic しない
    assert_eq!(
        transport.request(handle, b"x", long()),
        Err(TransportError::Unavailable)
    );
}

#[test]
fn timeout_maps_to_timeout() {
    let (mut listener, connector) = mem::listener();
    // 受けるだけで返事をしないサーバー
    let server = thread::spawn(move || listener.accept().unwrap().unwrap());
    let transport = PipeTransport::new(
        MemOpen {
            connector: Mutex::new(Some(connector)),
            pid: None,
        },
        DownSink::new(),
    );
    let handle = transport.connect().unwrap();
    let _server_side = server.join().unwrap();
    assert_eq!(
        transport.request(handle, b"x", short()),
        Err(TransportError::Timeout)
    );
}

#[test]
fn client_errors_map_to_transport_errors() {
    assert_eq!(
        transport_error(ClientError::Timeout),
        TransportError::Timeout
    );
    for error in [
        ClientError::Disconnected,
        ClientError::TooLarge,
        ClientError::Rejected,
    ] {
        assert_eq!(transport_error(error), TransportError::Unavailable);
    }
}

#[test]
fn engine_link_is_a_down_target() {
    fn assert_target<T: DownTarget>() {}
    assert_target::<EngineLink>();
}

/// 接続を開く順に、成功（`true`）か失敗（`false`）を返すコネクタ。開いた回数を数える
struct Scripted {
    inner: MemConnector,
    script: Mutex<Vec<bool>>,
    opens: std::sync::atomic::AtomicUsize,
}

impl Scripted {
    fn new(inner: MemConnector, script: &[bool]) -> Self {
        Self {
            inner,
            script: Mutex::new(script.iter().rev().copied().collect()),
            opens: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn opens(&self) -> usize {
        self.opens.load(Ordering::SeqCst)
    }
}

impl Connector for Arc<Scripted> {
    type Conn = MemDuplex;

    fn open(&self) -> Option<(MemDuplex, Option<u32>)> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        // 台本が尽きたあとは、つなぐ
        let ok = self.script.lock().unwrap().pop().unwrap_or(true);
        if !ok {
            return None;
        }
        Some((self.inner.connect()?, None))
    }
}

/// 返事をするサーバーを動かし、接続ごとの切る手段を返す
fn echo_server() -> (MemConnector, mpsc::Receiver<Arc<dyn ikari_pipe::Shutdown>>) {
    let (mut listener, connector) = mem::listener();
    let (kill_tx, kill) = mpsc::channel();
    thread::spawn(move || {
        while let Ok(Some(duplex)) = listener.accept() {
            let (reader, writer, shutdown) = duplex.split();
            let _ = kill_tx.send(Arc::clone(&shutdown));
            thread::spawn(move || {
                serve_connection(Rejoined(reader, writer, shutdown), |body| {
                    let mut reply = b"re:".to_vec();
                    reply.extend_from_slice(body);
                    Some(reply)
                });
            });
        }
    });
    (connector, kill)
}

fn backoff_initial() -> Duration {
    ikari_testtime::scaled(Duration::from_millis(100))
}

fn backoff_max() -> Duration {
    ikari_testtime::scaled(Duration::from_millis(400))
}

#[test]
fn keys_before_the_engine_starts_are_unavailable_then_it_connects() {
    // 起動前（2 回つながらない）→ 起動後
    let (connector, _kill) = echo_server();
    let scripted = Arc::new(Scripted::new(connector, &[false, false]));
    let transport = PipeTransport::with_backoff(
        Arc::clone(&scripted),
        DownSink::new(),
        backoff_initial(),
        backoff_max(),
    );
    assert_eq!(transport.connect(), Err(TransportError::Unavailable));
    thread::sleep(backoff_initial() * 2);
    assert_eq!(transport.connect(), Err(TransportError::Unavailable));
    thread::sleep(backoff_initial() * 4);
    let handle = transport.connect().unwrap();
    assert_eq!(scripted.opens(), 3);
    assert_eq!(transport.request(handle, b"ok", long()).unwrap(), b"re:ok");
}

#[test]
fn failed_connect_is_not_retried_during_the_backoff() {
    let (connector, _kill) = echo_server();
    let scripted = Arc::new(Scripted::new(connector, &[false]));
    // 控えている間は、OS（コネクタ）を呼ばない。長く控えるので、テストの間は明かない
    let hold = ikari_testtime::scaled(Duration::from_secs(60));
    let transport = PipeTransport::with_backoff(Arc::clone(&scripted), DownSink::new(), hold, hold);
    assert_eq!(transport.connect(), Err(TransportError::Unavailable));
    for _ in 0..5 {
        assert_eq!(transport.connect(), Err(TransportError::Unavailable));
    }
    assert_eq!(scripted.opens(), 1);
}

#[test]
fn backoff_grows_up_to_the_cap_and_resets_on_success() {
    let t0 = Instant::now();
    let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_millis(350));
    assert!(backoff.ready(t0));
    let mut delays = Vec::new();
    for _ in 0..4 {
        backoff.failed(t0);
        let at = backoff.not_before.unwrap();
        delays.push(at - t0);
        assert!(!backoff.ready(t0));
        assert!(backoff.ready(at));
    }
    assert_eq!(
        delays,
        [100, 200, 350, 350].map(Duration::from_millis).to_vec()
    );
    backoff.succeeded();
    assert!(backoff.ready(t0));
    backoff.failed(t0);
    assert_eq!(backoff.not_before.unwrap() - t0, Duration::from_millis(100));
}

#[test]
fn every_reconnect_gets_a_new_generation_and_one_notification() {
    // つなぐ → 切れる → つなぎ直す、を繰り返す。世代は毎回変わり、通知は切れた接続ごとに 1 回
    let f = fixture(None);
    let mut handles = Vec::new();
    for round in 0..3 {
        let handle = f.transport.connect().unwrap();
        assert!(!handles.contains(&handle));
        handles.push(handle);
        let kill = f.kill.recv_timeout(long()).unwrap();
        assert_eq!(f.transport.request(handle, b"x", long()).unwrap(), b"re:x");
        kill.shutdown();
        f.down.recv_timeout(long()).unwrap();
        assert!(f.down.recv_timeout(short()).is_err(), "round {round}");
        f.transport.close(handle);
    }
    assert_eq!(
        *f.recorder.0.lock().unwrap(),
        handles
            .iter()
            .map(|h| (*h, DownKind::Invalidated))
            .collect::<Vec<_>>()
    );
}

#[test]
fn request_in_flight_returns_when_the_server_goes_away() {
    // 受けて返事をしないサーバー。要求を待っている間に切れたら、時間切れまで待たずに戻る
    let (mut listener, connector) = mem::listener();
    let (tx, rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let duplex = listener.accept().unwrap().unwrap();
        let (reader, writer, shutdown) = duplex.split();
        tx.send(Arc::clone(&shutdown)).unwrap();
        (reader, writer, shutdown)
    });
    let transport = Arc::new(PipeTransport::new(
        MemOpen {
            connector: Mutex::new(Some(connector)),
            pid: None,
        },
        DownSink::new(),
    ));
    let handle = transport.connect().unwrap();
    let _server_side = server.join().unwrap();
    let kill = rx.recv_timeout(long()).unwrap();
    let waiting = {
        let transport = Arc::clone(&transport);
        thread::spawn(move || {
            let started = Instant::now();
            let result = transport.request(
                handle,
                b"x",
                ikari_testtime::scaled(Duration::from_secs(30)),
            );
            (result, started.elapsed())
        })
    };
    thread::sleep(short());
    kill.shutdown();
    let (result, elapsed) = waiting.join().unwrap();
    assert_eq!(result, Err(TransportError::Unavailable));
    assert!(elapsed < ikari_testtime::scaled(Duration::from_secs(10)));
}

#[test]
fn a_missed_notification_is_repeated_when_the_dead_connection_is_used() {
    // 切れたのが `EngineLink` に記録される前だと、最初の通知は捨てられる。使われたら、もう一度知らせる
    let f = fixture(None);
    let handle = f.transport.connect().unwrap();
    let kill = f.kill.recv_timeout(long()).unwrap();
    kill.shutdown();
    f.down.recv_timeout(long()).unwrap();
    assert_eq!(
        f.transport.request(handle, b"x", long()),
        Err(TransportError::Unavailable)
    );
    f.down.recv_timeout(long()).unwrap();
    assert_eq!(
        *f.recorder.0.lock().unwrap(),
        vec![(handle, DownKind::Invalidated); 2]
    );
}

/// 測定の点が配線されている（S1）。時計は呼ばれるたびに 100µs 進む。
#[test]
fn meter_records_first_connect_and_requests() {
    use crate::latency::{LineSink, Ticks};
    use std::sync::atomic::AtomicU64;

    struct Step(AtomicU64);
    impl Ticks for Step {
        fn now_us(&self) -> u64 {
            self.0.fetch_add(100, Ordering::SeqCst)
        }
        fn unix_secs(&self) -> u64 {
            0
        }
    }
    struct Lines(Arc<Mutex<Vec<String>>>);
    impl LineSink for Lines {
        fn write_lines(&self, lines: &[String]) {
            self.0.lock().unwrap().extend_from_slice(lines);
        }
    }

    let written = Arc::new(Mutex::new(Vec::new()));
    let meter = Arc::new(Meter::new(
        Box::new(Step(AtomicU64::new(0))),
        Box::new(Lines(Arc::clone(&written))),
        1,
    ));
    let mut f = fixture(None);
    f.transport = f.transport.with_meter(Arc::clone(&meter));
    let handle = f.transport.connect().unwrap();
    assert_eq!(f.transport.request(handle, b"a", long()).unwrap(), b"re:a");
    meter.flush_all();
    let lines = written.lock().unwrap().clone();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines[0].contains("kind=request n=1 window=1 err=0"),
        "{lines:?}"
    );
    assert!(lines[1].contains("kind=first_connect n=1"), "{lines:?}");
    // 入力の本体は行に入らない
    assert!(lines.iter().all(|l| !l.contains("re:a")));
}
