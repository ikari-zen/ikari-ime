//! 殻（入力メニュー）が使う、エンジンの管理の要求の型付きの口（ADR 0005 §8.3「殻の口」。今の Swift の `EngineAdmin`）。
//! 今はプロファイル（分野）だけ。辞書と学習は設定アプリが Bridge の中継で直接話す。
//! 送るのは `EngineLink::relay`。分野の名前は利用者向けの文字列だが、ログには書かない。

use ikari_proto::v1::{self, envelope::Body};
use prost::Message;

use super::{EngineLink, SCHEMA_VERSION, codec};
use crate::menu::{ProfileDomain, ProfileWeight};
use crate::timeouts;
use crate::types::{EngineError, EngineErrorCode};

/// 返事の検証の上限（壊れた返事だけを弾く。同梱のプリセットは数個）
pub const MAX_DOMAINS: usize = 64;

fn weight_from_proto(kind: i32) -> ProfileWeight {
    match v1::ProfileWeightKind::try_from(kind) {
        Ok(v1::ProfileWeightKind::ProfileWeightSometimes) => ProfileWeight::Sometimes,
        Ok(v1::ProfileWeightKind::ProfileWeightOften) => ProfileWeight::Often,
        _ => ProfileWeight::Off,
    }
}

fn weight_to_proto(weight: ProfileWeight) -> v1::ProfileWeightKind {
    match weight {
        ProfileWeight::Off => v1::ProfileWeightKind::ProfileWeightOff,
        ProfileWeight::Sometimes => v1::ProfileWeightKind::ProfileWeightSometimes,
        ProfileWeight::Often => v1::ProfileWeightKind::ProfileWeightOften,
    }
}

/// `EngineLink` を借りる管理の口。待ちの上限は `timeouts::ADMIN`（今の `EngineTimeouts.admin`）。
/// 型（`ProfileDomain`、`ProfileWeight`）はメニューと共有する（`menu`）
pub struct EngineAdmin<'a> {
    link: &'a EngineLink,
}

impl<'a> EngineAdmin<'a> {
    pub fn new(link: &'a EngineLink) -> Self {
        Self { link }
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
                    weight: weight_to_proto(domain.weight) as i32,
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
        let reply = self.link.relay(&request.encode_to_vec(), timeouts::ADMIN)?;
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
                weight: weight_from_proto(state.weight),
                id: state.id,
                name: state.name,
            })
        })
        .collect()
}
