//! 接続の口（ADR 0064 §4 の決定 1、案 K1）。
//!
//! エンジンの中のセッションのキーを（接続, `session_id`）にするための型。接続の番号は
//! エンジンが振り、殻は知らない（proto は変えない）。
//!
//! **契約だけの段階（ADR 0064 §8 の 1）**: ここにあるのは型とシグネチャだけで、
//! `Connection::handle` はまだ接続の番号を使わず、`Engine::handle` と同じに動く。
//! キーの置き換えと、`Drop` での片づけ（D2）はレーン A（ADR 0064 §8 の 2）が入れる。

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
/// 落とすと、その接続のセッションと状態を捨てる（D2。レーン A が `Drop` に入れる。今は何もしない）。
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
    /// 接続の番号をキーに使うのはレーン A（今は `Engine::handle` と同じ動き）。
    pub fn handle(&self, req: &[u8]) -> Vec<u8> {
        self.inner.handle_frame(self.id, req)
    }
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
