//! 流れの抽象。名前付きパイプの OS 層（W1-b）とメモリ上のテスト用（`mem`）が実装する。

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::Duration;

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

/// 書きの締め切り（暫定。未計測の仮置き。ユーザーに確認する）。
///
/// 流れの書き（`Write::write`）は、相手が読まずにバッファが埋まると待つ。TIP はアプリのキーの通知の中で
/// 書くので、相手（エンジン、候補窓）が固まると、フォーカスのあるアプリの入力が止まる。そこで、`Duplex` の
/// 書く側は、1 回の `write` がこの長さを超えて進まなければ `ErrorKind::TimedOut` で戻る。`Client` と
/// `Server` は、書きの誤りを「流れが壊れた」として接続を切り、送れなかったものは捨てる（再送しない。
/// 切れたあとは、使う側の再接続の道に乗る）。
///
/// 200ms の根拠: キー 1 回の処理（TIP から見て、変換の要求と返事の往復）に使える時間より短く、人が
/// 「止まった」と感じる前（数百 ms）に諦められる大きさ。正常なら、数 KiB の書きはバッファ（64KiB）に
/// 入って即座に終わるので、この値に達するのは相手が読んでいないときだけ（推測。計測で決めた値ではない）。
pub const WRITE_DEADLINE: Duration = Duration::from_millis(200);

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
