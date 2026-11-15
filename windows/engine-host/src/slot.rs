//! エンジンの置き場所（ADR 0062 §5.4 の決定 1。#414）。OS に依存しない（Mac でテストする）。
//!
//! engine-host は、辞書がないなどでエンジンを開けなくても終わらず、受け口を立てて常駐する。開けていない間は、
//! どの要求にも `ENGINE_NOT_READY` を返し（接続は切らない。殻はかなの候補に落ちる）、次の要求で開き直す。
//! macOS の `IkariEngineService` の `EngineHost`（`openIfDue` と `notReady`）と同じ動き。
//!
//! - 開くのは、shell-core の寿命の門（`EngineGate`）の `enter` の中（門の錠の中）。同時に来た要求が二重に開かない。
//! - 開き直しは、前の試みから `reopen_interval` 以上たっているときだけ（要求のたびに辞書を開かない）。
//! - 一度開けたら、門が持ち続ける。最後の `Arc<EngineSlot>` が落ちると `Engine` も落ちる（学習の保存とロックの解放）。
//! - 接続ごとの `Connection` は、開けたあとの最初の要求で取る（`serve`）。開く前の要求は、その接続のセッションを
//!   作らないので、殻は `ENGINE_NOT_READY` を見て、次の要求の前に hello とセッションを送り直す（shell-core の `EngineLink`）。
//!
//! 判断の置き場所: Rust。

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ikari_engine::{Connection, Engine, not_ready_reply};
use ikari_shell_core::engine::EngineGate;

/// 開けなかったあと、開き直すまでの最短の間隔（macOS の `ServiceConstants.reopenInterval` と同じ値）
pub const REOPEN_INTERVAL: Duration = Duration::from_secs(2);

/// エンジンを開く手段。開けなければ `None`（理由の記録は、この手段が持つ）
pub type OpenEngine = Box<dyn FnMut() -> Option<Engine> + Send>;

struct Opener {
    open: OpenEngine,
    interval: Duration,
    last_attempt: Option<Instant>,
}

impl Opener {
    /// `force` でなければ、前の試みから `interval` 以上たっているときだけ開く。門の錠の中で呼ばれる
    fn attempt(&mut self, force: bool) -> Option<Arc<Engine>> {
        let now = Instant::now();
        let due = self
            .last_attempt
            .is_none_or(|last| now.duration_since(last) >= self.interval);
        if !force && !due {
            return None;
        }
        self.last_attempt = Some(now);
        (self.open)().map(Arc::new)
    }
}

/// 開けているかもしれないエンジン。engine-host の受け口（エンジンと設定画面）が共有する。
pub struct EngineSlot {
    gate: EngineGate<Arc<Engine>>,
    /// `gate` の錠の中（`enter` の `open`）でだけ使う
    opener: Mutex<Option<Opener>>,
}

impl EngineSlot {
    /// まだ開いていない置き場所。`open` は、最初の `open_now` か、間隔をおいた要求のときに呼ぶ。
    pub fn new(open: OpenEngine, reopen_interval: Duration) -> Self {
        Self {
            gate: EngineGate::new(),
            opener: Mutex::new(Some(Opener {
                open,
                interval: reopen_interval,
                last_attempt: None,
            })),
        }
    }

    /// すでに開いたエンジンを持つ置き場所（開き直す手段はない）。
    pub fn opened(engine: Arc<Engine>) -> Self {
        let slot = Self {
            gate: EngineGate::new(),
            opener: Mutex::new(None),
        };
        if slot.gate.enter(|| Some(engine)).is_some() {
            slot.gate.leave();
        }
        slot
    }

    fn open_with(&self, force: bool) -> Option<Arc<Engine>> {
        self.opener
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
            .and_then(|opener| opener.attempt(force))
    }

    /// 開いていなければ、間隔を見ずに今開く（起動の最初の 1 回）。開いているかを返す。
    pub fn open_now(&self) -> bool {
        self.gate
            .with_resource(|| self.open_with(true), |_| ())
            .is_some()
    }

    /// 開いているか（開こうとはしない）。
    pub fn is_open(&self) -> bool {
        self.with_open(|_| ()).is_some()
    }

    /// 開いていれば `body` を呼ぶ（開こうとはしない。診断の行を書くときなど）。
    pub fn with_open<T>(&self, body: impl FnOnce(&Engine) -> T) -> Option<T> {
        self.gate.with_resource(|| None, |engine| body(&engine))
    }

    /// 接続 1 本の口を取る。開いていなければ、間隔をおいて開き直す。開けなければ `None`。
    pub fn connect(&self) -> Option<Connection> {
        self.gate
            .with_resource(|| self.open_with(false), |engine| engine.connect())
    }

    /// 既定の接続（設定画面の中継）で要求に答える。開けなければ `ENGINE_NOT_READY` を返す。
    pub fn handle(&self, request: &[u8]) -> Vec<u8> {
        self.gate
            .with_resource(|| self.open_with(false), |engine| engine.handle(request))
            .unwrap_or_else(|| not_ready_reply(request))
    }
}

impl fmt::Debug for EngineSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineSlot")
            .field("open", &self.is_open())
            .finish_non_exhaustive()
    }
}

/// 接続 1 本の要求に答える（`serve` が接続ごとに 1 つ持つ）。開けたあとの最初の要求で `Connection` を取る。
pub struct ConnectionSlot {
    slot: Arc<EngineSlot>,
    connection: Option<Connection>,
}

impl ConnectionSlot {
    pub fn new(slot: Arc<EngineSlot>) -> Self {
        let connection = slot.connect();
        Self { slot, connection }
    }

    /// 開けていなければ `ENGINE_NOT_READY`。どの要求にも返事を返す（接続を切らない）。
    pub fn handle(&mut self, request: &[u8]) -> Vec<u8> {
        if self.connection.is_none() {
            self.connection = self.slot.connect();
        }
        match &self.connection {
            Some(connection) => connection.handle(request),
            None => not_ready_reply(request),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ikari_proto::v1::{Envelope, ErrorCode, envelope};
    use prost::Message;

    /// 開けない手段（呼ばれた数を数える）
    fn never_opens(calls: &Arc<AtomicUsize>) -> OpenEngine {
        let calls = Arc::clone(calls);
        Box::new(move || {
            calls.fetch_add(1, Ordering::SeqCst);
            None
        })
    }

    fn request(request_id: u64) -> Vec<u8> {
        Envelope {
            schema_version: ikari_proto::SCHEMA_VERSION,
            request_id,
            body: Some(envelope::Body::Hello(Default::default())),
        }
        .encode_to_vec()
    }

    fn assert_not_ready(reply: &[u8], request_id: u64) {
        let reply = Envelope::decode(reply).expect("an Envelope");
        assert_eq!(reply.request_id, request_id);
        match reply.body {
            Some(envelope::Body::Error(e)) => assert_eq!(e.code, ErrorCode::EngineNotReady as i32),
            other => panic!("expected ENGINE_NOT_READY, got {other:?}"),
        }
    }

    #[test]
    fn an_unopened_slot_answers_not_ready_with_the_request_id() {
        let calls = Arc::new(AtomicUsize::new(0));
        let slot = Arc::new(EngineSlot::new(never_opens(&calls), Duration::ZERO));
        assert!(!slot.open_now());
        assert_not_ready(&slot.handle(&request(7)), 7);
        let mut connection = ConnectionSlot::new(Arc::clone(&slot));
        assert_not_ready(&connection.handle(&request(8)), 8);
        assert!(!slot.is_open());
    }

    #[test]
    fn reopening_waits_for_the_interval() {
        let calls = Arc::new(AtomicUsize::new(0));
        let slot = EngineSlot::new(never_opens(&calls), Duration::from_secs(3600));
        assert!(!slot.open_now());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // 間隔の内の要求では開き直さない
        assert_not_ready(&slot.handle(&request(1)), 1);
        assert!(slot.connect().is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // 起動の最初の 1 回（`open_now`）は間隔を見ない
        assert!(!slot.open_now());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn with_zero_interval_every_request_tries_again() {
        let calls = Arc::new(AtomicUsize::new(0));
        let slot = EngineSlot::new(never_opens(&calls), Duration::ZERO);
        slot.handle(&request(1));
        slot.handle(&request(2));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn looking_does_not_open() {
        let calls = Arc::new(AtomicUsize::new(0));
        let slot = EngineSlot::new(never_opens(&calls), Duration::ZERO);
        assert!(!slot.is_open());
        assert!(slot.with_open(|_| ()).is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
