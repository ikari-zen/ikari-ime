//! 受け付けの繰り返しと `Engine::handle` の結合（W1-c）。メモリ上の流れと、テスト用の辞書（`testdata/sudachi-mini`）。

mod common;

use std::sync::Arc;
use std::thread;

use common::*;
use ikari_engine::Engine;
use ikari_engine_host::serve;
use ikari_pipe::mem;
use ikari_pipe::{Client, ClientError};
use ikari_proto::v1 as pb;
use ikari_proto::v1::{Envelope, envelope};
use prost::Message;

/// `serve` を別のスレッドで動かす。`connector` を全部落とすと受け口が閉じ、`serve` が戻る。
fn spawn_host(engine: Arc<Engine>) -> (mem::MemConnector, thread::JoinHandle<()>) {
    let (listener, connector) = mem::listener();
    let host = thread::spawn(move || serve(listener, engine).expect("serve"));
    (connector, host)
}

#[test]
fn convert_makes_a_round_trip_through_the_pipe() {
    let (engine, _data) = open_engine();
    let (connector, host) = spawn_host(Arc::new(engine));
    let client = Client::new(connector.connect().unwrap(), None);

    begin_session(&client, SESSION);
    let reply = convert(&client, SESSION);
    let surfaces: Vec<&str> = reply
        .candidates
        .iter()
        .map(|c| c.surface.as_str())
        .collect();
    for expected in CANDIDATES {
        assert!(surfaces.contains(&expected), "{expected} in {surfaces:?}");
    }

    drop(client);
    drop(connector);
    host.join().unwrap();
}

#[test]
fn bad_requests_come_back_as_error_envelopes_not_as_a_dropped_connection() {
    let (engine, _data) = open_engine();
    let (connector, host) = spawn_host(Arc::new(engine));
    let client = Client::new(connector.connect().unwrap(), None);

    // Envelope として読めないバイト列
    let reply = client.request(&[0xff; 16], long()).expect("an error reply");
    let reply = Envelope::decode(reply.as_slice()).unwrap();
    assert!(matches!(reply.body, Some(envelope::Body::Error(_))));
    // 接続はそのまま使える
    begin_session(&client, SESSION);

    drop(client);
    drop(connector);
    host.join().unwrap();
}

#[test]
fn several_connections_share_one_engine_and_run_in_parallel() {
    let (engine, _data) = open_engine();
    let (connector, host) = spawn_host(Arc::new(engine));
    let workers: Vec<_> = (1..=4u64)
        .map(|session| {
            let connector = connector.clone();
            thread::spawn(move || {
                let client = Client::new(connector.connect().unwrap(), None);
                begin_session(&client, session);
                for _ in 0..5 {
                    let reply = convert(&client, session);
                    assert!(!reply.candidates.is_empty());
                }
            })
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    drop(connector);
    host.join().unwrap();
}

#[test]
fn two_connections_with_the_same_session_id_do_not_mix() {
    let (engine, _data) = open_engine();
    let (connector, host) = spawn_host(Arc::new(engine));
    let a = Client::new(connector.connect().unwrap(), None);
    let b = Client::new(connector.connect().unwrap(), None);
    assert_connections_with_the_same_session_id_stay_apart(&a, &b);
    drop(a);
    drop(b);
    drop(connector);
    host.join().unwrap();
}

#[test]
fn a_dropped_connection_takes_its_conversions_with_it() {
    let (engine, _data) = open_engine();
    let (connector, host) = spawn_host(Arc::new(engine));
    let a = Client::new(connector.connect().unwrap(), None);
    begin_session(&a, SESSION);
    let ra = convert(&a, SESSION);
    drop(a);
    // つなぎ直した殻は新しい接続になる。前の接続のセッションはもうない（D2。殻は BeginSession を送り直す）
    let again = Client::new(connector.connect().unwrap(), None);
    let reply = call(
        &again,
        5,
        envelope::Body::Convert(pb::Convert {
            session_id: SESSION,
            reading: READING.into(),
            context: None,
            context_rev: 0,
            page_size: 0,
        }),
    );
    assert_eq!(error_code(&reply), pb::ErrorCode::UnknownSession);
    assert_eq!(
        error_code(&page(&again, ra.conversion_id)),
        pb::ErrorCode::UnknownConversion
    );
    begin_session(&again, SESSION);
    assert!(!convert(&again, SESSION).candidates.is_empty());
    drop(again);
    drop(connector);
    host.join().unwrap();
}

#[test]
fn closing_the_listener_closes_open_connections_and_releases_the_engine() {
    let (engine, _data) = open_engine();
    let engine = Arc::new(engine);
    let (connector, host) = spawn_host(Arc::clone(&engine));
    let client = Client::new(connector.connect().unwrap(), None);
    begin_session(&client, SESSION);

    drop(connector);
    host.join().unwrap();
    // 接続は serve が閉じた
    assert_eq!(
        client.request(
            &envelope_bytes(3, envelope::Body::Ack(pb::Ack::default())),
            long()
        ),
        Err(ClientError::Disconnected)
    );
    // 接続のスレッドの持つ `Arc` はなくなった。最後の 1 つを落とせば、`Engine` の `Drop` が走る
    assert_eq!(Arc::strong_count(&engine), 1);
}
