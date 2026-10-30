//! 設定アプリから届いた枠の振り分け（今の Swift の `BridgeServer.handle`・`relay`・`send` の判断）。
//! ソケットの待ち受け、相手の確認、接続ごとのスレッドは OS 側に残す。
//! 設定の読み書き（`GetSettings`、`UpdateSettings`）は振り分けるだけで、中身は設定（L-C）と殻（L-A2）が行う。

use std::sync::Arc;
use std::time::Duration;

use prost::Message;

use super::frame_codec::FrameError;
use super::proto::{BridgeErrorCode, Frame, UpdateSettings, frame::Body};
use super::protocol::error_frame;
use super::relay_gate::{LARGE_RELAY_BYTES, RelayGate, RelayPermit};
use super::relay_policy;
use crate::engine::EngineLink;
use crate::types::EngineError;
use ikari_proto::v1;

/// 枠 1 つへの対応
#[derive(Debug)]
pub enum Route {
    /// この枠をすぐ返す（許可リストで断った、満員、送ってよくない種類）
    Reply(Frame),
    /// 今効いている設定の一式を返す（`protocol::snapshot_frame`）
    GetSettings { request_id: u64 },
    /// 設定を書き換えて、反映後の一式を返す。不正なキーは `update_failure`
    UpdateSettings {
        request_id: u64,
        update: UpdateSettings,
    },
    /// エンジンへ中継する（待つので、受けたスレッドとは別で走らせる。`RelayJob::run`）
    Relay(RelayJob),
}

/// 受け付けた中継 1 本。枠（`RelayPermit`）を持ち、終わると空く
#[derive(Debug)]
pub struct RelayJob {
    pub request_id: u64,
    /// 直列化は一度だけ行い、そのバイト列を中継へ渡す（複製を増やさない）
    pub envelope: Vec<u8>,
    pub timeout: Duration,
    permit: RelayPermit,
}

impl RelayJob {
    pub fn is_large(&self) -> bool {
        self.permit.is_large()
    }

    /// 管理の要求をエンジンへそのまま流し、返す枠を作る（種類の許可リストは `route` で確認済み。中身は解釈しない）
    pub fn run(self, link: &EngineLink) -> Frame {
        let result = link.relay(&self.envelope, self.timeout);
        let frame = relay_reply(self.request_id, result);
        drop(self.permit);
        frame
    }
}

/// 設定アプリから届いた枠を振り分ける（`BridgeHello` を交わしたあとの枠）。
/// 中継は受け付けた時点で枠を取り、満員なら待たせず断る（連打でキューとメモリを溜めない）
pub fn route(frame: Frame, gate: &Arc<RelayGate>) -> Route {
    if let Some(rejection) = relay_policy::rejection(&frame) {
        return Route::Reply(rejection);
    }
    let request_id = frame.request_id;
    match frame.body {
        Some(Body::GetSettings(_)) => Route::GetSettings { request_id },
        Some(Body::UpdateSettings(update)) => Route::UpdateSettings { request_id, update },
        Some(Body::Engine(envelope)) => {
            let timeout = relay_policy::timeout(envelope.body.as_ref());
            let bytes = envelope.encode_to_vec();
            match gate.try_enter(bytes.len() >= LARGE_RELAY_BYTES) {
                Some(permit) => Route::Relay(RelayJob {
                    request_id,
                    envelope: bytes,
                    timeout,
                    permit,
                }),
                None => Route::Reply(error_frame(
                    request_id,
                    BridgeErrorCode::EngineUnavailable,
                    "",
                )),
            }
        }
        // 設定アプリから送ってよいのは、上の要求だけ（二度目の Hello、殻からの通知や返事の種類は断る）
        _ => Route::Reply(error_frame(request_id, BridgeErrorCode::NotAllowed, "")),
    }
}

/// 中継の結果を返す枠にする。エンジンの返事は解釈せずに包む。届かない・読めないなら `ENGINE_UNAVAILABLE`
pub fn relay_reply(request_id: u64, result: Result<Vec<u8>, EngineError>) -> Frame {
    match result
        .ok()
        .and_then(|bytes| v1::Envelope::decode(bytes.as_slice()).ok())
    {
        Some(reply) => Frame {
            request_id,
            body: Some(Body::Engine(reply)),
        },
        None => error_frame(request_id, BridgeErrorCode::EngineUnavailable, ""),
    }
}

/// 設定の書き換えの失敗の枠。`invalid_key` は不正だった設定のキー名だけ（値は入れない）。`None` は書けなかった
pub fn update_failure(request_id: u64, invalid_key: Option<&str>) -> Frame {
    match invalid_key {
        Some(key) => error_frame(request_id, BridgeErrorCode::InvalidSetting, key),
        None => error_frame(request_id, BridgeErrorCode::WriteFailed, ""),
    }
}

/// 枠を作れなかったとき（`frame_codec::encode` の失敗）に代わりに送る枠。返事が大きすぎる（エクスポートが上限を超えた）
/// なら理由だけ返す。ほかは送らない
pub fn encode_failure_reply(request_id: u64, error: FrameError) -> Option<Frame> {
    match error {
        FrameError::TooLarge { .. } => Some(error_frame(request_id, BridgeErrorCode::TooLarge, "")),
        FrameError::Empty | FrameError::Malformed => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::proto::{BridgeError, GetSettings, SettingEntry};
    use crate::host::{Clock, EngineTransport, SystemClock, TransportError, TransportHandle};
    use ikari_proto::v1::envelope::Body as EnvelopeBody;
    use std::sync::Mutex;

    fn engine_frame(id: u64, body: EnvelopeBody) -> Frame {
        Frame {
            request_id: id,
            body: Some(Body::Engine(v1::Envelope {
                schema_version: 1,
                request_id: 0,
                body: Some(body),
            })),
        }
    }

    fn error_code(frame: &Frame) -> Option<(i32, &str)> {
        match &frame.body {
            Some(Body::Error(BridgeError { code, key })) => Some((*code, key.as_str())),
            _ => None,
        }
    }

    /// 管理の要求に、分野 1 つのプロファイルで答える替え玉
    struct ProfileEngine {
        relayed: Arc<Mutex<Vec<v1::Envelope>>>,
    }

    impl EngineTransport for ProfileEngine {
        fn connect(&self) -> Result<TransportHandle, TransportError> {
            Ok(TransportHandle(1))
        }
        fn request(
            &self,
            _: TransportHandle,
            bytes: &[u8],
            _: Duration,
        ) -> Result<Vec<u8>, TransportError> {
            let request = v1::Envelope::decode(bytes).unwrap();
            self.relayed.lock().unwrap().push(request.clone());
            if matches!(request.body, Some(EnvelopeBody::ListUserDict(_))) {
                return Err(TransportError::Timeout);
            }
            Ok(v1::Envelope {
                schema_version: 1,
                request_id: request.request_id,
                body: Some(EnvelopeBody::ProfileReply(v1::ProfileReply {
                    domains: vec![v1::ProfileDomainState {
                        id: "it".into(),
                        name: "IT".into(),
                        weight: 3,
                    }],
                })),
            }
            .encode_to_vec())
        }
        fn post(&self, _: TransportHandle, _: &[u8]) {}
        fn close(&self, _: TransportHandle) {}
        fn pid(&self, _: TransportHandle) -> Option<i32> {
            None
        }
    }

    fn link() -> (EngineLink, Arc<Mutex<Vec<v1::Envelope>>>) {
        let relayed = Arc::new(Mutex::new(Vec::new()));
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let transport = ProfileEngine {
            relayed: relayed.clone(),
        };
        (EngineLink::new(Box::new(transport), clock), relayed)
    }

    #[test]
    fn engine_relay_allows_admin_and_refuses_commit() {
        let gate = Arc::new(RelayGate::default());
        let (link, relayed) = link();
        let Route::Relay(job) = route(
            engine_frame(21, EnvelopeBody::GetProfile(Default::default())),
            &gate,
        ) else {
            panic!("expected a relay");
        };
        assert_eq!(job.timeout, Duration::from_secs(3));
        assert!(!job.is_large());
        let reply = job.run(&link);
        assert_eq!(reply.request_id, 21);
        let Some(Body::Engine(out)) = reply.body else {
            panic!("no relay reply");
        };
        let Some(EnvelopeBody::ProfileReply(profile)) = out.body else {
            panic!("no profile reply");
        };
        assert_eq!(profile.domains[0].id, "it");

        let commit = engine_frame(22, EnvelopeBody::Commit(Default::default()));
        let Route::Reply(refused) = route(commit, &gate) else {
            panic!("expected a refusal");
        };
        assert_eq!(refused.request_id, 22);
        assert_eq!(
            error_code(&refused),
            Some((BridgeErrorCode::NotAllowed as i32, ""))
        );
        assert_eq!(relayed.lock().unwrap().len(), 1); // Commit は流れていない
    }

    #[test]
    fn a_relay_that_fails_answers_engine_unavailable() {
        let gate = Arc::new(RelayGate::default());
        let (link, _) = link();
        let Route::Relay(job) = route(
            engine_frame(5, EnvelopeBody::ListUserDict(Default::default())),
            &gate,
        ) else {
            panic!("expected a relay");
        };
        let reply = job.run(&link);
        assert_eq!(
            error_code(&reply),
            Some((BridgeErrorCode::EngineUnavailable as i32, ""))
        );
        assert_eq!(
            relay_reply(6, Ok(vec![0xFF, 0xFF])).body,
            error_frame(6, BridgeErrorCode::EngineUnavailable, "").body
        );
    }

    #[test]
    fn relay_flood_is_refused_beyond_the_limit_without_queueing() {
        let gate = Arc::new(RelayGate::default());
        let mut accepted = Vec::new();
        let mut refused = 0;
        for i in 0..24 {
            match route(
                engine_frame(100 + i, EnvelopeBody::GetProfile(Default::default())),
                &gate,
            ) {
                Route::Relay(job) => accepted.push(job),
                Route::Reply(frame) => {
                    assert_eq!(
                        error_code(&frame),
                        Some((BridgeErrorCode::EngineUnavailable as i32, ""))
                    );
                    refused += 1;
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(accepted.len(), 4);
        assert_eq!(refused, 20);
        // 終われば、また受け付ける
        let (link, _) = link();
        for job in accepted {
            job.run(&link);
        }
        assert!(matches!(
            route(
                engine_frame(999, EnvelopeBody::GetProfile(Default::default())),
                &gate
            ),
            Route::Relay(_)
        ));
    }

    #[test]
    fn large_requests_take_the_large_slot_and_transfers_wait_thirty_seconds() {
        let gate = Arc::new(RelayGate::default());
        let big = || {
            engine_frame(
                1,
                EnvelopeBody::ImportData(v1::ImportData {
                    content: vec![0; LARGE_RELAY_BYTES],
                    dry_run: true,
                }),
            )
        };
        let Route::Relay(first) = route(big(), &gate) else {
            panic!("expected a relay");
        };
        assert!(first.is_large());
        assert_eq!(first.timeout, Duration::from_secs(30));
        assert!(matches!(route(big(), &gate), Route::Reply(_)));
        drop(first);
        assert!(matches!(route(big(), &gate), Route::Relay(_)));
    }

    #[test]
    fn settings_requests_are_routed_and_other_kinds_are_not_allowed() {
        let gate = Arc::new(RelayGate::default());
        let get = Frame {
            request_id: 3,
            body: Some(Body::GetSettings(GetSettings {})),
        };
        assert!(matches!(
            route(get, &gate),
            Route::GetSettings { request_id: 3 }
        ));
        let update = UpdateSettings {
            set: vec![SettingEntry {
                key: "paging.size".into(),
                value: "5".into(),
            }],
            reset: vec!["engine.lm".into()],
        };
        let frame = Frame {
            request_id: 4,
            body: Some(Body::UpdateSettings(update.clone())),
        };
        let Route::UpdateSettings {
            request_id,
            update: routed,
        } = route(frame, &gate)
        else {
            panic!("expected an update");
        };
        assert_eq!((request_id, routed), (4, update));
        // 二度目の Hello、通知、空の枠は断る
        for body in [
            Some(Body::Hello(Default::default())),
            Some(Body::Event(Default::default())),
            None,
        ] {
            let Route::Reply(reply) = route(
                Frame {
                    request_id: 7,
                    body,
                },
                &gate,
            ) else {
                panic!("expected a refusal");
            };
            assert_eq!(
                error_code(&reply),
                Some((BridgeErrorCode::NotAllowed as i32, ""))
            );
        }
    }

    #[test]
    fn update_failures_carry_only_the_key_name() {
        assert_eq!(
            error_code(&update_failure(1, Some("paging.size"))),
            Some((BridgeErrorCode::InvalidSetting as i32, "paging.size"))
        );
        assert_eq!(
            error_code(&update_failure(1, None)),
            Some((BridgeErrorCode::WriteFailed as i32, ""))
        );
    }

    #[test]
    fn an_oversized_reply_is_replaced_by_too_large() {
        let reply = encode_failure_reply(8, FrameError::TooLarge { length: 1 }).unwrap();
        assert_eq!(reply.request_id, 8);
        assert_eq!(
            error_code(&reply),
            Some((BridgeErrorCode::TooLarge as i32, ""))
        );
        assert!(encode_failure_reply(8, FrameError::Malformed).is_none());
    }
}
