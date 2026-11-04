//! 変換の結果（`Conversion`、`Page`）と確定のイベント（`CommitEvent`）。

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use ikari_input::hiragana_to_katakana;

use super::ConvertOptions;
use crate::analysis::AnalyzedContext;
use crate::candidate::{Candidate, CandidateOrigin, Segment};
use crate::context::Context;
use crate::dict::{Dictionary, Grammar};
use crate::lattice::Lattice;
use crate::nbest::{NBest, Path};
use crate::ranker::english_first;
use crate::ranker::{RankInput, Ranker};
use crate::rewriter::{Clock, RewriteInput, RewritePosition, Rewriter};

/// 小書きの母音かな（ぁぃぅぇぉ）と、そのカタカナ。辞書の「目ぇ」（読み メ）のような表記を見分けるのに使う
const SMALL_VOWEL_KANA: [(char, char); 5] = [
    ('ぁ', 'ァ'),
    ('ぃ', 'ィ'),
    ('ぅ', 'ゥ'),
    ('ぇ', 'ェ'),
    ('ぉ', 'ォ'),
];

fn is_han(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '々' | '〆')
}

/// 漢字を含む表記が、読み（lattice の部分文字列。ひらがなに正規化済み）にない小書き母音かなを足しているか。
/// 「め」に対する「目ぇ」。入力していない文字を足す候補は出さない（ADR 0027、#217）。
/// 読みに小書き仮名があれば（「わりぃ」→「悪ぃ」）対象外。「めぇ」は「目」+「ぇ」の経路で出る。
fn adds_uninput_small_kana(surface: &str, reading: &str) -> bool {
    surface.chars().any(is_han)
        && SMALL_VOWEL_KANA.iter().any(|&(hira, kata)| {
            surface.contains([hira, kata]) && !reading.contains(hira) && !reading.contains(kata)
        })
}

/// Arc の複製を持つ所有型（'static かつ Send）。
pub struct Conversion {
    pub(super) dict: Arc<dyn Dictionary>,
    pub(super) grammar: Arc<dyn Grammar>,
    pub(super) ranker: Arc<dyn Ranker>,
    pub(super) rewriters: Vec<Arc<dyn Rewriter>>,
    pub(super) clock: Arc<dyn Clock>,
    pub(super) opts: ConvertOptions,
    /// Ranker に渡す文脈（secure なら空）
    pub(super) ctx: Context,
    /// 解析済みの左文脈（Ranker と CommitEvent に渡す）。secure なら空
    pub(super) analysis: Arc<AnalyzedContext>,
    pub(super) lattice: Lattice,
    pub(super) nbest: NBest,
    /// 確定済みの候補列。末尾への追加のみで、既存の要素は変えない
    pub(super) generated: Vec<Candidate>,
    /// 重複チェック用の表記集合（辞書由来とかな候補の両方）
    pub(super) seen: HashSet<String>,
    /// これ以上候補が増えない
    pub(super) exhausted: bool,
    /// 現在の呼び出し（`convert_until` / `page_until`）の締め切り。`RankInput.deadline` に渡す
    pub(super) deadline: Option<Instant>,
}

impl Conversion {
    /// 正規化（ひらがな化）した後の読み
    pub fn reading(&self) -> &str {
        self.lattice.key()
    }

    /// index ページ目（0始まり）。必要な分だけ遅延生成する。範囲外なら None。
    /// 一度返したページの中身は二度と変わらない。
    ///
    /// `has_next` を正確に返すため、ページの次の1件まで（足りなければ `chunk_size` 件の塊で）生成する。
    pub fn page(&mut self, index: usize) -> Option<Page<'_>> {
        let deadline = Instant::now() + self.opts.lm_budget.for_reading(self.lattice.key());
        self.page_until(index, Some(deadline))
    }

    /// `page` の締め切り指定版（ADR 0004 §3.4）。1回の呼び出しの中の `rank` はすべて同じ `deadline` を使う。
    /// `None` は待たない（LM を使わない）。
    pub fn page_until(&mut self, index: usize, deadline: Option<Instant>) -> Option<Page<'_>> {
        self.deadline = deadline;
        let size = self.opts.page_size;
        let start = index.checked_mul(size)?;
        let end = start.checked_add(size)?;
        let need = end.saturating_add(1);
        while self.generated.len() < need && !self.exhausted {
            self.extend_chunk();
        }
        if start >= self.generated.len() {
            return None;
        }
        let has_next = self.generated.len() > end;
        let end = end.min(self.generated.len());
        Some(Page {
            index,
            candidates: &self.generated[start..end],
            has_next,
        })
    }

    pub fn generated(&self) -> &[Candidate] {
        &self.generated
    }

    /// `generated()[index]` を確定したことを表すイベント。範囲外か、secure なら None（学習しない。NFR-18）。
    pub fn commit(&self, index: usize) -> Option<CommitEvent> {
        if self.ctx.secure {
            return None;
        }
        let candidate = self.generated.get(index)?.clone();
        Some(CommitEvent {
            reading: self.lattice.key().to_owned(),
            candidate,
            analysis: Arc::clone(&self.analysis),
        })
    }

    /// §4.6-7: 辞書の候補を pool_size 件、ひらがなとカタカナの候補を加えて rank し、並べ替えて確定する。
    pub(super) fn build_first_pool(&mut self) {
        let mut pool = self.take_dictionary_candidates(self.opts.pool_size);
        self.exhausted = self.nbest.is_exhausted();

        // かなの候補は辞書の候補だけを基準にする（Rewriter の候補で動かさない）
        let kana_cost = pool.iter().map(|c| c.base_cost).max().unwrap_or(0) + 1;
        let best_cost = pool.iter().map(|c| c.base_cost).min().unwrap_or(0);
        self.add_rewriter_candidates(&mut pool);
        let hiragana = self.lattice.key().to_owned();
        let katakana = hiragana_to_katakana(&hiragana);
        // カタカナ語らしい読みだけ、カタカナの候補のコストを下げる（#271。既定は無効）。
        // 辞書に同じ表記の 1 語の候補があるときは、その候補のコストを下げる
        // （ファイル + メイ のように複数の語に分かれる経路は動かさない）
        let like = &self.opts.katakana_like;
        // K1（ADR 0037）: 未知語のカタカナ（辞書に同じ表記の 1 語がないとき）は、読みの文字数で上限を掛けてから下げる
        let katakana_cost = like.lowered_cost(
            &hiragana,
            like.unknown_cost(&hiragana, kana_cost),
            best_cost,
        );
        for c in pool
            .iter_mut()
            .filter(|c| c.surface == katakana && c.segments.len() == 1)
        {
            c.base_cost = like.lowered_cost(&hiragana, c.base_cost, best_cost);
            c.score = c.base_cost;
        }
        for (surface, origin, cost) in [
            (hiragana, CandidateOrigin::Hiragana, kana_cost),
            (katakana, CandidateOrigin::Katakana, katakana_cost),
        ] {
            if self.seen.insert(surface.clone()) {
                pool.push(self.single_segment_candidate(surface, origin, cost));
            }
        }
        self.add_english_replacements(&mut pool);
        self.rank_and_append(pool);
    }

    /// ADR 0048 §3 の規則で予約枠の候補を作る。`pool` は A* から取った候補だけ。
    /// 契約スタブ（空を返す）。本体はレーン A。
    #[allow(dead_code)]
    fn reserve_word_candidates(&mut self, _pool: &[Candidate]) -> Vec<Candidate> {
        Vec::new()
    }

    /// 英字の語を含む候補ごとに、その語を読みのカタカナなどに置き換えた候補を足す（ADR 0020 §3.4）。
    /// 表記が未出のものだけ。コストは元の候補と同じ（最後の段で1位にするのは `CompositeRanker`）。
    fn add_english_replacements(&mut self, pool: &mut Vec<Candidate>) {
        let replaced: Vec<Candidate> = pool
            .iter()
            .filter_map(|c| english_first::variant(c, &*self.dict))
            .collect();
        for cand in replaced {
            if self.seen.insert(cand.surface.clone()) {
                pool.push(cand);
            }
        }
    }

    /// Rewriter の候補をプールに足す（ADR 0002 §6.2-5）。表記が既に出ているものは捨てる。
    /// base_cost = 辞書の候補を base_cost の昇順に並べた `rewriter_anchor` 番目 + 1。
    /// 番号が候補数を超えれば最後の候補を基準にする。`rewriter_anchor = 0` は 1 として扱う。
    /// 日付（`AfterWords`）は同じ規則を `date_anchor` で行う。
    fn add_rewriter_candidates(&mut self, pool: &mut Vec<Candidate>) {
        if self.rewriters.is_empty() {
            return;
        }
        let mut costs: Vec<i32> = pool.iter().map(|c| c.base_cost).collect();
        costs.sort_unstable();
        let cost_after = |n: usize| {
            let n = n.max(1).min(costs.len().max(1));
            costs.get(n - 1).copied().unwrap_or(0).saturating_add(1)
        };
        let base_cost = cost_after(self.opts.rewriter_anchor);
        // `AfterWords` は `date_anchor` 番目の直後（日付用。#80）
        let after_words_cost = cost_after(self.opts.date_anchor);
        // `Front` は辞書の候補で一番低いコストより 1 低くする（ADR 0032 §8）。
        // 同じコスト同士は足した順のまま並ぶ（score の安定ソート）。
        let front_cost = costs.first().copied().unwrap_or(0).saturating_sub(1);

        let input = RewriteInput {
            reading: self.lattice.key(),
            context: &self.ctx,
            now: self.clock.now(),
        };
        let mut produced = Vec::new();
        for rewriter in &self.rewriters {
            let mut out = Vec::new();
            rewriter.rewrite(&input, &mut out);
            produced.extend(out.into_iter().map(|c| (rewriter.name(), c)));
        }
        for (name, cand) in produced {
            if cand.surface.is_empty() || !self.seen.insert(cand.surface.clone()) {
                continue;
            }
            let origin = CandidateOrigin::Rewriter {
                name,
                format: cand.format,
            };
            let cost = match cand.position {
                RewritePosition::Anchor => base_cost,
                RewritePosition::Front => front_cost,
                RewritePosition::AfterWords => after_words_cost,
            };
            pool.push(self.single_segment_candidate(cand.surface, origin, cost));
        }
    }

    /// §4.6-8: A* を再開して chunk_size 件を追加し、その塊だけを rank して末尾に足す。
    fn extend_chunk(&mut self) {
        let chunk = self.take_dictionary_candidates(self.opts.chunk_size);
        if chunk.is_empty() || self.nbest.is_exhausted() {
            self.exhausted = true;
        }
        self.rank_and_append(chunk);
    }

    /// A* から、表記が未出のパスを最大 `want` 件取り出して候補にする。
    /// 1回の呼び出しで展開する数は `max_astar_pops` まで。
    fn take_dictionary_candidates(&mut self, want: usize) -> Vec<Candidate> {
        let mut out = Vec::with_capacity(want);
        let mut pops = self.opts.max_astar_pops;
        while out.len() < want {
            let Some(path) = self.nbest.next(&self.lattice, &*self.grammar, &mut pops) else {
                break;
            };
            let Some(cand) = self.path_to_candidate(&path) else {
                continue;
            };
            if self.seen.insert(cand.surface.clone()) {
                out.push(cand);
            }
        }
        out
    }

    /// 辞書から表記を引いて候補にする。辞書側のエントリが引けなければ None（そのパスは捨てる）。
    fn path_to_candidate(&self, path: &Path) -> Option<Candidate> {
        let mut segments = Vec::with_capacity(path.nodes.len());
        let mut surface = String::new();
        for &id in &path.nodes {
            let node = self.lattice.node(id);
            let reading = self.lattice.slice(node);
            let seg_surface = match node.word {
                Some(word) => self.dict.entry(word)?.surface,
                None => reading,
            };
            if adds_uninput_small_kana(seg_surface, reading) {
                return None;
            }
            surface.push_str(seg_surface);
            segments.push(Segment {
                reading: reading.to_owned(),
                surface: seg_surface.to_owned(),
                word: node.word,
                left_id: node.left_id,
                right_id: node.right_id,
                pos_id: node.pos_id,
            });
        }
        Some(Candidate {
            surface,
            segments,
            base_cost: path.cost,
            score: path.cost,
            origin: CandidateOrigin::Dictionary,
        })
    }

    /// 未知語の1セグメントだけを持つ候補（かな、Rewriter の候補）。
    fn single_segment_candidate(
        &self,
        surface: String,
        origin: CandidateOrigin,
        cost: i32,
    ) -> Candidate {
        let unk = self.grammar.unknown();
        Candidate {
            segments: vec![Segment {
                reading: self.lattice.key().to_owned(),
                surface: surface.clone(),
                word: None,
                left_id: unk.left_id,
                right_id: unk.right_id,
                pos_id: unk.pos_id,
            }],
            surface,
            base_cost: cost,
            score: cost,
            origin,
        }
    }

    /// 塊を Ranker に渡し、score の安定ソートをして確定済みの列の末尾に足す。
    fn rank_and_append(&mut self, mut chunk: Vec<Candidate>) {
        if chunk.is_empty() {
            return;
        }
        let input = RankInput {
            reading: self.lattice.key(),
            context: &self.ctx,
            grammar: &*self.grammar,
            analysis: &self.analysis,
            dict: &*self.dict,
            deadline: self.deadline,
        };
        self.ranker.rank(&input, &mut chunk);
        chunk.sort_by_key(|c| c.score);
        self.generated.append(&mut chunk);
    }
}

/// 確定のイベント。特徴量は学習側（`LearningStore::record`）で作る。
#[derive(Clone, Debug)]
pub struct CommitEvent {
    pub reading: String,
    pub candidate: Candidate,
    pub analysis: Arc<AnalyzedContext>,
}

pub struct Page<'a> {
    pub index: usize,
    pub candidates: &'a [Candidate],
    pub has_next: bool,
}
