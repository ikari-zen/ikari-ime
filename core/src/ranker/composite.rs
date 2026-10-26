//! Ranker を順に適用する（ADR 0002 §7.1）。
//! 既定の並び: DocumentCache -> Learning -> Profile -> Topic -> Zenz（存在するものだけ）。
//!
//! 共有の部品は `config.rs`（`ScopedKv`）と `common.rs`（候補のセグメントの扱い）にある。

use std::sync::Arc;

use super::english_first;
use super::guard::grammar::{
    GrammarGuard, GrammarGuardConfig, PARTICLE_AUX_MARK, is_continuing_form,
    is_particle_plus_auxiliary,
};
use super::guard::learning::{LearningGuard, LearningGuardConfig};
use super::{RankInput, Ranker, names};
use crate::candidate::Candidate;
use crate::lattice::has_noun_suru_edge;

/// 文法の差分を作る Ranker の名前（GrammarGuard が「文法候補」を見つけるのに使う）
const GRAMMAR_RANKER: &str = names::LEFT_CONTEXT;
/// 内容語に基づく差分を作る Ranker の名前（GrammarGuard が制限する対象）
const CONTENT_RANKERS: [&str; 3] = [names::DOC_CACHE, names::PROFILE, names::TOPIC];
/// 学習の Ranker の名前（LearningGuard が、文脈に依らない加点を取り出すのに使う）
const LEARNING_RANKER: &str = names::LEARNING;
/// 文脈・学習・表記の Ranker の名前。これ以外の最初の Ranker（LM）の直前が、B40 の段の「LM の前」
const NON_LM_RANKERS: [&str; 7] = [
    names::COST,
    names::LEFT_CONTEXT,
    names::KANA_STYLE,
    names::DOC_CACHE,
    names::LEARNING,
    names::PROFILE,
    names::TOPIC,
];

pub struct CompositeRanker {
    rankers: Vec<Arc<dyn Ranker>>,
    grammar_guard: GrammarGuard,
    learning_guard: LearningGuard,
    /// 2つの Guard を適用する位置（文脈の Ranker と学習の Ranker のうち最後のものの番号）。
    /// 条件に合う並びでなければ None（何もしない）
    guard_after: Option<usize>,
    /// LM（`NON_LM_RANKERS` にない最初の Ranker）の番号。なければ None（B40 は働かない）
    lm_at: Option<usize>,
    /// B40（ADR 0024）: LM が規則 S の守った1位を覆さない。規則 S を止めたとき（`penalty == 0`）は false
    keep_noun_suru: bool,
}

impl CompositeRanker {
    /// GrammarGuard と LearningGuard を既定の設定で有効にする（条件に合う Ranker がなければ働かない）。
    pub fn new(rankers: Vec<Arc<dyn Ranker>>) -> Self {
        Self::with_guards(
            rankers,
            GrammarGuardConfig::default(),
            LearningGuardConfig::default(),
        )
    }

    /// 2つの Guard の設定を指定する。
    ///
    /// 並びは「文法（`left_context`）-> 内容語（`doc_cache`、`profile`、`topic`）と学習（`learning`）
    /// -> その他（LM）」を想定する。Guard は、文脈の Ranker と学習の Ranker のうち最後のものの直後に働き、
    /// それより後ろの Ranker（LM）の差分は制限しない。
    /// - GrammarGuard: 内容語の加点が文法候補を覆さない（ADR 0003 §3.2）。
    /// - LearningGuard: 学習の加点のうち文脈に依らない部分が、文脈の証拠を覆さない（spec の基本原則）。
    pub(crate) fn with_guards(
        rankers: Vec<Arc<dyn Ranker>>,
        grammar_cfg: GrammarGuardConfig,
        learning_cfg: LearningGuardConfig,
    ) -> Self {
        let grammar_at = rankers.iter().position(|r| r.name() == GRAMMAR_RANKER);
        let last_content = rankers
            .iter()
            .rposition(|r| CONTENT_RANKERS.contains(&r.name()));
        let learning_at = rankers.iter().position(|r| r.name() == LEARNING_RANKER);
        let last_context = grammar_at.max(last_content);
        let guard_after = match (last_context, learning_at) {
            // 文脈の Ranker と学習の Ranker の両方がある
            (Some(c), Some(l)) => Some(c.max(l)),
            // 学習がなければ、文法の Ranker が内容語の Ranker より前にあるときだけ
            (Some(_), None) => match (grammar_at, last_content) {
                (Some(g), Some(c)) if g < c => Some(c),
                _ => None,
            },
            _ => None,
        };
        let lm_at = rankers
            .iter()
            .position(|r| !NON_LM_RANKERS.contains(&r.name()));
        Self {
            rankers,
            lm_at,
            keep_noun_suru: true,
            grammar_guard: GrammarGuard::new(grammar_cfg),
            learning_guard: LearningGuard::new(learning_cfg),
            guard_after,
        }
    }

    /// B40（ADR 0024）の段を使うか。規則 S（`lattice.noun_suru.penalty`）を止めたときは false にする。既定 true。
    #[must_use]
    pub fn keep_noun_suru(mut self, on: bool) -> Self {
        self.keep_noun_suru = on;
        self
    }

    /// 最後の段（LM の後。ADR 0024。B40）。LM を含めた並びの1位が規則 S の辺（サ変でない名詞 → する）を含み、
    /// LM の前の並び（`before`。Guard のあと）の1位が含まないとき、LM の前の1位を1位に戻す。
    /// 戻された候補以外の順は、LM の並びのまま。
    fn keep_noun_suru_first(&self, input: &RankInput<'_>, before: &[i32], cands: &mut [Candidate]) {
        if !self.keep_noun_suru || before.len() != cands.len() {
            return;
        }
        let argmin = |scores: &mut dyn Iterator<Item = i32>| {
            let mut best: Option<(usize, i32)> = None;
            for (i, s) in scores.enumerate() {
                if best.is_none_or(|(_, b)| s < b) {
                    best = Some((i, s));
                }
            }
            best.map(|(i, _)| i)
        };
        let (Some(prior), Some(now)) = (
            argmin(&mut before.iter().copied()),
            argmin(&mut cands.iter().map(|c| c.score)),
        ) else {
            return;
        };
        if prior == now
            || !has_noun_suru_edge(input.grammar, &cands[now].segments)
            || has_noun_suru_edge(input.grammar, &cands[prior].segments)
        {
            return;
        }
        cands[prior].score = cands[now].score.saturating_sub(1);
    }

    /// 最後の段（LM の後。ADR 0020）。1位の候補が英字の語を含むとき、その語を読みのカタカナなどに
    /// 置き換えた候補（塊にあるもの）を1位にする。英字の候補は2位になり、ほかの順は変えない。
    ///
    /// E4: 学習の加点が、置き換えた候補より英字の候補に多く付いているとき（その読みとその文脈で、
    /// カタカナより英字を多く選んでいる）は繰り上げない。secure では学習の加点がなく、いつも繰り上げる。
    fn english_first(&self, input: &RankInput<'_>, cands: &mut [Candidate]) {
        let mut best = 0;
        for (i, c) in cands.iter().enumerate() {
            if c.score < cands[best].score {
                best = i;
            }
        }
        let Some(first) = cands.get(best) else {
            return;
        };
        let Some(replaced) = english_first::variant(first, input.dict) else {
            return;
        };
        let Some(target) = cands.iter().position(|c| c.surface == replaced.surface) else {
            return;
        };
        if self.learning_prefers_first(input, &cands[best], &cands[target]) {
            return;
        }
        cands[target].score = cands[best].score.saturating_sub(1);
    }

    /// 学習の Ranker の加点（文脈に依らない分と文脈の分の合計）が、`first` のほうが `other` より多いか。
    fn learning_prefers_first(
        &self,
        input: &RankInput<'_>,
        first: &Candidate,
        other: &Candidate,
    ) -> bool {
        let mut probe = [first.clone(), other.clone()];
        for c in &mut probe {
            c.score = 0;
        }
        for ranker in self.rankers.iter().filter(|r| r.name() == LEARNING_RANKER) {
            ranker.rank(input, &mut probe);
        }
        // score は小さいほど上位（加点は負の差分）
        probe[0].score < probe[1].score
    }

    fn rank_guarded(
        &self,
        input: &RankInput<'_>,
        cands: &mut [Candidate],
        guard_after: usize,
        before_lm: &mut Option<Vec<i32>>,
    ) {
        let mut grammar = vec![0i32; cands.len()];
        let mut content = vec![0i32; cands.len()];
        let mut free = vec![0i32; cands.len()];
        for (index, ranker) in self.rankers.iter().enumerate().take(guard_after + 1) {
            let name = ranker.name();
            let tracked = name == GRAMMAR_RANKER || CONTENT_RANKERS.contains(&name);
            let before: Vec<i32> = if tracked {
                cands.iter().map(|c| c.score).collect()
            } else {
                Vec::new()
            };
            ranker.rank(input, cands);
            if tracked {
                let sink = if name == GRAMMAR_RANKER {
                    &mut grammar
                } else {
                    &mut content
                };
                for (i, (c, b)) in cands.iter().zip(&before).enumerate() {
                    sink[i] = sink[i].saturating_add(c.score.saturating_sub(*b));
                }
            }
            if name == LEARNING_RANKER
                && let Some(bonus) = ranker.context_free_bonus(input, cands)
                && bonus.len() == cands.len()
            {
                for (f, b) in free.iter_mut().zip(bonus) {
                    *f = f.saturating_add(b);
                }
            }
            if index == guard_after {
                // 文脈の証拠: 左文脈の連接、文書内キャッシュ、プロファイル、話題の差分の合計（負が支持）
                let evidence: Vec<i32> = grammar
                    .iter()
                    .zip(&content)
                    .map(|(g, c)| g.saturating_add(*c))
                    .collect();
                // 連用形などの「続く形」は、左文脈との連接が良くても、その語で終わる単位としては
                // 文法の根拠にならない（「ボスが」+ ひんし の 瀕し など）。文法候補から外す
                for (g, c) in grammar.iter_mut().zip(cands.iter()) {
                    if *g < 0 && is_continuing_form(input.grammar, c) {
                        *g = 0;
                    }
                    // 「助詞 + 非自立の動詞」の候補（に + する など）も文法候補にする。内容語の加点が
                    // 受けられない候補なので、話題が希少な語（に擦る）で覆すのを防ぐ
                    if *g == 0 && is_particle_plus_auxiliary(input.grammar, c) {
                        *g = PARTICLE_AUX_MARK;
                    }
                }
                self.grammar_guard.apply(cands, &grammar, &content);
                self.learning_guard.apply(cands, &evidence, &free);
            }
        }
        for (index, ranker) in self.rankers.iter().enumerate().skip(guard_after + 1) {
            if Some(index) == self.lm_at {
                *before_lm = Some(cands.iter().map(|c| c.score).collect());
            }
            ranker.rank(input, cands);
        }
    }
}

impl Ranker for CompositeRanker {
    /// 各 Ranker を順に呼ぶ。各 Ranker は score に差分を足すだけなので、順序は結果に影響しない
    /// （並べると決まった順で呼ばれる、というだけ）。GrammarGuard だけは、ここまでの差分を見て
    /// score を直す（内容語の加点が文法を覆すのを防ぐ）。
    fn rank(&self, input: &RankInput<'_>, cands: &mut [Candidate]) {
        let guards_on = self.grammar_guard.config().enabled || self.learning_guard.config().enabled;
        // LM の直前の score（B40 の段が使う。LM がなければ None）
        let mut before_lm: Option<Vec<i32>> = None;
        match self.guard_after {
            Some(after) if guards_on => self.rank_guarded(input, cands, after, &mut before_lm),
            _ => {
                for (index, ranker) in self.rankers.iter().enumerate() {
                    if Some(index) == self.lm_at {
                        before_lm = Some(cands.iter().map(|c| c.score).collect());
                    }
                    ranker.rank(input, cands);
                }
            }
        }
        // B40 は英字の繰り上げ（B32）より前: LM の並びを規則 S で戻してから、残った1位に英字の規則を当てる
        if let Some(before) = &before_lm {
            self.keep_noun_suru_first(input, before, cands);
        }
        self.english_first(input, cands);
    }

    fn name(&self) -> &str {
        names::COMPOSITE
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::common::apply_bonus;
    use super::super::config::ScopedKv;
    use super::super::testutil::*;
    use super::*;
    use crate::analysis::AnalyzedContext;
    use crate::context::Context;

    /// score を固定値だけ下げる Ranker（呼ばれた順の確認用）
    struct Shift {
        name: &'static str,
        bonus: i32,
        log: Arc<std::sync::Mutex<Vec<&'static str>>>,
    }

    impl Ranker for Shift {
        fn rank(&self, _input: &RankInput<'_>, cands: &mut [Candidate]) {
            self.log.lock().unwrap().push(self.name);
            for c in cands {
                apply_bonus(c, self.bonus);
            }
        }
        fn name(&self) -> &str {
            self.name
        }
    }

    #[test]
    fn composite_applies_every_ranker_in_order_and_adds_deltas() {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mk = |name, bonus| -> Arc<dyn Ranker> {
            Arc::new(Shift {
                name,
                bonus,
                log: Arc::clone(&log),
            })
        };
        let composite = CompositeRanker::new(vec![mk("a", 100), mk("b", 20), mk("c", 3)]);
        assert_eq!(composite.name(), "composite");
        let env = Env::new();
        let mut cands = vec![cand(Vec::new(), 1000), cand(Vec::new(), 500)];
        env.rank(
            &composite,
            &Context::new(),
            &AnalyzedContext::default(),
            &mut cands,
        );
        assert_eq!(cands[0].score, 1000 - 123);
        assert_eq!(cands[1].score, 500 - 123);
        assert_eq!(*log.lock().unwrap(), ["a", "b", "c"]);
        // base_cost は変わらない
        assert_eq!(cands[0].base_cost, 1000);
    }

    /// 表記ごとに決まった差分を足す Ranker（名前は GrammarGuard の分類に使われる）
    struct ByTable {
        name: &'static str,
        deltas: &'static [(&'static str, i32)],
    }

    impl Ranker for ByTable {
        fn rank(&self, _input: &RankInput<'_>, cands: &mut [Candidate]) {
            for c in cands {
                if let Some((_, d)) = self.deltas.iter().find(|(s, _)| *s == c.surface) {
                    c.score = c.score.saturating_add(*d);
                }
            }
        }
        fn name(&self) -> &str {
            self.name
        }
    }

    fn named_cand(surface: &str, cost: i32) -> Candidate {
        let mut c = cand(Vec::new(), cost);
        c.surface = surface.to_owned();
        c
    }

    /// ext-neg-10 の型: 文法（しよう）が左文脈で加点され、話題が「使用」を押し上げる
    fn neg_chain(guard: GrammarGuardConfig) -> CompositeRanker {
        let r = |name, deltas| -> Arc<dyn Ranker> { Arc::new(ByTable { name, deltas }) };
        CompositeRanker::with_guards(
            vec![
                r("left_context", &[("しよう", -2500)]),
                r("kana_style", &[]),
                r("doc_cache", &[]),
                r("learning", &[("仕様", -1000)]),
                r("topic", &[("使用", -4500), ("仕様", -4500)]),
                // LM 相当（名前は何でもよい）。ガードの外
                r("zenz", &[("仕様", 200)]),
            ],
            guard,
            LearningGuardConfig::default(),
        )
    }

    fn ranked(composite: &CompositeRanker) -> Vec<(String, i32)> {
        let env = Env::new();
        let mut cands = vec![
            named_cand("使用", 5000),
            named_cand("しよう", 6500),
            named_cand("仕様", 5200),
        ];
        env.rank(
            composite,
            &Context::new(),
            &AnalyzedContext::default(),
            &mut cands,
        );
        cands.sort_by_key(|c| c.score);
        cands.into_iter().map(|c| (c.surface, c.score)).collect()
    }

    #[test]
    fn guard_stops_topic_from_overturning_the_grammar_candidate() {
        // ガードなし: 使用 500、仕様 -100 が、しよう 4000 より上
        let off = ranked(&neg_chain(GrammarGuardConfig {
            enabled: false,
            ..GrammarGuardConfig::default()
        }));
        assert_eq!(off[0].0, "仕様");
        // ガードあり: しよう（4000）が1位。話題で上がった2つは、順序を保ったまま 4001 から後ろに置かれる
        // （仕様は 4001、使用は 4801。仕様には LM 相当の +200 が足される）
        let on = ranked(&neg_chain(GrammarGuardConfig::default()));
        assert_eq!(
            on,
            vec![
                ("しよう".to_owned(), 4000),
                ("仕様".to_owned(), 4201),
                ("使用".to_owned(), 4801)
            ]
        );
    }

    #[test]
    fn guard_does_not_limit_what_comes_after_the_content_rankers() {
        // 最後の Ranker（LM 相当）が文法候補を押し下げるのは、ガードの対象外
        let r = |name, deltas| -> Arc<dyn Ranker> { Arc::new(ByTable { name, deltas }) };
        let composite = CompositeRanker::new(vec![
            r("left_context", &[("しよう", -2500)]),
            r("topic", &[("使用", -4500)]),
            r("zenz", &[("しよう", 9000)]),
        ]);
        let env = Env::new();
        let mut cands = vec![named_cand("使用", 5000), named_cand("しよう", 6500)];
        env.rank(
            &composite,
            &Context::new(),
            &AnalyzedContext::default(),
            &mut cands,
        );
        cands.sort_by_key(|c| c.score);
        assert_eq!(cands[0].surface, "使用");
    }

    /// 学習のふり: 表記ごとの加点と、そのうち文脈に依らない部分を返す
    struct FakeLearning {
        bonus: &'static [(&'static str, i32)],
        free: &'static [(&'static str, i32)],
    }

    impl Ranker for FakeLearning {
        fn rank(&self, _input: &RankInput<'_>, cands: &mut [Candidate]) {
            for c in cands {
                if let Some((_, b)) = self.bonus.iter().find(|(s, _)| *s == c.surface) {
                    apply_bonus(c, *b);
                }
            }
        }
        fn name(&self) -> &str {
            "learning"
        }
        fn context_free_bonus(
            &self,
            _input: &RankInput<'_>,
            cands: &[Candidate],
        ) -> Option<Vec<i32>> {
            Some(
                cands
                    .iter()
                    .map(|c| {
                        self.free
                            .iter()
                            .find(|(s, _)| *s == c.surface)
                            .map_or(0, |(_, b)| *b)
                    })
                    .collect(),
            )
        }
    }

    fn learning_chain(
        bonus: &'static [(&'static str, i32)],
        free: &'static [(&'static str, i32)],
    ) -> Vec<(String, i32)> {
        let topic: Arc<dyn Ranker> = Arc::new(ByTable {
            name: "topic",
            deltas: &[("瀕死", -300)],
        });
        let learning: Arc<dyn Ranker> = Arc::new(FakeLearning { bonus, free });
        // 実際の並び: 学習 -> 話題 -> LM
        let lm: Arc<dyn Ranker> = Arc::new(ByTable {
            name: "zenz",
            deltas: &[],
        });
        let composite = CompositeRanker::new(vec![learning, topic, lm]);
        let env = Env::new();
        let mut cands = vec![named_cand("瀕死", 5000), named_cand("品詞", 5100)];
        env.rank(
            &composite,
            &Context::new(),
            &AnalyzedContext::default(),
            &mut cands,
        );
        cands.sort_by_key(|c| c.score);
        cands.into_iter().map(|c| (c.surface, c.score)).collect()
    }

    #[test]
    fn context_free_learning_cannot_overturn_the_context_evidence() {
        // 品詞 は学習で 600 加点された（すべて文脈に依らない部分）。話題は 瀕死 を 300 支持している
        let r = learning_chain(&[("品詞", 600)], &[("品詞", 600)]);
        assert_eq!(
            r,
            vec![("瀕死".to_owned(), 4700), ("品詞".to_owned(), 4701)]
        );
    }

    #[test]
    fn learning_matched_to_the_current_context_is_not_limited() {
        // 同じ加点でも、現在の文脈の特徴量に一致した部分（free が 0）なら、文脈の証拠より優先する
        let r = learning_chain(&[("品詞", 600)], &[]);
        assert_eq!(
            r,
            vec![("品詞".to_owned(), 4500), ("瀕死".to_owned(), 4700)]
        );
    }

    #[test]
    fn learning_guard_can_be_disabled() {
        let learning: Arc<dyn Ranker> = Arc::new(FakeLearning {
            bonus: &[("品詞", 600)],
            free: &[("品詞", 600)],
        });
        let topic: Arc<dyn Ranker> = Arc::new(ByTable {
            name: "topic",
            deltas: &[("瀕死", -300)],
        });
        let composite = CompositeRanker::with_guards(
            vec![learning, topic],
            GrammarGuardConfig::default(),
            LearningGuardConfig {
                enabled: false,
                ..LearningGuardConfig::default()
            },
        );
        let env = Env::new();
        let mut cands = vec![named_cand("瀕死", 5000), named_cand("品詞", 5100)];
        env.rank(
            &composite,
            &Context::new(),
            &AnalyzedContext::default(),
            &mut cands,
        );
        assert_eq!((cands[0].score, cands[1].score), (4700, 4500));
    }

    #[test]
    fn continuing_forms_are_not_grammar_candidates() {
        // 「ボスが」+ ひんし: 瀕し（連用形）は左文脈との連接が良いだけで、文法の根拠にしない
        let mut env = Env::new();
        let pos_stem = env.grammar.add_pos("動詞,一般,*,*,サ行変格,連用形-一般");
        let pos_end = env.grammar.add_pos("動詞,一般,*,*,サ行変格,終止形-一般");
        let r = |name, deltas| -> Arc<dyn Ranker> { Arc::new(ByTable { name, deltas }) };
        let composite = CompositeRanker::new(vec![
            r("left_context", &[("瀕し", -6000), ("瀕する", -6000)]),
            r("topic", &[("瀕死", -4500)]),
        ]);
        let mut mk = |surface: &str, cost, pos| {
            let mut seg = env.word(surface, surface, None, pos);
            seg.surface = surface.to_owned();
            let mut c = cand(vec![seg], cost);
            c.surface = surface.to_owned();
            c
        };
        let noun = mk("瀕死", 5000, POS_NOUN);
        let stem = mk("瀕し", 8000, pos_stem);
        let ending = mk("瀕する", 9000, pos_end);
        let rank = |cands: &mut Vec<Candidate>| {
            env.rank(
                &composite,
                &Context::new(),
                &AnalyzedContext::default(),
                cands,
            );
        };
        // 連用形の 瀕し（2000）だけでは文法候補にならず、瀕死（500）は戻されない
        let mut cands = vec![noun.clone(), stem];
        rank(&mut cands);
        assert_eq!(cands[0].score, 500);
        assert_eq!(cands[1].score, 2000);
        // 終止形の 瀕する（3000）は文法候補。瀕死は内容語の加点なしでは 5000 で下だったので戻される
        let mut cands = vec![noun, ending];
        rank(&mut cands);
        assert_eq!(cands[0].score, 3001);
        assert_eq!(cands[1].score, 3000);
    }

    #[test]
    fn guard_is_inactive_without_a_grammar_ranker_before_a_content_ranker() {
        let r = |name, deltas| -> Arc<dyn Ranker> { Arc::new(ByTable { name, deltas }) };
        let only_content = CompositeRanker::new(vec![r("topic", &[("使用", -4500)])]);
        assert!(only_content.guard_after.is_none());
        let wrong_order = CompositeRanker::new(vec![r("topic", &[]), r("left_context", &[])]);
        assert!(wrong_order.guard_after.is_none());
        let both = CompositeRanker::new(vec![
            r("left_context", &[]),
            r("doc_cache", &[]),
            r("zenz", &[]),
            r("topic", &[]),
        ]);
        assert_eq!(both.guard_after, Some(3));
    }

    #[test]
    fn empty_composite_changes_nothing() {
        let env = Env::new();
        let mut cands = vec![cand(Vec::new(), 7)];
        env.rank(
            &CompositeRanker::new(Vec::new()),
            &Context::new(),
            &AnalyzedContext::default(),
            &mut cands,
        );
        assert_eq!(cands[0].score, 7);
    }

    #[test]
    fn apply_bonus_saturates() {
        let mut c = cand(Vec::new(), i32::MIN + 1);
        apply_bonus(&mut c, 1_000_000);
        assert_eq!(c.score, i32::MIN);
    }

    fn kv(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|&(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }

    #[test]
    fn scoped_kv_reads_own_keys_and_ignores_other_prefixes() {
        let map = kv(&[("a.x", "5"), ("b.unknown", "1"), ("a.y", "0.5")]);
        let scope = ScopedKv::new("a", &["x", "y", "z"], &map).unwrap();
        assert_eq!(scope.get("x", 1i32).unwrap(), 5);
        assert_eq!(scope.get("z", 9i32).unwrap(), 9);
        assert_eq!(scope.get_float("y", 0.0).unwrap(), 0.5);
    }

    #[test]
    fn scoped_kv_rejects_unknown_key_with_own_prefix() {
        let map = kv(&[("a.nope", "1")]);
        let err = ScopedKv::new("a", &["x"], &map).err().unwrap();
        assert_eq!(err.key, "a.nope");
    }

    #[test]
    fn scoped_kv_rejects_bad_values() {
        for bad in ["abc", "-1", "", "1.5"] {
            let map = kv(&[("a.x", bad)]);
            let scope = ScopedKv::new("a", &["x"], &map).unwrap();
            let err = scope.get("x", 1i32).unwrap_err();
            assert_eq!(err.key, "a.x", "{bad:?}");
        }
        for bad in ["NaN", "inf", "-0.5", "x"] {
            let map = kv(&[("a.x", bad)]);
            let scope = ScopedKv::new("a", &["x"], &map).unwrap();
            assert!(scope.get_float("x", 1.0).is_err(), "{bad:?}");
        }
    }
}

/// 最後の段（英字の1位をカタカナにする。ADR 0020）のテスト
#[cfg(test)]
mod english_first_tests {
    use super::super::common::apply_bonus;
    use super::super::testutil::Env;
    use super::*;
    use crate::analysis::AnalyzedContext;
    use crate::candidate::{CandidateOrigin, Segment};
    use crate::context::Context;
    use crate::dict::{Dictionary, InMemoryDictionary};

    const POS: u16 = 0;
    const READING: &str = "はるしねーしょん";
    const LATIN: &str = "Hallucination";
    const KATAKANA: &str = "ハルシネーション";

    /// 表記ごとに決まった差分を足す Ranker
    struct ByTable {
        name: &'static str,
        deltas: &'static [(&'static str, i32)],
    }

    impl Ranker for ByTable {
        fn rank(&self, _input: &RankInput<'_>, cands: &mut [Candidate]) {
            for c in cands {
                if let Some((_, d)) = self.deltas.iter().find(|(s, _)| *s == c.surface) {
                    apply_bonus(c, *d);
                }
            }
        }
        fn name(&self) -> &str {
            self.name
        }
    }

    fn ranker(name: &'static str, deltas: &'static [(&'static str, i32)]) -> Arc<dyn Ranker> {
        Arc::new(ByTable { name, deltas })
    }

    fn dict() -> InMemoryDictionary {
        let mut d = InMemoryDictionary::new(0);
        d.insert(READING, LATIN, 1, 1, 5000, POS);
        d.insert(READING, KATAKANA, 1, 1, 15000, POS);
        d
    }

    fn candidate(
        dict: &dyn Dictionary,
        surface: &str,
        cost: i32,
        origin: CandidateOrigin,
    ) -> Candidate {
        let mut word = None;
        dict.lookup_exact(READING, &mut |e| {
            if e.surface == surface {
                word = Some(e.id);
            }
        });
        Candidate {
            surface: surface.to_owned(),
            segments: vec![Segment {
                reading: READING.to_owned(),
                surface: surface.to_owned(),
                word,
                left_id: 1,
                right_id: 1,
                pos_id: POS,
            }],
            base_cost: cost,
            score: cost,
            origin,
        }
    }

    /// 英字（コスト 5000）、カタカナ（15000）、ひらがな（16000）の順位を付けた表記の並び
    fn order(composite: &CompositeRanker, secure: bool) -> Vec<(String, i32)> {
        let env = Env::new();
        let d = dict();
        let mut cands = vec![
            candidate(&d, LATIN, 5000, CandidateOrigin::Dictionary),
            candidate(&d, KATAKANA, 15000, CandidateOrigin::Dictionary),
            candidate(&d, READING, 16000, CandidateOrigin::Hiragana),
        ];
        let ctx = Context::new().with_secure(secure);
        let analysis = AnalyzedContext::default();
        let input = RankInput {
            reading: READING,
            context: &ctx,
            grammar: &env.grammar,
            analysis: &analysis,
            dict: &d,
            deadline: None,
        };
        composite.rank(&input, &mut cands);
        cands.sort_by_key(|c| c.score);
        cands.into_iter().map(|c| (c.surface, c.score)).collect()
    }

    fn surfaces(order: &[(String, i32)]) -> Vec<&str> {
        order.iter().map(|(s, _)| s.as_str()).collect()
    }

    #[test]
    fn katakana_goes_first_and_latin_second_without_changing_the_rest() {
        let c = CompositeRanker::new(vec![ranker("topic", &[])]);
        let o = order(&c, false);
        assert_eq!(surfaces(&o), [KATAKANA, LATIN, READING]);
        assert_eq!(o[0].1, 5000 - 1);
    }

    #[test]
    fn it_overturns_a_language_model_that_prefers_latin() {
        let c = CompositeRanker::new(vec![ranker("topic", &[]), ranker("zenz", &[(LATIN, 9000)])]);
        let o = order(&c, false);
        assert_eq!(surfaces(&o), [KATAKANA, LATIN, READING]);
    }

    #[test]
    fn nothing_changes_when_the_first_is_not_latin() {
        let c = CompositeRanker::new(vec![ranker("zenz", &[(KATAKANA, 20000)])]);
        let o = order(&c, false);
        assert_eq!(surfaces(&o), [KATAKANA, LATIN, READING]);
        // 繰り上げではなく、もともとの1位。スコアは差分だけ
        assert_eq!(o[0].1, 15000 - 20000);
    }

    #[test]
    fn learning_more_for_latin_keeps_latin_first() {
        // E4: 英字の加点のほうが多いときは繰り上げない
        let c = CompositeRanker::new(vec![ranker("learning", &[(LATIN, 300), (KATAKANA, 100)])]);
        assert_eq!(surfaces(&order(&c, false)), [LATIN, KATAKANA, READING]);
        // 加点が同じか、カタカナのほうが多ければ繰り上げる
        let c = CompositeRanker::new(vec![ranker("learning", &[(LATIN, 200), (KATAKANA, 200)])]);
        assert_eq!(surfaces(&order(&c, false))[0], KATAKANA);
        let c = CompositeRanker::new(vec![ranker("learning", &[(LATIN, 200), (KATAKANA, 300)])]);
        assert_eq!(surfaces(&order(&c, false))[0], KATAKANA);
    }

    #[test]
    fn learning_does_not_count_for_a_secure_field() {
        // 実際の LearningRanker は secure のとき加点しない（ここでは同じ動きの Ranker）
        struct SecureAware;
        impl Ranker for SecureAware {
            fn rank(&self, input: &RankInput<'_>, cands: &mut [Candidate]) {
                if input.context.secure {
                    return;
                }
                for c in cands {
                    if c.surface == LATIN {
                        apply_bonus(c, 300);
                    }
                }
            }
            fn name(&self) -> &str {
                "learning"
            }
        }
        let c = CompositeRanker::new(vec![Arc::new(SecureAware)]);
        assert_eq!(surfaces(&order(&c, false))[0], LATIN);
        assert_eq!(surfaces(&order(&c, true))[0], KATAKANA);
    }

    #[test]
    fn a_replacement_missing_from_the_chunk_changes_nothing() {
        let env = Env::new();
        let d = dict();
        let mut cands = vec![candidate(&d, LATIN, 5000, CandidateOrigin::Dictionary)];
        let ctx = Context::new();
        let analysis = AnalyzedContext::default();
        let input = RankInput {
            reading: READING,
            context: &ctx,
            grammar: &env.grammar,
            analysis: &analysis,
            dict: &d,
            deadline: None,
        };
        CompositeRanker::new(Vec::new()).rank(&input, &mut cands);
        assert_eq!(cands[0].score, 5000);
        CompositeRanker::new(Vec::new()).rank(&input, &mut []);
    }
}

/// B40（ADR 0024）: LM が規則 S の守った1位を覆さない
#[cfg(test)]
mod keep_noun_suru_tests {
    use super::super::testutil::*;
    use super::*;
    use crate::analysis::AnalyzedContext;
    use crate::candidate::Segment;
    use crate::context::Context;

    const POS_SAHEN_NOUN: &str = "名詞,普通名詞,サ変可能,*,*,*";
    const POS_GENERAL_NOUN: &str = "名詞,普通名詞,一般,*,*,*";
    const POS_SURU: &str = "動詞,非自立可能,*,*,サ行変格,終止形-一般";

    /// 表記ごとの差分を足す LM のふり（名前は "zenz"）
    struct FakeLm(&'static [(&'static str, i32)]);

    impl Ranker for FakeLm {
        fn rank(&self, _input: &RankInput<'_>, cands: &mut [Candidate]) {
            for c in cands {
                if let Some((_, d)) = self.0.iter().find(|(s, _)| *s == c.surface) {
                    c.score = c.score.saturating_add(*d);
                }
            }
        }
        fn name(&self) -> &str {
            "zenz"
        }
    }

    struct Fixture {
        env: Env,
        pos_general: u16,
        pos_sahen: u16,
        pos_suru: u16,
    }

    impl Fixture {
        fn new() -> Self {
            let mut env = Env::new();
            let pos_general = env.grammar.add_pos(POS_GENERAL_NOUN);
            let pos_sahen = env.grammar.add_pos(POS_SAHEN_NOUN);
            let pos_suru = env.grammar.add_pos(POS_SURU);
            Self {
                env,
                pos_general,
                pos_sahen,
                pos_suru,
            }
        }

        /// 語の列から候補を作る（表記は語の表記の連結）
        fn cand(&mut self, words: &[(&str, u16)], cost: i32) -> Candidate {
            let segments: Vec<Segment> = words
                .iter()
                .map(|(surface, pos)| self.env.word(surface, surface, None, *pos))
                .collect();
            cand(segments, cost)
        }

        /// `rankers` で順位を付けた表記の並び
        fn order(
            &self,
            rankers: Vec<Arc<dyn Ranker>>,
            keep: bool,
            cands: &mut [Candidate],
        ) -> Vec<String> {
            let composite = CompositeRanker::new(rankers).keep_noun_suru(keep);
            self.env.rank(
                &composite,
                &Context::new(),
                &AnalyzedContext::default(),
                cands,
            );
            let mut sorted: Vec<&Candidate> = cands.iter().collect();
            sorted.sort_by_key(|c| c.score);
            sorted.iter().map(|c| c.surface.clone()).collect()
        }
    }

    fn lm(deltas: &'static [(&'static str, i32)]) -> Vec<Arc<dyn Ranker>> {
        vec![Arc::new(FakeLm(deltas))]
    }

    /// げんきんする: 規則 S が効いて 厳禁する（サ変の名詞）が1位。LM が 現金＋する（一般の名詞）を押し上げる
    #[test]
    fn lm_cannot_overturn_a_first_that_rule_s_protected() {
        let mut f = Fixture::new();
        let cands = vec![
            f.cand(&[("厳禁", f.pos_sahen), ("する", f.pos_suru)], 9000),
            f.cand(&[("現金", f.pos_general), ("する", f.pos_suru)], 9500),
        ];
        const LM: &[(&str, i32)] = &[("現金する", -3000)];
        // 戻さない設定では LM が覆す
        let mut off = cands.clone();
        assert_eq!(f.order(lm(LM), false, &mut off)[0], "現金する");
        // 戻す: 厳禁する が1位。現金する は2位のまま
        let mut on = cands;
        assert_eq!(f.order(lm(LM), true, &mut on), ["厳禁する", "現金する"]);
    }

    /// LM が規則 S と無関係に1位を変えるときは戻さない
    #[test]
    fn lm_may_change_the_first_when_rule_s_is_not_involved() {
        let mut f = Fixture::new();
        let mut cands = vec![
            f.cand(&[("再会", f.pos_sahen), ("する", f.pos_suru)], 9000),
            f.cand(&[("再開", f.pos_sahen), ("する", f.pos_suru)], 9300),
        ];
        const LM: &[(&str, i32)] = &[("再開する", -1000)];
        assert_eq!(f.order(lm(LM), true, &mut cands), ["再開する", "再会する"]);
    }

    /// LM の前の1位にも 一般の名詞＋する の辺があれば（守られていない）、LM の変更は戻さない
    #[test]
    fn nothing_is_restored_when_the_prior_first_also_has_the_edge() {
        let mut f = Fixture::new();
        let mut cands = vec![
            f.cand(&[("ゲーム", f.pos_general), ("する", f.pos_suru)], 9000),
            f.cand(&[("現金", f.pos_general), ("する", f.pos_suru)], 9500),
        ];
        const LM: &[(&str, i32)] = &[("現金する", -3000)];
        assert_eq!(
            f.order(lm(LM), true, &mut cands),
            ["現金する", "ゲームする"]
        );
    }

    /// LM がなければ何も変わらない（文脈の Ranker だけの並び）
    #[test]
    fn without_a_language_model_nothing_changes() {
        let mut f = Fixture::new();
        let mut cands = vec![
            f.cand(&[("現金", f.pos_general), ("する", f.pos_suru)], 8000),
            f.cand(&[("厳禁", f.pos_sahen), ("する", f.pos_suru)], 9000),
        ];
        let topic: Vec<Arc<dyn Ranker>> = vec![Arc::new(FakeTopic)];
        assert_eq!(f.order(topic, true, &mut cands), ["現金する", "厳禁する"]);
    }

    /// 名前が topic で、何もしない Ranker（LM ではない Ranker のふり）
    struct FakeTopic;

    impl Ranker for FakeTopic {
        fn rank(&self, _input: &RankInput<'_>, _cands: &mut [Candidate]) {}
        fn name(&self) -> &str {
            names::TOPIC
        }
    }
}
