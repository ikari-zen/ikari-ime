//! 流れの抽象。名前付きパイプの OS 層（W1-b）とメモリ上のテスト用（`mem`）が実装する。

use std::io::{self, Read, Write};
use std::sync::Arc;

/// 流れを閉じる（`Duplex` を分けたあとも、別のスレッドから呼べる）。
///
/// 閉じたら、ブロック中の読みを EOF かエラーで戻し、以降の書きをエラーにする。何度呼んでもよい。
pub trait Shutdown: Send + Sync {
    fn shutdown(&self);
}

/// 分けた流れの読む側、書く側、閉じる手段。
pub type Halves = (
    Box<dyn Read + Send>,
    Box<dyn Write + Send>,
    Arc<dyn Shutdown>,
);

/// 読み書きの双方向の流れ 1 本（接続 1 本）。
///
/// 読みと書きは別のスレッドで同時に行うので、半分ずつに分ける。名前付きパイプの同期の I/O は、
/// 同じハンドルへの読みと書きを直列にしてしまうので、W1-b は重ね合わせ（overlapped）の I/O で実装する。
pub trait Duplex: Send + 'static {
    /// 読む側、書く側、閉じる手段に分ける。
    fn split(self) -> Halves
    where
        Self: Sized;
}

/// 接続の受け口（名前付きパイプでは、インスタンスを作って `ConnectNamedPipe` で待つ層）。
pub trait Listener: Send {
    type Conn: Duplex;

    /// 次の接続を待つ。`Ok(None)` は、受け口が閉じられた（受け付けを終える）こと。
    /// `Err` は 1 回の失敗（`Server` が数回までやり直す）。
    fn accept(&mut self) -> io::Result<Option<Self::Conn>>;
}
