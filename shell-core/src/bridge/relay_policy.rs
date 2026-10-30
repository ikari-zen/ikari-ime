//! 殻がエンジンへ中継してよい要求の許可リスト（ADR 0005 §6.3「中継の許可リスト」。今の Swift の `RelayPolicy`）。
//! 殻は `Frame.engine`（Envelope）の oneof の種類だけを見る。中身は解釈せず、ログにも書かない。
//! 設定アプリから `Commit` などを送って学習を汚す経路を作らないための関所。

use std::time::Duration;

use ikari_proto::v1::envelope::Body as EnvelopeBody;

use super::proto::{BridgeErrorCode, Frame, frame::Body};
use super::protocol::error_frame;
use crate::timeouts;

/// 中継してよい要求か。**許可するのは `Hello` と ADR 0005 §8.2 の管理の要求だけ。**
/// `match` はワイルドカードを持たない: 要求の種類が増えたら、ここでコンパイルが通らなくなり、許可するかを決めさせる。
pub fn is_allowed(body: Option<&EnvelopeBody>) -> bool {
    let Some(body) = body else {
        return false;
    };
    match body {
        EnvelopeBody::Hello(_)
        | EnvelopeBody::ListUserDict(_)
        | EnvelopeBody::AddUserDict(_)
        | EnvelopeBody::RemoveUserDict(_)
        | EnvelopeBody::LearningStatus(_)
        | EnvelopeBody::ResetLearning(_)
        | EnvelopeBody::ExportData(_)
        | EnvelopeBody::ImportData(_)
        | EnvelopeBody::GetProfile(_)
        | EnvelopeBody::SetProfile(_) => true,
        EnvelopeBody::HelloReply(_)
        | EnvelopeBody::BeginSession(_)
        | EnvelopeBody::Ack(_)
        | EnvelopeBody::EndSession(_)
        | EnvelopeBody::Prefetch(_)
        | EnvelopeBody::Convert(_)
        | EnvelopeBody::ConvertReply(_)
        | EnvelopeBody::Page(_)
        | EnvelopeBody::PageReply(_)
        | EnvelopeBody::Commit(_)
        | EnvelopeBody::CommitText(_)
        | EnvelopeBody::Cancel(_)
        | EnvelopeBody::Shutdown(_)
        | EnvelopeBody::Error(_)
        // 文節の編集と再変換（ADR 0006）は殻の入力だけが送る。設定アプリから中継しない
        | EnvelopeBody::SegmentPage(_)
        | EnvelopeBody::SegmentPageReply(_)
        | EnvelopeBody::ResizeSegment(_)
        | EnvelopeBody::SegmentLayout(_)
        | EnvelopeBody::CommitSegments(_)
        | EnvelopeBody::Reconvert(_)
        | EnvelopeBody::UserDictListReply(_)
        | EnvelopeBody::LearningStatusReply(_)
        | EnvelopeBody::ExportReply(_)
        | EnvelopeBody::ImportReply(_)
        | EnvelopeBody::ProfileReply(_)
        // 絵文字の表（ADR 0027）は殻の入力だけが頼む
        | EnvelopeBody::EmojiTable(_)
        | EnvelopeBody::EmojiTableReply(_)
        // 診断ログ（ADR 0034）は殻の入力だけが送る
        | EnvelopeBody::Diag(_)
        // 英語のモードの郵便番号（ADR 0032 §9.1）は殻の入力だけが頼む
        | EnvelopeBody::PostalLookup(_)
        | EnvelopeBody::PostalLookupReply(_) => false,
    }
}

/// 設定アプリから届いた枠が、中継できないとき `NOT_ALLOWED` の枠を返す。中継してよい（または `engine` の枠でない）なら `None`
pub fn rejection(frame: &Frame) -> Option<Frame> {
    let Some(Body::Engine(envelope)) = &frame.body else {
        return None;
    };
    if is_allowed(envelope.body.as_ref()) {
        None
    } else {
        Some(error_frame(
            frame.request_id,
            BridgeErrorCode::NotAllowed,
            "",
        ))
    }
}

/// 中継の待ちの上限。インポートとエクスポートは 30 秒、ほかは 3 秒（ADR 0005 §8.3）
pub fn timeout(body: Option<&EnvelopeBody>) -> Duration {
    match body {
        Some(EnvelopeBody::ImportData(_) | EnvelopeBody::ExportData(_)) => timeouts::ADMIN_TRANSFER,
        _ => timeouts::ADMIN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::proto::{BridgeError, GetSettings};
    use ikari_proto::v1;

    fn engine_frame(id: u64, body: Option<EnvelopeBody>) -> Frame {
        Frame {
            request_id: id,
            body: Some(Body::Engine(v1::Envelope {
                schema_version: 1,
                request_id: id,
                body,
            })),
        }
    }

    #[test]
    fn allows_hello_and_every_admin_request() {
        let allowed = [
            EnvelopeBody::Hello(Default::default()),
            EnvelopeBody::ListUserDict(Default::default()),
            EnvelopeBody::AddUserDict(Default::default()),
            EnvelopeBody::RemoveUserDict(Default::default()),
            EnvelopeBody::LearningStatus(Default::default()),
            EnvelopeBody::ResetLearning(Default::default()),
            EnvelopeBody::ExportData(Default::default()),
            EnvelopeBody::ImportData(Default::default()),
            EnvelopeBody::GetProfile(Default::default()),
            EnvelopeBody::SetProfile(Default::default()),
        ];
        for body in allowed {
            let frame = engine_frame(4, Some(body));
            assert_eq!(rejection(&frame), None, "{frame:?}");
        }
    }

    #[test]
    fn rejects_the_requests_that_would_let_the_settings_app_touch_typing() {
        // 設定アプリから Commit などを送って学習を汚す経路を作らない
        let rejected = [
            EnvelopeBody::BeginSession(Default::default()),
            EnvelopeBody::EndSession(Default::default()),
            EnvelopeBody::Prefetch(Default::default()),
            EnvelopeBody::Convert(Default::default()),
            EnvelopeBody::Page(Default::default()),
            EnvelopeBody::Commit(Default::default()),
            EnvelopeBody::CommitText(Default::default()),
            EnvelopeBody::Cancel(Default::default()),
            EnvelopeBody::Shutdown(Default::default()),
            EnvelopeBody::SegmentPage(Default::default()),
            EnvelopeBody::SegmentPageReply(Default::default()),
            EnvelopeBody::ResizeSegment(Default::default()),
            EnvelopeBody::SegmentLayout(Default::default()),
            EnvelopeBody::CommitSegments(Default::default()),
            EnvelopeBody::Reconvert(Default::default()),
            EnvelopeBody::PostalLookup(Default::default()),
            EnvelopeBody::PostalLookupReply(Default::default()),
            EnvelopeBody::EmojiTable(Default::default()),
            EnvelopeBody::Diag(Default::default()),
        ];
        for body in rejected {
            let frame = engine_frame(12, Some(body));
            let Some(rejection) = rejection(&frame) else {
                panic!("expected a BridgeError for {frame:?}");
            };
            assert_eq!(rejection.request_id, 12);
            let Some(Body::Error(BridgeError { code, key })) = rejection.body else {
                panic!("expected a BridgeError for {frame:?}");
            };
            assert_eq!(code, BridgeErrorCode::NotAllowed as i32);
            assert!(key.is_empty());
        }
    }

    #[test]
    fn rejects_replies_and_errors_sent_as_requests() {
        let rejected = [
            EnvelopeBody::HelloReply(Default::default()),
            EnvelopeBody::Ack(Default::default()),
            EnvelopeBody::ConvertReply(Default::default()),
            EnvelopeBody::PageReply(Default::default()),
            EnvelopeBody::Error(Default::default()),
            EnvelopeBody::UserDictListReply(Default::default()),
            EnvelopeBody::LearningStatusReply(Default::default()),
            EnvelopeBody::ExportReply(Default::default()),
            EnvelopeBody::ImportReply(Default::default()),
            EnvelopeBody::ProfileReply(Default::default()),
            EnvelopeBody::EmojiTableReply(Default::default()),
        ];
        for body in rejected {
            assert!(rejection(&engine_frame(4, Some(body))).is_some());
        }
    }

    #[test]
    fn rejects_an_envelope_without_a_body() {
        assert_eq!(
            rejection(&engine_frame(4, None)).map(|f| f.request_id),
            Some(4)
        );
        assert!(!is_allowed(None));
    }

    #[test]
    fn leaves_frames_that_are_not_for_the_engine_alone() {
        let frame = Frame {
            request_id: 1,
            body: Some(Body::GetSettings(GetSettings {})),
        };
        assert_eq!(rejection(&frame), None);
    }

    #[test]
    fn transfers_get_thirty_seconds_and_the_rest_three() {
        let thirty = Duration::from_secs(30);
        let three = Duration::from_secs(3);
        assert_eq!(
            timeout(Some(&EnvelopeBody::ImportData(Default::default()))),
            thirty
        );
        assert_eq!(
            timeout(Some(&EnvelopeBody::ExportData(Default::default()))),
            thirty
        );
        assert_eq!(
            timeout(Some(&EnvelopeBody::ListUserDict(Default::default()))),
            three
        );
        assert_eq!(
            timeout(Some(&EnvelopeBody::Hello(Default::default()))),
            three
        );
        assert_eq!(timeout(None), three);
    }
}
