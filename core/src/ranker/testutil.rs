//! Ranker の単体テストが共有する小さな環境（テスト用の辞書 `TestDict` と `DenseGrammar` だけで組む）。
use std::collections::HashMap;

use crate::analysis::{AnalyzedContext, ContextToken};
use crate::candidate::{Candidate, CandidateOrigin, Segment};
use crate::context::Context;
use crate::dict::{DenseGrammar, Grammar, UnknownParams, WordId};
use crate::lattice::testutil::TestDict;
use crate::vectors::WordVectors;

use super::{RankInput, Ranker};

pub const POS_NOUN: u16 = 0;
pub const POS_PARTICLE: u16 = 1;
pub const POS_NON_INDEPENDENT: u16 = 2;
pub const POS_UNK: u16 = 3;

pub struct Env {
    pub grammar: DenseGrammar,
    pub dict: TestDict,
}

impl Env {
    pub fn new() -> Self {
        let mut grammar = DenseGrammar::new(
            4,
            4,
            0,
            UnknownParams {
                left_id: 3,
                right_id: 3,
                pos_id: POS_UNK,
                cost_per_char: 5000,
            },
        );
        grammar.add_pos("名詞,普通名詞,一般,*,*,*");
        grammar.add_pos("助詞,格助詞,*,*,*,*");
        grammar.add_pos("名詞,非自立可能,*,*,*,*");
        grammar.add_pos("記号,一般,*,*,*,*");
        let dict = TestDict::new(grammar.grammar_id());
        Self { grammar, dict }
    }

    /// 辞書に語を足してセグメントを返す。`lemma_id` があればベクトルを引ける。
    pub fn word(&mut self, surface: &str, lemma: &str, lemma_id: Option<u32>, pos: u16) -> Segment {
        let index = self.dict.add(surface, surface, 1, 1, 1000, pos);
        self.dict.set_lemma(index, lemma, lemma_id);
        Segment {
            reading: surface.to_owned(),
            surface: surface.to_owned(),
            word: Some(WordId { dict: 0, index }),
            left_id: 1,
            right_id: 1,
            pos_id: pos,
        }
    }

    pub fn rank(
        &self,
        ranker: &dyn Ranker,
        ctx: &Context,
        analysis: &AnalyzedContext,
        cands: &mut [Candidate],
    ) {
        let input = RankInput {
            reading: "てすと",
            context: ctx,
            grammar: &self.grammar,
            analysis,
            dict: &self.dict,
            deadline: None,
        };
        ranker.rank(&input, cands);
    }
}

pub fn cand(segments: Vec<Segment>, cost: i32) -> Candidate {
    Candidate {
        surface: segments.iter().map(|s| s.surface.as_str()).collect(),
        segments,
        base_cost: cost,
        score: cost,
        origin: CandidateOrigin::Dictionary,
    }
}

pub fn token(
    surface: &str,
    lemma: &str,
    lemma_id: Option<u32>,
    is_content: bool,
    char_end: u32,
) -> ContextToken {
    ContextToken {
        surface: surface.to_owned(),
        lemma: lemma.to_owned(),
        lemma_id,
        word: None,
        left_id: 1,
        right_id: 1,
        pos_id: POS_NOUN,
        is_content,
        char_end,
    }
}

pub fn analysis(tokens: Vec<ContextToken>) -> AnalyzedContext {
    let content_count = tokens.iter().filter(|t| t.is_content).count();
    AnalyzedContext {
        tokens,
        bos_right_id: 0,
        bos_alternatives: Vec::new(),
        content_count,
    }
}

/// 内容語のトークン n 個（ベクトルは lemma 番号 `lemma_id` のもの）の解析結果。
pub fn analysis_of(lemma_id: u32, n: usize) -> AnalyzedContext {
    analysis(
        (0..n)
            .map(|i| token("語", "語", Some(lemma_id), true, (n - i) as u32))
            .collect(),
    )
}

/// lemma 番号 -> 行ベクトル。f32 で渡して L2 正規化し、×127 で丸める（vectors.bin と同じ）。
pub struct FakeVectors {
    dims: usize,
    rows: HashMap<u32, Vec<i8>>,
}

impl FakeVectors {
    pub fn new(dims: usize, rows: &[(u32, &[f32])]) -> Self {
        let rows = rows
            .iter()
            .map(|&(id, v)| {
                assert_eq!(v.len(), dims);
                let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                let q = v.iter().map(|x| (x / norm * 127.0).round() as i8).collect();
                (id, q)
            })
            .collect();
        Self { dims, rows }
    }
}

impl WordVectors for FakeVectors {
    fn dims(&self) -> usize {
        self.dims
    }
    fn vector(&self, lemma_id: u32) -> Option<&[i8]> {
        self.rows.get(&lemma_id).map(Vec::as_slice)
    }
    fn nearest_cluster(&self, _v: &[f32]) -> Option<u16> {
        None
    }
    fn rank(&self, _lemma_id: u32) -> Option<u32> {
        None
    }
}
