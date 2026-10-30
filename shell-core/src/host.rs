//! ホストの口（ADR 0041 §4.1・§4.3・§4.5）。**契約**。
//!
//! OS 側（macOS の Swift の薄い入口、Windows の TIP）が実装する。`shell-core` は要るときだけ同期に呼ぶ。
//! macOS では `shell-ffi` が C の関数ポインタ（`IkariShellHost`。§4.4）をこの trait に包む。

use std::time::Duration;

use crate::types::{SelectedText, SurroundingText};

/// 周辺テキストと選択範囲（`Shell::dispatch` の間だけ借りる）。
///
/// 呼ぶのは要るときだけ（今の `@autoclosure` と同じ遅延）。コールバックの中から `Shell` を呼び直さない。
pub trait TextAccess {
    /// 周辺テキストの生の値。取れなければ `None`。
    fn surrounding(&mut self) -> Option<SurroundingText>;
    /// 選択範囲とその文字列。選択がない、取れない、取れた範囲が選択と違うときは `None`。
    fn selection(&mut self) -> Option<SelectedText>;
}

/// エンジンへの接続 1 本の番号。ホストが `connect` で付ける。接続ごとに違う値にする（再接続の世代を兼ねる）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TransportHandle(pub u64);

/// トランスポートの誤り（メッセージの意味を知らない層の誤り）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportError {
    /// つながらない、つながりが切れた
    Unavailable,
    /// 待ちの上限を超えた
    Timeout,
}

/// 接続が落ちた通知の種類（XPC の `interruptionHandler`・`invalidationHandler`。名前付きパイプは切断）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DownKind {
    Interrupted,
    Invalidated,
}

/// エンジンとのバイト列の運び手。**メッセージの意味を知らない**（`Envelope` の組み立てと読み取りは `EngineLink`）。
///
/// macOS は `NSXPCConnection`、Windows は名前付きパイプ。落ちた通知は、ホストが別のスレッドから
/// `EngineLink::notify_down` を呼んで知らせる（境界の唯一の例外。ADR 0041 §4.3）。
pub trait EngineTransport: Send {
    /// 新しい接続を作る（macOS は署名の要件を付けて `resume`）。
    fn connect(&mut self) -> Result<TransportHandle, TransportError>;
    /// 要求を送り、返事のバイト列を同期に待つ。
    fn request(
        &mut self,
        handle: TransportHandle,
        bytes: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, TransportError>;
    /// 返事を待たずに送る。
    fn post(&mut self, handle: TransportHandle, bytes: &[u8]);
    /// 接続を閉じる。
    fn close(&mut self, handle: TransportHandle);
    /// 相手のプロセスの番号。分からなければ `None`。
    fn pid(&self, handle: TransportHandle) -> Option<i32>;
}

/// 単調に進む時計。本番は `std::time::Instant`、テストは差し替える（ADR 0041 §4.2）。
pub trait Clock: Send + Sync {
    /// 時計の起点からの経過時間（単調増加）。
    fn now(&self) -> Duration;
}

/// `std::time::Instant` による時計。
#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    origin: std::time::Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self {
            origin: std::time::Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}
