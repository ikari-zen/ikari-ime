//! `EngineLink` の単体テスト（移行の前の Swift の `XPCEngineClientTests`、`RelayTests`、`EngineAdminTests`、`XPCSegmentTests`、
//! `XPCEngineDiagTests`、絵文字と郵便番号の口のテストを写した）。替え玉のトランスポートと時計で、落ちる・遅れる・休むを流す。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use ikari_proto::v1::{self, envelope::Body};
use prost::Message;

use super::*;
use crate::host::{Clock, DownKind, EngineTransport, TransportError, TransportHandle};
use crate::menu::{ProfileDomain, ProfileWeight};
use crate::types::{EngineError, EngineErrorCode};

type Responder = Arc<dyn Fn(&v1::Envelope) -> Option<v1::Envelope> + Send + Sync>;

/// 1 本の接続の記録
#[derive(Default, Clone)]
struct Conn {
    handle: u64,
    requests: Vec<v1::Envelope>,
    posts: Vec<v1::Envelope>,
    closed: bool,
}

#[derive(Default)]
struct Shared {
    conns: Vec<Conn>,
    /// 要求の返事を待たせる（並行のテスト）。`Some` なら、要求ごとにここから 1 つ受けるまで返さない
    gate: Option<mpsc::Receiver<()>>,
}

/// 替え玉のトランスポート。受け取った Envelope を記録し、`respond` が返事を決める（`None` は時間切れ）。
struct FakeTransport {
    shared: Arc<Mutex<Shared>>,
    respond: Arc<Mutex<Responder>>,
}

impl EngineTransport for FakeTransport {
    fn connect(&self) -> Result<TransportHandle, TransportError> {
        let mut shared = self.shared.lock().unwrap();
        let handle = 10 + shared.conns.len() as u64;
        shared.conns.push(Conn {
            handle,
            ..Conn::default()
        });
        Ok(TransportHandle(handle))
    }

    fn request(
        &self,
        handle: TransportHandle,
        bytes: &[u8],
        _timeout: Duration,
    ) -> Result<Vec<u8>, TransportError> {
        let env = v1::Envelope::decode(bytes).unwrap();
        let gate = {
            let mut shared = self.shared.lock().unwrap();
            conn_mut(&mut shared, handle).requests.push(env.clone());
            shared.gate.take()
        };
        if let Some(gate) = gate {
            let _ = gate.recv();
            self.shared.lock().unwrap().gate = Some(gate);
        }
        let respond = self.respond.lock().unwrap().clone();
        let mut reply = respond(&env).ok_or(TransportError::Timeout)?;
        if reply.request_id == 0 {
            reply.request_id = env.request_id;
        }
        Ok(reply.encode_to_vec())
    }

    fn post(&self, handle: TransportHandle, bytes: &[u8]) {
        let env = v1::Envelope::decode(bytes).unwrap();
        let mut shared = self.shared.lock().unwrap();
        conn_mut(&mut shared, handle).posts.push(env);
    }

    fn close(&self, handle: TransportHandle) {
        let mut shared = self.shared.lock().unwrap();
        conn_mut(&mut shared, handle).closed = true;
    }

    fn pid(&self, handle: TransportHandle) -> Option<i32> {
        Some(100 + (handle.0 - 10) as i32)
    }
}

fn conn_mut(shared: &mut Shared, handle: TransportHandle) -> &mut Conn {
    shared
        .conns
        .iter_mut()
        .find(|conn| conn.handle == handle.0)
        .unwrap()
}

/// 替え玉の時計
#[derive(Default)]
struct FakeClock {
    now: Mutex<Duration>,
}

impl FakeClock {
    fn advance(&self, by: Duration) {
        *self.now.lock().unwrap() += by;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap()
    }
}

struct Harness {
    link: Arc<EngineLink>,
    shared: Arc<Mutex<Shared>>,
    respond: Arc<Mutex<Responder>>,
    clock: Arc<FakeClock>,
}

impl Harness {
    fn new() -> Self {
        Self::with_version("1.0")
    }

    fn with_version(version: &'static str) -> Self {
        Self::with_version_and_owner(version, true)
    }

    fn with_version_and_owner(version: &'static str, engine_owner: bool) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let responder: Responder = Arc::new(move |env| default_response(env, version));
        let respond = Arc::new(Mutex::new(responder));
        let clock = Arc::new(FakeClock::default());
        let transport = FakeTransport {
            shared: shared.clone(),
            respond: respond.clone(),
        };
        let link = Arc::new(
            EngineLink::new(Box::new(transport), clock.clone()).with_engine_owner(engine_owner),
        );
        Self {
            link,
            shared,
            respond,
            clock,
        }
    }

    fn respond(&self, f: impl Fn(&v1::Envelope) -> Option<v1::Envelope> + Send + Sync + 'static) {
        *self.respond.lock().unwrap() = Arc::new(f);
    }

    fn conns(&self) -> Vec<Conn> {
        self.shared.lock().unwrap().conns.clone()
    }

    fn latest(&self) -> Conn {
        self.conns().last().cloned().unwrap()
    }

    fn down(&self, kind: DownKind) {
        self.link
            .notify_down(TransportHandle(self.latest().handle), kind);
    }

    fn advance(&self, by: Duration) {
        self.clock.advance(by);
    }
}

fn envelope(body: Body) -> v1::Envelope {
    v1::Envelope {
        schema_version: 1,
        request_id: 0,
        body: Some(body),
    }
}

fn error_reply(code: v1::ErrorCode) -> v1::Envelope {
    envelope(Body::Error(v1::ErrorReply {
        code: code as i32,
        message: String::new(),
    }))
}

fn candidates(surfaces: &[&str]) -> Vec<v1::Candidate> {
    surfaces
        .iter()
        .map(|surface| v1::Candidate {
            surface: (*surface).to_owned(),
            ..Default::default()
        })
        .collect()
}

fn default_response(request: &v1::Envelope, engine_version: &str) -> Option<v1::Envelope> {
    let body = match &request.body {
        Some(Body::Hello(_)) => Body::HelloReply(v1::HelloReply {
            engine_version: engine_version.to_owned(),
            dict_label: "test".into(),
            lm: v1::LmState::LmReady as i32,
            ..Default::default()
        }),
        Some(Body::Convert(_)) | Some(Body::Page(_)) => {
            let reply = v1::ConvertReply {
                conversion_id: 7,
                candidates: candidates(&["品詞", "瀕死"]),
                has_next: true,
                ..Default::default()
            };
            if matches!(request.body, Some(Body::Convert(_))) {
                Body::ConvertReply(reply)
            } else {
                Body::PageReply(reply)
            }
        }
        _ => Body::Ack(v1::Ack {}),
    };
    Some(envelope(body))
}

fn sample_request() -> v1::Convert {
    v1::Convert {
        session_id: 1,
        reading: "ひんし".into(),
        context: Some(v1::SurroundingText {
            left: "ユーザー辞書に".into(),
            right: String::new(),
            available: true,
        }),
        context_rev: 1,
        page_size: 9,
    }
}

fn session(id: u64, bundle: &str, secure: bool) -> v1::BeginSession {
    v1::BeginSession {
        session_id: id,
        client_bundle_id: bundle.into(),
        secure,
    }
}

const SEC: Duration = Duration::from_secs(1);
const MS: Duration = Duration::from_millis(1);

/// 送ったものの種類（再同期の順を比べる）
fn kinds(envs: &[v1::Envelope]) -> Vec<String> {
    envs.iter()
        .map(|env| match &env.body {
            Some(Body::Hello(_)) => "hello".to_owned(),
            Some(Body::BeginSession(s)) => format!("begin{}", s.session_id),
            Some(Body::Prefetch(_)) => "prefetch".to_owned(),
            Some(Body::CommitText(_)) => "commit_text".to_owned(),
            Some(Body::Convert(_)) => "convert".to_owned(),
            Some(Body::Shutdown(_)) => "shutdown".to_owned(),
            Some(Body::Commit(_)) => "commit".to_owned(),
            _ => "other".to_owned(),
        })
        .collect()
}

// ---------------------------------------------------------------- XPCEngineClientTests

#[test]
fn hello_returns_the_engine_information_when_the_version_matches() {
    let h = Harness::new();
    let hello = h.link.hello("1.0", SEC).unwrap();
    assert_eq!(hello.engine_version, "1.0");
    assert_eq!(hello.dict_label, "test");
    assert_eq!(hello.lm, v1::LmState::LmReady as i32);
    assert_eq!(h.conns().len(), 1);
    assert_eq!(h.latest().requests.len(), 1);
    assert_eq!(h.latest().requests[0].schema_version, 1);
}

#[test]
fn version_mismatch_sends_shutdown_retries_once_and_gives_up() {
    let h = Harness::with_version("0.9");
    // 替え玉は終了の通知を出さないので、終了の確認を SHUTDOWN_WAIT だけ待つ
    assert_eq!(h.link.hello("1.0", SEC), Err(EngineError::Unavailable));
    assert_eq!(
        kinds(&h.latest().requests),
        vec!["hello", "shutdown", "hello"]
    );
    assert_eq!(h.link.last_engine_version().as_deref(), Some("0.9"));
}

#[test]
fn non_owner_shell_coexists_with_a_different_engine_version_without_shutdown() {
    let h = Harness::with_version_and_owner("0.9", false);
    let started = Instant::now();
    let hello = h.link.hello("1.0", SEC).unwrap();
    assert!(started.elapsed() < SHUTDOWN_WAIT);
    assert_eq!(hello.engine_version, "0.9");
    assert_eq!(h.link.last_engine_version().as_deref(), Some("0.9"));
    assert!(!h.link.is_suspended());
    // 変換も続く
    let page = h.link.convert(sample_request(), 100 * MS).unwrap();
    assert_eq!(page.conversion_id, 7);
    let sent = kinds(&h.latest().requests);
    assert!(!sent.contains(&"shutdown".to_owned()), "{sent:?}");
    assert_eq!(sent.iter().filter(|k| *k == "hello").count(), 1, "{sent:?}");
}

#[test]
fn version_mismatch_waits_for_the_exit_notice_and_then_succeeds() {
    let h = Harness::with_version("0.9");
    let link = h.link.clone();
    let shared = h.shared.clone();
    let responder: Arc<Mutex<Responder>> = h.respond.clone();
    let swapped = Arc::new(AtomicBool::new(false));
    let flag = swapped.clone();
    // Shutdown を受けたら、別のスレッドから終了を知らせ、以後は新しい版で答える
    h.respond(move |env| {
        if let Some(Body::Shutdown(_)) = env.body {
            let link = link.clone();
            let handle = shared.lock().unwrap().conns.last().unwrap().handle;
            flag.store(true, Ordering::SeqCst);
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(20));
                link.notify_down(TransportHandle(handle), DownKind::Interrupted);
            });
        }
        let version = if flag.load(Ordering::SeqCst) {
            "1.0"
        } else {
            "0.9"
        };
        default_response(env, version)
    });
    let _ = responder;
    let started = Instant::now();
    assert_eq!(h.link.hello("1.0", SEC).unwrap().engine_version, "1.0");
    assert!(started.elapsed() < SHUTDOWN_WAIT);
    assert!(swapped.load(Ordering::SeqCst));
    assert!(!h.link.is_suspended());
}

#[test]
fn convert_maps_and_validates_candidates() {
    let h = Harness::new();
    let page = h.link.convert(sample_request(), 100 * MS).unwrap();
    assert_eq!(page.conversion_id, 7);
    let surfaces: Vec<_> = page.candidates.iter().map(|c| c.surface.as_str()).collect();
    assert_eq!(surfaces, ["品詞", "瀕死"]);
    assert!(page.has_next);
    let Some(Body::Convert(sent)) = &h.latest().requests[0].body else {
        panic!("convert was not sent");
    };
    assert_eq!(sent.reading, "ひんし");
    assert_eq!(sent.context.as_ref().unwrap().left, "ユーザー辞書に");
    assert_eq!(sent.page_size, 9);
}

#[test]
fn left_context_is_clipped_to_the_last_two_thousand_characters() {
    let h = Harness::new();
    let mut request = sample_request();
    request.context = Some(v1::SurroundingText {
        left: "あ".repeat(2500) + "終",
        right: String::new(),
        available: true,
    });
    h.link.convert(request, 100 * MS).unwrap();
    let Some(Body::Convert(sent)) = &h.latest().requests[0].body else {
        panic!();
    };
    let left = &sent.context.as_ref().unwrap().left;
    assert_eq!(left.chars().count(), 2000);
    assert!(left.ends_with('終'));
}

#[test]
fn invalid_requests_are_rejected_before_sending() {
    let h = Harness::new();
    let bad = Err(EngineError::Engine(EngineErrorCode::BadRequest));
    let mut request = sample_request();
    request.reading = "あ".repeat(257);
    assert_eq!(h.link.convert(request.clone(), SEC), bad);
    request.reading = String::new();
    assert_eq!(h.link.convert(request.clone(), SEC), bad);
    request.reading = "あ".into();
    request.page_size = 10;
    assert_eq!(h.link.convert(request.clone(), SEC), bad);
    request.page_size = 0;
    assert_eq!(h.link.convert(request, SEC), bad);
    assert_eq!(h.link.page(1, u32::MAX, SEC), bad);
    assert!(h.conns().is_empty());
}

#[test]
fn too_many_candidates_is_an_invalid_reply() {
    let h = Harness::new();
    h.respond(|_| {
        let surfaces: Vec<String> = (0..200).map(|i| format!("候補{i}")).collect();
        let refs: Vec<&str> = surfaces.iter().map(String::as_str).collect();
        Some(envelope(Body::ConvertReply(v1::ConvertReply {
            candidates: candidates(&refs),
            ..Default::default()
        })))
    });
    assert_eq!(
        h.link.convert(sample_request(), SEC),
        Err(EngineError::InvalidReply)
    );
}

#[test]
fn broken_candidates_and_wrong_reply_kinds_are_invalid() {
    let h = Harness::new();
    h.respond(|_| {
        Some(envelope(Body::ConvertReply(v1::ConvertReply {
            candidates: candidates(&[""]),
            ..Default::default()
        })))
    });
    assert_eq!(
        h.link.convert(sample_request(), SEC),
        Err(EngineError::InvalidReply)
    );
    // Convert に page_reply が返った
    h.respond(|_| Some(envelope(Body::PageReply(v1::ConvertReply::default()))));
    assert_eq!(
        h.link.convert(sample_request(), SEC),
        Err(EngineError::InvalidReply)
    );
    assert_eq!(h.link.page(7, 1, SEC).map(|r| r.conversion_id), Ok(0));
}

#[test]
fn a_mismatched_request_id_is_an_invalid_reply() {
    let h = Harness::new();
    h.respond(|_| {
        let mut env = envelope(Body::Ack(v1::Ack {}));
        env.request_id = 999_999;
        Some(env)
    });
    assert_eq!(
        h.link.convert(sample_request(), SEC),
        Err(EngineError::InvalidReply)
    );
}

#[test]
fn error_replies_become_engine_errors() {
    let h = Harness::new();
    h.respond(|_| Some(error_reply(v1::ErrorCode::EngineNotReady)));
    assert_eq!(
        h.link.convert(sample_request(), SEC),
        Err(EngineError::Engine(EngineErrorCode::EngineNotReady))
    );
}

#[test]
fn timeouts_pass_through() {
    let h = Harness::new();
    h.respond(|_| None);
    assert_eq!(
        h.link.convert(sample_request(), MS),
        Err(EngineError::Timeout)
    );
}

#[test]
fn notifications_are_posted_without_waiting() {
    let h = Harness::new();
    h.respond(|_| None); // request() は使われない
    h.link.prefetch(v1::Prefetch {
        session_id: 1,
        reading: "ひんし".into(),
        context: None,
        context_rev: 1,
    });
    h.link.commit(7, 0);
    h.link.commit_text(v1::CommitText {
        session_id: 1,
        reading: "か".into(),
        surface: "か".into(),
    });
    h.link.cancel(7);
    assert_eq!(h.latest().posts.len(), 4);
    assert!(h.latest().requests.is_empty());
}

#[test]
fn invalid_notifications_are_dropped() {
    let h = Harness::new();
    h.link.prefetch(v1::Prefetch::default()); // 読みが空
    h.link.commit(7, u32::MAX);
    h.link.commit_segments(v1::CommitSegments::default()); // 選択が空
    assert!(h.conns().is_empty());
}

#[test]
fn a_crash_resyncs_hello_and_sessions_before_the_next_request() {
    let h = Harness::new();
    h.link.hello("1.0", SEC).unwrap();
    h.link
        .begin_session(session(1, "com.apple.TextEdit", false));
    h.link.begin_session(session(2, "com.apple.Notes", true));
    h.link.end_session(2);
    let before = h.latest().posts.len();

    h.down(DownKind::Interrupted); // エンジンが落ちた
    h.link.convert(sample_request(), SEC).unwrap();

    assert_eq!(kinds(&h.latest().posts[before..]), vec!["hello", "begin1"]); // 終わったセッション 2 は送り直さない
    assert_eq!(h.conns().len(), 1); // 中断では接続を作り直さない
}

#[test]
fn an_engine_that_opened_late_gets_hello_and_sessions_before_the_next_request() {
    let h = Harness::new();
    let not_ready = Arc::new(AtomicBool::new(true));
    let flag = not_ready.clone();
    h.respond(move |env| {
        if flag.load(Ordering::SeqCst) {
            Some(error_reply(v1::ErrorCode::EngineNotReady))
        } else {
            default_response(env, "1.0")
        }
    });
    let _ = h.link.hello("1.0", SEC);
    h.link
        .begin_session(session(1, "com.apple.TextEdit", false));
    assert_eq!(
        h.link.convert(sample_request(), SEC),
        Err(EngineError::Engine(EngineErrorCode::EngineNotReady))
    );
    // ロックが空いて、エンジンが開けた。開いたエンジンは、先の BeginSession を知らない
    not_ready.store(false, Ordering::SeqCst);
    let before = h.latest().posts.len();
    h.link.convert(sample_request(), SEC).unwrap();
    assert_eq!(kinds(&h.latest().posts[before..]), vec!["hello", "begin1"]);
}

#[test]
fn unknown_session_resends_hello_and_sessions_then_retries_once() {
    // 後から開いたエンジンは、ENGINE_NOT_READY ではなく UNKNOWN_SESSION を返す（実機で起きた）
    let h = Harness::new();
    let forgotten = Arc::new(AtomicBool::new(false));
    let flag = forgotten.clone();
    h.respond(move |env| {
        if flag.load(Ordering::SeqCst) && matches!(env.body, Some(Body::Convert(_))) {
            flag.store(false, Ordering::SeqCst);
            return Some(error_reply(v1::ErrorCode::UnknownSession));
        }
        default_response(env, "1.0")
    });
    let _ = h.link.hello("1.0", SEC);
    h.link
        .begin_session(session(1, "com.apple.TextEdit", false));
    forgotten.store(true, Ordering::SeqCst); // エンジンが開き直して、セッションを忘れた
    let before = h.latest().posts.len();
    h.link.convert(sample_request(), SEC).unwrap();
    assert_eq!(kinds(&h.latest().posts[before..]), vec!["hello", "begin1"]);
}

#[test]
fn a_prefetch_after_a_crash_resyncs_before_it_is_posted() {
    let h = Harness::new();
    h.link.hello("1.0", SEC).unwrap();
    h.link
        .begin_session(session(1, "com.apple.TextEdit", false));
    let before = h.latest().posts.len();

    h.down(DownKind::Interrupted);
    h.link.prefetch(v1::Prefetch {
        session_id: 1,
        reading: "ひんし".into(),
        ..Default::default()
    });
    h.link.commit_text(v1::CommitText {
        session_id: 1,
        reading: "か".into(),
        surface: "か".into(),
    });
    assert_eq!(
        kinds(&h.latest().posts[before..]),
        vec!["hello", "begin1", "prefetch", "commit_text"]
    );
}

#[test]
fn an_invalidated_connection_resyncs_on_the_new_connection() {
    let h = Harness::new();
    h.link.hello("1.0", SEC).unwrap();
    h.link
        .begin_session(session(1, "com.apple.TextEdit", false));
    h.down(DownKind::Invalidated);

    h.link.prefetch(v1::Prefetch {
        session_id: 1,
        reading: "ひんし".into(),
        ..Default::default()
    });
    h.link.convert(sample_request(), SEC).unwrap();

    assert_eq!(h.conns().len(), 2);
    assert_eq!(
        kinds(&h.latest().posts),
        vec!["hello", "begin1", "prefetch"]
    );
    assert_eq!(h.latest().requests.len(), 1); // convert は 1 回で通る
    assert!(h.conns()[0].closed); // 無効になった接続は閉じる
}

#[test]
fn a_session_that_was_never_begun_cannot_be_resynced_which_matches_the_logs() {
    let h = Harness::new();
    h.respond(|env| match env.body {
        Some(Body::Convert(_)) => Some(error_reply(v1::ErrorCode::UnknownSession)),
        _ => default_response(env, "1.0"),
    });
    assert_eq!(
        h.link.convert(sample_request(), SEC),
        Err(EngineError::Engine(EngineErrorCode::UnknownSession))
    );
    assert_eq!(kinds(&h.latest().requests), vec!["convert", "convert"]);
    assert!(h.latest().posts.is_empty());
}

#[test]
fn invalidation_rebuilds_the_connection() {
    let h = Harness::new();
    h.link.convert(sample_request(), SEC).unwrap();
    h.down(DownKind::Invalidated);
    h.link.convert(sample_request(), SEC).unwrap();
    assert_eq!(h.conns().len(), 2);
}

#[test]
fn stale_connection_notifications_are_ignored() {
    let h = Harness::new();
    h.link.convert(sample_request(), SEC).unwrap();
    let first = h.latest().handle;
    h.down(DownKind::Invalidated);
    h.link.convert(sample_request(), SEC).unwrap();
    // 古い接続の遅れた通知は、新しい接続を壊さない
    h.link
        .notify_down(TransportHandle(first), DownKind::Invalidated);
    h.link.convert(sample_request(), SEC).unwrap();
    assert_eq!(h.conns().len(), 2);
    assert_eq!(h.latest().requests.len(), 2);
}

#[test]
fn three_crashes_in_a_suspension_window_stop_calling_the_engine_for_five_minutes() {
    let h = Harness::new();
    h.link.convert(sample_request(), SEC).unwrap();
    for _ in 0..3 {
        h.down(DownKind::Interrupted);
        h.advance(Duration::from_secs(10));
    }
    assert!(h.link.is_suspended());
    assert_eq!(
        h.link.convert(sample_request(), SEC),
        Err(EngineError::Unavailable)
    );
    h.advance(SUSPENSION + SEC);
    assert!(!h.link.is_suspended());
    h.link.convert(sample_request(), SEC).unwrap();
}

#[test]
fn crashes_spread_over_more_than_a_minute_do_not_suspend() {
    let h = Harness::new();
    h.link.convert(sample_request(), SEC).unwrap();
    for _ in 0..5 {
        h.down(DownKind::Interrupted);
        h.advance(Duration::from_secs(40));
    }
    assert!(!h.link.is_suspended());
    h.link.convert(sample_request(), SEC).unwrap();
}

#[test]
fn shutdown_is_not_counted_as_a_crash() {
    let h = Harness::new();
    for _ in 0..4 {
        h.link.shutdown(v1::ShutdownReason::SettingsChanged);
        h.down(DownKind::Interrupted);
    }
    assert!(!h.link.is_suspended());
}

#[test]
fn shutdown_carries_the_reason() {
    let h = Harness::new();
    h.link.shutdown(v1::ShutdownReason::SettingsChanged);
    h.link.shutdown(v1::ShutdownReason::VersionMismatch);
    let reasons: Vec<i32> = h
        .conns()
        .iter()
        .flat_map(|c| c.requests.clone())
        .filter_map(|env| match env.body {
            Some(Body::Shutdown(s)) => Some(s.reason),
            _ => None,
        })
        .collect();
    assert_eq!(
        reasons,
        vec![
            v1::ShutdownReason::SettingsChanged as i32,
            v1::ShutdownReason::VersionMismatch as i32
        ]
    );
}

#[test]
fn a_failed_shutdown_does_not_hide_the_next_crash() {
    let h = Harness::new();
    h.respond(|_| None);
    h.link.shutdown(v1::ShutdownReason::SettingsChanged); // 時間切れ。終了を待つ印は戻す
    for _ in 0..3 {
        h.down(DownKind::Interrupted);
    }
    assert!(h.link.is_suspended());
}

#[test]
fn pid_is_reported_from_the_transport() {
    let h = Harness::new();
    assert_eq!(h.link.pid(), None);
    h.link.convert(sample_request(), SEC).unwrap();
    assert_eq!(h.link.pid(), Some(100));
    h.down(DownKind::Invalidated);
    assert_eq!(h.link.pid(), None);
}

// ---------------------------------------------------------------- 並行（Swift の殻ではセマフォと XPC のキューが受け持っていた分）

#[test]
fn a_down_notice_from_another_thread_is_not_blocked_by_a_waiting_request() {
    let h = Harness::new();
    h.link.convert(sample_request(), SEC).unwrap();
    let (release, gate) = mpsc::channel();
    h.shared.lock().unwrap().gate = Some(gate);
    let link = h.link.clone();
    let waiting = thread::spawn(move || link.convert(sample_request(), SEC));
    // 要求が返事を待っている間に、別のスレッドから落ちた通知が届く
    while h.latest().requests.len() < 2 {
        thread::yield_now();
    }
    let handle = TransportHandle(h.latest().handle);
    let notifier = {
        let link = h.link.clone();
        thread::spawn(move || link.notify_down(handle, DownKind::Interrupted))
    };
    notifier.join().unwrap(); // 待っている要求に止められない
    release.send(()).unwrap();
    waiting.join().unwrap().unwrap();
    h.shared.lock().unwrap().gate = None;
    // 次の要求の前に送り直す
    h.link.hello("1.0", SEC).unwrap();
    let posts = h.latest().posts;
    assert_eq!(kinds(&posts), vec!["hello"]);
}

#[test]
fn posts_while_another_thread_waits_are_sent_at_once() {
    let h = Harness::new();
    h.link.convert(sample_request(), SEC).unwrap();
    let (release, gate) = mpsc::channel();
    h.shared.lock().unwrap().gate = Some(gate);
    let link = h.link.clone();
    let waiting = thread::spawn(move || link.relay(&profile_request(), SEC));
    while h.latest().requests.len() < 2 {
        thread::yield_now();
    }
    // 中継が返事を待っている間に、主スレッドは片道を送る（待たない）
    let started = Instant::now();
    h.link.commit(7, 0);
    h.link.commit_text(v1::CommitText {
        session_id: 1,
        reading: "か".into(),
        surface: "か".into(),
    });
    assert!(started.elapsed() < Duration::from_millis(100));
    assert_eq!(kinds(&h.latest().posts), vec!["commit", "commit_text"]);
    release.send(()).unwrap();
    waiting.join().unwrap().unwrap();
}

#[test]
fn a_convert_is_not_blocked_by_a_long_relay() {
    let h = Harness::new();
    h.link.convert(sample_request(), SEC).unwrap();
    let (release, gate) = mpsc::channel();
    h.shared.lock().unwrap().gate = Some(gate);
    let link = h.link.clone();
    let waiting = thread::spawn(move || link.relay(&profile_request(), Duration::from_secs(30)));
    while h.latest().requests.len() < 2 {
        thread::yield_now();
    }
    // 中継（インポートで最大 30 秒）が返事を待っている間も、主スレッドの変換は並行に通る
    let started = Instant::now();
    assert_eq!(
        h.link
            .convert(sample_request(), 50 * MS)
            .map(|r| r.conversion_id),
        Ok(7)
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    release.send(()).unwrap();
    waiting.join().unwrap().unwrap();
}

#[test]
fn concurrent_callers_get_distinct_request_ids() {
    let h = Harness::new();
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let link = h.link.clone();
            thread::spawn(move || {
                for _ in 0..25 {
                    link.convert(sample_request(), SEC).unwrap();
                    link.commit(7, 0);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let conn = h.latest();
    let mut ids: Vec<u64> = conn
        .requests
        .iter()
        .chain(conn.posts.iter())
        .map(|e| e.request_id)
        .collect();
    assert_eq!(ids.len(), 200);
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 200);
}

// ---------------------------------------------------------------- RelayTests

fn profile_request() -> Vec<u8> {
    v1::Envelope {
        schema_version: 1,
        request_id: 999,
        body: Some(Body::GetProfile(v1::ProfileGet {})),
    }
    .encode_to_vec()
}

fn profile_reply(domains: &[(&str, &str, v1::ProfileWeightKind)]) -> v1::Envelope {
    envelope(Body::ProfileReply(v1::ProfileReply {
        domains: domains
            .iter()
            .map(|(id, name, weight)| v1::ProfileDomainState {
                id: (*id).into(),
                name: (*name).into(),
                weight: *weight as i32,
            })
            .collect(),
    }))
}

#[test]
fn relays_the_bytes_and_returns_the_reply_as_is() {
    let h = Harness::new();
    h.respond(|_| {
        Some(profile_reply(&[(
            "it",
            "IT",
            v1::ProfileWeightKind::ProfileWeightOften,
        )]))
    });
    let reply = h.link.relay(&profile_request(), SEC).unwrap();
    let Some(Body::ProfileReply(body)) = v1::Envelope::decode(reply.as_slice()).unwrap().body
    else {
        panic!("expected profile reply");
    };
    assert_eq!(body.domains[0].id, "it");
}

#[test]
fn relay_reassigns_the_request_id_and_schema_version() {
    let h = Harness::new();
    h.respond(|_| Some(profile_reply(&[])));
    h.link.relay(&profile_request(), SEC).unwrap();
    h.link.relay(&profile_request(), SEC).unwrap();
    let sent = h.latest().requests;
    assert_eq!(sent.len(), 2);
    assert!(
        sent.iter()
            .all(|e| e.schema_version == 1 && e.request_id != 999 && e.request_id != 0)
    );
    assert_ne!(sent[0].request_id, sent[1].request_id);
    assert!(matches!(sent[0].body, Some(Body::GetProfile(_))));
}

#[test]
fn relay_returns_an_error_reply_instead_of_failing() {
    let h = Harness::new();
    h.respond(|_| Some(error_reply(v1::ErrorCode::BadRequest)));
    let reply = h.link.relay(&profile_request(), SEC).unwrap();
    let Some(Body::Error(error)) = v1::Envelope::decode(reply.as_slice()).unwrap().body else {
        panic!("expected error reply");
    };
    assert_eq!(error.code, v1::ErrorCode::BadRequest as i32);
}

#[test]
fn relay_timeout_passes_through_and_the_next_relay_still_works() {
    let h = Harness::new();
    let counter = Arc::new(AtomicUsize::new(0));
    let c = counter.clone();
    h.respond(move |_| {
        if c.fetch_add(1, Ordering::SeqCst) == 0 {
            None
        } else {
            Some(profile_reply(&[]))
        }
    });
    assert_eq!(
        h.link.relay(&profile_request(), MS),
        Err(EngineError::Timeout)
    );
    h.link.relay(&profile_request(), SEC).unwrap();
}

#[test]
fn relay_resyncs_after_a_crash_and_reconnects_after_invalidation() {
    let h = Harness::new();
    h.respond(|env| match env.body {
        Some(Body::Hello(_)) => default_response(env, "1.0"),
        _ => Some(profile_reply(&[])),
    });
    h.link.hello("1.0", SEC).unwrap();
    h.down(DownKind::Interrupted);
    h.link.relay(&profile_request(), SEC).unwrap();
    // 落ちたあとの最初の要求の前に Hello を送り直している
    assert!(kinds(&h.latest().posts).contains(&"hello".to_owned()));
    h.down(DownKind::Invalidated);
    h.link.relay(&profile_request(), SEC).unwrap();
    assert_eq!(h.conns().len(), 2);
}

#[test]
fn malformed_relay_requests_are_rejected_before_sending() {
    let h = Harness::new();
    let bad = Err(EngineError::Engine(EngineErrorCode::BadRequest));
    assert_eq!(h.link.relay(&[], SEC), bad);
    assert_eq!(h.link.relay(&[0xFF, 0xFF, 0xFF, 0xFF], SEC), bad);
    assert!(h.conns().iter().all(|c| c.requests.is_empty()));
}

#[test]
fn settings_changed_shutdown_is_an_expected_exit_so_repeated_restarts_do_not_suspend() {
    let h = Harness::new();
    for _ in 0..5 {
        h.link.shutdown(v1::ShutdownReason::SettingsChanged);
        h.down(DownKind::Interrupted);
        h.link.hello("1.0", SEC).unwrap();
    }
    assert!(!h.link.is_suspended());
    assert_eq!(h.conns().len(), 1);
}

// ---------------------------------------------------------------- EngineAdminTests

#[test]
fn admin_reads_the_profile_domains() {
    use v1::ProfileWeightKind as W;
    let h = Harness::new();
    h.respond(|_| {
        Some(profile_reply(&[
            ("it", "IT・ソフトウェア", W::ProfileWeightOften),
            ("novel", "小説", W::ProfileWeightOff),
            ("biz", "ビジネス", W::ProfileWeightSometimes),
            ("x", "未指定", W::ProfileWeightUnspecified),
        ]))
    });
    let domains = EngineAdmin::new(&h.link).profile().unwrap();
    let d = |id: &str, name: &str, weight| ProfileDomain {
        id: id.into(),
        name: name.into(),
        weight,
    };
    assert_eq!(
        domains,
        vec![
            d("it", "IT・ソフトウェア", ProfileWeight::Often),
            d("novel", "小説", ProfileWeight::Off),
            d("biz", "ビジネス", ProfileWeight::Sometimes),
            d("x", "未指定", ProfileWeight::Off),
        ]
    );
    assert!(matches!(
        h.latest().requests.last().unwrap().body,
        Some(Body::GetProfile(_))
    ));
}

#[test]
fn admin_set_profile_sends_only_the_domains_that_are_not_off() {
    let h = Harness::new();
    h.respond(|_| {
        Some(profile_reply(&[(
            "it",
            "IT",
            v1::ProfileWeightKind::ProfileWeightOften,
        )]))
    });
    let d = |id: &str, name: &str, weight| ProfileDomain {
        id: id.into(),
        name: name.into(),
        weight,
    };
    let result = EngineAdmin::new(&h.link)
        .set_profile(&[
            d("it", "IT", ProfileWeight::Often),
            d("novel", "小説", ProfileWeight::Off),
            d("biz", "ビジネス", ProfileWeight::Sometimes),
        ])
        .unwrap();
    let Some(Body::SetProfile(message)) = h.latest().requests.last().unwrap().body.clone() else {
        panic!("expected set_profile");
    };
    let ids: Vec<_> = message.domains.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["it", "biz"]);
    let weights: Vec<_> = message.domains.iter().map(|s| s.weight).collect();
    assert_eq!(
        weights,
        [
            v1::ProfileWeightKind::ProfileWeightOften as i32,
            v1::ProfileWeightKind::ProfileWeightSometimes as i32
        ]
    );
    assert_eq!(result, vec![d("it", "IT", ProfileWeight::Often)]);
}

#[test]
fn admin_set_weight_reads_the_latest_state_first_so_other_domains_survive() {
    use v1::ProfileWeightKind as W;
    let h = Harness::new();
    h.respond(|env| match &env.body {
        Some(Body::GetProfile(_)) => Some(profile_reply(&[
            ("it", "IT", W::ProfileWeightOften),
            ("biz", "ビジネス", W::ProfileWeightSometimes),
            ("novel", "小説", W::ProfileWeightOff),
        ])),
        Some(Body::SetProfile(set)) => Some(envelope(Body::ProfileReply(v1::ProfileReply {
            domains: set.domains.clone(),
        }))),
        _ => None,
    });
    let result = EngineAdmin::new(&h.link)
        .set_weight(ProfileWeight::Often, "novel")
        .unwrap();
    let mut on: Vec<_> = result
        .iter()
        .filter(|d| d.weight != ProfileWeight::Off)
        .map(|d| d.id.as_str())
        .collect();
    on.sort_unstable();
    assert_eq!(on, ["biz", "it", "novel"]);
    assert_eq!(
        result.iter().find(|d| d.id == "biz").unwrap().weight,
        ProfileWeight::Sometimes
    );
    assert_eq!(
        result.iter().find(|d| d.id == "novel").unwrap().weight,
        ProfileWeight::Often
    );
}

#[test]
fn admin_set_weight_for_an_unknown_domain_fails_without_writing() {
    let h = Harness::new();
    h.respond(|env| {
        assert!(
            !matches!(env.body, Some(Body::SetProfile(_))),
            "must not write"
        );
        Some(profile_reply(&[(
            "it",
            "IT",
            v1::ProfileWeightKind::ProfileWeightOften,
        )]))
    });
    assert_eq!(
        EngineAdmin::new(&h.link).set_weight(ProfileWeight::Off, "nope"),
        Err(EngineError::Engine(EngineErrorCode::BadRequest))
    );
}

#[test]
fn admin_surfaces_errors_wrong_kinds_and_too_many_domains() {
    let h = Harness::new();
    let admin = EngineAdmin::new(&h.link);
    h.respond(|_| Some(error_reply(v1::ErrorCode::BadRequest)));
    assert_eq!(
        admin.profile(),
        Err(EngineError::Engine(EngineErrorCode::BadRequest))
    );
    h.respond(|_| Some(envelope(Body::Ack(v1::Ack {}))));
    assert_eq!(admin.profile(), Err(EngineError::InvalidReply));
    h.respond(|_| {
        let ids: Vec<String> = (0..65).map(|i| format!("d{i}")).collect();
        let domains: Vec<(&str, &str, v1::ProfileWeightKind)> = ids
            .iter()
            .map(|id| (id.as_str(), "n", v1::ProfileWeightKind::ProfileWeightOff))
            .collect();
        Some(profile_reply(&domains))
    });
    assert_eq!(admin.profile(), Err(EngineError::InvalidReply));
}

// ---------------------------------------------------------------- XPCSegmentTests

fn layout(segments: &[(&str, &str)], focused: u32, rev: u32, with_page: bool) -> v1::SegmentLayout {
    v1::SegmentLayout {
        conversion_id: 9,
        layout_rev: rev,
        focused,
        segments: segments
            .iter()
            .map(|(reading, surface)| v1::SegmentInfo {
                reading: (*reading).into(),
                surface: (*surface).into(),
            })
            .collect(),
        focused_page: with_page.then(|| v1::SegmentPageReply {
            segment: focused,
            candidates: candidates(&[segments[focused as usize].1, "別"]),
            has_next: true,
            ..Default::default()
        }),
        engine_micros: 0,
    }
}

fn candidate_choice(index: u32) -> v1::SegmentChoice {
    v1::SegmentChoice {
        candidate_index: index,
        literal: String::new(),
    }
}

fn literal_choice(text: &str) -> v1::SegmentChoice {
    v1::SegmentChoice {
        candidate_index: 0,
        literal: text.into(),
    }
}

#[test]
fn segment_page_sends_the_layout_revision_and_left_choices_and_maps_the_reply() {
    let h = Harness::new();
    h.respond(|env| match env.body {
        Some(Body::SegmentPage(_)) => {
            Some(envelope(Body::SegmentPageReply(v1::SegmentPageReply {
                segment: 1,
                page_index: 2,
                candidates: vec![v1::Candidate {
                    surface: "知った".into(),
                    kind: v1::CandidateKind::Literal as i32,
                    ..Default::default()
                }],
                has_next: true,
                ..Default::default()
            })))
        }
        _ => None,
    });
    let request = v1::SegmentPageRequest {
        conversion_id: 9,
        layout_rev: 3,
        segment: 1,
        index: 2,
        left: vec![candidate_choice(4), literal_choice("ヒンシ")],
    };
    let page = h.link.segment_page(request.clone(), SEC).unwrap();
    assert!(page.segment == 1 && page.page_index == 2 && page.has_next);
    assert_eq!(page.candidates[0].surface, "知った");
    assert_eq!(page.candidates[0].kind, v1::CandidateKind::Literal as i32);
    let Some(Body::SegmentPage(sent)) = &h.latest().requests[0].body else {
        panic!("segment_page was not sent");
    };
    assert_eq!(sent, &request);
}

#[test]
fn resize_sends_a_signed_delta_and_returns_the_new_layout() {
    let h = Harness::new();
    h.respond(|_| {
        Some(envelope(Body::SegmentLayout(layout(
            &[("りぽじとり", "リポジトリ"), ("めい", "名")],
            1,
            1,
            true,
        ))))
    });
    let result = h
        .link
        .resize(
            v1::ResizeSegment {
                conversion_id: 9,
                layout_rev: 0,
                segment: 0,
                delta: -2,
                left: vec![],
            },
            SEC,
        )
        .unwrap();
    assert!(result.layout_rev == 1 && result.focused == 1 && result.conversion_id == 9);
    let segments: Vec<_> = result
        .segments
        .iter()
        .map(|s| (s.reading.as_str(), s.surface.as_str()))
        .collect();
    assert_eq!(segments, [("りぽじとり", "リポジトリ"), ("めい", "名")]);
    let page: Vec<_> = result
        .focused_page
        .unwrap()
        .candidates
        .into_iter()
        .map(|c| c.surface)
        .collect();
    assert_eq!(page, ["名", "別"]);
    let Some(Body::ResizeSegment(sent)) = &h.latest().requests[0].body else {
        panic!("resize_segment was not sent");
    };
    assert!(sent.delta == -2 && sent.segment == 0 && sent.layout_rev == 0);
}

#[test]
fn stale_layout_surfaces_as_its_own_error() {
    let h = Harness::new();
    h.respond(|_| Some(error_reply(v1::ErrorCode::StaleLayout)));
    let request = v1::ResizeSegment {
        conversion_id: 9,
        segment: 0,
        delta: 1,
        ..Default::default()
    };
    assert_eq!(
        h.link.resize(request, SEC),
        Err(EngineError::Engine(EngineErrorCode::StaleLayout))
    );
}

#[test]
fn commit_segments_is_a_notification_with_one_choice_per_segment() {
    let h = Harness::new();
    h.respond(|_| None);
    h.link.commit_segments(v1::CommitSegments {
        conversion_id: 9,
        layout_rev: 2,
        choices: vec![
            candidate_choice(0),
            candidate_choice(3),
            literal_choice("ヒ"),
        ],
    });
    assert!(h.latest().requests.is_empty());
    let Some(Body::CommitSegments(sent)) = &h.latest().posts[0].body else {
        panic!("commit_segments was not posted");
    };
    assert!(sent.conversion_id == 9 && sent.layout_rev == 2);
    let indices: Vec<_> = sent.choices.iter().map(|c| c.candidate_index).collect();
    assert_eq!(indices, [0, 3, 0]);
    let literals: Vec<_> = sent.choices.iter().map(|c| c.literal.as_str()).collect();
    assert_eq!(literals, ["", "", "ヒ"]);
}

#[test]
fn reconvert_sends_the_selection_and_left_context_and_returns_the_layout() {
    let h = Harness::new();
    h.respond(|_| {
        Some(envelope(Body::SegmentLayout(layout(
            &[("ひんし", "品詞")],
            0,
            0,
            true,
        ))))
    });
    let result = h
        .link
        .reconvert(
            v1::Reconvert {
                session_id: 5,
                surface: "品詞".into(),
                context: Some(v1::SurroundingText {
                    left: "辞書に".into(),
                    right: "を".into(),
                    available: true,
                }),
                page_size: 5,
            },
            SEC,
        )
        .unwrap();
    assert_eq!(result.segments[0].surface, "品詞");
    let Some(Body::Reconvert(sent)) = &h.latest().requests[0].body else {
        panic!("reconvert was not sent");
    };
    assert!(sent.session_id == 5 && sent.surface == "品詞" && sent.page_size == 5);
    let context = sent.context.as_ref().unwrap();
    assert!(context.left == "辞書に" && context.available);
}

#[test]
fn segment_requests_over_the_limits_are_refused_before_sending() {
    let h = Harness::new();
    let bad = Err(EngineError::Engine(EngineErrorCode::BadRequest));
    let reconvert = |surface: String| v1::Reconvert {
        session_id: 1,
        surface,
        context: None,
        page_size: 9,
    };
    assert_eq!(h.link.reconvert(reconvert("あ".repeat(65)), SEC), bad);
    assert_eq!(h.link.reconvert(reconvert(String::new()), SEC), bad);
    let mut zero_page = reconvert("品詞".into());
    zero_page.page_size = 0;
    assert_eq!(h.link.reconvert(zero_page, SEC), bad);
    let long_literal = v1::SegmentPageRequest {
        conversion_id: 1,
        left: vec![literal_choice(&"あ".repeat(257))],
        ..Default::default()
    };
    assert_eq!(
        h.link.segment_page(long_literal, SEC).err(),
        bad.clone().err()
    );
    let out_of_range = v1::SegmentPageRequest {
        segment: MAX_SEGMENTS as u32,
        ..Default::default()
    };
    assert_eq!(
        h.link.segment_page(out_of_range, SEC).err(),
        bad.clone().err()
    );
    assert_eq!(h.link.resize(v1::ResizeSegment::default(), SEC), bad); // delta = 0
    let huge_delta = v1::ResizeSegment {
        delta: -(MAX_TEXT_CHARACTERS as i32) - 1,
        ..Default::default()
    };
    assert_eq!(h.link.resize(huge_delta, SEC), bad);
    h.link.commit_segments(v1::CommitSegments::default());
    assert!(h.conns().is_empty());
}

#[test]
fn a_broken_layout_is_an_invalid_reply() {
    let h = Harness::new();
    let resize = || v1::ResizeSegment {
        conversion_id: 1,
        delta: 1,
        ..Default::default()
    };
    // 文節がない
    h.respond(|_| Some(envelope(Body::SegmentLayout(v1::SegmentLayout::default()))));
    assert_eq!(h.link.resize(resize(), SEC), Err(EngineError::InvalidReply));
    // フォーカスが範囲外
    h.respond(|_| {
        let mut broken = layout(&[("あ", "亜")], 0, 0, true);
        broken.focused = 3;
        Some(envelope(Body::SegmentLayout(broken)))
    });
    assert_eq!(h.link.resize(resize(), SEC), Err(EngineError::InvalidReply));
    // 違う種類の返事
    h.respond(|_| Some(envelope(Body::Ack(v1::Ack {}))));
    assert_eq!(
        h.link.segment_page(v1::SegmentPageRequest::default(), SEC),
        Err(EngineError::InvalidReply)
    );
}

#[test]
fn convert_reply_carries_the_layout_and_an_old_engine_has_none() {
    let h = Harness::new();
    h.respond(|_| {
        Some(envelope(Body::ConvertReply(v1::ConvertReply {
            conversion_id: 9,
            candidates: candidates(&["初めて知った"]),
            layout: Some(layout(
                &[("はじめて", "初めて"), ("しった", "知った")],
                0,
                0,
                true,
            )),
            ..Default::default()
        })))
    });
    let page = h.link.convert(sample_request(), SEC).unwrap();
    let layout = page.layout.unwrap();
    let readings: Vec<_> = layout.segments.iter().map(|s| s.reading.as_str()).collect();
    assert_eq!(readings, ["はじめて", "しった"]);
    assert_eq!(layout.focused, 0);

    let plain = Harness::new().link.convert(sample_request(), SEC).unwrap();
    assert_eq!(plain.layout, None);
}

#[test]
fn a_broken_layout_inside_a_convert_reply_is_invalid() {
    let h = Harness::new();
    h.respond(|_| {
        Some(envelope(Body::ConvertReply(v1::ConvertReply {
            candidates: candidates(&["亜"]),
            layout: Some(v1::SegmentLayout::default()),
            ..Default::default()
        })))
    });
    assert_eq!(
        h.link.convert(sample_request(), SEC),
        Err(EngineError::InvalidReply)
    );
}

// ---------------------------------------------------------------- XPCEngineDiagTests

fn episode() -> v1::DiagEvent {
    v1::DiagEvent {
        session_id: 1,
        kind: "sh_episode".into(),
        nums: vec![],
        tags: vec![v1::DiagTag {
            key: "mode".into(),
            value: "romaji".into(),
        }],
    }
}

fn diags(conn: &Conn) -> Vec<v1::DiagEvent> {
    conn.posts
        .iter()
        .filter_map(|env| match &env.body {
            Some(Body::Diag(event)) => Some(event.clone()),
            _ => None,
        })
        .collect()
}

fn nums(event: &v1::DiagEvent) -> Vec<(&str, i64)> {
    event
        .nums
        .iter()
        .map(|n| (n.key.as_str(), n.value))
        .collect()
}

#[test]
fn diag_nothing_is_sent_when_nothing_happened() {
    let h = Harness::new();
    h.link.convert(sample_request(), SEC).unwrap();
    h.link.diag(episode());
    let kinds: Vec<_> = diags(&h.latest()).into_iter().map(|e| e.kind).collect();
    assert_eq!(kinds, ["sh_episode"]);
}

#[test]
fn diag_a_timeout_and_the_shell_fallback_are_sent_once_with_the_next_event() {
    let h = Harness::new();
    h.respond(|_| None);
    assert_eq!(
        h.link.convert(sample_request(), MS),
        Err(EngineError::Timeout)
    );
    h.link.diag(episode());
    h.link.diag(episode());
    let sent = diags(&h.latest());
    let kinds: Vec<_> = sent.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, ["sh_engine", "sh_episode", "sh_episode"]); // 2 回目は相乗りしない
    assert_eq!(nums(&sent[0]), [("fallback_convs", 1), ("timeout", 1)]);
    assert_eq!(sent[0].session_id, 1);
    assert!(sent[0].tags.is_empty());
}

#[test]
fn diag_crashes_interruptions_and_resync_are_counted() {
    let h = Harness::new();
    h.link.hello("1.0", SEC).unwrap();
    h.link
        .begin_session(session(1, "com.apple.TextEdit", false));
    h.down(DownKind::Interrupted);
    h.advance(Duration::from_secs(100));
    h.down(DownKind::Invalidated);
    h.link.convert(sample_request(), SEC).unwrap(); // 接続を作り直し、送り直す
    h.link.diag(episode());
    let sent = diags(&h.latest());
    let engine = sent.iter().find(|e| e.kind == "sh_engine").unwrap();
    assert_eq!(
        nums(engine),
        [
            ("interrupted", 1),
            ("invalidated", 1),
            ("reconnect", 1),
            ("resync", 1)
        ]
    );
}

#[test]
fn diag_opening_the_breaker_is_counted_and_unsent_counts_carry_over() {
    let h = Harness::new();
    h.link.convert(sample_request(), SEC).unwrap();
    for _ in 0..3 {
        h.down(DownKind::Interrupted);
        h.advance(Duration::from_secs(10));
    }
    // 休んでいる間は送れない（数は持ち越す）
    h.link.diag(episode());
    assert!(diags(&h.latest()).iter().all(|e| e.kind != "sh_engine"));

    h.advance(SUSPENSION + SEC);
    h.link.diag(episode());
    let sent = diags(&h.latest());
    let engine = sent.iter().find(|e| e.kind == "sh_engine").unwrap();
    let n = nums(engine);
    assert!(n.contains(&("breaker_open", 1)));
    assert!(n.contains(&("interrupted", 3)));
}

#[test]
fn diag_keys_are_sorted_so_the_same_event_gives_the_same_bytes() {
    let h = Harness::new();
    let mut event = episode();
    event.nums = vec![
        v1::DiagNum {
            key: "b".into(),
            value: 2,
        },
        v1::DiagNum {
            key: "a".into(),
            value: 1,
        },
    ];
    h.link.diag(event);
    let sent = diags(&h.latest());
    assert_eq!(nums(&sent[0]), [("a", 1), ("b", 2)]);
}

// ---------------------------------------------------------------- 絵文字と郵便番号の口

#[test]
fn emoji_table_asks_for_the_table_and_decodes_the_reply() {
    let h = Harness::new();
    h.respond(|env| match env.body {
        Some(Body::EmojiTable(_)) => Some(envelope(Body::EmojiTableReply(v1::EmojiTableReply {
            entries: vec![v1::EmojiEntry {
                name: "smile".into(),
                emoji: "😄".into(),
            }],
            data_label: "emoji".into(),
        }))),
        _ => None,
    });
    let table = h.link.emoji_table(SEC).unwrap();
    assert_eq!(table.entries[0].name, "smile");
    assert!(matches!(
        h.latest().requests[0].body,
        Some(Body::EmojiTable(_))
    ));
}

#[test]
fn emoji_table_and_postal_lookup_fail_without_blocking_when_the_engine_does_not_answer() {
    let h = Harness::new();
    h.respond(|_| None);
    assert_eq!(h.link.emoji_table(MS), Err(EngineError::Timeout));
    assert_eq!(
        h.link.postal_lookup("1000001", MS),
        Err(EngineError::Timeout)
    );
}

#[test]
fn postal_lookup_sends_the_code_and_decodes_the_reply() {
    let h = Harness::new();
    h.respond(|env| match &env.body {
        Some(Body::PostalLookup(request)) if request.code == "1000001" => {
            Some(envelope(Body::PostalLookupReply(v1::PostalLookupReply {
                addresses: vec!["東京都千代田区千代田".into()],
            })))
        }
        _ => None,
    });
    assert_eq!(
        h.link.postal_lookup("1000001", SEC).unwrap(),
        vec!["東京都千代田区千代田"]
    );
}

// ---------------------------------------------------------------- 異常終了の記録（ADR 0051 §5.2）

fn crashed(sig: i32) -> ikari_runmark::PrevExit {
    ikari_runmark::PrevExit {
        exit: ikari_runmark::Exit::Crash,
        sig: Some(sig),
        up_s: Some(5310),
    }
}

fn diag_kinds(conn: &Conn) -> Vec<String> {
    diags(conn).into_iter().map(|e| e.kind).collect()
}

#[test]
fn a_crashed_previous_run_rides_on_the_first_non_secure_event_once() {
    let h = Harness::new();
    h.link.set_prev_exit(crashed(11));
    h.link.diag(episode());
    h.link.diag(episode());
    let sent = diags(&h.latest());
    let k: Vec<_> = sent.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(k, ["sh_prev_exit", "sh_episode", "sh_episode"]);
    assert_eq!(nums(&sent[0]), [("sig", 11), ("up_s", 5310)]);
    assert_eq!(sent[0].tags.len(), 1);
    assert_eq!(
        (sent[0].tags[0].key.as_str(), sent[0].tags[0].value.as_str()),
        ("exit", "crash")
    );
    assert_eq!(sent[0].session_id, 1);
}

#[test]
fn prev_exit_is_not_sent_for_a_secure_session() {
    // secure のセッションの事象は殻が `diag` を呼ばない（NFR-18）ので、相乗りもしない。
    // secure のセッションを開いて閉じただけでは何も送らず、次の secure でない事象で初めて送る
    let h = Harness::new();
    h.link.set_prev_exit(crashed(11));
    h.link.begin_session(session(1, "com.apple.TextEdit", true));
    h.link.end_session(1);
    assert!(diag_kinds(&h.latest()).is_empty());
    h.link.diag(episode());
    assert_eq!(diag_kinds(&h.latest()), ["sh_prev_exit", "sh_episode"]);
}

#[test]
fn prev_exit_is_carried_over_when_the_send_fails() {
    let h = Harness::new();
    h.link.convert(sample_request(), SEC).unwrap();
    h.link.set_prev_exit(crashed(11));
    for _ in 0..3 {
        h.down(DownKind::Interrupted);
        h.advance(Duration::from_secs(10));
    }
    // 休んでいる間は送れない
    h.link.diag(episode());
    assert!(diag_kinds(&h.latest()).iter().all(|k| k != "sh_prev_exit"));

    h.advance(SUSPENSION + SEC);
    h.link.diag(episode());
    let sent = diag_kinds(&h.latest());
    assert_eq!(sent.iter().filter(|k| *k == "sh_prev_exit").count(), 1);
}

#[test]
fn sh_panic_count_is_reduced_by_what_was_sent() {
    let h = Harness::new();
    h.link.note_panic("main", "shell-core/src/a.rs:1");
    h.link.note_panic("ikari-log", "shell-core/src/b.rs:2");
    h.link.diag(episode());
    let sent = diags(&h.latest());
    let panic = sent.iter().find(|e| e.kind == "sh_panic").unwrap();
    assert_eq!(nums(panic), [("n", 2)]);
    let tag = |key: &str| {
        panic
            .tags
            .iter()
            .find(|t| t.key == key)
            .map(|t| t.value.as_str())
    };
    assert_eq!(tag("loc"), Some("shell-core/src/b.rs:2"));
    assert_eq!(tag("thread"), Some("other"));

    // 送った分は引かれ、増えた分だけが次に出る。何もなければ出ない
    h.link.diag(episode());
    let panic_count = |h: &Harness| {
        diag_kinds(&h.latest())
            .iter()
            .filter(|k| *k == "sh_panic")
            .count()
    };
    assert_eq!(panic_count(&h), 1); // 2 回目は出ない
    h.link.note_panic("main", "shell-core/src/c.rs:3");
    h.link.diag(episode());
    assert_eq!(panic_count(&h), 2);
    let sent = diags(&h.latest());
    let last = sent.iter().rfind(|e| e.kind == "sh_panic").unwrap();
    assert_eq!(nums(last), [("n", 1)]);
}
