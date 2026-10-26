//! 文書内キャッシュ（ADR 0002 §7.2）。メモリ内だけで持ち、保存しない（NFR-19）。
//!
//! 解析した左文脈の内容語と、確定履歴の表記を「最近この文書で使われた語」とみなし、
//! 候補の内容語セグメントがそれと一致したら加点する。状態は持たず、入力から毎回求める。

use std::collections::{BTreeMap, HashMap};

use super::common::{apply_bonus, content_segments, segment_lemma};
use super::config::ScopedKv;
use super::names;
use super::{RankInput, Ranker, RankerConfigError};
use crate::candidate::Candidate;

const CONFIG_PREFIX: &str = names::DOC_CACHE;
const CONFIG_KEYS: [&str; 4] = ["surface_bonus", "lemma_bonus", "near_chars", "max_total"];

/// 加点はコスト単位の大きさ（正の値）。Ranker は score からこの分を引く。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentCacheConfig {
    /// 同じ表記が窓の中にある
    pub surface_bonus: i32,
    /// 表記は違うが同じ lemma がある
    pub lemma_bonus: i32,
    /// これより遠い出現（文字）は加点を半分にする
    pub near_chars: u32,
    /// 1候補あたりの合計の上限
    pub max_total: i32,
}

impl Default for DocumentCacheConfig {
    fn default() -> Self {
        Self {
            surface_bonus: 1500,
            lemma_bonus: 800,
            near_chars: 500,
            max_total: 3000,
        }
    }
}

impl DocumentCacheConfig {
    /// `doc_cache.surface_bonus` のように、`doc_cache.` を前置したキーだけを読む。
    /// 他の接頭辞のキーは無視し、`doc_cache.` で未知のキーや不正な値ならエラー。
    pub fn from_kv(kv: &BTreeMap<String, String>) -> Result<Self, RankerConfigError> {
        let scope = ScopedKv::new(CONFIG_PREFIX, &CONFIG_KEYS, kv)?;
        let d = Self::default();
        Ok(Self {
            surface_bonus: scope.get("surface_bonus", d.surface_bonus)?,
            lemma_bonus: scope.get("lemma_bonus", d.lemma_bonus)?,
            near_chars: scope.get("near_chars", d.near_chars)?,
            max_total: scope.get("max_total", d.max_total)?,
        })
    }
}

pub struct DocumentCacheRanker {
    cfg: DocumentCacheConfig,
}

impl DocumentCacheRanker {
    pub fn new(cfg: DocumentCacheConfig) -> Self {
        Self { cfg }
    }
}

/// 窓の中の語と、その最も近い出現（左文脈の末尾からの文字数）。
#[derive(Default)]
struct Seen<'a> {
    surfaces: HashMap<&'a str, u32>,
    lemmas: HashMap<&'a str, u32>,
}

impl<'a> Seen<'a> {
    fn note(map: &mut HashMap<&'a str, u32>, key: &'a str, distance: u32) {
        map.entry(key)
            .and_modify(|d| *d = (*d).min(distance))
            .or_insert(distance);
    }
}

impl Ranker for DocumentCacheRanker {
    fn rank(&self, input: &RankInput<'_>, cands: &mut [Candidate]) {
        if input.context.secure {
            return;
        }
        // 解析結果の内容語（secure なら空）と、確定履歴の表記。履歴は直近の確定なので距離 0 とみなす
        let mut seen = Seen::default();
        for t in input.analysis.tokens.iter().filter(|t| t.is_content) {
            Seen::note(&mut seen.surfaces, &t.surface, t.char_end);
            Seen::note(&mut seen.lemmas, &t.lemma, t.char_end);
        }
        for commit in &input.context.history {
            Seen::note(&mut seen.surfaces, &commit.surface, 0);
            Seen::note(&mut seen.lemmas, &commit.surface, 0);
        }
        if seen.surfaces.is_empty() {
            return;
        }

        for cand in cands.iter_mut() {
            let mut total: i32 = 0;
            for seg in content_segments(input.grammar, cand) {
                let (lemma, _) = segment_lemma(input.dict, seg);
                let (bonus, distance) = if let Some(&d) = seen.surfaces.get(seg.surface.as_str()) {
                    (self.cfg.surface_bonus, d)
                } else if let Some(&d) = seen.lemmas.get(lemma) {
                    (self.cfg.lemma_bonus, d)
                } else {
                    continue;
                };
                let bonus = if distance > self.cfg.near_chars {
                    bonus / 2
                } else {
                    bonus
                };
                total = total.saturating_add(bonus);
            }
            apply_bonus(cand, total.min(self.cfg.max_total));
        }
    }

    fn name(&self) -> &str {
        names::DOC_CACHE
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::analysis::AnalyzedContext;
    use crate::candidate::{Candidate, CandidateOrigin};
    use crate::context::{Commit, Context};

    fn ranker() -> DocumentCacheRanker {
        DocumentCacheRanker::new(DocumentCacheConfig::default())
    }

    #[test]
    fn same_surface_in_window_gets_surface_bonus() {
        let mut env = Env::new();
        let hinshi = env.word("品詞", "品詞", Some(1), POS_NOUN);
        let analysis = analysis(vec![token("品詞", "品詞", Some(1), true, 10)]);
        let mut cands = vec![cand(vec![hinshi], 5000)];
        env.rank(&ranker(), &Context::new(), &analysis, &mut cands);
        assert_eq!(cands[0].score, 5000 - 1500);
        assert_eq!(cands[0].base_cost, 5000);
    }

    #[test]
    fn same_lemma_with_different_surface_gets_lemma_bonus() {
        let mut env = Env::new();
        let user = env.word("ユーザ", "ユーザー", Some(2), POS_NOUN);
        let analysis = analysis(vec![token("ユーザー", "ユーザー", Some(2), true, 10)]);
        let mut cands = vec![cand(vec![user], 5000)];
        env.rank(&ranker(), &Context::new(), &analysis, &mut cands);
        assert_eq!(cands[0].score, 5000 - 800);
    }

    #[test]
    fn only_content_words_are_boosted() {
        let mut env = Env::new();
        let particle = env.word("に", "に", Some(3), POS_PARTICLE);
        let non_independent = env.word("こと", "こと", Some(4), POS_NON_INDEPENDENT);
        // 窓に「に」「こと」があっても（トークン側が内容語でないので）、候補側も内容語でないので加点しない
        let analysis = analysis(vec![
            token("に", "に", Some(3), false, 3),
            token("こと", "こと", Some(4), false, 2),
            token("に", "に", Some(3), true, 1), // 仮に内容語とされていても、候補が機能語なら無視
        ]);
        let mut cands = vec![cand(vec![particle, non_independent], 100)];
        env.rank(&ranker(), &Context::new(), &analysis, &mut cands);
        assert_eq!(cands[0].score, 100);
    }

    #[test]
    fn total_is_capped() {
        let mut env = Env::new();
        let words: Vec<_> = (0..5)
            .map(|i| env.word(&format!("語{i}"), &format!("語{i}"), Some(10 + i), POS_NOUN))
            .collect();
        let tokens = (0..5)
            .map(|i| token(&format!("語{i}"), &format!("語{i}"), Some(10 + i), true, 5))
            .collect();
        let mut cands = vec![cand(words, 9000)];
        env.rank(&ranker(), &Context::new(), &analysis(tokens), &mut cands);
        assert_eq!(cands[0].score, 9000 - 3000);
    }

    #[test]
    fn far_occurrences_get_half() {
        let mut env = Env::new();
        let far = env.word("遠い", "遠い", Some(5), POS_NOUN);
        let near = env.word("近い", "近い", Some(6), POS_NOUN);
        let analysis = analysis(vec![
            token("遠い", "遠い", Some(5), true, 501),
            token("近い", "近い", Some(6), true, 500),
        ]);
        let mut cands = vec![cand(vec![far], 1000), cand(vec![near], 1000)];
        env.rank(&ranker(), &Context::new(), &analysis, &mut cands);
        assert_eq!(cands[0].score, 1000 - 750);
        assert_eq!(cands[1].score, 1000 - 1500);
    }

    #[test]
    fn nearest_occurrence_decides_the_distance() {
        let mut env = Env::new();
        let w = env.word("春", "春", Some(7), POS_NOUN);
        let analysis = analysis(vec![
            token("春", "春", Some(7), true, 900),
            token("春", "春", Some(7), true, 20),
        ]);
        let mut cands = vec![cand(vec![w], 1000)];
        env.rank(&ranker(), &Context::new(), &analysis, &mut cands);
        assert_eq!(cands[0].score, 1000 - 1500);
    }

    #[test]
    fn history_surfaces_count_even_without_analysis_tokens() {
        let mut env = Env::new();
        let hinshi = env.word("品詞", "品詞", Some(1), POS_NOUN);
        let other = env.word("品視", "品視", Some(8), POS_NOUN);
        let ctx = Context::new().with_history(vec![Commit {
            reading: "ひんし".into(),
            surface: "品詞".into(),
        }]);
        let mut cands = vec![cand(vec![hinshi], 5000), cand(vec![other], 5000)];
        env.rank(&ranker(), &ctx, &AnalyzedContext::default(), &mut cands);
        assert_eq!(cands[0].score, 5000 - 1500);
        assert_eq!(cands[1].score, 5000);
    }

    #[test]
    fn non_dictionary_candidates_and_unknown_segments_are_ignored() {
        let mut env = Env::new();
        let w = env.word("春", "春", Some(7), POS_NOUN);
        let analysis = analysis(vec![token("春", "春", Some(7), true, 1)]);
        let mut kana = cand(vec![w.clone()], 100);
        kana.origin = CandidateOrigin::Hiragana;
        let mut unknown_seg = w;
        unknown_seg.word = None;
        let mut cands: Vec<Candidate> = vec![kana, cand(vec![unknown_seg], 100)];
        env.rank(&ranker(), &Context::new(), &analysis, &mut cands);
        assert_eq!(cands[0].score, 100);
        assert_eq!(cands[1].score, 100);
    }

    #[test]
    fn empty_context_changes_nothing() {
        let mut env = Env::new();
        let w = env.word("春", "春", Some(7), POS_NOUN);
        let mut cands = vec![cand(vec![w], 100)];
        env.rank(
            &ranker(),
            &Context::new(),
            &AnalyzedContext::default(),
            &mut cands,
        );
        assert_eq!(cands[0].score, 100);
    }

    #[test]
    fn config_from_kv() {
        let kv: BTreeMap<String, String> = [
            ("doc_cache.surface_bonus", "2000"),
            ("doc_cache.max_total", "10"),
            ("topic.weight", "1"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let cfg = DocumentCacheConfig::from_kv(&kv).unwrap();
        assert_eq!(cfg.surface_bonus, 2000);
        assert_eq!(cfg.max_total, 10);
        assert_eq!(cfg.lemma_bonus, DocumentCacheConfig::default().lemma_bonus);
        assert_eq!(
            DocumentCacheConfig::from_kv(&BTreeMap::new()).unwrap(),
            DocumentCacheConfig::default()
        );
        let bad: BTreeMap<String, String> = [("doc_cache.typo".to_owned(), "1".to_owned())]
            .into_iter()
            .collect();
        assert!(DocumentCacheConfig::from_kv(&bad).is_err());
    }
}
