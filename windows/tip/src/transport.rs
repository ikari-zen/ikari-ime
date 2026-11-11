//! `pipe::Client` を `shell-core` の `EngineTransport` に適合させる（ADR 0040 §2.2。純粋。Mac でテストする）。
//!
//! - 接続の開き方（名前付きパイプ）は `Connector` で受ける。Windows の実装は COM の層（`com::connector`）が持ち、
//!   テストは `pipe::mem` の流れを渡す。
//! - 落ちた通知: 接続ごとに読む側の流れを包み、`pipe::Client` の読みのスレッドが終わって流れを捨てたとき
//!   （EOF、読みの誤り、壊れた枠のどれでも）に、`EngineLink::notify_down` を `Invalidated` で呼ぶ。
//!   `close` で閉じた接続は知らせない（自分で閉じたものは落ちたのではない）。
//! - 再接続: 判断（いつつなぎ直すか、再同期の `Hello`、休止）は `EngineLink`（`prepare` が、要求のたびに、つながっていなければ
//!   `connect` を呼ぶ）。ここは、`connect` のたびに新しい接続（世代）を作ることと、つながらない間にアプリのキー入力を
//!   塞がないこと（失敗のあとの試行の間隔。`Backoff`）、`EngineLink` が落ちた通知を取りこぼしても詰まらないこと
//!   （切れた接続に要求が来たら、もう一度知らせる）だけを持つ。
//! - エンジンの起動（常駐の `engine-host` がないとき）は W5。つながらなければ `TransportError::Unavailable` を返すだけ。
//!
//! 判断は持たない（再接続と再同期の判断は `EngineLink`）。

use std::collections::HashMap;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

use crate::latency::{Kind, Meter};
use ikari_pipe::duplex::Halves;
use ikari_pipe::{Client, ClientError, Duplex};
use ikari_shell_core::{DownKind, EngineLink, EngineTransport, TransportError, TransportHandle};

/// 接続の番号の最初の値（0 は「つながらない」と紛れるので使わない。`shell-ffi` の `NO_CONNECTION` と同じ考え）
const FIRST_HANDLE: u64 = 1;

/// つながらなかったあと、最初に次の試行を控える長さ。パイプが「ふさがっている」と、1 回の試行が最大 約 0.3 秒
/// アプリのキー入力を塞ぐ（`connect_duplex`）ので、その繰り返しを避ける。**暫定**（計測した値ではない。W2 の実機で見直す）。
pub const BACKOFF_INITIAL: Duration = Duration::from_millis(100);
/// 控える長さの上限。エンジンが戻ってから、次のキーで変換できるまでの遅れの最大（これより長くは待たせない）。
/// 1 秒は「キーを打ち直す間に戻る」程度の目安で、**暫定**（計測した値ではない）。
pub const BACKOFF_MAX: Duration = Duration::from_secs(1);
/// 控える長さを伸ばす倍率
const BACKOFF_FACTOR: u32 = 2;

/// つながらなかったあとの試行の間隔（指数的に伸ばし、上限で止める。つながったら戻す）。
struct Backoff {
    initial: Duration,
    max: Duration,
    /// 次に試してよい時刻（`None` は、いつでも）
    not_before: Option<Instant>,
    /// 次に失敗したときに控える長さ
    next_delay: Duration,
}

impl Backoff {
    fn new(initial: Duration, max: Duration) -> Self {
        Self {
            initial,
            max,
            not_before: None,
            next_delay: initial,
        }
    }

    /// 試してよいか
    fn ready(&self, now: Instant) -> bool {
        self.not_before.is_none_or(|at| now >= at)
    }

    fn failed(&mut self, now: Instant) {
        self.not_before = Some(now + self.next_delay);
        self.next_delay = self.next_delay.saturating_mul(BACKOFF_FACTOR).min(self.max);
    }

    fn succeeded(&mut self) {
        self.not_before = None;
        self.next_delay = self.initial;
    }
}

/// 流れを開く手段（名前付きパイプへの接続。テストはメモリ上の流れ）。
pub trait Connector: Send + Sync + 'static {
    type Conn: Duplex;

    /// 新しい流れを開く。つながらなければ `None`。相手のプロセスの番号が分かれば一緒に返す。
    fn open(&self) -> Option<(Self::Conn, Option<u32>)>;
}

/// 落ちた通知を受ける側（本番は `EngineLink`。テストは記録する型）。
pub trait DownTarget: Send + Sync {
    fn down(&self, handle: TransportHandle, kind: DownKind);
}

impl DownTarget for EngineLink {
    fn down(&self, handle: TransportHandle, kind: DownKind) {
        self.notify_down(handle, kind);
    }
}

/// 落ちた通知の送り先。`Shell::new` に渡したあとで `EngineLink` が決まるので、あとから結ぶ。
///
/// `EngineLink` はトランスポートを持つので、ここは弱い参照にする（循環させない）。結ぶ前の通知は捨てる。
#[derive(Default)]
pub struct DownSink(OnceLock<Weak<dyn DownTarget>>);

impl DownSink {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 送り先を結ぶ（1 回だけ。2 回目は無視する）。
    pub fn bind<T: DownTarget + 'static>(&self, target: &Arc<T>) {
        let weak: Weak<T> = Arc::downgrade(target);
        let _ = self.0.set(weak);
    }

    fn notify(&self, handle: TransportHandle) {
        if let Some(target) = self.0.get().and_then(Weak::upgrade) {
            // 名前付きパイプが切れたら、その接続はもう使えない（つなぎ直す）
            target.down(handle, DownKind::Invalidated);
        }
    }
}

/// 接続 1 本の「落ちたら知らせる」印。
struct Watch {
    handle: TransportHandle,
    sink: Arc<DownSink>,
    /// `close` で閉じた（知らせない）
    closing: AtomicBool,
    /// 知らせた（2 回知らせない）
    fired: AtomicBool,
}

impl Watch {
    /// 落ちた通知を送ったか（切れている）
    fn is_down(&self) -> bool {
        self.fired.load(Ordering::Acquire)
    }

    /// 切れた接続に要求が来た。`EngineLink` がまだこの接続を「今の接続」だと思っている印なので、もう一度知らせる。
    ///
    /// `connect` が返す前（または `EngineLink` が接続を記録する前）に切れると、最初の通知は古い世代として捨てられ、
    /// つなぎ直されない。`EngineLink` は今の接続でなければ通知を捨てるので、重ねて知らせても害はない。
    fn renotify(&self) {
        if !self.closing.load(Ordering::Acquire) {
            self.sink.notify(self.handle);
        }
    }

    fn fire(&self) {
        if self.closing.load(Ordering::Acquire) || self.fired.swap(true, Ordering::AcqRel) {
            return;
        }
        self.sink.notify(self.handle);
    }
}

/// 読む側の流れの包み。捨てられたとき（`pipe::Client` の読みのスレッドが終わったとき）に知らせる。
struct WatchedReader {
    inner: Box<dyn Read + Send>,
    watch: Arc<Watch>,
}

impl Read for WatchedReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Drop for WatchedReader {
    fn drop(&mut self) {
        self.watch.fire();
    }
}

/// 読む側を包んだ流れ。
struct WatchedDuplex<D> {
    inner: D,
    watch: Arc<Watch>,
}

impl<D: Duplex> Duplex for WatchedDuplex<D> {
    fn split(self) -> Halves {
        let (reader, writer, shutdown) = self.inner.split();
        let reader: Box<dyn Read + Send> = Box::new(WatchedReader {
            inner: reader,
            watch: self.watch,
        });
        (reader, writer, shutdown)
    }
}

struct Connection {
    client: Client,
    watch: Arc<Watch>,
}

/// `EngineTransport` の実装。すべて `&self` で、複数のスレッドから同時に呼んでよい。
pub struct PipeTransport<C: Connector> {
    connector: C,
    sink: Arc<DownSink>,
    next_handle: AtomicU64,
    /// つながらなかったあとの試行の間隔
    backoff: Mutex<Backoff>,
    /// 接続の表。錠は表を引く間だけ持つ（`request` の待ちの間は持たない）
    connections: Mutex<HashMap<TransportHandle, Arc<Connection>>>,
    /// 往復時間の測定（S1。オフなら何も測らない。`latency`）
    meter: Arc<Meter>,
}

impl<C: Connector> PipeTransport<C> {
    pub fn new(connector: C, sink: Arc<DownSink>) -> Self {
        Self::with_backoff(connector, sink, BACKOFF_INITIAL, BACKOFF_MAX)
    }

    /// 試行の間隔を決めて作る（テストが短くする）。
    pub fn with_backoff(
        connector: C,
        sink: Arc<DownSink>,
        backoff_initial: Duration,
        backoff_max: Duration,
    ) -> Self {
        Self {
            connector,
            sink,
            next_handle: AtomicU64::new(FIRST_HANDLE),
            backoff: Mutex::new(Backoff::new(backoff_initial, backoff_max)),
            connections: Mutex::new(HashMap::new()),
            meter: Arc::new(Meter::disabled()),
        }
    }

    /// 往復時間を測る（S1）。
    #[must_use]
    pub fn with_meter(mut self, meter: Arc<Meter>) -> Self {
        self.meter = meter;
        self
    }

    fn table(&self) -> MutexGuard<'_, HashMap<TransportHandle, Arc<Connection>>> {
        self.connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn backoff(&self) -> MutexGuard<'_, Backoff> {
        self.backoff.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 接続を引く。切れていれば、落ちた通知をもう一度送ってから返す（`Watch::renotify`）
    fn get(&self, handle: TransportHandle) -> Option<Arc<Connection>> {
        let connection = self.table().get(&handle).cloned()?;
        if connection.watch.is_down() {
            connection.watch.renotify();
        }
        Some(connection)
    }
}

fn transport_error(error: ClientError) -> TransportError {
    match error {
        ClientError::Timeout => TransportError::Timeout,
        // 切れた、上限超過（何も送っていない）、エンジンが返事を作れなかった。どれも「運べなかった」
        ClientError::Disconnected | ClientError::TooLarge | ClientError::Rejected => {
            TransportError::Unavailable
        }
    }
}

impl<C: Connector> EngineTransport for PipeTransport<C> {
    fn connect(&self) -> Result<TransportHandle, TransportError> {
        // TODO(W5): つながらないとき、デスクトップのアプリからに限ってエンジンを起動する（ADR 0040 §2.2、S3）
        // 失敗のすぐあとは、OS に聞かずに断る（ふさがったパイプで、キーのたびに最大 約 0.3 秒ブロックしない）
        if !self.backoff().ready(Instant::now()) {
            return Err(TransportError::Unavailable);
        }
        self.meter.connect_attempt();
        let Some((duplex, server_pid)) = self.connector.open() else {
            self.backoff().failed(Instant::now());
            return Err(TransportError::Unavailable);
        };
        self.backoff().succeeded();
        self.meter.connect_succeeded();
        let handle = TransportHandle(self.next_handle.fetch_add(1, Ordering::Relaxed));
        let watch = Arc::new(Watch {
            handle,
            sink: Arc::clone(&self.sink),
            closing: AtomicBool::new(false),
            fired: AtomicBool::new(false),
        });
        let client = Client::new(
            WatchedDuplex {
                inner: duplex,
                watch: Arc::clone(&watch),
            },
            server_pid,
        );
        self.table()
            .insert(handle, Arc::new(Connection { client, watch }));
        Ok(handle)
    }

    fn request(
        &self,
        handle: TransportHandle,
        bytes: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, TransportError> {
        let connection = self.get(handle).ok_or(TransportError::Unavailable)?;
        // 測るのは、送ってから返事を受けるまで（接続の引きは含めない）
        let stamp = self.meter.start();
        let result = connection
            .client
            .request(bytes, timeout)
            .map_err(transport_error);
        self.meter.finish(Kind::Request, stamp, result.is_ok());
        result
    }

    fn post(&self, handle: TransportHandle, bytes: &[u8]) {
        // 返事を待たない。送れなかったことは、読みのスレッドの終わり（落ちた通知）で分かる
        if let Some(connection) = self.get(handle) {
            let _ = connection.client.post(bytes);
        }
    }

    fn close(&self, handle: TransportHandle) {
        let removed = self.table().remove(&handle);
        if let Some(connection) = removed {
            connection.watch.closing.store(true, Ordering::Release);
            connection.client.close();
            // 最後の参照が落ちたところで `Client` の `Drop` が読みのスレッドを待つ。待つのは錠の外
            drop(connection);
        }
    }

    fn pid(&self, handle: TransportHandle) -> Option<i32> {
        let pid = self.get(handle)?.client.server_pid()?;
        i32::try_from(pid).ok().filter(|pid| *pid > 0)
    }
}

#[cfg(test)]
mod tests;
