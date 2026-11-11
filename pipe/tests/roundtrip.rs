//! `Client` と `Server` を、メモリ上の流れでつないだ試験（W1-a）。時間を使うものは `ikari-testtime` の倍率を掛ける。

use std::io::{self, Read, Write};
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use ikari_pipe::frame::{Frame, FrameKind, MAX_FRAME_LEN, read_frame, write_frame};
use ikari_pipe::mem::{self, MemDuplex, duplex_pair};
use ikari_pipe::{Client, ClientError, Duplex, Listener, Server, serve_connection};

/// 待ちの上限（返事が来るはずのもの）。遅い CI でも足りる長さ
fn long() -> Duration {
    ikari_testtime::ms(10_000)
}

/// 起こすべきでない待ち（時間切れを試す）
fn short() -> Duration {
    ikari_testtime::ms(60)
}

/// 本体をそのまま返すサーバーをつないだクライアント。サーバーのスレッドも返す。
fn echo_client() -> (Client, thread::JoinHandle<()>) {
    let (client_side, server_side) = duplex_pair();
    let server = thread::spawn(move || serve_connection(server_side, |b: &[u8]| Some(b.to_vec())));
    (Client::new(client_side, Some(4242)), server)
}

/// 生の枠を読み書きする側（壊れた相手や、返事の順番を変える相手の役）
struct Raw {
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
}

fn raw(d: MemDuplex) -> Raw {
    let (reader, writer, _) = d.split();
    Raw { reader, writer }
}

fn frame(kind: FrameKind, seq: u64, body: &[u8]) -> Frame {
    Frame {
        kind,
        seq,
        body: body.to_vec(),
    }
}

#[test]
fn request_gets_the_reply_and_server_pid_is_kept() {
    let (client, server) = echo_client();
    assert_eq!(client.request(b"hello", long()).unwrap(), b"hello");
    assert_eq!(client.request(b"", long()).unwrap(), b"");
    assert_eq!(client.server_pid(), Some(4242));
    drop(client);
    server.join().unwrap();
}

#[test]
fn post_reaches_the_handler_in_order_without_a_reply() {
    let (client_side, server_side) = duplex_pair();
    let (tx, rx) = mpsc::channel();
    let server = thread::spawn(move || {
        serve_connection(server_side, move |b: &[u8]| {
            tx.send(b.to_vec()).unwrap();
            Some(b"ignored".to_vec())
        })
    });
    let client = Client::new(client_side, None);
    client.post(b"one").unwrap();
    client.post(b"two").unwrap();
    assert_eq!(rx.recv_timeout(long()).unwrap(), b"one");
    assert_eq!(rx.recv_timeout(long()).unwrap(), b"two");
    // 返事を捨てているので、次の要求の返事と取り違えない
    assert_eq!(client.request(b"three", long()).unwrap(), b"ignored");
    assert_eq!(rx.recv_timeout(long()).unwrap(), b"three");
    assert_eq!(client.server_pid(), None);
    drop(client);
    server.join().unwrap();
}

#[test]
fn concurrent_requests_are_matched_by_seq_even_when_replies_arrive_out_of_order() {
    const N: usize = 6;
    let (client_side, server_side) = duplex_pair();
    let mut server = raw(server_side);
    let client = Arc::new(Client::new(client_side, None));
    // 返事を、受けた順の逆で返すサーバー
    let server_thread = thread::spawn(move || {
        let mut received = Vec::new();
        for _ in 0..N {
            let f = read_frame(&mut server.reader).unwrap().unwrap();
            assert_eq!(f.kind, FrameKind::Request);
            received.push(f);
        }
        for f in received.into_iter().rev() {
            let mut body = b"re:".to_vec();
            body.extend(&f.body);
            write_frame(&mut server.writer, &frame(FrameKind::Reply, f.seq, &body)).unwrap();
        }
        server
    });
    let start = Arc::new(Barrier::new(N));
    let workers: Vec<_> = (0..N)
        .map(|i| {
            let client = Arc::clone(&client);
            let start = Arc::clone(&start);
            thread::spawn(move || {
                start.wait();
                let body = format!("req{i}");
                let reply = client.request(body.as_bytes(), long()).unwrap();
                assert_eq!(reply, format!("re:{body}").into_bytes());
            })
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    let _server = server_thread.join().unwrap();
}

#[test]
fn many_threads_share_one_client() {
    let (client, server) = echo_client();
    let client = Arc::new(client);
    let workers: Vec<_> = (0..8)
        .map(|t| {
            let client = Arc::clone(&client);
            thread::spawn(move || {
                for i in 0..100 {
                    let body = format!("{t}-{i}");
                    assert_eq!(
                        client.request(body.as_bytes(), long()).unwrap(),
                        body.as_bytes()
                    );
                }
            })
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    drop(client);
    server.join().unwrap();
}

#[test]
fn timeout_leaves_the_connection_usable_and_a_late_reply_is_dropped() {
    let (client_side, server_side) = duplex_pair();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let server = thread::spawn(move || {
        let mut first = true;
        serve_connection(server_side, move |b: &[u8]| {
            if first {
                first = false;
                // 時間切れのあとまで返事を遅らせる
                let _ = release_rx.recv_timeout(Duration::from_secs(10));
            }
            Some(b.to_vec())
        })
    });
    let client = Client::new(client_side, None);
    assert_eq!(client.request(b"slow", short()), Err(ClientError::Timeout));
    release_tx.send(()).unwrap();
    // 遅れた返事（番号 1）は捨てられ、次の要求の返事と取り違えない
    assert_eq!(client.request(b"next", long()).unwrap(), b"next");
    drop(client);
    server.join().unwrap();
}

#[test]
fn rejected_when_the_handler_has_no_reply() {
    let (client_side, server_side) = duplex_pair();
    let server = thread::spawn(move || {
        serve_connection(server_side, |b: &[u8]| (b != b"bad").then(|| b.to_vec()))
    });
    let client = Client::new(client_side, None);
    assert_eq!(client.request(b"bad", long()), Err(ClientError::Rejected));
    assert_eq!(client.request(b"good", long()).unwrap(), b"good");
    drop(client);
    server.join().unwrap();
}

#[test]
fn oversized_reply_becomes_rejected_and_the_connection_survives() {
    let (client_side, server_side) = duplex_pair();
    let server = thread::spawn(move || {
        serve_connection(server_side, |b: &[u8]| {
            Some(vec![
                0;
                if b == b"big" {
                    MAX_FRAME_LEN as usize
                } else {
                    1
                }
            ])
        })
    });
    let client = Client::new(client_side, None);
    assert_eq!(client.request(b"big", long()), Err(ClientError::Rejected));
    assert_eq!(client.request(b"small", long()).unwrap(), vec![0]);
    drop(client);
    server.join().unwrap();
}

#[test]
fn oversized_request_is_refused_locally_and_nothing_is_sent() {
    let (client, server) = echo_client();
    let too_big = vec![0u8; MAX_FRAME_LEN as usize];
    assert_eq!(client.request(&too_big, long()), Err(ClientError::TooLarge));
    assert_eq!(client.post(&too_big), Err(ClientError::TooLarge));
    assert_eq!(client.request(b"still ok", long()).unwrap(), b"still ok");
    drop(client);
    server.join().unwrap();
}

#[test]
fn server_closing_fails_waiting_and_later_requests() {
    let (client_side, server_side) = duplex_pair();
    let mut server = raw(server_side);
    let client = Arc::new(Client::new(client_side, None));
    let waiting = {
        let client = Arc::clone(&client);
        thread::spawn(move || client.request(b"x", long()))
    };
    // 要求を受けてから、返事をせずに閉じる
    let f = read_frame(&mut server.reader).unwrap().unwrap();
    assert_eq!(f.kind, FrameKind::Request);
    drop(server);
    assert_eq!(waiting.join().unwrap(), Err(ClientError::Disconnected));
    assert_eq!(client.request(b"y", long()), Err(ClientError::Disconnected));
    assert_eq!(client.post(b"z"), Err(ClientError::Disconnected));
}

#[test]
fn close_wakes_waiting_requests_and_is_idempotent() {
    let (client_side, server_side) = duplex_pair();
    let _server = raw(server_side);
    let client = Arc::new(Client::new(client_side, None));
    let waiting = {
        let client = Arc::clone(&client);
        thread::spawn(move || client.request(b"x", long()))
    };
    // 要求が登録されるまで、少し待つ
    thread::sleep(short());
    client.close();
    client.close();
    assert_eq!(waiting.join().unwrap(), Err(ClientError::Disconnected));
    assert_eq!(client.request(b"y", long()), Err(ClientError::Disconnected));
}

#[test]
fn broken_frames_from_the_server_disconnect_the_client() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("over the limit", (MAX_FRAME_LEN + 1).to_le_bytes().to_vec()),
        ("too short", 3u32.to_le_bytes().to_vec()),
        ("unknown kind", {
            let mut v = 9u32.to_le_bytes().to_vec();
            v.push(9);
            v.extend([0; 8]);
            v
        }),
        ("a request kind from the server", {
            let mut v = Vec::new();
            write_frame(&mut v, &frame(FrameKind::Request, 1, b"")).unwrap();
            v
        }),
    ];
    for (name, bytes) in cases {
        let (client_side, server_side) = duplex_pair();
        let mut server = raw(server_side);
        let client = Arc::new(Client::new(client_side, None));
        let waiting = {
            let client = Arc::clone(&client);
            thread::spawn(move || client.request(b"x", long()))
        };
        let _ = read_frame(&mut server.reader).unwrap().unwrap();
        server.writer.write_all(&bytes).unwrap();
        assert_eq!(
            waiting.join().unwrap(),
            Err(ClientError::Disconnected),
            "{name}"
        );
    }
}

#[test]
fn truncated_frame_then_eof_disconnects_the_client() {
    let (client_side, server_side) = duplex_pair();
    let mut server = raw(server_side);
    let client = Client::new(client_side, None);
    thread::scope(|s| {
        let waiting = s.spawn(|| client.request(b"x", long()));
        let _ = read_frame(&mut server.reader).unwrap().unwrap();
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &frame(FrameKind::Reply, 1, b"abcdef")).unwrap();
        server.writer.write_all(&bytes[..bytes.len() - 2]).unwrap();
        drop(server.writer);
        assert_eq!(waiting.join().unwrap(), Err(ClientError::Disconnected));
    });
}

/// サーバーに壊れた枠を送ると、サーバーは接続を閉じ、ハンドラを呼ばない。
#[test]
fn server_closes_the_connection_on_bad_frames() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("over the limit", u32::MAX.to_le_bytes().to_vec()),
        ("unknown kind", {
            let mut v = 9u32.to_le_bytes().to_vec();
            v.push(0);
            v.extend([0; 8]);
            v
        }),
        ("a reply kind from the client", {
            let mut v = Vec::new();
            write_frame(&mut v, &frame(FrameKind::Reply, 1, b"")).unwrap();
            v
        }),
    ];
    for (name, bytes) in cases {
        let (client_side, server_side) = duplex_pair();
        let (called_tx, called_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            serve_connection(server_side, move |_: &[u8]| {
                called_tx.send(()).unwrap();
                None
            })
        });
        let mut client = raw(client_side);
        client.writer.write_all(&bytes).unwrap();
        let mut rest = Vec::new();
        client.reader.read_to_end(&mut rest).unwrap();
        assert!(rest.is_empty(), "{name}");
        server.join().unwrap();
        assert!(called_rx.try_recv().is_err(), "{name}: handler was called");
    }
}

/// 接続ごとにハンドラを作り、状態（`FnMut`）が接続の間で混ざらない。受け口が閉じたら戻る。
#[test]
fn server_runs_each_connection_with_its_own_handler_and_stops_when_the_listener_closes() {
    let (listener, connector) = mem::listener();
    let server = thread::spawn(move || {
        Server::new(listener).serve(|| {
            let mut count = 0u32;
            move |_: &[u8]| {
                count += 1;
                Some(count.to_string().into_bytes())
            }
        })
    });
    let a = Client::new(connector.connect().unwrap(), None);
    let b = Client::new(connector.connect().unwrap(), None);
    assert_eq!(a.request(b"", long()).unwrap(), b"1");
    assert_eq!(a.request(b"", long()).unwrap(), b"2");
    assert_eq!(b.request(b"", long()).unwrap(), b"1");
    // 受け口を閉じる。つながったままの接続は、serve が閉じる
    drop(connector);
    server.join().unwrap().unwrap();
    assert_eq!(a.request(b"", long()), Err(ClientError::Disconnected));
    assert_eq!(b.request(b"", long()), Err(ClientError::Disconnected));
}

/// 別の接続の遅い要求が、ほかの接続を止めない。
#[test]
fn a_slow_connection_does_not_block_another() {
    let (listener, connector) = mem::listener();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(Some(release_rx));
    let server = thread::spawn(move || {
        Server::new(listener).serve(move || {
            let gate = release_rx.lock().unwrap().take();
            move |b: &[u8]| {
                if let Some(gate) = &gate {
                    let _ = gate.recv_timeout(Duration::from_secs(10));
                }
                Some(b.to_vec())
            }
        })
    });
    let slow = Arc::new(Client::new(connector.connect().unwrap(), None));
    let fast = Client::new(connector.connect().unwrap(), None);
    let slow_call = {
        let slow = Arc::clone(&slow);
        thread::spawn(move || slow.request(b"slow", long()))
    };
    assert_eq!(fast.request(b"fast", long()).unwrap(), b"fast");
    release_tx.send(()).unwrap();
    assert_eq!(slow_call.join().unwrap().unwrap(), b"slow");
    drop(connector);
    server.join().unwrap().unwrap();
}

struct FlakyListener {
    failures_left: u32,
    always: bool,
}

impl Listener for FlakyListener {
    type Conn = MemDuplex;

    fn accept(&mut self) -> io::Result<Option<MemDuplex>> {
        if self.always || self.failures_left > 0 {
            self.failures_left = self.failures_left.saturating_sub(1);
            return Err(io::Error::other("accept failed"));
        }
        Ok(None)
    }
}

#[test]
fn accept_failures_are_retried_a_few_times() {
    let listener = FlakyListener {
        failures_left: 3,
        always: false,
    };
    let result = Server::new(listener).serve(|| |_: &[u8]| None);
    assert!(result.is_ok());
}

#[test]
fn accept_failing_forever_ends_with_the_error() {
    let listener = FlakyListener {
        failures_left: 0,
        always: true,
    };
    let err = Server::new(listener).serve(|| |_: &[u8]| None).unwrap_err();
    assert_eq!(err.to_string(), "accept failed");
}
