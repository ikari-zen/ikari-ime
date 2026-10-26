//! 各 Ranker が共有する候補のセグメントの扱い。

use crate::candidate::{Candidate, CandidateOrigin, Segment};
use crate::dict::{Dictionary, Grammar};
use crate::pos::is_content_word;

/// 加点（正の値）を score から引く。
pub(super) fn apply_bonus(cand: &mut Candidate, bonus: i32) {
    cand.score = cand.score.saturating_sub(bonus);
}

/// Ranker が見るセグメント: 辞書由来の候補の、辞書の語で、品詞が内容語のもの。
/// かな候補、Rewriter の候補、未知語は対象にしない。
pub(super) fn content_segments<'a>(
    grammar: &'a dyn Grammar,
    cand: &'a Candidate,
) -> impl Iterator<Item = &'a Segment> {
    let dictionary_origin = cand.origin == CandidateOrigin::Dictionary;
    cand.segments.iter().filter(move |seg| {
        dictionary_origin
            && seg.word.is_some()
            && grammar.pos_name(seg.pos_id).is_some_and(is_content_word)
    })
}

/// セグメントの lemma と lemma 番号。辞書が lemma を持たなければ表記を lemma とみなす。
pub(super) fn segment_lemma<'a>(
    dict: &'a dyn Dictionary,
    seg: &'a Segment,
) -> (&'a str, Option<u32>) {
    match seg.word.and_then(|w| dict.lemma(w)) {
        Some(l) => (l.text, l.id),
        None => (&seg.surface, None),
    }
}
