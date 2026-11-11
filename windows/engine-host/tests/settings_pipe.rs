//! 設定画面の口（W4-s2）と shell-core の `bridge`・`settings` の結合。メモリ上の流れで、設定画面の役を `Client` が行う。

use std::io::{Read, Write};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use ikari_engine_host::serve;
use ikari_engine_host::settings_pipe::{SettingsHost, serve_settings};
use ikari_pipe::mem;
use ikari_pipe::{Client, ClientError, Duplex, MAX_FRAME_LEN};
use ikari_proto::SCHEMA_VERSION;
use ikari_proto::v1 as pb;
use ikari_proto::v1::{Envelope, envelope};
use ikari_shell_core::bridge::proto::{
    BridgeErrorCode, Frame, GetSettings, SettingEntry, UpdateSettings, frame::Body,
};
use ikari_shell_core::bridge::protocol::{BRIDGE_VERSION, hello};
use ikari_shell_core::settings::{KEY_PAGING_SIZE, SettingsStore};
use prost::Message;

mod common;
use common::open_engine;

const HOST_VERSION: &str = "0.0.0-test";

fn long() -> Duration {
    ikari_testtime::ms(10_000)
}

struct Fixture {
    settings_file: std::path::PathBuf,
    connector: Option<mem::MemConnector>,
    server: Option<thread::JoinHandle<()>>,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

impl Fixture {
    fn new() -> Self {
        let (engine, data) = open_engine();
        let dir = tempfile::tempdir().unwrap();
        let settings_file = dir.path().join("settings.tsv");
        let (store, _) = SettingsStore::open(&settings_file);
        let host = SettingsHost::new(Arc::new(engine), store, HOST_VERSION);
        let (listener, connector) = mem::listener();
        let server = thread::spawn(move || serve_settings(listener, host).unwrap());
        Fixture {
            settings_file,
            connector: Some(connector),
            server: Some(server),
            _dirs: (data, dir),
        }
    }

    fn connect(&self) -> mem::MemDuplex {
        self.connector.as_ref().unwrap().connect().unwrap()
    }

    fn client(&self) -> Client {
        Client::new(self.connect(), None)
    }

    /// `Hello` を交わした設定画面
    fn ready_client(&self) -> Client {
        let client = self.client();
        let reply = call(&client, hello(1, "test-app"));
        assert!(matches!(reply.body, Some(Body::HelloReply(_))), "{reply:?}");
        client
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // 受け口を閉じて、接続のスレッドを待つ
        self.connector = None;
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

fn try_call(client: &Client, frame: Frame) -> Result<Frame, ClientError> {
    let reply = client.request(&frame.encode_to_vec(), long())?;
    Ok(Frame::decode(reply.as_slice()).expect("reply is a bridge Frame"))
}

fn call(client: &Client, frame: Frame) -> Frame {
    let id = frame.request_id;
    let reply = try_call(client, frame).expect("reply");
    assert_eq!(reply.request_id, id, "request_id is echoed");
    reply
}

fn error_code(frame: &Frame) -> Option<(i32, &str)> {
    match &frame.body {
        Some(Body::Error(e)) => Some((e.code, e.key.as_str())),
        _ => None,
    }
}

fn get_settings(id: u64) -> Frame {
    Frame {
        request_id: id,
        body: Some(Body::GetSettings(GetSettings {})),
    }
}

fn update(id: u64, key: &str, value: &str) -> Frame {
    Frame {
        request_id: id,
        body: Some(Body::UpdateSettings(UpdateSettings {
            set: vec![SettingEntry {
                key: key.into(),
                value: value.into(),
            }],
            reset: Vec::new(),
        })),
    }
}

fn engine_frame(id: u64, body: envelope::Body) -> Frame {
    Frame {
        request_id: id,
        body: Some(Body::Engine(Envelope {
            schema_version: SCHEMA_VERSION,
            request_id: id,
            body: Some(body),
        })),
    }
}

fn value_of<'a>(frame: &'a Frame, key: &str) -> Option<&'a str> {
    let Some(Body::Settings(snapshot)) = &frame.body else {
        panic!("expected a settings snapshot: {frame:?}");
    };
    snapshot
        .effective
        .iter()
        .find(|e| e.key == key)
        .map(|e| e.value.as_str())
}

#[test]
fn hello_is_answered_with_the_host_version() {
    let fixture = Fixture::new();
    let client = fixture.client();
    let reply = call(&client, hello(1, "test-app"));
    let Some(Body::HelloReply(reply)) = reply.body else {
        panic!("expected a hello reply");
    };
    assert_eq!(reply.bridge_version, BRIDGE_VERSION);
    assert_eq!(reply.shell_version, HOST_VERSION);
}

#[test]
fn a_version_mismatch_is_refused_and_the_next_request_must_say_hello_again() {
    let fixture = Fixture::new();
    let client = fixture.client();
    let mut wrong = hello(1, "test-app");
    let Some(Body::Hello(h)) = &mut wrong.body else {
        unreachable!()
    };
    h.bridge_version += 1;
    let reply = call(&client, wrong);
    assert_eq!(
        error_code(&reply),
        Some((BridgeErrorCode::VersionMismatch as i32, ""))
    );
    // Hello がまだなので、設定は読めない
    assert!(matches!(
        try_call(&client, get_settings(2)),
        Err(ClientError::Rejected)
    ));
}

#[test]
fn requests_before_hello_are_rejected_but_the_connection_stays() {
    let fixture = Fixture::new();
    let client = fixture.client();
    assert!(matches!(
        try_call(&client, get_settings(1)),
        Err(ClientError::Rejected)
    ));
    let reply = call(&client, hello(2, "test-app"));
    assert!(matches!(reply.body, Some(Body::HelloReply(_))));
}

#[test]
fn a_broken_body_is_an_error_and_the_connection_stays() {
    let fixture = Fixture::new();
    let client = fixture.ready_client();
    for broken in [&[][..], &[0xff, 0xff, 0xff][..]] {
        assert!(matches!(
            client.request(broken, long()),
            Err(ClientError::Rejected)
        ));
    }
    assert!(matches!(
        call(&client, get_settings(5)).body,
        Some(Body::Settings(_))
    ));
}

#[test]
fn get_settings_returns_the_defaults_and_update_persists_and_returns_the_new_values() {
    let fixture = Fixture::new();
    let client = fixture.ready_client();
    let before = call(&client, get_settings(2));
    let default = value_of(&before, KEY_PAGING_SIZE)
        .expect("known key")
        .to_owned();
    let new = if default == "7" { "8" } else { "7" };

    let after = call(&client, update(3, KEY_PAGING_SIZE, new));
    assert_eq!(value_of(&after, KEY_PAGING_SIZE), Some(new));
    // 次に読んでも、ファイルに書かれている
    assert_eq!(
        value_of(&call(&client, get_settings(4)), KEY_PAGING_SIZE),
        Some(new)
    );
    let text = std::fs::read_to_string(&fixture.settings_file).unwrap();
    assert!(text.contains(KEY_PAGING_SIZE), "{text}");
    // 別の接続（別の画面）からも同じ値が見える
    let other = fixture.ready_client();
    assert_eq!(
        value_of(&call(&other, get_settings(1)), KEY_PAGING_SIZE),
        Some(new)
    );
}

#[test]
fn an_invalid_update_names_only_the_key_and_writes_nothing() {
    let fixture = Fixture::new();
    let client = fixture.ready_client();
    let reply = call(&client, update(2, KEY_PAGING_SIZE, "not-a-number"));
    assert_eq!(
        error_code(&reply),
        Some((BridgeErrorCode::InvalidSetting as i32, KEY_PAGING_SIZE))
    );
    let reply = call(&client, update(3, "no.such.key", "1"));
    assert_eq!(
        error_code(&reply),
        Some((BridgeErrorCode::InvalidSetting as i32, ""))
    );
    assert!(!fixture.settings_file.exists());
}

#[test]
fn admin_requests_reach_the_engine_and_typing_requests_are_refused() {
    let fixture = Fixture::new();
    let client = fixture.ready_client();
    let reply = call(
        &client,
        engine_frame(2, envelope::Body::GetProfile(pb::ProfileGet::default())),
    );
    let Some(Body::Engine(Envelope {
        body: Some(envelope::Body::ProfileReply(_)),
        ..
    })) = reply.body
    else {
        panic!("expected a profile reply: {reply:?}");
    };
    let reply = call(
        &client,
        engine_frame(3, envelope::Body::Commit(pb::Commit::default())),
    );
    assert_eq!(
        error_code(&reply),
        Some((BridgeErrorCode::NotAllowed as i32, ""))
    );
}

#[test]
fn a_request_over_the_pipe_limit_closes_the_connection_without_a_reply() {
    let fixture = Fixture::new();
    let (mut reader, mut writer, _shutdown) = fixture.connect().split();
    // 長さの欄だけで、上限を超えているのが分かる（本体は読まれない）
    writer
        .write_all(&(MAX_FRAME_LEN + 1).to_le_bytes())
        .unwrap();
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty(), "no reply, just EOF");
    // ほかの接続は無事
    let other = fixture.ready_client();
    assert!(matches!(
        call(&other, get_settings(9)).body,
        Some(Body::Settings(_))
    ));
}

#[test]
fn closing_a_settings_connection_does_not_stop_the_engine_pipe() {
    let (engine, _data) = open_engine();
    let engine = Arc::new(engine);
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = SettingsStore::open(dir.path().join("settings.tsv"));
    let host = SettingsHost::new(Arc::clone(&engine), store, HOST_VERSION);

    let (engine_listener, engine_connector) = mem::listener();
    let (settings_listener, settings_connector) = mem::listener();
    let engine_for_serve = Arc::clone(&engine);
    let engine_thread = thread::spawn(move || serve(engine_listener, engine_for_serve).unwrap());
    let settings_thread = thread::spawn(move || serve_settings(settings_listener, host).unwrap());

    let screen = Client::new(settings_connector.connect().unwrap(), None);
    assert!(matches!(
        call(&screen, hello(1, "test-app")).body,
        Some(Body::HelloReply(_))
    ));
    screen.close();
    drop(screen);
    drop(settings_connector);
    settings_thread.join().unwrap();

    let tip = Client::new(engine_connector.connect().unwrap(), None);
    let request = Envelope {
        schema_version: SCHEMA_VERSION,
        request_id: 1,
        body: Some(envelope::Body::BeginSession(pb::BeginSession {
            session_id: 1,
            client_bundle_id: "engine-host-test".into(),
            secure: false,
        })),
    }
    .encode_to_vec();
    let reply = Envelope::decode(tip.request(&request, long()).unwrap().as_slice()).unwrap();
    assert!(
        matches!(reply.body, Some(envelope::Body::Ack(_))),
        "{reply:?}"
    );
    drop(tip);
    drop(engine_connector);
    engine_thread.join().unwrap();
}
