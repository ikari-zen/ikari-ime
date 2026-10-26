//! LearningGuard: 文脈に依らない学習の加点が、現在の文脈の証拠を覆すことを禁じる（spec の基本原則）。
//!
//! 「その場の文脈に、学習と食い違う証拠があるときは、学習の加点で文脈の判断を覆さない」。
//! 学習の加点のうち、現在の文脈の特徴量（直前の語、語の袋、話題）に一致して付いた部分は、
//! 「この文脈でこれを選んだ」という記録なので制限しない。制限するのは、文脈に依らない部分
//! （`Ranker::context_free_bonus`。Unconditional の特徴量）だけ。
//!
//! 規則:
//! 1. 文脈に依らない加点を除いた score で最良の候補 b を求める（文脈と、文脈つきの学習の判断）。
//! 2. 文脈に依らない加点のおかげで b の前に出た候補 j が、b より弱い文脈の証拠しか持たない
//!    （証拠 = 左文脈の連接、文書内キャッシュ、プロファイル、話題の差分の合計。負が支持）ときは、
//!    j の score を「b の score + `margin`」まで戻す。
//! 3. 証拠に差がなければ（どちらの候補も文脈が支持しない）、学習が順位を決めてよい。
//!
//! 戻すときは、戻す候補どうしの順序を保ったまま平行移動する。

use crate::candidate::Candidate;

const DEFAULT_MARGIN: i32 = 1;
/// 証拠の差がこれ以上あるとき「支持している」とみなす（コスト）
const MIN_EVIDENCE_GAP: i32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LearningGuardConfig {
    /// false なら何もしない（比較実験用）
    pub enabled: bool,
    /// 戻した候補を、文脈の最良の候補よりこれだけ下に置く
    pub margin: i32,
}

impl Default for LearningGuardConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            margin: DEFAULT_MARGIN,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LearningGuard {
    cfg: LearningGuardConfig,
}

impl LearningGuard {
    pub(crate) fn new(cfg: LearningGuardConfig) -> Self {
        Self { cfg }
    }

    pub(crate) fn config(&self) -> &LearningGuardConfig {
        &self.cfg
    }

    /// `evidence[i]` は候補 i への文脈の差分の合計（負が支持）、`free[i]` は学習の加点のうち
    /// 文脈に依らない部分（正が加点。すでに score に反映済み）。どちらも `cands` と同じ長さ。違えば何もしない。
    pub(crate) fn apply(&self, cands: &mut [Candidate], evidence: &[i32], free: &[i32]) {
        if !self.cfg.enabled
            || evidence.len() != cands.len()
            || free.len() != cands.len()
            || free.iter().all(|&f| f <= 0)
        {
            return;
        }
        let without_free = |i: usize| i64::from(cands[i].score) + i64::from(free[i].max(0));
        // 文脈に依らない加点を除いた最良の候補（同点なら先のもの）
        let Some(best) = (0..cands.len()).min_by_key(|&i| without_free(i)) else {
            return;
        };
        let best_score = cands[best].score;
        let limit = i64::from(best_score) + i64::from(self.cfg.margin);

        let mut flipped: Vec<usize> = (0..cands.len())
            .filter(|&j| {
                j != best
                    && free[j] > 0
                    && cands[j].score <= best_score
                    && i64::from(evidence[best]) + i64::from(MIN_EVIDENCE_GAP)
                        <= i64::from(evidence[j])
            })
            .collect();
        let Some(lowest) = flipped.iter().map(|&j| cands[j].score).min() else {
            return;
        };
        let shift = limit - i64::from(lowest);
        flipped.sort_unstable();
        for j in flipped {
            let moved =
                (i64::from(cands[j].score) + shift).clamp(i64::from(i32::MIN), i64::from(i32::MAX));
            cands[j].score = moved as i32;
        }
    }
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
    fn context_free_bonus_cannot_flip_a_context_supported_candidate() {
        // 瀕死 は文脈（話題）が 3000 支持している。品詞 は文脈なしの学習で 600 加点されて前に出た
        let mut c = vec![cand("瀕死", 5000), cand("品詞", 4900)];
        LearningGuard::default().apply(&mut c, &[-3000, 0], &[0, 600]);
        assert_eq!(scores(&c), vec![5000, 5001]);
    }

    #[test]
    fn learning_decides_when_the_context_has_no_opinion() {
        let mut c = vec![cand("瀕死", 5000), cand("品詞", 4900)];
        LearningGuard::default().apply(&mut c, &[0, 0], &[0, 600]);
        assert_eq!(scores(&c), vec![5000, 4900]);
        // 文脈が学習された候補のほうを支持するときも、そのまま
        let mut c = vec![cand("瀕死", 5000), cand("品詞", 4900)];
        LearningGuard::default().apply(&mut c, &[0, -2000], &[0, 600]);
        assert_eq!(scores(&c), vec![5000, 4900]);
    }

    #[test]
    fn context_matched_learning_is_not_limited() {
        // 学習の加点 2500 はすべて文脈つきの特徴量（free は 0）。文脈の証拠が反対でも制限しない
        let mut c = vec![cand("瀕死", 5000), cand("品詞", 4000)];
        LearningGuard::default().apply(&mut c, &[-3000, 0], &[0, 0]);
        assert_eq!(scores(&c), vec![5000, 4000]);
    }

    #[test]
    fn candidates_that_were_ahead_without_the_free_bonus_stay() {
        // 品詞は文脈なしの加点がなくても前にいる
        let mut c = vec![cand("瀕死", 5000), cand("品詞", 4000)];
        LearningGuard::default().apply(&mut c, &[-3000, 0], &[0, 600]);
        // 加点を除くと 4600 で瀕死(5000)より前 -> best は 品詞 自身。戻さない
        assert_eq!(scores(&c), vec![5000, 4000]);
    }

    #[test]
    fn order_among_flipped_candidates_is_kept() {
        let mut c = vec![cand("a", 5000), cand("b", 4900), cand("c", 4950)];
        LearningGuard::default().apply(&mut c, &[-3000, 0, 0], &[0, 600, 600]);
        assert_eq!(scores(&c), vec![5000, 5001, 5051]);
    }

    #[test]
    fn disabled_or_mismatched_inputs_do_nothing() {
        let mut c = vec![cand("瀕死", 5000), cand("品詞", 4900)];
        let off = LearningGuard::new(LearningGuardConfig {
            enabled: false,
            ..LearningGuardConfig::default()
        });
        off.apply(&mut c, &[-3000, 0], &[0, 600]);
        assert_eq!(scores(&c), vec![5000, 4900]);
        LearningGuard::default().apply(&mut c, &[-3000], &[0, 600]);
        assert_eq!(scores(&c), vec![5000, 4900]);
    }
}
