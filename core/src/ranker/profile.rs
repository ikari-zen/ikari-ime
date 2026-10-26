//! 属性プロファイルの加点（ADR 0002 §9.2）。内容語だけ、上限あり、文脈で減衰。
//!
//! 規則（spec の制約を実装に落としたもの）:
//! 1. 内容語のセグメントだけを見る。
//! 2. vectors があれば、プロファイルのベクトルとのコサイン類似度 `K * max(0, cos - min_cos)`。
//!    ベクトルがない語、vectors がないときは、種語に lemma が一致したら `seed_bonus`。
//!    どちらも、利用者が付けた分野の重み（often = 1.0、sometimes = 0.5）を掛ける。
//! 3. 1候補あたりの合計は `max_total` まで。
//! 4. 合計に `1 / (1 + content_count / decay_content)` を掛ける（文脈が増えるほど弱める）。

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use super::common::{apply_bonus, content_segments, segment_lemma};
use super::config::ScopedKv;
use super::names;
use super::topic::{WeightedMean, cosine};
use super::{RankInput, Ranker, RankerConfigError};
use crate::candidate::Candidate;
use crate::dict::Dictionary;
use crate::user::Profile;
use crate::vectors::WordVectors;

const CONFIG_PREFIX: &str = names::PROFILE;
const CONFIG_KEYS: [&str; 5] = ["k", "min_cos", "seed_bonus", "max_total", "decay_content"];

#[derive(Clone, Debug, PartialEq)]
pub struct ProfileConfig {
    /// vectors あり: `-K * max(0, cos - min_cos)`
    pub k: i32,
    pub min_cos: f32,
    /// vectors なし: 種語に lemma が一致したときの加点（正の値）
    pub seed_bonus: i32,
    /// 1候補あたりの合計の上限
    pub max_total: i32,
    /// 減衰: `1 / (1 + content_count / decay_content)`
    pub decay_content: u32,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            k: 3000,
            min_cos: 0.15,
            seed_bonus: 800,
            max_total: 1200,
            decay_content: 20,
        }
    }
}

impl ProfileConfig {
    /// `profile.k` のように、`profile.` を前置したキーだけを読む。
    /// 他の接頭辞のキーは無視し、`profile.` で未知のキーや不正な値ならエラー。
    pub fn from_kv(kv: &BTreeMap<String, String>) -> Result<Self, RankerConfigError> {
        let scope = ScopedKv::new(CONFIG_PREFIX, &CONFIG_KEYS, kv)?;
        let d = Self::default();
        Ok(Self {
            k: scope.get("k", d.k)?,
            min_cos: scope.get_float("min_cos", d.min_cos)?,
            seed_bonus: scope.get("seed_bonus", d.seed_bonus)?,
            max_total: scope.get("max_total", d.max_total)?,
            decay_content: scope.get("decay_content", d.decay_content)?,
        })
    }
}

/// プロファイルから前計算したもの。同じ `Arc<Profile>` の間は使い回す。
struct Prepared {
    profile: Arc<Profile>,
    /// 種語の lemma -> 分野の重み（複数の分野に出てくれば大きい方）
    seeds: HashMap<String, f32>,
    /// 種語のベクトルを分野の重みで平均した、L2 正規化済みのベクトル
    vector: Option<Vec<f32>>,
}

/// プロファイルは `RankInput.context.profile` から取る。
pub struct ProfileRanker {
    vectors: Option<Arc<dyn WordVectors>>,
    cfg: ProfileConfig,
    prepared: Mutex<Option<Arc<Prepared>>>,
}

impl ProfileRanker {
    pub fn new(vectors: Option<Arc<dyn WordVectors>>, cfg: ProfileConfig) -> Self {
        Self {
            vectors,
            cfg,
            prepared: Mutex::new(None),
        }
    }

    fn prepared_for(&self, profile: &Arc<Profile>, dict: &dyn Dictionary) -> Arc<Prepared> {
        let mut guard = self
            .prepared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(p) = guard.as_ref()
            && Arc::ptr_eq(&p.profile, profile)
        {
            return Arc::clone(p);
        }
        let prepared = Arc::new(self.prepare(profile, dict));
        *guard = Some(Arc::clone(&prepared));
        prepared
    }

    fn prepare(&self, profile: &Arc<Profile>, dict: &dyn Dictionary) -> Prepared {
        let mut seeds: HashMap<String, f32> = HashMap::new();
        let mut mean = self.vectors.as_ref().map(|v| WeightedMean::new(v.dims()));
        for domain in &profile.domains {
            let weight = domain.weight.0;
            for seed in &domain.seeds {
                let w = seeds.entry(seed.clone()).or_insert(weight);
                *w = w.max(weight);
                if let (Some(mean), Some(vectors)) = (mean.as_mut(), self.vectors.as_ref())
                    && let Some(v) = seed_lemma_id(dict, seed).and_then(|id| vectors.vector(id))
                {
                    mean.add(v, weight);
                }
            }
        }
        Prepared {
            profile: Arc::clone(profile),
            seeds,
            vector: mean.and_then(WeightedMean::unit),
        }
    }
}

/// 種語（lemma の文字列）から lemma 番号を求める。lemma が一致する語を優先し、
/// なければ表記が一致する語の lemma 番号を使う。
fn seed_lemma_id(dict: &dyn Dictionary, seed: &str) -> Option<u32> {
    let (mut exact, mut by_surface) = (None, None);
    dict.common_prefix_search_surface(seed, &mut |len, entry| {
        if len != seed.len() {
            return;
        }
        if let Some(lemma) = dict.lemma(entry.id)
            && let Some(id) = lemma.id
        {
            if lemma.text == seed {
                exact.get_or_insert(id);
            } else {
                by_surface.get_or_insert(id);
            }
        }
    });
    exact.or(by_surface)
}

impl Ranker for ProfileRanker {
    fn rank(&self, input: &RankInput<'_>, cands: &mut [Candidate]) {
        if input.context.secure {
            return;
        }
        let Some(profile) = input.context.profile.as_ref().filter(|p| !p.is_empty()) else {
            return;
        };
        let prepared = self.prepared_for(profile, input.dict);
        let decay_content = self.cfg.decay_content.max(1) as f32;
        let decay = 1.0 / (1.0 + input.analysis.content_count as f32 / decay_content);

        for cand in cands.iter_mut() {
            let mut total = 0.0f32;
            for seg in content_segments(input.grammar, cand) {
                let (lemma, lemma_id) = segment_lemma(input.dict, seg);
                let by_vector = match (&self.vectors, &prepared.vector, lemma_id) {
                    (Some(vectors), Some(profile_vec), Some(id)) => vectors
                        .vector(id)
                        .and_then(|v| cosine(v, profile_vec))
                        .map(|cos| self.cfg.k as f32 * (cos - self.cfg.min_cos).max(0.0)),
                    _ => None,
                };
                total += by_vector.unwrap_or_else(|| {
                    prepared
                        .seeds
                        .get(lemma)
                        .map_or(0.0, |w| self.cfg.seed_bonus as f32 * w)
                });
            }
            let capped = total.min(self.cfg.max_total as f32);
            apply_bonus(cand, (capped * decay).round() as i32);
        }
    }

    fn name(&self) -> &str {
        names::PROFILE
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::analysis::AnalyzedContext;
    use crate::context::Context;
    use crate::user::{ProfileDomain, ProfileWeight};

    const SEED_IT: u32 = 1;
    const SEED_IT_B: u32 = 2;
    const NOVEL: u32 = 3;
    const IT_LIKE: u32 = 4;

    fn profile(weight: ProfileWeight, seeds: &[&str]) -> Arc<Profile> {
        Arc::new(Profile::new(vec![ProfileDomain {
            id: "it".into(),
            name: "IT・ソフトウェア".into(),
            weight,
            seeds: seeds.iter().map(|s| (*s).to_owned()).collect(),
        }]))
    }

    fn ctx_with(p: Arc<Profile>) -> Context {
        Context::new().with_profile(Some(p))
    }

    fn vectors() -> Arc<dyn WordVectors> {
        Arc::new(FakeVectors::new(
            3,
            &[
                (SEED_IT, &[1.0, 0.0, 0.0]),
                (SEED_IT_B, &[1.0, 0.2, 0.0]),
                (NOVEL, &[0.0, 0.0, 1.0]),
                (IT_LIKE, &[0.9, 0.3, 0.0]),
            ],
        ))
    }

    /// 種語「品詞」「関数」を辞書に入れた環境
    fn env_with_seeds() -> Env {
        let mut env = Env::new();
        env.word("品詞", "品詞", Some(SEED_IT), POS_NOUN);
        env.word("関数", "関数", Some(SEED_IT_B), POS_NOUN);
        env
    }

    #[test]
    fn no_profile_or_empty_profile_changes_nothing() {
        let mut env = env_with_seeds();
        let seg = env.word("品詞", "品詞", Some(SEED_IT), POS_NOUN);
        let r = ProfileRanker::new(None, ProfileConfig::default());
        let mut cands = vec![cand(vec![seg], 5000)];
        env.rank(&r, &Context::new(), &AnalyzedContext::default(), &mut cands);
        assert_eq!(cands[0].score, 5000);
        let empty = Arc::new(Profile::default());
        env.rank(
            &r,
            &ctx_with(empty),
            &AnalyzedContext::default(),
            &mut cands,
        );
        assert_eq!(cands[0].score, 5000);
    }

    #[test]
    fn seed_match_without_vectors() {
        let mut env = Env::new();
        let hit = env.word("品詞", "品詞", None, POS_NOUN);
        let miss = env.word("品視", "品視", None, POS_NOUN);
        let r = ProfileRanker::new(None, ProfileConfig::default());
        let ctx = ctx_with(profile(ProfileWeight::OFTEN, &["品詞"]));
        let mut cands = vec![cand(vec![hit], 5000), cand(vec![miss], 5000)];
        env.rank(&r, &ctx, &AnalyzedContext::default(), &mut cands);
        assert_eq!(cands[0].score, 5000 - 800);
        assert_eq!(cands[1].score, 5000);
    }

    #[test]
    fn seed_match_uses_lemma_not_surface() {
        let mut env = Env::new();
        let seg = env.word("ユーザ", "ユーザー", None, POS_NOUN);
        let r = ProfileRanker::new(None, ProfileConfig::default());
        let ctx = ctx_with(profile(ProfileWeight::OFTEN, &["ユーザー"]));
        let mut cands = vec![cand(vec![seg], 5000)];
        env.rank(&r, &ctx, &AnalyzedContext::default(), &mut cands);
        assert_eq!(cands[0].score, 5000 - 800);
    }

    #[test]
    fn domain_weight_scales_the_bonus() {
        let mut env = Env::new();
        let seg = env.word("品詞", "品詞", None, POS_NOUN);
        let r = ProfileRanker::new(None, ProfileConfig::default());
        let ctx = ctx_with(profile(ProfileWeight::SOMETIMES, &["品詞"]));
        let mut cands = vec![cand(vec![seg], 5000)];
        env.rank(&r, &ctx, &AnalyzedContext::default(), &mut cands);
        assert_eq!(cands[0].score, 5000 - 400);
    }

    #[test]
    fn only_content_words_are_boosted() {
        let mut env = Env::new();
        let particle = env.word("に", "に", None, POS_PARTICLE);
        let non_independent = env.word("こと", "こと", None, POS_NON_INDEPENDENT);
        let r = ProfileRanker::new(None, ProfileConfig::default());
        let ctx = ctx_with(profile(ProfileWeight::OFTEN, &["に", "こと"]));
        let mut cands = vec![cand(vec![particle, non_independent], 100)];
        env.rank(&r, &ctx, &AnalyzedContext::default(), &mut cands);
        assert_eq!(cands[0].score, 100);
    }

    #[test]
    fn total_is_capped() {
        let mut env = Env::new();
        let words: Vec<_> = ["あ", "い", "う", "え"]
            .iter()
            .map(|s| env.word(s, s, None, POS_NOUN))
            .collect();
        let r = ProfileRanker::new(None, ProfileConfig::default());
        let ctx = ctx_with(profile(ProfileWeight::OFTEN, &["あ", "い", "う", "え"]));
        let mut cands = vec![cand(words, 9000)];
        env.rank(&r, &ctx, &AnalyzedContext::default(), &mut cands);
        // 4 * 800 = 3200 -> 1200
        assert_eq!(cands[0].score, 9000 - 1200);
    }

    #[test]
    fn bonus_decays_as_context_grows() {
        let mut env = Env::new();
        let seg = env.word("品詞", "品詞", None, POS_NOUN);
        let r = ProfileRanker::new(None, ProfileConfig::default());
        let ctx = ctx_with(profile(ProfileWeight::OFTEN, &["品詞"]));
        let bonus_at = |content: usize| {
            let mut cands = vec![cand(vec![seg.clone()], 5000)];
            let analysis = AnalyzedContext {
                content_count: content,
                ..AnalyzedContext::default()
            };
            env.rank(&r, &ctx, &analysis, &mut cands);
            5000 - cands[0].score
        };
        assert_eq!(bonus_at(0), 800);
        assert_eq!(bonus_at(20), 400);
        assert_eq!(bonus_at(60), 200);
        assert!(bonus_at(1000) < 20);
    }

    #[test]
    fn grammar_sized_differences_are_never_overturned() {
        // 加点の上限は 1200。BOS の連接で決まる 4000 以上の差は覆らない
        let mut env = Env::new();
        let seed = env.word("仕様", "仕様", None, POS_NOUN);
        let r = ProfileRanker::new(None, ProfileConfig::default());
        let ctx = ctx_with(profile(ProfileWeight::OFTEN, &["仕様"]));
        let mut cands = vec![cand(vec![seed.clone(), seed], 4500), cand(Vec::new(), 100)];
        env.rank(&r, &ctx, &AnalyzedContext::default(), &mut cands);
        assert!(cands[0].score >= 4500 - 1200);
        assert!(cands[0].score > cands[1].score);
    }

    #[test]
    fn vector_similarity_is_used_when_available() {
        let mut env = env_with_seeds();
        let it_like = env.word("類似", "類似", Some(IT_LIKE), POS_NOUN);
        let novel = env.word("物語", "物語", Some(NOVEL), POS_NOUN);
        let r = ProfileRanker::new(Some(vectors()), ProfileConfig::default());
        let ctx = ctx_with(profile(ProfileWeight::OFTEN, &["品詞", "関数"]));
        let mut cands = vec![cand(vec![it_like], 5000), cand(vec![novel], 5000)];
        env.rank(&r, &ctx, &AnalyzedContext::default(), &mut cands);
        // cos が高い（約 0.95）ので上限の 1200 に当たる。直交する語は 0
        assert_eq!(cands[0].score, 5000 - 1200);
        assert_eq!(cands[1].score, 5000);
    }

    #[test]
    fn vector_bonus_is_proportional_to_cos_above_min() {
        let mut env = env_with_seeds();
        let it_like = env.word("類似", "類似", Some(IT_LIKE), POS_NOUN);
        let cfg = ProfileConfig {
            k: 1000,
            max_total: 100_000,
            ..ProfileConfig::default()
        };
        let r = ProfileRanker::new(Some(vectors()), cfg);
        let ctx = ctx_with(profile(ProfileWeight::OFTEN, &["品詞"]));
        let mut cands = vec![cand(vec![it_like], 5000)];
        env.rank(&r, &ctx, &AnalyzedContext::default(), &mut cands);
        // cos(IT_LIKE, SEED_IT) = 0.9 / sqrt(0.9^2 + 0.3^2) ≒ 0.9487
        let bonus = 5000 - cands[0].score;
        assert!((780..=810).contains(&bonus), "{bonus}");
    }

    #[test]
    fn words_without_vectors_fall_back_to_seed_match() {
        let mut env = env_with_seeds();
        let seed_without_vector = env.word("固有語", "固有語", None, POS_NOUN);
        let r = ProfileRanker::new(Some(vectors()), ProfileConfig::default());
        let ctx = ctx_with(profile(ProfileWeight::OFTEN, &["品詞", "固有語"]));
        let mut cands = vec![cand(vec![seed_without_vector], 5000)];
        env.rank(&r, &ctx, &AnalyzedContext::default(), &mut cands);
        assert_eq!(cands[0].score, 5000 - 800);
    }

    #[test]
    fn unresolvable_seeds_fall_back_to_seed_match_for_every_word() {
        // 種語が辞書にない -> プロファイルのベクトルが作れない。種語の一致だけで動く
        let mut env = Env::new();
        let seg = env.word("品詞", "品詞", Some(SEED_IT), POS_NOUN);
        let r = ProfileRanker::new(Some(vectors()), ProfileConfig::default());
        let ctx = ctx_with(profile(ProfileWeight::OFTEN, &["品詞"]));
        // env に「品詞」は登録済み（word が辞書に足す）ので、ベクトルは作れる。作れない例として別の種語を使う
        let ctx_missing = ctx_with(profile(ProfileWeight::OFTEN, &["辞書にない語"]));
        let mut cands = vec![cand(vec![seg], 5000)];
        env.rank(&r, &ctx_missing, &AnalyzedContext::default(), &mut cands);
        assert_eq!(cands[0].score, 5000);
        env.rank(&r, &ctx, &AnalyzedContext::default(), &mut cands);
        assert!(cands[0].score < 5000);
    }

    #[test]
    fn secure_context_gets_nothing() {
        let mut env = Env::new();
        let seg = env.word("品詞", "品詞", None, POS_NOUN);
        let r = ProfileRanker::new(None, ProfileConfig::default());
        let ctx = ctx_with(profile(ProfileWeight::OFTEN, &["品詞"])).with_secure(true);
        let mut cands = vec![cand(vec![seg], 5000)];
        env.rank(&r, &ctx, &AnalyzedContext::default(), &mut cands);
        assert_eq!(cands[0].score, 5000);
    }

    #[test]
    fn prepared_profile_is_reused_and_refreshed_on_change() {
        let mut env = Env::new();
        let a = env.word("品詞", "品詞", None, POS_NOUN);
        let b = env.word("物語", "物語", None, POS_NOUN);
        let r = ProfileRanker::new(None, ProfileConfig::default());
        let p1 = profile(ProfileWeight::OFTEN, &["品詞"]);
        let p2 = profile(ProfileWeight::OFTEN, &["物語"]);
        for (p, hit_a) in [(&p1, true), (&p2, false), (&p1, true)] {
            let mut cands = vec![cand(vec![a.clone()], 1000), cand(vec![b.clone()], 1000)];
            env.rank(
                &r,
                &ctx_with(Arc::clone(p)),
                &AnalyzedContext::default(),
                &mut cands,
            );
            assert_eq!(cands[0].score < 1000, hit_a);
            assert_eq!(cands[1].score < 1000, !hit_a);
        }
    }

    #[test]
    fn config_from_kv() {
        let kv: BTreeMap<String, String> =
            [("profile.max_total", "50"), ("profile.min_cos", "0.3")]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect();
        let cfg = ProfileConfig::from_kv(&kv).unwrap();
        assert_eq!((cfg.max_total, cfg.min_cos), (50, 0.3));
        assert_eq!(cfg.k, 3000);
        let bad: BTreeMap<String, String> = [("profile.k".to_owned(), "-3".to_owned())]
            .into_iter()
            .collect();
        assert!(ProfileConfig::from_kv(&bad).is_err());
    }
}
