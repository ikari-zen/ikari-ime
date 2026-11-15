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

use ikari_engine::{Engine, error_envelope};
use ikari_pipe::{Listener, Server};
use ikari_proto::v1::{Envelope, ErrorCode, envelope};
use prost::Message;

/// エンジンのパイプで断る理由（固定の文。要求の中身は入れない）
const ADMIN_REFUSED_MESSAGE: &str = "admin requests are not accepted on the engine pipe";

/// 管理の要求（ADR 0005 §8。ユーザー辞書、学習の状態・書き出し・取り込み・消去、プロファイル）か。
/// これらは設定のパイプ（`settings_pipe`。中継の許可リストを通る）だけが受ける。TIP が話すエンジンのパイプでは受けない
/// （どのアプリの中の TIP からでも学習が読み書きできてしまわないように）。
/// `match` はワイルドカードを持たない: 要求の種類が増えたら、ここでコンパイルが通らなくなり、どちらのパイプで受けるかを決めさせる。
fn is_admin_request(body: &envelope::Body) -> bool {
    use envelope::Body as B;
    match body {
        B::ListUserDict(_)
        | B::AddUserDict(_)
        | B::RemoveUserDict(_)
        | B::LearningStatus(_)
        | B::ResetLearning(_)
        | B::ExportData(_)
        | B::ImportData(_)
        | B::GetProfile(_)
        | B::SetProfile(_) => true,
        B::Hello(_)
        | B::HelloReply(_)
        | B::BeginSession(_)
        | B::Ack(_)
        | B::EndSession(_)
        | B::Prefetch(_)
        | B::Convert(_)
        | B::ConvertReply(_)
        | B::Page(_)
        | B::PageReply(_)
        | B::Commit(_)
        | B::CommitText(_)
        | B::Cancel(_)
        | B::Shutdown(_)
        | B::Error(_)
        | B::SegmentPage(_)
        | B::SegmentPageReply(_)
        | B::ResizeSegment(_)
        | B::RemoveSegments(_)
        | B::SegmentLayout(_)
        | B::CommitSegments(_)
        | B::Reconvert(_)
        | B::UserDictListReply(_)
        | B::LearningStatusReply(_)
        | B::ExportReply(_)
        | B::ImportReply(_)
        | B::ProfileReply(_)
        | B::EmojiTable(_)
        | B::EmojiTableReply(_)
        | B::Diag(_)
        | B::PostalLookup(_)
        | B::PostalLookupReply(_) => false,
    }
}

/// 管理の要求なら、エンジンに渡さず `BAD_REQUEST` の `Envelope` を返す。そうでなければ `None`
fn refuse_admin(request: &[u8]) -> Option<Vec<u8>> {
    // 読めない本体はエンジンに渡す（エンジンが `BAD_REQUEST` を返す）
    let envelope = Envelope::decode(request).ok()?;
    if envelope.body.as_ref().is_some_and(is_admin_request) {
        return Some(error_envelope(
            envelope.request_id,
            ErrorCode::BadRequest,
            ADMIN_REFUSED_MESSAGE,
        ));
    }
    None
}

/// 受け口が閉じられるまで受け付け、受けた要求を、接続ごとに `engine.connect()` で取った `Connection` に渡す。
/// ただし管理の要求（`is_admin_request`）は渡さず、`BAD_REQUEST` を返す（設定のパイプだけが受ける）。
///
/// 戻る前に、残っている接続を閉じてスレッドを待つ（どの `Connection` も `Engine` より先に落ちる。
/// 接続のスレッドは `Arc<Engine>` を持たないので、呼び出し側が最後の `Arc` を落とせば、`Engine` の
/// `Drop`（学習の保存）が走る）。
pub fn serve<L: Listener>(listener: L, engine: Arc<Engine>) -> io::Result<()> {
    Server::new(listener).serve(move || {
        let connection = engine.connect();
        move |request: &[u8]| {
            Some(refuse_admin(request).unwrap_or_else(|| connection.handle(request)))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikari_proto::SCHEMA_VERSION;
    use ikari_proto::v1 as pb;

    fn request(request_id: u64, body: envelope::Body) -> Vec<u8> {
        Envelope {
            schema_version: SCHEMA_VERSION,
            request_id,
            body: Some(body),
        }
        .encode_to_vec()
    }

    #[test]
    fn admin_requests_are_refused_with_the_request_id_echoed() {
        let bodies = [
            envelope::Body::ListUserDict(pb::UserDictList::default()),
            envelope::Body::LearningStatus(pb::LearningStatus::default()),
            envelope::Body::ResetLearning(pb::LearningReset::default()),
            envelope::Body::ExportData(pb::ExportData::default()),
            envelope::Body::ImportData(pb::ImportData::default()),
            envelope::Body::GetProfile(pb::ProfileGet::default()),
        ];
        for body in bodies {
            let reply = refuse_admin(&request(7, body)).expect("refused");
            let reply = Envelope::decode(reply.as_slice()).unwrap();
            assert_eq!(reply.request_id, 7);
            let Some(envelope::Body::Error(e)) = reply.body else {
                panic!("expected an error: {reply:?}");
            };
            assert_eq!(e.code, ErrorCode::BadRequest as i32);
        }
    }

    #[test]
    fn typing_requests_and_garbage_are_passed_on() {
        let hello = request(1, envelope::Body::Hello(pb::Hello::default()));
        assert!(refuse_admin(&hello).is_none());
        assert!(refuse_admin(&[0xff; 16]).is_none());
    }
}
