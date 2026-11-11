//! `pipe::Client` を `shell-core` の `EngineTransport` に適合させる（ADR 0040 §2.2。純粋。Mac でテストする）。
//!
//! - 接続の開き方（名前付きパイプ）は `Connector` で受ける。Windows の実装は COM の層（`com::connector`）が持ち、
//!   テストは `pipe::mem` の流れを渡す。
//! - 落ちた通知: 接続ごとに読む側の流れを包み、`pipe::Client` の読みのスレッドが終わって流れを捨てたとき
//!   （EOF、読みの誤り、壊れた枠のどれでも）に、`EngineLink::notify_down` を `Invalidated` で呼ぶ。
//!   `close` で閉じた接続は知らせない（自分で閉じたものは落ちたのではない）。
//! - エンジンの起動（常駐の `engine-host` がないとき）は W5。つながらなければ `TransportError::Unavailable` を返すだけ。
//!
//! 判断は持たない（再接続と再同期の判断は `EngineLink`）。

use std::collections::HashMap;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::Duration;

use ikari_pipe::duplex::Halves;
use ikari_pipe::{Client, ClientError, Duplex};
use ikari_shell_core::{DownKind, EngineLink, EngineTransport, TransportError, TransportHandle};

/// 接続の番号の最初の値（0 は「つながらない」と紛れるので使わない。`shell-ffi` の `NO_CONNECTION` と同じ考え）
const FIRST_HANDLE: u64 = 1;

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
    /// 接続の表。錠は表を引く間だけ持つ（`request` の待ちの間は持たない）
    connections: Mutex<HashMap<TransportHandle, Arc<Connection>>>,
}

impl<C: Connector> PipeTransport<C> {
    pub fn new(connector: C, sink: Arc<DownSink>) -> Self {
        Self {
            connector,
            sink,
            next_handle: AtomicU64::new(FIRST_HANDLE),
            connections: Mutex::new(HashMap::new()),
        }
    }

    fn table(&self) -> MutexGuard<'_, HashMap<TransportHandle, Arc<Connection>>> {
        self.connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn get(&self, handle: TransportHandle) -> Option<Arc<Connection>> {
        self.table().get(&handle).cloned()
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
        let (duplex, server_pid) = self.connector.open().ok_or(TransportError::Unavailable)?;
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
        connection
            .client
            .request(bytes, timeout)
            .map_err(transport_error)
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
