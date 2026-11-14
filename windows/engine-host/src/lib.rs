//! ikari-engine-host: Windows の常駐のエンジン（ADR 0040 §2.2、§7 の W1-c）。
//!
//! 名前付きパイプで受けた枠の本体（`Envelope` のバイト列）を `Engine::handle` に渡し、返事を同じ通し番号で返す。
//! 接続の受け付けは `ikari_pipe::Listener` の抽象の上に書く。このファイルは OS に依存しない
//! （名前付きパイプの `Listener` は W1-b。`cfg(windows)`）。判断の置き場所: Rust。
//!
//! - 接続ごとにスレッドを持つ。`Engine` は 1 つで、すべての接続が共有する（`Engine::handle` は複数のスレッドから
//!   同時に呼んでよい）。
//! - 1 本の接続の要求は届いた順に処理する。別の接続（別のアプリの TIP）とは並行に動く。
//! - `Engine::handle` は決して panic せず、エラーも `Envelope` で返すので、ハンドラは常に返事を返す。

#![deny(unsafe_code)]

#[cfg(windows)]
pub mod ctrl;
#[cfg(windows)]
pub mod endsession;
pub mod run;
pub mod settings_pipe;

use std::io;
use std::sync::Arc;

use ikari_engine::Engine;
use ikari_pipe::{Listener, Server};

/// 受け口が閉じられるまで受け付け、受けた要求を `engine` に渡す。
///
/// 戻る前に、残っている接続を閉じてスレッドを待つ（`Arc<Engine>` を持つスレッドがなくなってから戻るので、
/// 呼び出し側が最後の `Arc` を落とせば、`Engine` の `Drop`（学習の保存）が走る）。
pub fn serve<L: Listener>(listener: L, engine: Arc<Engine>) -> io::Result<()> {
    Server::new(listener).serve(move || {
        let engine = Arc::clone(&engine);
        move |request: &[u8]| Some(engine.handle(request))
    })
}
