//! モジュールをまたいで使うテスト用の部品。

use std::sync::Arc;
use std::time::Duration;

use ikari_core::{
    CandidateOrigin, ConvertOptions, Converter, CostRanker, DenseGrammar, Dictionary, Grammar,
    InMemoryDictionary, Profile, ProfileDomain, ProfileWeight, RankInput, Ranker, UnknownParams,
};

use crate::{CaseResult, EvalCase, load_cases};

pub(crate) fn d(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

pub(crate) fn result(id: &str, kind: &str, tags: &[&str], rank: Option<usize>) -> CaseResult {
    CaseResult {
        case: EvalCase {
            id: id.into(),
            kind: kind.into(),
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
            profile: None,
            context: "ctx|x".into(),
            right: String::new(),
            field: None,
            history: String::new(),
            reading: "よみ".into(),
            expected: "期待".into(),
            note: String::new(),
            split: None,
        },
        rank,
        top1: Some("一位".into()),
        cold: d(5),
        warm: vec![d(1), d(2), d(3)],
    }
}

pub(crate) fn mock_grammar() -> DenseGrammar {
    let mut g = DenseGrammar::new(
        2,
        2,
        0,
        UnknownParams {
            left_id: 1,
            right_id: 1,
            pos_id: 0,
            cost_per_char: 5000,
        },
    );
    g.add_pos("名詞,普通名詞,一般,*,*,*");
    g
}

pub(crate) fn mock_converter_with(ranker: Arc<dyn Ranker>) -> Converter {
    let g = mock_grammar();
    let mut dict = InMemoryDictionary::new(g.grammar_id());
    dict.insert("ひんし", "品詞", 1, 1, 100, 0);
    dict.insert("ひんし", "瀕死", 1, 1, 200, 0);
    let (dict, g): (Arc<dyn Dictionary>, Arc<dyn Grammar>) = (Arc::new(dict), Arc::new(g));
    Converter::new(dict, g, ranker, ConvertOptions::default()).unwrap()
}

pub(crate) fn mock_converter() -> Converter {
    mock_converter_with(Arc::new(CostRanker))
}

pub(crate) fn mock_cases() -> Vec<EvalCase> {
    let tsv = "id\tkind\ttags\tcontext\treading\texpected\n\
               m-1\t話題\t\tctx\tひんし\t品詞\n\
               m-2\t話題\t怒り\tctx\tひんし\t瀕死\n\
               m-3\t否定\t\tctx\tひんし\tなし\n\
               m-4\t機能語\t\t\tヒンシ\tヒンシ\n";
    load_cases(tsv).unwrap()
}

/// プロファイルに種語を持つ領域があれば、その表記を加点する Ranker（A/B のテスト用）
pub(crate) struct ProfileBoostRanker {
    pub boost: &'static str,
}

impl Ranker for ProfileBoostRanker {
    fn rank(&self, input: &RankInput<'_>, cands: &mut [ikari_core::Candidate]) {
        let Some(profile) = &input.context.profile else {
            return;
        };
        if profile.is_empty() {
            return;
        }
        for c in cands.iter_mut() {
            if c.surface == self.boost && !matches!(c.origin, CandidateOrigin::Rewriter { .. }) {
                c.score -= 10_000;
            }
        }
    }

    fn name(&self) -> &str {
        "profile-boost"
    }
}

pub(crate) fn test_profile(id: &str) -> Arc<Profile> {
    Arc::new(Profile::new(vec![ProfileDomain {
        id: id.to_owned(),
        name: id.to_uppercase(),
        weight: ProfileWeight::OFTEN,
        seeds: vec!["瀕死".to_owned()],
    }]))
}
