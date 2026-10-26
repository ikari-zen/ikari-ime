//! Ranker（ADR 0001 §4.4, ADR 0002 §6.2, §7）。

use std::fmt;
use std::time::Instant;

use crate::analysis::AnalyzedContext;
use crate::candidate::Candidate;
use crate::context::Context;
use crate::dict::{Dictionary, Grammar};

mod common;
mod composite;
pub(crate) mod config;
mod cost;
mod doc_cache;
pub(crate) mod english_first;
mod guard;
mod kana_style;
mod learning;
mod left_context;
pub mod names;
mod profile;
#[cfg(test)]
mod testutil;
mod topic;

pub use composite::CompositeRanker;
pub use cost::CostRanker;
pub use doc_cache::{DocumentCacheConfig, DocumentCacheRanker};
pub use kana_style::{KanaStyleConfig, KanaStyleParseError, KanaStyleRanker, KanaStyleTable};
pub use learning::{LearningConfig, LearningRanker};
pub use left_context::{LeftContextConfig, LeftContextRanker};
pub use profile::{ProfileConfig, ProfileRanker};
pub use topic::{TopicConfig, TopicRanker};

pub struct RankInput<'a> {
    pub reading: &'a str,
    /// secure なら空にしたもの
    pub context: &'a Context,
    pub grammar: &'a dyn Grammar,
    /// 解析済みの左文脈。secure なら `AnalyzedContext::default()`
    pub analysis: &'a AnalyzedContext,
    /// 語ID から表記や lemma を引く用
    pub dict: &'a dyn Dictionary,
    /// 変換1回の締め切り（ADR 0004 §3.4）。`None` = 待たない（LM を使わない）。
    /// 1回の `convert` / `page` の中の `rank` はすべて同じ値を受け取る。
    pub deadline: Option<Instant>,
}

pub trait Ranker: Send + Sync {
    /// cands の score を調整する。並べ替えは Converter が score の安定ソートで行う。
    /// 呼ばれるのは「まだ表示していない候補の塊」だけ（表示済みのページは固定する。NFR-05）。
    /// 各 Ranker は score に差分を足すだけにする（負の値が加点）。
    fn rank(&self, input: &RankInput<'_>, cands: &mut [Candidate]);
    fn name(&self) -> &str;

    /// `rank` の加点のうち、**現在の文脈に依らない部分**（学習の Unconditional の特徴量。正の値が加点）。
    /// 候補ごとに1つ。`cands` の表記と分割だけを見る（score には依らない）。
    /// 文脈に依らない加点は、現在の文脈の証拠（文書内キャッシュ、話題、左文脈の連接）を覆してはならない
    /// （spec の基本原則）。[`CompositeRanker`] がこの値を使って、逆転を止める。
    /// 該当する部分がない Ranker は `None`。
    fn context_free_bonus(&self, _input: &RankInput<'_>, _cands: &[Candidate]) -> Option<Vec<i32>> {
        None
    }
}

/// `*Config::from_kv` のエラー（未知のキー、数値として読めない値など）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankerConfigError {
    pub key: String,
    pub message: String,
}

impl fmt::Display for RankerConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ranker config `{}`: {}", self.key, self.message)
    }
}

impl std::error::Error for RankerConfigError {}
