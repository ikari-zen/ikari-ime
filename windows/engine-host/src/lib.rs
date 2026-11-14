//! ikari-engine-host: Windows の常駐のエンジン（ADR 0040 §2.2、§7 の W1-c）。
//!
//! 名前付きパイプで受けた枠の本体（`Envelope` のバイト列）を、その接続の `Connection::handle` に渡し、
//! 返事を同じ通し番号で返す。
//! 接続の受け付けは `ikari_pipe::Listener` の抽象の上に書く。このファイルは OS に依存しない
//! （名前付きパイプの `Listener` は W1-b。`cfg(windows)`）。判断の置き場所: Rust。
//!
//! - 接続ごとにスレッドを持つ。`Engine` は 1 つで、すべての接続が共有する。接続ごとに `Engine::connect` で
//!   `Connection` を 1 つ取り、セッションはその接続の名前空間で区別する（ADR 0064 D1。TIP の `Shell` は
//!   どれも番号を 1 から振るので、共有の `Engine::handle` に渡すと別のアプリのセッションが混ざる）。
//! - 接続が切れると `Connection` が落ち、その接続のセッションと状態をエンジンが捨てる（ADR 0064 D2）。
//! - 1 本の接続の要求は届いた順に処理する。別の接続（別のアプリの TIP）とは並行に動く。
//! - `Connection::handle` は決して panic せず、エラーも `Envelope` で返すので、ハンドラは常に返事を返す。

#![deny(unsafe_code)]

#[cfg(windows)]
pub mod ctrl;
#[cfg(windows)]
pub mod endsession;
pub mod run;
pub mod session_end;
pub mod settings_pipe;

use std::io;
use std::sync::Arc;

use ikari_engine::Engine;
use ikari_pipe::{Listener, Server};

/// 受け口が閉じられるまで受け付け、受けた要求を、接続ごとに `engine.connect()` で取った `Connection` に渡す。
///
/// 戻る前に、残っている接続を閉じてスレッドを待つ（どの `Connection` も `Engine` より先に落ちる。
/// 接続のスレッドは `Arc<Engine>` を持たないので、呼び出し側が最後の `Arc` を落とせば、`Engine` の
/// `Drop`（学習の保存）が走る）。
pub fn serve<L: Listener>(listener: L, engine: Arc<Engine>) -> io::Result<()> {
    Server::new(listener).serve(move || {
        let connection = engine.connect();
        move |request: &[u8]| Some(connection.handle(request))
    })
}
