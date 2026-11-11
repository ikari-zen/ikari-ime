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
    assert_eq!(
        f.transport.request(first, b"x", long()),
        Err(TransportError::Unavailable)
    );
    // 切れた接続を閉じても、もう一度は知らせない
    f.transport.close(first);
    assert!(f.down.recv_timeout(short()).is_err());
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
