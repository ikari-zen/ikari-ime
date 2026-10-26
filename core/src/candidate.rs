//! 変換候補（ADR 0001 §4.3）。

use crate::dict::WordId;

#[derive(Clone, Debug)]
pub struct Segment {
    pub reading: String,
    pub surface: String,
    /// 未知語ノードなら None
    pub word: Option<WordId>,
    pub left_id: u16,
    pub right_id: u16,
    /// 未知語は `UnknownParams.pos_id`
    pub pos_id: u16,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateOrigin {
    Dictionary,
    Hiragana,
    Katakana,
    /// Rewriter の候補。学習は表記ではなく書式ID（`format`）で行う
    Rewriter {
        name: &'static str,
        format: u16,
    },
    /// 再変換で読めなかった部分の、元の表記だけの候補（ADR 0006 §9.3）。学習しない
    Literal,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub surface: String,
    /// 語の分割（FR-06。「へんかんする」なら 変換 + する）
    pub segments: Vec<Segment>,
    /// パス全体のコスト（単語コストと連接コスト）
    pub base_cost: i32,
    /// Ranker が付けた最終スコア。小さいほど上位。初期値は base_cost
    pub score: i32,
    pub origin: CandidateOrigin,
}
