//! エンジンとの通信（ADR 0041 §4.3。レーン L-B）。
//!
//! `EngineLink` は、移行の前の Swift の `XPCEngineClient` の中身（`Envelope` の組み立てと読み取り、要求番号、誤りの 4 種への写し、
//! 返事の検証、再接続の世代、落ちた回数の休止、診断の片道、管理の要求の素通し）を持つ。OS 側はバイト列を運ぶだけ。
//!
//! **契約**: 公開の関数のシグネチャ（下の `impl EngineLink`）は M1-a で固めた。L-B は中身だけを書き、シグネチャは変えない。
//! 要求と返事は `ikari_proto::v1` の型をそのまま使う（`shell-core` は proto に依存する。Swift のような写しの型は作らない）。
//!
//! スレッド: `EngineLink` は `Send + Sync`。状態機械からは 1 つのスレッドで呼ぶ。`notify_down` だけは、どのスレッドからでも呼べる。
//!
//! # スレッドとロックの設計
//! - トランスポート（`EngineTransport`）は `&self` で、複数のスレッドから同時に呼ぶ。往復の要求は並行に流れ、
//!   設定アプリの中継（インポートで最大 30 秒）が返事を待つ間も、主スレッドの変換は止まらない（Swift の XPC と同じ）。
//! - **`state`**（`Mutex<LinkState>`）: 今の接続、要求番号、セッション、再同期の要否、落ちた時刻の列、休止の期限、診断の数。
//!   短い間だけ持つ。**持ったままトランスポートを呼ばない**（ホストが `request` の中から `notify_down` を
//!   同じスレッドで呼んでも、行き詰まらない）。
//! - **`prepare_lock`**（`Mutex<()>`）: 接続の用意（無効になった接続を閉じる、つなぐ、再同期の `Hello` とセッションを送る）を
//!   1 本にする。持つ間にトランスポートの `close`・`connect`・`post` を呼ぶが、返事を待つ `request` は持たずに呼ぶ。
//!   再同期を送り終えるまで、ほかのスレッドの要求は送られない（Swift の `prepare` がロックの中で送っていたのと同じ）。
//! - ロックの順: `prepare_lock` → `state` だけ（逆はない）。`notify_down` は `state` だけを取るので、待っている要求にも
//!   接続の用意にも止められない（ADR 0041 §4.3）。
//! - 版の不一致のときの終了の確認（`shutdown_and_wait_for_exit`）は、`state` の `Condvar`（`exited`）で待つ
//!   （`notify_down` が起こす）。待ちは実時間（`Clock` は経過の判断だけに使う）。

mod admin;
mod codec;
mod counts;
mod gate;

#[cfg(test)]
mod crash_record;
#[cfg(test)]
mod tests;

pub use admin::EngineAdmin;
pub use codec::{
    MAX_CANDIDATES, MAX_CONTEXT_CHARACTERS, MAX_EMOJI_ENTRIES, MAX_EMOJI_NAME_CHARACTERS,
    MAX_EMOJI_SCALARS, MAX_LITERAL_CHARACTERS, MAX_POSTAL_ADDRESS_CHARACTERS, MAX_POSTAL_ADDRESSES,
    MAX_READING_CHARACTERS, MAX_RECONVERT_CHARACTERS, MAX_REPLY_BYTES, MAX_SEGMENTS,
    MAX_TEXT_CHARACTERS, SCHEMA_VERSION, diag_message, error_code, lm_state,
};
pub use counts::EngineCounts;
pub use gate::{EngineGate, GateClose};

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use ikari_proto::v1::{self, envelope::Body};
use prost::Message;

use crate::host::{Clock, DownKind, EngineTransport, TransportError, TransportHandle};
use crate::types::{EngineError, EngineErrorCode};

/// 落ちた回数がこの数に達したら休む（ADR 0004 §7.5）
pub const CRASH_LIMIT: usize = 3;
/// 落ちた回数を数える期間
pub const CRASH_WINDOW: Duration = Duration::from_secs(60);
/// 休む長さ
pub const SUSPENSION: Duration = Duration::from_secs(300);
/// 終了の頼み（`Shutdown`）の返事と、終了の確認を待つ上限
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(1);
/// 中継する要求と返事の上限（エンジンの `MAX_ADMIN_REQUEST_BYTES` と枠の上限と同じ。docs/phase2-contract.md §5）
pub const MAX_RELAY_BYTES: usize = 64 * 1024 * 1024;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // 持ち主が panic しても、状態は整数と列だけなので使い続ける（入力を止めない）
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn transport_error(error: TransportError) -> EngineError {
    match error {
        TransportError::Unavailable => EngineError::Unavailable,
        TransportError::Timeout => EngineError::Timeout,
    }
}

fn bad_request() -> EngineError {
    EngineError::Engine(EngineErrorCode::BadRequest)
}

/// `state` が守るもの
#[derive(Default)]
struct LinkState {
    /// 今の接続。`Invalidated` で外れる（`Interrupted` では残す。launchd が次のメッセージで起動し直す）
    current: Option<TransportHandle>,
    /// 一度でもつないだか（2 回目からのつなぎ直しを `reconnect` に数える）
    connected_once: bool,
    /// 無効になった接続。次に接続を用意するときに `close` する
    stale: Vec<TransportHandle>,
    next_request_id: u64,
    /// 開いているセッション（落ちたあとに送り直す）。番号の順に送る
    sessions: BTreeMap<u64, v1::BeginSession>,
    shell_version: Option<String>,
    needs_resync: bool,
    expecting_exit: bool,
    down_events: u64,
    crash_times: Vec<Duration>,
    suspended_until: Option<Duration>,
    observed_engine_version: Option<String>,
    /// 前回の `sh_engine` を送ってからの事象の数（ADR 0034 §4.2）
    counts: EngineCounts,
}

impl LinkState {
    fn take_request_id(&mut self) -> u64 {
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        id
    }

    /// 休んでいれば `Unavailable`。期限が過ぎていれば休止を解く
    fn admit(&mut self, now: Duration) -> Result<(), EngineError> {
        if let Some(until) = self.suspended_until {
            if now < until {
                return Err(EngineError::Unavailable);
            }
            self.suspended_until = None;
            self.crash_times.clear();
        }
        Ok(())
    }

    fn record_crash(&mut self, now: Duration) {
        self.crash_times.push(now);
        self.crash_times
            .retain(|time| now.saturating_sub(*time) <= CRASH_WINDOW);
        if self.crash_times.len() >= CRASH_LIMIT {
            self.suspended_until = Some(now + SUSPENSION);
            self.crash_times.clear();
            self.counts.breaker_open += 1;
        }
    }
}

/// エンジンへの口。
pub struct EngineLink {
    transport: Box<dyn EngineTransport>,
    /// 接続の用意を 1 本にする（上の「スレッドとロックの設計」）
    prepare_lock: Mutex<()>,
    state: Mutex<LinkState>,
    /// `down_events` が増えたら起こす（終了の確認の待ち）
    exited: Condvar,
    clock: Arc<dyn Clock>,
}

impl EngineLink {
    /// まだつながない（最初の要求のときにつなぐ）。
    pub fn new(transport: Box<dyn EngineTransport>, clock: Arc<dyn Clock>) -> Self {
        Self {
            transport,
            prepare_lock: Mutex::new(()),
            state: Mutex::new(LinkState {
                next_request_id: 1,
                ..LinkState::default()
            }),
            exited: Condvar::new(),
            clock,
        }
    }

    /// 版の照合とエンジンの起動の前倒し（今の `hello`）。
    pub fn hello(
        &self,
        shell_version: &str,
        timeout: Duration,
    ) -> Result<v1::HelloReply, EngineError> {
        lock(&self.state).shell_version = Some(shell_version.to_owned());
        let body = Body::Hello(v1::Hello {
            shell_version: shell_version.to_owned(),
        });
        let mut restarted = false;
        loop {
            let reply = codec::hello(self.round_trip(timeout, &body)?)?;
            lock(&self.state).observed_engine_version = Some(reply.engine_version.clone());
            if reply.engine_version == shell_version {
                return Ok(reply);
            }
            // 入れ替えた直後に古いエンジンが残っている。止めて、新しい版を起動し直す（ADR 0004 §5.2）
            if restarted {
                return Err(EngineError::Unavailable);
            }
            restarted = true;
            self.shutdown_and_wait_for_exit();
        }
    }

    pub fn begin_session(&self, session: v1::BeginSession) {
        lock(&self.state)
            .sessions
            .insert(session.session_id, session.clone());
        self.post(Body::BeginSession(session));
    }

    pub fn end_session(&self, session_id: u64) {
        lock(&self.state).sessions.remove(&session_id);
        self.post(Body::EndSession(v1::EndSession { session_id }));
    }

    /// 読みが変わるたびに呼ぶ。返事を待たない。
    pub fn prefetch(&self, mut request: v1::Prefetch) {
        if !codec::is_valid_reading(&request.reading) {
            return;
        }
        request.context = codec::clipped(request.context);
        self.post(Body::Prefetch(request));
    }

    /// 同期で待つ（上限は `timeout`。今の `EngineTimeouts.convertTimeout`）。返事は検証してから返す。
    pub fn convert(
        &self,
        request: v1::Convert,
        timeout: Duration,
    ) -> Result<v1::ConvertReply, EngineError> {
        let result = self.convert_once(request, timeout);
        if result.is_err() {
            // 殻は、失敗した変換をエンジンなしの候補で補う
            lock(&self.state).counts.fallback_convs += 1;
        }
        result
    }

    fn convert_once(
        &self,
        mut request: v1::Convert,
        timeout: Duration,
    ) -> Result<v1::ConvertReply, EngineError> {
        if !codec::is_valid_reading(&request.reading)
            || !codec::PAGE_SIZE_RANGE.contains(&request.page_size)
        {
            return Err(bad_request());
        }
        request.context = codec::clipped(request.context);
        codec::page(self.round_trip(timeout, &Body::Convert(request))?, false)
    }

    pub fn page(
        &self,
        conversion_id: u64,
        index: u32,
        timeout: Duration,
    ) -> Result<v1::ConvertReply, EngineError> {
        if index > codec::MAX_INDEX {
            return Err(bad_request());
        }
        let body = Body::Page(v1::PageRequest {
            conversion_id,
            index,
        });
        codec::page(self.round_trip(timeout, &body)?, true)
    }

    /// `candidate_index` は生成済みの列の中の通し番号（ページをまたぐ）。
    pub fn commit(&self, conversion_id: u64, candidate_index: u32) {
        if candidate_index > codec::MAX_INDEX {
            return;
        }
        self.post(Body::Commit(v1::Commit {
            conversion_id,
            candidate_index,
        }));
    }

    /// 変換せずに確定した文字列（確定履歴のため）。
    pub fn commit_text(&self, request: v1::CommitText) {
        self.post(Body::CommitText(request));
    }

    pub fn cancel(&self, conversion_id: u64) {
        self.post(Body::Cancel(v1::Cancel { conversion_id }));
    }

    /// 文節の候補の 1 ページ（ADR 0006 §5.2）。
    pub fn segment_page(
        &self,
        request: v1::SegmentPageRequest,
        timeout: Duration,
    ) -> Result<v1::SegmentPageReply, EngineError> {
        if request.segment as usize >= codec::MAX_SEGMENTS
            || request.index > codec::MAX_INDEX
            || !codec::is_valid_choices(&request.left)
        {
            return Err(bad_request());
        }
        codec::segment_page(self.round_trip(timeout, &Body::SegmentPage(request))?)
    }

    /// 区切り直し。返事は新しい区切り（`layout_rev` が上がる）。
    pub fn resize(
        &self,
        request: v1::ResizeSegment,
        timeout: Duration,
    ) -> Result<v1::SegmentLayout, EngineError> {
        if request.segment as usize >= codec::MAX_SEGMENTS
            || request.delta == 0
            || request.delta.unsigned_abs() as usize > codec::MAX_TEXT_CHARACTERS
            || !codec::is_valid_choices(&request.left)
        {
            return Err(bad_request());
        }
        codec::segment_layout(self.round_trip(timeout, &Body::ResizeSegment(request))?)
    }

    /// 変換中の文節 `[first, first + count)` を取り除く（ADR 0050 §5.3）。返事は新しい区切り（`layout_rev` が上がる）。
    pub fn remove_segments(
        &self,
        request: v1::RemoveSegments,
        timeout: Duration,
    ) -> Result<v1::SegmentLayout, EngineError> {
        if request.count == 0
            || request.first as usize >= codec::MAX_SEGMENTS
            || request.count as usize > codec::MAX_SEGMENTS
        {
            return Err(bad_request());
        }
        codec::segment_layout(self.round_trip(timeout, &Body::RemoveSegments(request))?)
    }

    /// 文節ごとの選択で確定する（返事を待たない）。
    pub fn commit_segments(&self, request: v1::CommitSegments) {
        if request.choices.is_empty() || !codec::is_valid_choices(&request.choices) {
            return;
        }
        self.post(Body::CommitSegments(request));
    }

    pub fn reconvert(
        &self,
        mut request: v1::Reconvert,
        timeout: Duration,
    ) -> Result<v1::SegmentLayout, EngineError> {
        if request.surface.is_empty()
            || request.surface.chars().count() > codec::MAX_RECONVERT_CHARACTERS
            || !codec::PAGE_SIZE_RANGE.contains(&request.page_size)
        {
            return Err(bad_request());
        }
        request.context = codec::clipped(request.context);
        codec::segment_layout(self.round_trip(timeout, &Body::Reconvert(request))?)
    }

    /// 保存して終了するよう頼む。終了は落ちた回数に数えない。
    pub fn shutdown(&self, reason: v1::ShutdownReason) {
        lock(&self.state).expecting_exit = true;
        let body = Body::Shutdown(v1::Shutdown {
            reason: reason as i32,
        });
        if self.round_trip(SHUTDOWN_WAIT, &body).is_err() {
            lock(&self.state).expecting_exit = false;
        }
    }

    /// 管理の要求（ADR 0005 §8.3）。`Envelope` のバイト列をそのまま送り、返事のバイト列をそのまま返す。
    /// `request_id` は付け直す。許可リストでの判定は呼ぶ側（`bridge`）が行う。
    pub fn relay(&self, envelope: &[u8], timeout: Duration) -> Result<Vec<u8>, EngineError> {
        if envelope.is_empty() || envelope.len() > MAX_RELAY_BYTES {
            return Err(bad_request());
        }
        let mut request = v1::Envelope::decode(envelope).map_err(|_| bad_request())?;
        let handle = self.prepare()?;
        request.schema_version = SCHEMA_VERSION;
        request.request_id = lock(&self.state).take_request_id();
        let reply = self
            .transport
            .request(handle, &request.encode_to_vec(), timeout)
            .map_err(transport_error)?;
        if reply.is_empty() || reply.len() > MAX_RELAY_BYTES {
            return Err(EngineError::InvalidReply);
        }
        Ok(reply)
    }

    /// 絵文字のショートコードの表（ADR 0027 §9.2）。
    pub fn emoji_table(&self, timeout: Duration) -> Result<v1::EmojiTableReply, EngineError> {
        let body = Body::EmojiTable(v1::EmojiTableRequest {});
        codec::emoji_table(self.round_trip(timeout, &body)?)
    }

    /// 診断ログの事象（ADR 0034 §2.2）。返事を待たない。secure のセッションでは呼ばない。
    pub fn diag(&self, mut event: v1::DiagEvent) {
        self.post_engine_counts_if_any(event.session_id);
        // キーを並べ替えて、同じ入力から同じバイト列にする
        event.nums.sort_by(|a, b| a.key.cmp(&b.key));
        event.tags.sort_by(|a, b| a.key.cmp(&b.key));
        self.post(Body::Diag(event));
    }

    /// 英語のモードの郵便番号（ADR 0032 §9.1）。`code` は ASCII 数字 7 桁。
    pub fn postal_lookup(&self, code: &str, timeout: Duration) -> Result<Vec<String>, EngineError> {
        let body = Body::PostalLookup(v1::PostalLookupRequest {
            code: code.to_owned(),
        });
        codec::postal_lookup(self.round_trip(timeout, &body)?)
    }

    /// 今の接続の相手のプロセスの番号。
    pub fn pid(&self) -> Option<i32> {
        let current = lock(&self.state).current;
        current.and_then(|handle| self.transport.pid(handle))
    }

    /// 接続が落ちた（**どのスレッドからでも呼べる**。ADR 0041 §4.3）。
    /// `connection` が今の接続でなければ（古い世代の通知なら）捨てる。
    pub fn notify_down(&self, connection: TransportHandle, kind: DownKind) {
        let now = self.clock.now();
        let mut state = lock(&self.state);
        if state.current != Some(connection) {
            return; // 古い接続の通知
        }
        state.down_events += 1;
        state.needs_resync = true;
        match kind {
            DownKind::Invalidated => {
                state.current = None;
                state.stale.push(connection);
                state.counts.invalidated += 1;
            }
            DownKind::Interrupted => state.counts.interrupted += 1,
        }
        if state.expecting_exit {
            state.expecting_exit = false;
        } else {
            state.record_crash(now);
        }
        self.exited.notify_all();
    }

    // ------------------------------------------------------------ 診断（smoke、メニュー、テストが使う）

    /// 連続して落ちたため、エンジンを呼ばずに休んでいるか
    pub fn is_suspended(&self) -> bool {
        let now = self.clock.now();
        lock(&self.state)
            .suspended_until
            .is_some_and(|until| now < until)
    }

    /// 直近の `Hello` の返事に入っていたエンジンの版
    pub fn last_engine_version(&self) -> Option<String> {
        lock(&self.state).observed_engine_version.clone()
    }

    // ------------------------------------------------------------ 内部

    /// 接続を用意して（要るなら、つなぎ直しと再同期をして）、接続を返す。`state` を持ったままトランスポートを呼ばない
    fn prepare(&self) -> Result<TransportHandle, EngineError> {
        let _preparing = lock(&self.prepare_lock);
        let now = self.clock.now();
        let (current, stale) = {
            let mut state = lock(&self.state);
            state.admit(now)?;
            (state.current, std::mem::take(&mut state.stale))
        };
        for handle in stale {
            self.transport.close(handle);
        }
        let handle = match current {
            Some(handle) => handle,
            None => {
                let handle = self.transport.connect().map_err(transport_error)?;
                let mut state = lock(&self.state);
                if state.connected_once {
                    state.counts.reconnect += 1;
                }
                state.connected_once = true;
                state.current = Some(handle);
                handle
            }
        };
        let resync: Vec<Vec<u8>> = {
            let mut state = lock(&self.state);
            let mut outgoing = Vec::new();
            if state.needs_resync {
                state.needs_resync = false;
                state.counts.resync += 1;
                // 落ちたあとに起動し直したエンジンには、セッションの記憶がない。返事は待たずに送り直す
                if let Some(version) = state.shell_version.clone() {
                    let id = state.take_request_id();
                    outgoing.push(codec::encode(
                        id,
                        Body::Hello(v1::Hello {
                            shell_version: version,
                        }),
                    ));
                }
                let sessions: Vec<v1::BeginSession> = state.sessions.values().cloned().collect();
                for session in sessions {
                    let id = state.take_request_id();
                    outgoing.push(codec::encode(id, Body::BeginSession(session)));
                }
            }
            outgoing
        };
        for bytes in resync {
            self.transport.post(handle, &bytes);
        }
        Ok(handle)
    }

    /// 返事を待たずに送る。つながらなければ捨てる（入力を止めない）。送ったら真
    fn post(&self, body: Body) -> bool {
        let Ok(handle) = self.prepare() else {
            return false;
        };
        let id = lock(&self.state).take_request_id();
        self.transport.post(handle, &codec::encode(id, body));
        true
    }

    /// 前回から事象があれば `sh_engine` を 1 行送る（ADR 0034 §4.2）。送れなかったら数は持ち越す
    fn post_engine_counts_if_any(&self, session_id: u64) {
        let counts = lock(&self.state).counts;
        if counts.is_empty() {
            return;
        }
        if self.post(Body::Diag(counts.event(session_id))) {
            lock(&self.state).counts.subtract(&counts);
        }
    }

    fn round_trip(&self, timeout: Duration, body: &Body) -> Result<v1::Envelope, EngineError> {
        let envelope = self.round_trip_once(timeout, body)?;
        if codec::is_error(&envelope, v1::ErrorCode::UnknownSession) {
            // 後から開いたエンジン（ロックが空くのを待っていた、など）はセッションを知らない。
            // hello とセッションを送り直して、同じ要求を 1 回だけ送り直す
            lock(&self.state).needs_resync = true;
            return self.round_trip_once(timeout, body);
        }
        Ok(envelope)
    }

    fn round_trip_once(&self, timeout: Duration, body: &Body) -> Result<v1::Envelope, EngineError> {
        let result = self.exchange(timeout, body);
        if result == Err(EngineError::Timeout) {
            lock(&self.state).counts.timeout += 1;
        }
        let envelope = result?;
        if codec::is_error(&envelope, v1::ErrorCode::EngineNotReady) {
            // エンジンがまだ開けていない（ロックを別のプロセスが持っている、など）。あとで開いたエンジンには
            // hello もセッションの記憶もないので、次の要求の前に送り直す
            lock(&self.state).needs_resync = true;
        }
        Ok(envelope)
    }

    fn exchange(&self, timeout: Duration, body: &Body) -> Result<v1::Envelope, EngineError> {
        let handle = self.prepare()?;
        let request_id = lock(&self.state).take_request_id();
        let bytes = codec::encode(request_id, body.clone());
        let reply = self
            .transport
            .request(handle, &bytes, timeout)
            .map_err(transport_error)?;
        codec::decode(&reply, request_id)
    }

    /// 版の不一致のとき: Shutdown を送り、プロセスが終わる（接続の中断の通知）まで待つ。
    fn shutdown_and_wait_for_exit(&self) {
        let before = {
            let mut state = lock(&self.state);
            state.expecting_exit = true;
            state.down_events
        };
        let body = Body::Shutdown(v1::Shutdown {
            reason: v1::ShutdownReason::VersionMismatch as i32,
        });
        let acknowledged = self.round_trip(SHUTDOWN_WAIT, &body).is_ok();
        let mut state = lock(&self.state);
        if acknowledged {
            let deadline = Instant::now() + SHUTDOWN_WAIT;
            while state.down_events == before {
                let Some(wait) = deadline.checked_duration_since(Instant::now()) else {
                    break;
                };
                if wait.is_zero() {
                    break;
                }
                state = self
                    .exited
                    .wait_timeout(state, wait)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
        }
        state.expecting_exit = false;
    }
}

impl Drop for EngineLink {
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        let handles: Vec<TransportHandle> = state
            .current
            .take()
            .into_iter()
            .chain(state.stale.drain(..))
            .collect();
        for handle in handles {
            self.transport.close(handle);
        }
    }
}
