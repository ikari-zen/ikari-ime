//! TSF のテキストサービスの COM の層（ADR 0040 §2.2、§2.8。W2-b。`cfg(windows)`）。
//!
//! **判断を持たない**。OS の API でしか取れないもの（キーと OS が返した文字、周辺テキスト、文字の位置）を
//! `shell-core` に渡し、返った動作を `ops` で `TsfOp` にして、順に実行するだけ。
//!
//! - `dll`: DLL の入口（`DllGetClassObject`、`DllCanUnloadNow`、`DllRegisterServer`、`DllUnregisterServer`）
//! - `factory`: `IClassFactory`
//! - `register`: レジストリ（CLSID）、TSF のプロファイルとカテゴリの登録と解除
//! - `service`: テキストサービス本体（`ITfTextInputProcessorEx`、`ITfKeyEventSink`、`ITfCompositionSink`、
//!   `ITfThreadMgrEventSink`）
//! - `edit`: 編集セッション（`ITfEditSession`）と、`TsfOp` の実行、`TextAccess` の実装
//! - `field`: 今の入力欄が secure（パスワード、PIN）かを、入力スコープから読む（判定の表は `secure`）
//! - `keyboard`: キーの押下 → `keys::RawKey`（`ToUnicodeEx` で配列によらない文字を取る）
//! - `timer`: `TsfOp::ScheduleTimer` の実行（スレッドのタイマー）
//! - `host`: `Shell` を作る（設定と資源の場所、トランスポート）
//! - `connector`: 名前付きパイプへの接続（`transport::Connector` の実装）
//! - `guard`: COM の入口を `catch_unwind` で包む（panic はその回を素通しにする。ADR 0041 §4.2）
//! - `log`: `OutputDebugStringW`。**入力された文字列を出さない**（固定の文と HRESULT だけを受ける型にしている）
//! - `guids`: この TIP の GUID
//!
//! # スレッドと寿命
//! TSF のテキストサービスは、アプリの UI スレッド（STA）で作られ、すべての呼び出しがそのスレッドに来る。
//! 状態は `RefCell` に置き、ほかのスレッドに渡さない（`Shell` の中のエンジンの読みのスレッドは、
//! `EngineLink` の `Sync` な口だけを使う）。COM のオブジェクトの参照カウントは `#[implement]` が持つ。
//! DLL を外してよいか（`DllCanUnloadNow`）は、生きているオブジェクトの数と `LockServer` の数で決める。
//!
//! # `unsafe`
//! この下にだけ置く（crate の根で `deny(unsafe_code)`）。1 つ 1 つに `SAFETY:` で根拠を書く。

mod connector;
mod dll;
mod edit;
mod factory;
mod field;
mod guard;
mod guids;
mod host;
mod keyboard;
mod log;
mod register;
mod service;
mod timer;
