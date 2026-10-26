//! ベース実装。score = base_cost のまま何もしない。

use super::names;
use super::{RankInput, Ranker};
use crate::candidate::Candidate;

pub struct CostRanker;

impl Ranker for CostRanker {
    fn rank(&self, _input: &RankInput<'_>, _cands: &mut [Candidate]) {}

    fn name(&self) -> &str {
        names::COST
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::AnalyzedContext;
    use crate::candidate::CandidateOrigin;
    use crate::context::Context;
    use crate::dict::{DenseGrammar, Grammar, InMemoryDictionary, UnknownParams};

    #[test]
    fn cost_ranker_leaves_scores_untouched() {
        let g = DenseGrammar::new(
            1,
            1,
            0,
            UnknownParams {
                left_id: 0,
                right_id: 0,
                pos_id: 0,
                cost_per_char: 0,
            },
        );
        let ctx = Context::new();
        let analysis = AnalyzedContext::default();
        let dict = InMemoryDictionary::new(g.grammar_id());
        let input = RankInput {
            reading: "あ",
            context: &ctx,
            grammar: &g,
            analysis: &analysis,
            dict: &dict,
            deadline: None,
        };
        let mut cands = vec![Candidate {
            surface: "亜".into(),
            segments: Vec::new(),
            base_cost: 42,
            score: 42,
            origin: CandidateOrigin::Dictionary,
        }];
        CostRanker.rank(&input, &mut cands);
        assert_eq!(cands[0].score, 42);
        assert_eq!(CostRanker.name(), "cost");
    }
}
