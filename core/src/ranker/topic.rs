//! 話題スコア（chiVe。ADR 0002 §13.1）。
//!
//! 解析窓の内容語のベクトルを、新しいものほど重くして平均した「文脈ベクトル」を作り、
//! 候補の内容語セグメントのベクトルとのコサイン類似度で加点する。

use std::collections::BTreeMap;
use std::sync::Arc;

use super::common::{apply_bonus, content_segments, segment_lemma};
use super::config::ScopedKv;
use super::names;
use super::{RankInput, Ranker, RankerConfigError};
use crate::candidate::Candidate;
use crate::vectors::WordVectors;

const CONFIG_PREFIX: &str = names::TOPIC;
const CONFIG_KEYS: [&str; 6] = [
    "weight",
    "tau",
    "max_total",
    "recent_tokens",
    "decay",
    "min_content",
];

#[derive(Clone, Debug, PartialEq)]
pub struct TopicConfig {
    /// `-weight * max(0, cos - tau)`
    pub weight: i32,
    pub tau: f32,
    /// 1候補あたりの合計の上限
    pub max_total: i32,
    /// 文脈ベクトルに使う直近の内容語の数
    pub recent_tokens: usize,
    /// 新しいものほど重く（`decay ^ 距離`）
    pub decay: f32,
    /// 内容語がこれ未満なら加点しない
    pub min_content: usize,
}

impl Default for TopicConfig {
    fn default() -> Self {
        Self {
            weight: 10000,
            tau: 0.10,
            max_total: 5000,
            recent_tokens: 50,
            decay: 0.8, // ADR 0047（#178）。掃引: eval/reports/topic-decay-sweep-2026-11-04.md
            min_content: 1,
        }
    }
}

impl TopicConfig {
    /// `topic.weight` のように、`topic.` を前置したキーだけを読む。
    /// 他の接頭辞のキーは無視し、`topic.` で未知のキーや不正な値ならエラー。
    pub fn from_kv(kv: &BTreeMap<String, String>) -> Result<Self, RankerConfigError> {
        let scope = ScopedKv::new(CONFIG_PREFIX, &CONFIG_KEYS, kv)?;
        let d = Self::default();
        Ok(Self {
            weight: scope.get("weight", d.weight)?,
            tau: scope.get_float("tau", d.tau)?,
            max_total: scope.get("max_total", d.max_total)?,
            recent_tokens: scope.get("recent_tokens", d.recent_tokens)?,
            decay: scope.get_float("decay", d.decay)?,
            min_content: scope.get("min_content", d.min_content)?,
        })
    }
}

pub struct TopicRanker {
    vectors: Arc<dyn WordVectors>,
    cfg: TopicConfig,
}

impl TopicRanker {
    pub fn new(vectors: Arc<dyn WordVectors>, cfg: TopicConfig) -> Self {
        Self { vectors, cfg }
    }

    /// 文脈ベクトル（L2 正規化済み）。使えるベクトルが `min_content` 個未満なら None。
    fn context_vector(&self, input: &RankInput<'_>) -> Option<Vec<f32>> {
        let dims = self.vectors.dims();
        let mut mean = WeightedMean::new(dims);
        let mut used = 0;
        let mut weight = 1.0f32;
        let newest_first = input.analysis.tokens.iter().rev().filter(|t| t.is_content);
        for token in newest_first {
            if used >= self.cfg.recent_tokens {
                break;
            }
            let Some(v) = token.lemma_id.and_then(|id| self.vectors.vector(id)) else {
                continue;
            };
            if mean.add(v, weight) {
                used += 1;
                weight *= self.cfg.decay;
            }
        }
        if used < self.cfg.min_content {
            return None;
        }
        mean.unit()
    }
}

impl Ranker for TopicRanker {
    fn rank(&self, input: &RankInput<'_>, cands: &mut [Candidate]) {
        if input.context.secure {
            return;
        }
        let Some(ctx_vec) = self.context_vector(input) else {
            return;
        };
        let weight = self.cfg.weight as f32;
        for cand in cands.iter_mut() {
            let mut total = 0.0f32;
            for seg in content_segments(input.grammar, cand) {
                let Some(v) = segment_lemma(input.dict, seg)
                    .1
                    .and_then(|id| self.vectors.vector(id))
                else {
                    continue;
                };
                if let Some(cos) = cosine(v, &ctx_vec) {
                    total += weight * (cos - self.cfg.tau).max(0.0);
                }
            }
            apply_bonus(cand, (total.round() as i32).min(self.cfg.max_total));
        }
    }

    fn name(&self) -> &str {
        names::TOPIC
    }
}

// ---- ベクトルの計算（profile.rs からも使う） ----

/// i8 のベクトルを重み付きで足していく。
pub(super) struct WeightedMean {
    sum: Vec<f32>,
}

impl WeightedMean {
    pub(super) fn new(dims: usize) -> Self {
        Self {
            sum: vec![0.0; dims],
        }
    }

    /// 次元が合わなければ足さずに false。
    pub(super) fn add(&mut self, v: &[i8], weight: f32) -> bool {
        if v.len() != self.sum.len() {
            return false;
        }
        for (s, &x) in self.sum.iter_mut().zip(v) {
            *s += weight * f32::from(x);
        }
        true
    }

    /// L2 正規化したベクトル。ゼロベクトルなら None。
    pub(super) fn unit(mut self) -> Option<Vec<f32>> {
        let norm = self.sum.iter().map(|x| x * x).sum::<f32>().sqrt();
        if !norm.is_finite() || norm <= f32::EPSILON {
            return None;
        }
        for x in &mut self.sum {
            *x /= norm;
        }
        Some(self.sum)
    }
}

/// i8 のベクトルと、L2 正規化済みの f32 ベクトルのコサイン類似度。
/// 次元が違う、またはゼロベクトルなら None。
pub(super) fn cosine(v: &[i8], unit: &[f32]) -> Option<f32> {
    if v.len() != unit.len() {
        return None;
    }
    let (mut dot, mut norm_sq) = (0.0f32, 0.0f32);
    for (&a, &b) in v.iter().zip(unit) {
        let a = f32::from(a);
        dot += a * b;
        norm_sq += a * a;
    }
    (norm_sq > 0.0).then(|| dot / norm_sq.sqrt())
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::analysis::AnalyzedContext;
    use crate::context::Context;

    const IT: u32 = 1;
    const IT2: u32 = 2;
    const NOVEL: u32 = 3;
    const FAR: u32 = 4;

    /// 4次元。IT 系は軸0、小説系は軸1、IT2 は軸0 に少し軸1 が混じる
    fn vectors() -> Arc<dyn WordVectors> {
        Arc::new(FakeVectors::new(
            4,
            &[
                (IT, &[1.0, 0.0, 0.0, 0.0]),
                (IT2, &[0.8, 0.6, 0.0, 0.0]),
                (NOVEL, &[0.0, 1.0, 0.0, 0.0]),
                (FAR, &[0.0, 0.0, 1.0, 0.0]),
            ],
        ))
    }

    /// ADR 0002 §13.1 の値（W=4000、上限 2500、内容語 3 個未満は加点しない）。
    /// 既定値は dev 分割の調整で変えたので、テストはこの値を明示して使う。
    fn adr_config() -> TopicConfig {
        TopicConfig {
            weight: 4000,
            max_total: 2500,
            min_content: 3,
            ..TopicConfig::default()
        }
    }

    #[test]
    fn defaults_are_the_tuned_values() {
        let d = TopicConfig::default();
        assert_eq!((d.weight, d.max_total, d.min_content), (10000, 5000, 1));
        assert_eq!(d.tau, 0.10);
    }

    fn ranker() -> TopicRanker {
        TopicRanker::new(vectors(), adr_config())
    }

    /// 同音異義語の2候補（IT 側が 更生 的、小説側が 構成 的、のつもり）
    fn two_candidates(env: &mut Env) -> Vec<Candidate> {
        let it = env.word("構成", "構成", Some(IT), POS_NOUN);
        let novel = env.word("更生", "更生", Some(NOVEL), POS_NOUN);
        vec![cand(vec![novel], 5000), cand(vec![it], 5004)]
    }

    #[test]
    fn matching_topic_gets_bonus_and_other_does_not() {
        let mut env = Env::new();
        let mut cands = two_candidates(&mut env);
        env.rank(&ranker(), &Context::new(), &analysis_of(IT, 5), &mut cands);
        // cos = 1.0 -> 4000 * 0.9 = 3600 を 2500 に切り詰める。直交する側は 0
        assert_eq!(cands[1].score, 5004 - 2500);
        assert_eq!(cands[0].score, 5000);
    }

    #[test]
    fn bonus_is_proportional_below_the_cap() {
        let mut env = Env::new();
        let it2 = env.word("近い", "近い", Some(IT2), POS_NOUN);
        let mut cands = vec![cand(vec![it2], 10_000)];
        env.rank(&ranker(), &Context::new(), &analysis_of(IT, 5), &mut cands);
        // cos(IT2, IT) は約 0.8 なので 4000 * (0.8 - 0.1) = 2800 -> 上限の 2500 に当たるはずだが、
        // 量子化の誤差を含めて上限以下かつ大きな加点であること
        let bonus = 10_000 - cands[0].score;
        assert!((2400..=2500).contains(&bonus), "{bonus}");

        let weak = TopicConfig {
            weight: 1000,
            ..adr_config()
        };
        let mut cands = vec![cand(
            vec![env.word("近い2", "近い", Some(IT2), POS_NOUN)],
            10_000,
        )];
        env.rank(
            &TopicRanker::new(vectors(), weak),
            &Context::new(),
            &analysis_of(IT, 5),
            &mut cands,
        );
        let bonus = 10_000 - cands[0].score;
        assert!((650..=720).contains(&bonus), "{bonus}");
    }

    #[test]
    fn below_tau_gives_nothing() {
        let mut env = Env::new();
        let mut cands = vec![cand(
            vec![env.word("遠い", "遠い", Some(FAR), POS_NOUN)],
            100,
        )];
        env.rank(&ranker(), &Context::new(), &analysis_of(IT, 5), &mut cands);
        assert_eq!(cands[0].score, 100);
    }

    #[test]
    fn fewer_than_three_content_words_gives_nothing() {
        let mut env = Env::new();
        let mut cands = two_candidates(&mut env);
        env.rank(&ranker(), &Context::new(), &analysis_of(IT, 2), &mut cands);
        assert_eq!(cands[1].score, 5004);
        // ちょうど3個なら効く
        env.rank(&ranker(), &Context::new(), &analysis_of(IT, 3), &mut cands);
        assert!(cands[1].score < 5004);
    }

    #[test]
    fn non_content_tokens_and_tokens_without_vectors_do_not_count() {
        let mut env = Env::new();
        let mut cands = two_candidates(&mut env);
        let tokens = vec![
            token("に", "に", Some(IT), false, 5),
            token("に", "に", Some(IT), false, 4),
            token("未知", "未知", None, true, 3),
            token("語彙外", "語彙外", Some(999), true, 2),
            token("語", "語", Some(IT), true, 1),
        ];
        env.rank(&ranker(), &Context::new(), &analysis(tokens), &mut cands);
        assert_eq!(cands[1].score, 5004);
    }

    #[test]
    fn newer_words_weigh_more() {
        let mut env = Env::new();
        // 古い 4 語が IT、新しい 4 語が小説。上限に当たらない設定で比べる
        let mut tokens: Vec<_> = (0..4)
            .map(|i| token("it", "it", Some(IT), true, 20 - i))
            .collect();
        tokens.extend((0..4).map(|i| token("nv", "nv", Some(NOVEL), true, 4 - i)));
        let mut cands = two_candidates(&mut env);
        let cfg = TopicConfig {
            weight: 1000,
            max_total: 100_000,
            ..adr_config()
        };
        let r = TopicRanker::new(vectors(), cfg);
        env.rank(&r, &Context::new(), &analysis(tokens), &mut cands);
        let novel_bonus = 5000 - cands[0].score;
        let it_bonus = 5004 - cands[1].score;
        assert!(novel_bonus > it_bonus, "{novel_bonus} {it_bonus}");
    }

    #[test]
    fn only_recent_tokens_are_used() {
        let mut env = Env::new();
        // 新しい 3 語だけが使われ、それより古い IT の語は文脈ベクトルに入らない
        let mut tokens: Vec<_> = (0..10)
            .map(|i| token("it", "it", Some(IT), true, 30 - i))
            .collect();
        tokens.extend((0..3).map(|i| token("nv", "nv", Some(NOVEL), true, 3 - i)));
        let cfg = TopicConfig {
            recent_tokens: 3,
            ..adr_config()
        };
        let mut cands = two_candidates(&mut env);
        env.rank(
            &TopicRanker::new(vectors(), cfg),
            &Context::new(),
            &analysis(tokens),
            &mut cands,
        );
        assert_eq!(cands[1].score, 5004);
        assert!(cands[0].score < 5000);
    }

    #[test]
    fn only_content_segments_of_dictionary_candidates_are_scored() {
        let mut env = Env::new();
        let it = env.word("構成", "構成", Some(IT), POS_NOUN);
        let particle = env.word("に", "に", Some(IT), POS_PARTICLE);
        let mut cands = vec![
            cand(vec![particle], 100),
            cand(vec![it.clone(), it.clone()], 100),
        ];
        env.rank(&ranker(), &Context::new(), &analysis_of(IT, 5), &mut cands);
        assert_eq!(cands[0].score, 100);
        // 2セグメント分でも上限を超えない
        assert_eq!(cands[1].score, 100 - 2500);
    }

    #[test]
    fn empty_analysis_and_secure_change_nothing() {
        let mut env = Env::new();
        let mut cands = two_candidates(&mut env);
        env.rank(
            &ranker(),
            &Context::new(),
            &AnalyzedContext::default(),
            &mut cands,
        );
        assert_eq!((cands[0].score, cands[1].score), (5000, 5004));
        env.rank(
            &ranker(),
            &Context::new().with_secure(true),
            &analysis_of(IT, 5),
            &mut cands,
        );
        assert_eq!((cands[0].score, cands[1].score), (5000, 5004));
    }

    /// dims() と実際の行の長さが食い違う壊れた vectors
    struct Broken;

    impl WordVectors for Broken {
        fn dims(&self) -> usize {
            4
        }
        fn vector(&self, _lemma_id: u32) -> Option<&[i8]> {
            Some(&[1, 2])
        }
        fn nearest_cluster(&self, _v: &[f32]) -> Option<u16> {
            None
        }
        fn rank(&self, _lemma_id: u32) -> Option<u32> {
            None
        }
    }

    #[test]
    fn mismatched_dims_are_ignored_without_panicking() {
        let mut env = Env::new();
        let r = TopicRanker::new(Arc::new(Broken), adr_config());
        let mut cands = two_candidates(&mut env);
        env.rank(&r, &Context::new(), &analysis_of(IT, 5), &mut cands);
        assert_eq!((cands[0].score, cands[1].score), (5000, 5004));
    }

    #[test]
    fn cosine_and_mean_helpers() {
        assert_eq!(cosine(&[1, 0], &[1.0]), None);
        assert_eq!(cosine(&[0, 0], &[1.0, 0.0]), None);
        let c = cosine(&[127, 0], &[1.0, 0.0]).unwrap();
        assert!((c - 1.0).abs() < 1e-6);
        let mut m = WeightedMean::new(2);
        assert!(!m.add(&[1, 2, 3], 1.0));
        assert!(m.unit().is_none());
    }

    #[test]
    fn config_from_kv() {
        let kv: BTreeMap<String, String> = [("topic.weight", "100"), ("topic.tau", "0.2")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let cfg = TopicConfig::from_kv(&kv).unwrap();
        assert_eq!((cfg.weight, cfg.tau), (100, 0.2));
        assert_eq!(cfg.recent_tokens, 50);
        let bad: BTreeMap<String, String> = [("topic.tau".to_owned(), "x".to_owned())]
            .into_iter()
            .collect();
        assert!(TopicConfig::from_kv(&bad).is_err());
    }
}
