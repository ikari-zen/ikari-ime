//! 接続の口（ADR 0064 §4 の決定 1、案 K1）。
//!
//! エンジンの中のセッションのキーを（接続, `session_id`）にするための型。接続の番号は
//! エンジンが振り、殻は知らない（proto は変えない）。
//!
//! - キー: エンジンの中のセッションごとの状態（確定履歴、secure、先行計算、文節の先読み、`ctx_trace`、
//!   変換の表）は、すべて `SessionKey`（接続, `session_id`）で引く（D1）。変換の表の要求（`Page`、`Commit`
//!   など）は、その変換を作った接続からだけ受ける（D2）。
//! - 片づけ: `Connection` を落とすと、その接続のセッションと状態をすべて捨てる（休止にしない。D2）。
//! - `Engine` との順序: `Connection` は `Inner` の `Arc` を持つので、`Engine` より長く残れる。`Engine` を
//!   先に落としたら（裏のスレッドを止め、学習を保存し、ログを閉じたあと）、残った `Connection::handle` は
//!   要求を処理せず `ENGINE_NOT_READY` を返し、ログも書かない。そのあとの `Connection` の `Drop` は、メモリの
//!   表を片づけるだけ（保存もログもしない）。engine-host の `serve` は、接続のスレッドを待ってから戻るので、
//!   ふつうは `Connection` が先に落ちる。

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::Inner;

/// 接続の番号。エンジンが振る（殻には見えない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConnectionId(u64);

impl ConnectionId {
    /// `Engine::handle` が使う「既定の接続」。macOS は殻が 1 つなので、これだけを使う
    /// （切れないので、D2 の片づけは起きない。ADR 0064 §4.3）。`connect` はこの番号を振らない。
    pub const DEFAULT: ConnectionId = ConnectionId(0);

    /// ログに書く番号（番号だけ。文字は出さない）
    pub fn get(self) -> u64 {
        self.0
    }
}

/// エンジンの中のセッションのキー（接続, 殻が振った `session_id`）。殻の番号は接続ごとの名前空間になる
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SessionKey {
    pub(crate) conn: ConnectionId,
    /// 殻が振った番号（ログの `session=` に書くのはこちら）
    pub(crate) id: u64,
}

impl SessionKey {
    pub(crate) const fn new(conn: ConnectionId, id: u64) -> SessionKey {
        SessionKey { conn, id }
    }
}

/// 接続の番号の払い出し。`DEFAULT`（0）の次の 1 から振る。
#[derive(Debug)]
pub(crate) struct ConnectionIds(AtomicU64);

impl ConnectionIds {
    pub(crate) fn new() -> Self {
        Self(AtomicU64::new(ConnectionId::DEFAULT.0 + 1))
    }

    pub(crate) fn next(&self) -> ConnectionId {
        ConnectionId(self.0.fetch_add(1, Ordering::Relaxed))
    }
}

/// 1 本の接続（Windows の TIP の 1 つの `Shell`）の口。`Engine::connect` で取る。
///
/// 落とすと、その接続のセッションと状態を捨てる（D2）。
/// `Engine` とは独立に持てる（`Inner` の `Arc` を持つ）ので、接続ごとのスレッドへ渡してよい。
pub struct Connection {
    inner: Arc<Inner>,
    id: ConnectionId,
}

impl Connection {
    pub(crate) fn new(inner: Arc<Inner>, id: ConnectionId) -> Self {
        Self { inner, id }
    }

    /// この接続の番号
    pub fn id(&self) -> ConnectionId {
        self.id
    }

    /// `Engine::handle` と同じ契約（Envelope のバイト列を受けて返す。panic しない）。
    /// セッションは、この接続の名前空間で引く（ほかの接続の同じ `session_id` とは別のもの）。
    pub fn handle(&self, req: &[u8]) -> Vec<u8> {
        self.inner.handle_frame(self.id, req)
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.inner.drop_connection(self.id);
    }
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
