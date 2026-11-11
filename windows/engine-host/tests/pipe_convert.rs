//! 実際の Windows の名前付きパイプの上で、`Engine::handle` に `Convert` が往復する結合テスト（W1-d）。
//! `serve.rs` のメモリ上の流れと同じ中身を、`NamedPipeListener` と `connect_duplex` で確かめる。

#![cfg(windows)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::{self, JoinHandle};

use common::*;
use ikari_engine_host::serve;
use ikari_pipe::os::windows::{ListenerCloser, NamedPipeListener, connect_duplex};
use ikari_pipe::{Client, Shutdown};
use ikari_proto::v1 as pb;
use ikari_proto::v1::envelope;

static NEXT_NAME: AtomicU32 = AtomicU32::new(0);

/// テストが並行しても、別のプロセスが走っても衝突しないパイプの名前。
fn unique_name() -> String {
    format!(
        r"\\.\pipe\ikari-engine-host-test-{}-{}",
        std::process::id(),
        NEXT_NAME.fetch_add(1, Ordering::Relaxed)
    )
}

/// 動いているホスト。`stop` で受け口を閉じ、`serve` が戻るのを待つ。
struct Host {
    name: String,
    closer: ListenerCloser,
    thread: Option<JoinHandle<()>>,
}

impl Host {
    fn start(engine: ikari_engine::Engine) -> Host {
        let name = unique_name();
        let listener = NamedPipeListener::bind(&name).expect("bind the pipe");
        let closer = listener.closer();
        let engine = Arc::new(engine);
        let thread = thread::spawn(move || serve(listener, engine).expect("serve"));
        Host {
            name,
            closer,
            thread: Some(thread),
        }
    }

    fn connect(&self) -> Client {
        let (duplex, server_pid) = connect_duplex(&self.name).expect("connect to the pipe");
        Client::new(duplex, server_pid)
    }

    fn stop(mut self) {
        self.closer.shutdown();
        if let Some(thread) = self.thread.take() {
            thread.join().expect("serve thread");
        }
    }
}

impl Drop for Host {
    /// テストが途中で落ちても、サーバーのスレッドが `accept` で待ち続けないようにする。
    fn drop(&mut self) {
        self.closer.shutdown();
    }
}

fn assert_has_expected_candidates(reply: &pb::ConvertReply) {
    let surfaces: Vec<&str> = reply
        .candidates
        .iter()
        .map(|c| c.surface.as_str())
        .collect();
    for expected in CANDIDATES {
        assert!(surfaces.contains(&expected), "{expected} in {surfaces:?}");
    }
}

fn convert_request_bytes(request_id: u64) -> Vec<u8> {
    envelope_bytes(
        request_id,
        envelope::Body::Convert(pb::Convert {
            session_id: SESSION,
            reading: READING.into(),
            context: None,
            context_rev: 0,
            page_size: 0,
        }),
    )
}

#[test]
fn convert_makes_a_round_trip_through_a_named_pipe() {
    let (engine, _data) = open_engine();
    let host = Host::start(engine);
    let client = host.connect();

    begin_session(&client, SESSION);
    assert_has_expected_candidates(&convert(&client, SESSION));

    drop(client);
    host.stop();
}

#[test]
fn requests_and_posts_can_be_mixed_on_one_connection() {
    let (engine, _data) = open_engine();
    let host = Host::start(engine);
    let client = host.connect();

    begin_session(&client, SESSION);
    // 返事を待たない送り。返事は捨てられ、あとの要求の返事に混ざらない
    client.post(&convert_request_bytes(10)).expect("post");
    assert_has_expected_candidates(&convert(&client, SESSION));
    client.post(&convert_request_bytes(11)).expect("post");
    client.post(&convert_request_bytes(12)).expect("post");
    assert_has_expected_candidates(&convert(&client, SESSION));

    drop(client);
    host.stop();
}

#[test]
fn the_server_keeps_accepting_after_a_client_disconnects() {
    let (engine, _data) = open_engine();
    let host = Host::start(engine);

    let first = host.connect();
    begin_session(&first, SESSION);
    assert_has_expected_candidates(&convert(&first, SESSION));
    first.close();
    drop(first);

    // 切断のあとも、別の接続を受け付けて、同じエンジンで変換できる
    for session in [2, 3] {
        let next = host.connect();
        begin_session(&next, session);
        assert_has_expected_candidates(&convert(&next, session));
        drop(next);
    }

    host.stop();
}
