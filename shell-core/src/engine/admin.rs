//! 殻（入力メニュー）が使う、エンジンの管理の要求の型付きの口（ADR 0005 §8.3「殻の口」。今の Swift の `EngineAdmin`）。
//! 今はプロファイル（分野）だけ。辞書と学習は設定アプリが Bridge の中継で直接話す。
//! 送るのは `EngineLink::relay`。分野の名前は利用者向けの文字列だが、ログには書かない。

use std::time::Duration;

use ikari_proto::v1::{self, envelope::Body};
use prost::Message;

use super::{EngineLink, SCHEMA_VERSION, codec};
use crate::types::{EngineError, EngineErrorCode};

/// 返事の検証の上限（壊れた返事だけを弾く。同梱のプリセットは数個）
pub const MAX_DOMAINS: usize = 64;

/// 分野の重み
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileWeight {
    Off,
    Sometimes,
    Often,
}

impl ProfileWeight {
    fn from_proto(kind: i32) -> Self {
        match v1::ProfileWeightKind::try_from(kind) {
            Ok(v1::ProfileWeightKind::ProfileWeightSometimes) => Self::Sometimes,
            Ok(v1::ProfileWeightKind::ProfileWeightOften) => Self::Often,
            _ => Self::Off,
        }
    }

    fn to_proto(self) -> v1::ProfileWeightKind {
        match self {
            Self::Off => v1::ProfileWeightKind::ProfileWeightOff,
            Self::Sometimes => v1::ProfileWeightKind::ProfileWeightSometimes,
            Self::Often => v1::ProfileWeightKind::ProfileWeightOften,
        }
    }
}

/// 同梱のプリセットの 1 つの分野
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileDomain {
    pub id: String,
    pub name: String,
    pub weight: ProfileWeight,
}

/// `EngineLink` を借りる管理の口。待ちの上限は呼ぶ側が渡す（今の `EngineTimeouts.admin`）
pub struct EngineAdmin<'a> {
    link: &'a EngineLink,
    timeout: Duration,
}

impl<'a> EngineAdmin<'a> {
    pub fn new(link: &'a EngineLink, timeout: Duration) -> Self {
        Self { link, timeout }
    }

    /// 同梱のプリセットすべての、今の重み（OFF を含む）
    pub fn profile(&self) -> Result<Vec<ProfileDomain>, EngineError> {
        self.round_trip(Body::GetProfile(v1::ProfileGet {}))
    }

    /// 分野全体を置き換える（`profile.tsv` に書き、その場で組み立て直される）。OFF の分野は送らない（エンジンの規則）。
    /// 返事は変更後の全体。
    pub fn set_profile(
        &self,
        domains: &[ProfileDomain],
    ) -> Result<Vec<ProfileDomain>, EngineError> {
        let message = v1::ProfileSet {
            domains: domains
                .iter()
                .filter(|domain| domain.weight != ProfileWeight::Off)
                .map(|domain| v1::ProfileDomainState {
                    id: domain.id.clone(),
                    name: domain.name.clone(),
                    weight: domain.weight.to_proto() as i32,
                })
                .collect(),
        };
        self.round_trip(Body::SetProfile(message))
    }

    /// 1 つの分野の重みだけを変える。最新の状態を取り直してから変えるので、メニューのキャッシュが古くても他の分野を壊さない。
    /// 知らない `id` なら `Engine(BadRequest)`
    pub fn set_weight(
        &self,
        weight: ProfileWeight,
        id: &str,
    ) -> Result<Vec<ProfileDomain>, EngineError> {
        let mut domains = self.profile()?;
        let domain = domains
            .iter_mut()
            .find(|domain| domain.id == id)
            .ok_or(EngineError::Engine(EngineErrorCode::BadRequest))?;
        domain.weight = weight;
        self.set_profile(&domains)
    }

    fn round_trip(&self, body: Body) -> Result<Vec<ProfileDomain>, EngineError> {
        // request_id は relay が付け直す
        let request = v1::Envelope {
            schema_version: SCHEMA_VERSION,
            request_id: 0,
            body: Some(body),
        };
        let reply = self.link.relay(&request.encode_to_vec(), self.timeout)?;
        let envelope =
            v1::Envelope::decode(reply.as_slice()).map_err(|_| EngineError::InvalidReply)?;
        domains(envelope)
    }
}

fn domains(envelope: v1::Envelope) -> Result<Vec<ProfileDomain>, EngineError> {
    let Body::ProfileReply(reply) = codec::body(envelope)? else {
        return Err(EngineError::InvalidReply);
    };
    if reply.domains.len() > MAX_DOMAINS {
        return Err(EngineError::InvalidReply);
    }
    reply
        .domains
        .into_iter()
        .map(|state| {
            if state.id.is_empty()
                || state.id.chars().count() > codec::MAX_TEXT_CHARACTERS
                || state.name.chars().count() > codec::MAX_TEXT_CHARACTERS
            {
                return Err(EngineError::InvalidReply);
            }
            Ok(ProfileDomain {
                weight: ProfileWeight::from_proto(state.weight),
                id: state.id,
                name: state.name,
            })
        })
        .collect()
}
