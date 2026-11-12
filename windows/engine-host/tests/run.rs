//! 起動と終了の手順（W5-a）。OS に依存する部分（名前付きパイプ、Ctrl+C）は、メモリ上の流れで置き換える。

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use ikari_core::user::UserDir;
use ikari_engine_host::run::{
    EXIT_OK, EXIT_START_FAILED, START_FAILURE_LOG, StartError, StopWait, engine_config,
    record_failure, resolve_data_dir, run_host, start,
};
use ikari_pipe::{Client, Shutdown, mem};
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
    let reply = started.engine.handle(&envelope_bytes(
        1,
        ikari_proto::v1::envelope::Body::Hello(Default::default()),
    ));
    assert!(!reply.is_empty());
}

#[test]
fn start_creates_a_missing_data_dir_and_then_fails_on_the_missing_dictionary() {
    let parent = tempfile::tempdir().unwrap();
    let dir = parent.path().join("new").join("ikari-ime");
    let error = start(&user_dir(&dir)).err().expect("no dictionary");
    assert!(dir.is_dir(), "the data directory is created");
    assert!(matches!(error, StartError::Failed(_)), "{error}");
    assert_eq!(error.exit_code(), EXIT_START_FAILED);
    assert!(!error.to_string().is_empty());
}

#[test]
fn a_second_start_on_the_same_data_dir_ends_quietly() {
    let data = data_dir_with_dict();
    let dir = user_dir(data.path());
    let first = start(&dir).expect("first start");
    let error = start(&dir).err().expect("second start is refused");
    assert!(matches!(error, StartError::AlreadyRunning), "{error}");
    assert_eq!(error.exit_code(), EXIT_OK);
    // 静かに終わる: 失敗のログを書かない
    record_failure(&dir, &error);
    assert!(!dir.root().join("logs").join(START_FAILURE_LOG).exists());
    drop(first);
}

#[test]
fn a_failed_start_is_written_to_the_start_failure_log() {
    let data = tempfile::tempdir().unwrap();
    let dir = user_dir(data.path());
    let error = start(&dir).err().expect("no dictionary");
    record_failure(&dir, &error);
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
    let started = start(dir).expect("start");
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

#[test]
fn both_pipes_are_served_and_stop_releases_the_engine_and_the_lock() {
    let data = data_dir_with_dict();
    let dir = user_dir(data.path());
    let mut running = run_in_memory(&dir);

    let client = Client::new(running.engine.as_ref().unwrap().connect().unwrap(), None);
    begin_session(&client, SESSION);
    assert!(!convert(&client, SESSION).candidates.is_empty());

    let settings = Client::new(running.settings.as_ref().unwrap().connect().unwrap(), None);
    let reply = settings
        .request(&hello(1, "test-app").encode_to_vec(), long())
        .expect("hello reply");
    let reply = Frame::decode(reply.as_slice()).unwrap();
    assert!(matches!(reply.body, Some(Body::HelloReply(_))), "{reply:?}");

    // 動いている間は、2 つ目は断られる
    assert!(matches!(start(&dir), Err(StartError::AlreadyRunning)));

    // 接続が残っていても止まる。受け口の接続する側は、テストの持つ分を先に落とす
    running.engine = None;
    running.settings = None;
    running.stop.stop();
    running
        .host
        .take()
        .unwrap()
        .join()
        .unwrap()
        .expect("run_host");
    drop((client, settings));

    // エンジンが閉じて、ロックが空いた
    assert!(start(&dir).is_ok());
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
