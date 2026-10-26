//! GrammarGuard: 内容語の加点が文法を覆すことを禁じる（ADR 0003 §3.2、ADR 0004 §11.2 Q）。
//!
//! 背景: `TopicRanker` の既定値（`min_content` 1、`weight` 10000）は精度を大きく上げたが、
//! 話題の加点が文法で決まる箇所を覆す誤りを生む（例: 「サーバーを再起動」+ しよう で「使用」が1位）。
//! LM が使えないとき（読み込み中、時間切れ、GPU が混んでいる）にも同じ誤りが出てはいけない。
//!
//! 規則:
//! - 「文法候補」= `LeftContextRanker` が加点（負の差分）した候補。
//! - それ以外の候補が、**内容語に基づく加点**（DocumentCache、Profile、Topic の差分の合計）によって、
//!   最良の文法候補を追い越したら、その候補の score を「最良の文法候補の score + `margin`」まで戻す。
//! - 内容語の加点を除いても既に最良の文法候補より上にいた候補は、そのままにする。
//! - 学習と LM の差分は制限しない（学習はユーザーの明示的な選択、LM は文法を見ている）。
//!
//! この型は差分の合計を受け取って score を直すだけの部品で、差分を数えるのは
//! [`CompositeRanker`](super::CompositeRanker)（各 Ranker の前後の score を比べる）。

use crate::candidate::{Candidate, CandidateOrigin, Segment};
use crate::dict::Grammar;
use crate::pos::is_content_word;

const DEFAULT_MARGIN: i32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GrammarGuardConfig {
    /// false なら何もしない（比較実験用）
    pub enabled: bool,
    /// 戻した候補を、最良の文法候補よりこれだけ下に置く
    pub margin: i32,
}

impl Default for GrammarGuardConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            margin: DEFAULT_MARGIN,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct GrammarGuard {
    cfg: GrammarGuardConfig,
}

impl GrammarGuard {
    pub(crate) fn new(cfg: GrammarGuardConfig) -> Self {
        Self { cfg }
    }

    pub(crate) fn config(&self) -> &GrammarGuardConfig {
        &self.cfg
    }

    /// `grammar[i]` は候補 i への文法の差分（負が加点）、`content[i]` は内容語の差分の合計（負が加点）。
    /// どちらも `cands` と同じ長さ。違えば何もしない。
    pub(crate) fn apply(&self, cands: &mut [Candidate], grammar: &[i32], content: &[i32]) {
        if !self.cfg.enabled || grammar.len() != cands.len() || content.len() != cands.len() {
            return;
        }
        // 最良の文法候補（score が最小。同点なら先のもの）
        let Some(best) = (0..cands.len())
            .filter(|&i| grammar[i] < 0)
            .min_by_key(|&i| cands[i].score)
        else {
            return;
        };
        let best_score = cands[best].score;
        let best_without_content = i64::from(best_score) - i64::from(content[best]);
        let limit = best_score.saturating_add(self.cfg.margin);

        // 内容語の加点で追い越した候補（内容語の加点を除くと、最良の文法候補より下にいた）
        let mut overtakers: Vec<usize> = (0..cands.len())
            .filter(|&i| {
                grammar[i] >= 0
                    && content[i] < 0
                    && cands[i].score <= best_score
                    && i64::from(cands[i].score) - i64::from(content[i]) >= best_without_content
            })
            .collect();
        let Some(lowest) = overtakers.iter().map(|&i| cands[i].score).min() else {
            return;
        };
        // 全員を同じ値にすると、内容語どうしの順位が失われる。順序を保ったまま、
        // 先頭が `limit` に来るように平行移動する
        let shift = i64::from(limit) - i64::from(lowest);
        overtakers.sort_unstable();
        for i in overtakers {
            let moved =
                (i64::from(cands[i].score) + shift).clamp(i64::from(i32::MIN), i64::from(i32::MAX));
            cands[i].score = moved as i32;
        }
    }
}

/// 活用形の列（品詞名の6番目）。
const POS_FORM_FIELD: usize = 5;
/// 単独では単位を終えない活用形の接頭辞
const CONTINUING_FORMS: [&str; 4] = ["連用形", "未然形", "語幹", "仮定形"];

/// 「助詞 + 非自立の動詞」の候補に付ける印（`GrammarGuard` は負の値を文法候補とみなす）
const POS1_PARTICLE: &str = "助詞";
const POS1_VERB: &str = "動詞";
const POS1_AUX_VERB: &str = "助動詞";
pub(in crate::ranker) const PARTICLE_AUX_MARK: i32 = -1;

/// 「助詞 + 非自立の動詞や助動詞」（に + する、と + なる など）の候補か。辞書の語だけでできていて、
/// どの語も内容語でない。機能語の連なりのうち、これに限る（い + し のような助詞の羅列の分割は含めない）。
pub(in crate::ranker) fn is_particle_plus_auxiliary(
    grammar: &dyn Grammar,
    cand: &Candidate,
) -> bool {
    if cand.origin != CandidateOrigin::Dictionary || cand.segments.len() < 2 {
        return false;
    }
    let pos_of = |seg: &Segment| {
        seg.word
            .and_then(|_| grammar.pos_name(seg.pos_id))
            .filter(|name| !is_content_word(name))
    };
    let names: Option<Vec<&str>> = cand.segments.iter().map(pos_of).collect();
    let Some(names) = names else {
        return false;
    };
    names[0].starts_with(POS1_PARTICLE)
        && names[1..]
            .iter()
            .all(|n| n.starts_with(POS1_VERB) || n.starts_with(POS1_AUX_VERB))
}

/// 1語でできた候補が、連用形などの「続く形」か。
pub(in crate::ranker) fn is_continuing_form(grammar: &dyn Grammar, cand: &Candidate) -> bool {
    let [seg] = cand.segments.as_slice() else {
        return false;
    };
    grammar
        .pos_name(seg.pos_id)
        .and_then(|name| name.split(',').nth(POS_FORM_FIELD))
        .is_some_and(|form| CONTINUING_FORMS.iter().any(|f| form.starts_with(f)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::CandidateOrigin;

    fn cand(surface: &str, score: i32) -> Candidate {
        Candidate {
            surface: surface.to_owned(),
            segments: Vec::new(),
            base_cost: score,
            score,
            origin: CandidateOrigin::Dictionary,
        }
    }

    fn scores(c: &[Candidate]) -> Vec<i32> {
        c.iter().map(|c| c.score).collect()
    }

    #[test]
    fn content_bonus_cannot_overtake_the_best_grammar_candidate() {
        // しよう（文法 -3000、現在 5000）と 使用（内容語 -4000 で 4000 まで上がった）
        let mut c = vec![cand("しよう", 5000), cand("使用", 4000)];
        GrammarGuard::default().apply(&mut c, &[-3000, 0], &[0, -4000]);
        assert_eq!(scores(&c), vec![5000, 5001]);
    }

    #[test]
    fn a_candidate_already_above_without_the_content_bonus_is_left_alone() {
        // 使用は内容語の加点を除いても 3500 で、しよう（5000）より上
        let mut c = vec![cand("しよう", 5000), cand("使用", 3500)];
        GrammarGuard::default().apply(&mut c, &[-3000, 0], &[0, -500]);
        assert_eq!(scores(&c), vec![5000, 3500]);
    }

    #[test]
    fn the_grammar_candidate_own_content_bonus_counts_in_its_score() {
        // 文法候補自身が内容語で 1000 加点されている（score 4000）。使用は 3000 + 内容語 -2500 = 5500 を除くと
        // 5500 で、しよう（内容語を除くと 5000）より下だった
        let mut c = vec![cand("しよう", 4000), cand("使用", 3000)];
        GrammarGuard::default().apply(&mut c, &[-3000, 0], &[-1000, -2500]);
        assert_eq!(scores(&c), vec![4000, 4001]);
    }

    #[test]
    fn relative_order_of_the_clamped_candidates_is_kept() {
        let mut c = vec![cand("しよう", 5000), cand("使用", 3000), cand("仕様", 4000)];
        GrammarGuard::default().apply(&mut c, &[-3000, 0, 0], &[0, -4000, -4000]);
        assert_eq!(scores(&c), vec![5000, 5001, 6001]);
    }

    #[test]
    fn candidates_below_the_grammar_candidate_are_untouched() {
        let mut c = vec![cand("しよう", 5000), cand("使用", 6000), cand("仕様", 7000)];
        GrammarGuard::default().apply(&mut c, &[-3000, 0, 0], &[0, -100, 0]);
        assert_eq!(scores(&c), vec![5000, 6000, 7000]);
    }

    #[test]
    fn without_a_grammar_candidate_nothing_changes() {
        let mut c = vec![cand("使用", 3000), cand("仕様", 4000)];
        GrammarGuard::default().apply(&mut c, &[0, 0], &[-4000, -4000]);
        assert_eq!(scores(&c), vec![3000, 4000]);
    }

    #[test]
    fn disabled_guard_and_bad_lengths_do_nothing() {
        let mut c = vec![cand("しよう", 5000), cand("使用", 4000)];
        let off = GrammarGuard::new(GrammarGuardConfig {
            enabled: false,
            ..GrammarGuardConfig::default()
        });
        off.apply(&mut c, &[-3000, 0], &[0, -4000]);
        assert_eq!(scores(&c), vec![5000, 4000]);
        GrammarGuard::default().apply(&mut c, &[-3000], &[0, -4000]);
        assert_eq!(scores(&c), vec![5000, 4000]);
    }

    #[test]
    fn margin_and_overflow() {
        let mut c = vec![cand("しよう", i32::MAX - 1), cand("使用", 0)];
        GrammarGuard::new(GrammarGuardConfig {
            margin: 10,
            ..GrammarGuardConfig::default()
        })
        .apply(&mut c, &[-1, 0], &[0, -i32::MAX]);
        assert_eq!(scores(&c), vec![i32::MAX - 1, i32::MAX]);
    }
}
