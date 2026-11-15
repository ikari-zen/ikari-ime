//! 起動と終了の手順（W5-a）。OS に依存する部分（名前付きパイプ、Ctrl+C）は、メモリ上の流れで置き換える。

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use ikari_core::user::UserDir;
use ikari_engine_host::run::{
    EXIT_OK, START_FAILURE_LOG, StartError, StopWait, engine_config, record_failure,
    resolve_data_dir, run_host, start, start_with,
};
use ikari_pipe::{Client, Shutdown, mem};
use ikari_proto::v1 as pb;
use ikari_proto::v1::envelope;
use ikari_shell_core::bridge::proto::{Frame, frame::Body};
use ikari_shell_core::bridge::protocol::hello;
use prost::Message;

mod common;
use common::*;

/// 受け口を閉じる手段（メモリ上では、接続する側を全部落とす）
struct DropConnector(Mutex<Option<mem::MemConnector>>);

impl Shutdown for DropConnector {
    fn shutdown(&self) {
        self.0.lock().unwrap().take();
    }
}

fn user_dir(path: &Path) -> UserDir {
    UserDir::new(path)
}

#[test]
fn an_explicit_data_dir_wins() {
    let dir = resolve_data_dir(Some(Path::new("/arg"))).unwrap();
    assert_eq!(dir.root(), Path::new("/arg"));
}

#[test]
fn engine_config_points_at_the_data_dir_and_leaves_lm_off() {
    let config = engine_config(&user_dir(Path::new("/data")));
    assert_eq!(config.data_dir, "/data");
    assert!(!config.enable_lm);
    assert!(config.idle_unload_secs > 0);
}

#[test]
fn start_opens_the_engine_on_the_test_dictionary() {
    let data = data_dir_with_dict();
    let started = start(&user_dir(data.path())).expect("start");
    assert!(started.engine.open_now());
    let reply = started.engine.handle(&envelope_bytes(
        1,
        ikari_proto::v1::envelope::Body::Hello(Default::default()),
    ));
    assert!(!reply.is_empty());
}

#[test]
fn start_creates_a_missing_data_dir_and_stays_without_the_dictionary() {
    let parent = tempfile::tempdir().unwrap();
    let dir = parent.path().join("new").join("ikari-ime");
    let started = start(&user_dir(&dir)).expect("start does not need the dictionary");
    assert!(dir.is_dir(), "the data directory is created");
    assert!(!started.engine.open_now(), "no dictionary");
    assert!(!started.engine.is_open());
}

#[test]
fn a_second_start_on_the_same_data_dir_ends_quietly() {
    let data = data_dir_with_dict();
    let dir = user_dir(data.path());
    let first = start(&dir).expect("first start");
    assert!(first.engine.open_now());
    let error = start(&dir).err().expect("second start is refused");
    assert!(matches!(error, StartError::AlreadyRunning), "{error}");
    assert_eq!(error.exit_code(), EXIT_OK);
    // 静かに終わる: 失敗のログを書かない
    record_failure(&dir, &error);
    assert!(!dir.root().join("logs").join(START_FAILURE_LOG).exists());
    drop(first);
}

#[test]
fn a_failed_open_is_written_to_the_start_failure_log() {
    let data = tempfile::tempdir().unwrap();
    let dir = user_dir(data.path());
    let started = start(&dir).expect("start");
    assert!(!started.engine.open_now());
    let text = std::fs::read_to_string(dir.root().join("logs").join(START_FAILURE_LOG)).unwrap();
    assert!(text.contains("start failed"), "{text}");
    assert_eq!(text.lines().count(), 1);
}

struct Running {
    engine: Option<mem::MemConnector>,
    settings: Option<mem::MemConnector>,
    host: Option<thread::JoinHandle<std::io::Result<()>>>,
    stop: ikari_engine_host::run::StopHandle,
}

fn run_in_memory(dir: &UserDir) -> Running {
    run_in_memory_with(dir, ikari_engine_host::slot::REOPEN_INTERVAL)
}

/// `main` と同じ順で動かす: `start` → 最初の 1 回を開く（開けなくても続ける）→ 受け口 → 受け付け
fn run_in_memory_with(dir: &UserDir, reopen_interval: Duration) -> Running {
    let started = start_with(dir, reopen_interval).expect("start");
    started.engine.open_now();
    let (engine_listener, engine) = mem::listener();
    let (settings_listener, settings) = mem::listener();
    let stoppers: Vec<Arc<dyn Shutdown>> = vec![
        Arc::new(DropConnector(Mutex::new(Some(engine.clone())))),
        Arc::new(DropConnector(Mutex::new(Some(settings.clone())))),
    ];
    let wait = StopWait::new();
    let stop = wait.handle();
    let host = thread::spawn(move || {
        run_host(engine_listener, settings_listener, started, &stoppers, wait)
    });
    Running {
        engine: Some(engine),
        settings: Some(settings),
        host: Some(host),
        stop,
    }
}

impl Running {
    fn engine_client(&self) -> Client {
        Client::new(self.engine.as_ref().unwrap().connect().unwrap(), None)
    }

    fn settings_client(&self) -> Client {
        Client::new(self.settings.as_ref().unwrap().connect().unwrap(), None)
    }

    /// 接続が残っていても止まる。受け口の接続する側は、テストの持つ分を先に落とす
    fn stop(mut self) {
        self.engine = None;
        self.settings = None;
        self.stop.stop();
        self.host.take().unwrap().join().unwrap().expect("run_host");
    }
}

fn settings_hello(client: &Client) -> Frame {
    let reply = client
        .request(&hello(1, "test-app").encode_to_vec(), long())
        .expect("hello reply");
    Frame::decode(reply.as_slice()).unwrap()
}

#[test]
fn both_pipes_are_served_and_stop_releases_the_engine_and_the_lock() {
    let data = data_dir_with_dict();
    let dir = user_dir(data.path());
    let running = run_in_memory(&dir);

    let client = running.engine_client();
    begin_session(&client, SESSION);
    assert!(!convert(&client, SESSION).candidates.is_empty());

    let settings = running.settings_client();
    let reply = settings_hello(&settings);
    assert!(matches!(reply.body, Some(Body::HelloReply(_))), "{reply:?}");

    // 動いている間は、2 つ目は断られる
    assert!(matches!(start(&dir), Err(StartError::AlreadyRunning)));

    running.stop();
    drop((client, settings));

    // エンジンが閉じて、ロックが空いた
    assert!(start(&dir).is_ok());
}

/// 要求を送り、`ENGINE_NOT_READY` が同じ `request_id` で返ることを確かめる（返事を落とさない・接続を切らない）
fn assert_not_ready(client: &Client, request_id: u64, body: envelope::Body) {
    let reply = call(client, request_id, body);
    assert_eq!(error_code(&reply), pb::ErrorCode::EngineNotReady);
}

fn hello_body() -> envelope::Body {
    envelope::Body::Hello(Default::default())
}

fn begin_session_body() -> envelope::Body {
    envelope::Body::BeginSession(pb::BeginSession {
        session_id: SESSION,
        client_bundle_id: "engine-host-test".into(),
        secure: false,
    })
}

#[test]
fn without_the_dictionary_the_host_stays_and_answers_engine_not_ready() {
    let data = tempfile::tempdir().unwrap();
    let dir = user_dir(data.path());
    let running = run_in_memory_with(&dir, Duration::ZERO);

    // 受け口は立っている。どの要求にも ENGINE_NOT_READY を返し、接続は保たれる（殻はかなの候補に落ちる）
    let client = running.engine_client();
    assert_not_ready(&client, 1, hello_body());
    assert_not_ready(&client, 2, begin_session_body());
    assert_not_ready(
        &client,
        3,
        envelope::Body::Convert(pb::Convert {
            session_id: SESSION,
            reading: READING.into(),
            context: None,
            context_rev: 0,
            page_size: 0,
        }),
    );
    // 設定画面の口は、エンジンがなくても答える
    let settings = running.settings_client();
    let reply = settings_hello(&settings);
    assert!(matches!(reply.body, Some(Body::HelloReply(_))), "{reply:?}");

    running.stop();
    drop((client, settings));
}

#[test]
fn once_the_dictionary_appears_the_next_request_opens_the_engine() {
    let data = tempfile::tempdir().unwrap();
    let dir = user_dir(data.path());
    let running = run_in_memory_with(&dir, Duration::ZERO);

    // 辞書を置く前から つながっている接続
    let early = running.engine_client();
    assert_not_ready(&early, 1, hello_body());

    install_dict(data.path());

    // 同じ接続の次の要求で開き直し、通常の返事に戻る（殻は ENGINE_NOT_READY を見て hello とセッションを送り直す）
    assert!(matches!(
        call(&early, 2, hello_body()),
        envelope::Body::HelloReply(_)
    ));
    begin_session(&early, SESSION);
    assert!(!convert(&early, SESSION).candidates.is_empty());

    // あとから来た接続も通常どおり
    let late = running.engine_client();
    begin_session(&late, SESSION);
    assert!(!convert(&late, SESSION).candidates.is_empty());

    // 開いたエンジンがロックを持っている
    assert!(matches!(start(&dir), Err(StartError::AlreadyRunning)));

    running.stop();
    drop((early, late));
    assert!(
        start(&dir).is_ok(),
        "stopping releases the engine and the lock"
    );
}

#[test]
fn reopening_waits_for_the_interval_between_requests() {
    let data = tempfile::tempdir().unwrap();
    let dir = user_dir(data.path());
    // 間隔を長く取る: 最初の 1 回（起動時）のあとは、間隔の内では開き直さない
    let running = run_in_memory_with(&dir, Duration::from_secs(3600));
    let client = running.engine_client();
    install_dict(data.path());
    assert_not_ready(&client, 1, hello_body());
    running.stop();
    drop(client);
}

#[test]
fn the_host_stops_when_one_of_the_accept_threads_ends_by_itself() {
    let data = data_dir_with_dict();
    let dir = user_dir(data.path());
    let started = start(&dir).expect("start");
    let (engine_listener, engine) = mem::listener();
    let (settings_listener, settings) = mem::listener();
    // 設定画面の受け口には止める手段を渡さない。その受け付けが自分で終わるのが、止まる合図になる
    let stoppers: Vec<Arc<dyn Shutdown>> = vec![Arc::new(DropConnector(Mutex::new(Some(engine))))];
    let wait = StopWait::new();
    let host = thread::spawn(move || {
        run_host(engine_listener, settings_listener, started, &stoppers, wait)
    });
    drop(settings);
    host.join().unwrap().expect("run_host");
    assert!(start(&dir).is_ok());
}
