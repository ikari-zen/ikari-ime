//! 文節の区切りと、文節ごとの候補（ADR 0006 §2〜§4、§5.1）。
//!
//! `Conversion` は変えない。文節は `Segmented` が包み、依存の向きは `Segmented -> Conversion` の一方向。
//!
//! 文節ごとの候補の列（`Conversion`）は、文節を初めて開くときに作る。作るときの左文脈は、
//! 左の文節の「そのとき選ばれていた候補」の語で、一度作った列は左の選択が変わっても作り直さない
//! （返したページを差し替えない。NFR-05）。列の 0 番は、いま表示している表記に固定する。

use std::collections::HashSet;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use super::{CommitEvent, Conversion, Converter, Page};
use crate::analysis::AnalyzedContext;
use crate::candidate::{Candidate, CandidateOrigin, Segment};
use crate::context::{Commit, Context};
use crate::dict::Grammar;
use crate::lattice;
use crate::nbest::NBest;
use crate::pos::{
    ClauseWord, SegmentRole, clause_starts, closes_phrase, joins_following_noun, segment_role,
};

/// 文節 1 つ。位置は読みの文字位置（表記ではなく読みで持つので、候補を選び直しても動かない）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentSpan {
    pub start: usize,
    pub end: usize,
    pub reading: String,
    /// 候補の 0 番（いま表示している表記）
    pub surface: String,
}

/// 区切り。`rev` は区切り直し（`Converter::resize`）で 1 つ上がる
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SegmentLayout {
    pub rev: u32,
    pub spans: Vec<SegmentSpan>,
}

/// 文節の選択
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SegmentChoice {
    /// 生成済みの列の中の位置（ページをまたいだ通し番号）
    Candidate(usize),
    /// 殻が作った表記（字種変換）。学習しない
    Literal(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentError {
    /// `layout_rev` が古い
    StaleLayout,
    /// 文節の番号、選択の番号、選択の数が範囲外
    OutOfRange,
    /// 変換するものがない（読みが空、再変換で読める部分がない）
    Empty,
}

impl fmt::Display for SegmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // ユーザーの文字列は入れない（NFR-19）
        match self {
            SegmentError::StaleLayout => f.write_str("stale layout"),
            SegmentError::OutOfRange => f.write_str("out of range"),
            SegmentError::Empty => f.write_str("nothing to convert"),
        }
    }
}

impl std::error::Error for SegmentError {}

/// 区切りの出どころ。再変換は確定履歴に積まず、変えなかったときは学習しない（ADR 0006 §9.7）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentedOrigin {
    Convert,
    Reconvert,
}

/// 区切りと、文節ごとの候補の列。読み全体の `Conversion` を中に持つ（1文節ならその列をそのまま使う。§2.3）
pub struct Segmented {
    pub(super) whole: Conversion,
    pub(super) layout: SegmentLayout,
    /// 文節ごとの候補の列。まだ作っていない文節は None
    pub(super) lists: Vec<Option<Conversion>>,
    pub(super) origin: SegmentedOrigin,
    /// 文節ごとの 0 番（いま表示している表記とその語）。`layout.spans` はここから作る
    pub(super) heads: Vec<Candidate>,
}

/// 確定の結果
#[derive(Clone, Debug)]
pub struct SegmentCommit {
    /// 確定する文字列（文節の表記をつないだもの）
    pub surface: String,
    /// `LearningStore::record` に順に渡すもの。変えた文節ごとと、読み全体（作れるときだけ）。secure なら空
    pub events: Vec<CommitEvent>,
    /// 確定履歴に積むもの。secure なら空
    pub history: Vec<Commit>,
}

/// 語の並びを文節に分ける（ADR 0006 §2.2）。未知語は名詞として扱う。範囲は語の添字
pub(super) fn group_words(words: &[Segment], grammar: &dyn Grammar) -> Vec<Range<usize>> {
    let clause_words: Vec<ClauseWord> = words
        .iter()
        .map(|w| {
            if w.word.is_none() {
                return ClauseWord {
                    role: SegmentRole::Compound,
                    joins_following_noun: true,
                    closes_phrase: false,
                };
            }
            let pos = grammar.pos_name(w.pos_id).unwrap_or("");
            ClauseWord {
                role: segment_role(pos),
                joins_following_noun: joins_following_noun(pos),
                closes_phrase: closes_phrase(pos),
            }
        })
        .collect();
    let mut groups: Vec<Range<usize>> = Vec::new();
    for (i, starts) in clause_starts(&clause_words).into_iter().enumerate() {
        match groups.last_mut() {
            Some(last) if !starts => last.end = i + 1,
            _ => groups.push(i..i + 1),
        }
    }
    groups
}

/// 語の並びを 0 番の候補にする。`score` は 0（先頭に固定した候補は最も小さい）
pub(super) fn head_of(words: &[Segment], origin: CandidateOrigin) -> Candidate {
    Candidate {
        surface: words.iter().map(|w| w.surface.as_str()).collect(),
        segments: words.to_vec(),
        base_cost: 0,
        score: 0,
        origin,
    }
}

/// 殻が作った表記や、読めなかった部分の候補。語は表記だけの未知語 1 つで、読みは空
pub(super) fn literal_candidate(text: &str, grammar: &dyn Grammar) -> Candidate {
    let unk = grammar.unknown();
    Candidate {
        surface: text.to_owned(),
        segments: vec![Segment {
            reading: String::new(),
            surface: text.to_owned(),
            word: None,
            left_id: unk.left_id,
            right_id: unk.right_id,
            pos_id: unk.pos_id,
        }],
        base_cost: 0,
        score: 0,
        origin: CandidateOrigin::Literal,
    }
}

fn reading_of(words: &[Segment]) -> String {
    words.iter().map(|w| w.reading.as_str()).collect()
}

/// 0 番の並びから区切りを作る。位置は読みの文字位置の累計
pub(super) fn layout_of(rev: u32, heads: &[Candidate]) -> SegmentLayout {
    let mut start = 0;
    let spans = heads
        .iter()
        .map(|head| {
            let reading = reading_of(&head.segments);
            let end = start + reading.chars().count();
            let span = SegmentSpan {
                start,
                end,
                reading,
                surface: head.surface.clone(),
            };
            start = end;
            span
        })
        .collect();
    SegmentLayout { rev, spans }
}

fn is_literal(head: &Candidate) -> bool {
    head.origin == CandidateOrigin::Literal
}

/// 右の文節の最初の語の左ID。右がなければ None（EOS は `bos_eos_id`）
fn left_id_of_first_word(head: Option<&Candidate>) -> Option<u16> {
    head?.segments.first().map(|w| w.left_id)
}

/// 先頭から、読みの文字数が `chars` に達するまでの語の数
fn words_within(words: &[Segment], chars: usize) -> usize {
    let mut consumed = 0;
    for (i, word) in words.iter().enumerate() {
        if consumed >= chars {
            return i;
        }
        consumed += word.reading.chars().count();
    }
    words.len()
}

impl Conversion {
    /// 左の文節の語を足した文脈と左文脈の解析（ADR 0006 §3.1-1）。secure は文脈を空のままにし、
    /// BOS の右ID だけ左の語に合わせる（同じ変換の中の連接で、ユーザーの文脈ではない）
    pub(super) fn left_context(&self, words: &[Segment]) -> (Context, Arc<AnalyzedContext>) {
        let Some(last) = words.last() else {
            return (self.ctx.clone(), Arc::clone(&self.analysis));
        };
        let mut ctx = self.ctx.clone();
        if self.ctx.secure {
            let analysis = AnalyzedContext {
                bos_right_id: last.right_id,
                ..AnalyzedContext::default()
            };
            return (ctx, Arc::new(analysis));
        }
        for word in words {
            ctx.left_text.push_str(&word.surface);
        }
        let analysis = self.analysis.extended(words, &*self.grammar, &*self.dict);
        (ctx, Arc::new(analysis))
    }

    /// 同じ辞書、Ranker、設定で、別の読み `key` の変換を作る（文節の候補、区切り直しのための変換）。
    /// `end_left_id` は右の文節の最初の語の左ID（なければ EOS）。`boundaries` は読みの中の強制する境界。
    /// `pin` があれば、並べ替えのあとに 0 番へ固定する。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn derive(
        &self,
        key: &str,
        ctx: Context,
        analysis: Arc<AnalyzedContext>,
        end_left_id: Option<u16>,
        boundaries: &[u32],
        pin: Option<Candidate>,
        deadline: Option<Instant>,
    ) -> Conversion {
        let starts = analysis.bos_options();
        // 元の変換と同じ辺の補正（ADR 0008 §4.3）
        let rules = self.lattice.rules();
        let lattice = if boundaries.is_empty() {
            lattice::build_with_starts(key, &*self.dict, &*self.grammar, &starts, rules)
        } else {
            lattice::build_constrained(key, &*self.dict, &*self.grammar, &starts, boundaries, rules)
        };
        let end = end_left_id.unwrap_or_else(|| self.grammar.bos_eos_id());
        let nbest = NBest::with_end(&lattice, &*self.grammar, end);
        let mut conv = Conversion {
            dict: Arc::clone(&self.dict),
            grammar: Arc::clone(&self.grammar),
            ranker: Arc::clone(&self.ranker),
            rewriters: self.rewriters.clone(),
            clock: Arc::clone(&self.clock),
            opts: self.opts.clone(),
            ctx,
            analysis,
            lattice,
            nbest,
            generated: Vec::new(),
            seen: HashSet::new(),
            exhausted: false,
            deadline,
        };
        if key.is_empty() {
            conv.exhausted = true;
        } else {
            conv.build_first_pool();
        }
        if let Some(pin) = pin {
            conv.pin_first(pin);
            conv.rescue_generated_kana();
        }
        conv
    }

    /// 同じ表記の候補を先頭へ移す。なければ `head` を先頭に足す（表示中の表記を 0 番にする。NFR-05）
    fn pin_first(&mut self, head: Candidate) {
        if head.surface.is_empty() {
            return;
        }
        match self
            .generated
            .iter()
            .position(|c| c.surface == head.surface)
        {
            Some(0) => {}
            Some(at) => {
                let found = self.generated.remove(at);
                self.generated.insert(0, found);
            }
            None => {
                self.seen.insert(head.surface.clone());
                self.generated.insert(0, head);
            }
        }
    }
}

impl Segmented {
    pub fn layout(&self) -> &SegmentLayout {
        &self.layout
    }

    pub fn origin(&self) -> SegmentedOrigin {
        self.origin
    }

    /// 読み全体の変換。`ConvertReply.candidates`（読み全体の 1 ページ目）、旧式の `PageRequest` と `Commit` が使う。
    /// 再変換（`Converter::reconvert`）では読みが空の変換で、候補はない
    pub fn whole(&self) -> &Conversion {
        &self.whole
    }

    pub fn whole_mut(&mut self) -> &mut Conversion {
        &mut self.whole
    }

    /// 区切りを変えていない 1 文節の変換は、読み全体の変換の候補の列をそのまま使う（§2.3）
    fn uses_whole_list(&self) -> bool {
        self.origin == SegmentedOrigin::Convert
            && self.layout.rev == 0
            && self.layout.spans.len() == 1
    }

    /// 文節 `seg` の `index` ページ目。初めて作るとき（だけ）`left`（文節 0〜seg-1 の選択）を左文脈に使う。
    /// 一度返したページは変えない。範囲外のページは `Ok(None)`、文節の番号が範囲外なら `OutOfRange`。
    /// `left` が文節の数より多い、または選択が範囲外なら（初めて作るときに）`OutOfRange`。
    /// 足りない分は 0 番を選んだものとして扱う
    pub fn page_until(
        &mut self,
        seg: usize,
        index: usize,
        left: &[SegmentChoice],
        deadline: Option<Instant>,
    ) -> Result<Option<Page<'_>>, SegmentError> {
        if seg >= self.layout.spans.len() {
            return Err(SegmentError::OutOfRange);
        }
        if self.uses_whole_list() {
            return Ok(self.whole.page_until(index, deadline));
        }
        if self.lists[seg].is_none() {
            let list = self.build_list(seg, left, deadline)?;
            self.lists[seg] = Some(list);
        }
        let list = self.lists[seg].as_mut().ok_or(SegmentError::OutOfRange)?;
        Ok(list.page_until(index, deadline))
    }

    /// 文節 `seg` の候補の列を新しく作る（0 番は `heads[seg]`）
    fn build_list(
        &self,
        seg: usize,
        left: &[SegmentChoice],
        deadline: Option<Instant>,
    ) -> Result<Conversion, SegmentError> {
        let head = self.heads[seg].clone();
        let left_words = self.left_words(seg, left)?;
        let (ctx, analysis) = self.whole.left_context(&left_words);
        let reading = self.layout.spans[seg].reading.clone();
        let end_left = left_id_of_first_word(self.heads.get(seg + 1));
        Ok(self
            .whole
            .derive(&reading, ctx, analysis, end_left, &[], Some(head), deadline))
    }

    /// 文節 `seg` の選択の候補（`Candidate` は生成済みの列の中の位置）
    fn chosen(&self, seg: usize, choice: &SegmentChoice) -> Result<Candidate, SegmentError> {
        match choice {
            SegmentChoice::Candidate(index) => {
                let found = if self.uses_whole_list() {
                    self.whole.generated().get(*index)
                } else if let Some(list) = &self.lists[seg] {
                    list.generated().get(*index)
                } else if *index == 0 {
                    self.heads.get(seg)
                } else {
                    None
                };
                found.cloned().ok_or(SegmentError::OutOfRange)
            }
            SegmentChoice::Literal(text) => {
                if text.is_empty() {
                    return Err(SegmentError::OutOfRange);
                }
                Ok(literal_candidate(text, &*self.whole.grammar))
            }
        }
    }

    /// 文節 0〜seg-1 の選択の語をつないだもの。`left` が短ければ残りは 0 番
    fn left_words(&self, seg: usize, left: &[SegmentChoice]) -> Result<Vec<Segment>, SegmentError> {
        if left.len() > seg {
            return Err(SegmentError::OutOfRange);
        }
        let mut words = Vec::new();
        for j in 0..seg {
            let choice = left.get(j).cloned().unwrap_or(SegmentChoice::Candidate(0));
            words.extend(self.chosen(j, &choice)?.segments);
        }
        Ok(words)
    }

    /// 確定。`choices` は文節ごとの選択（長さは文節の数）。変えた文節ごとの `CommitEvent` と、
    /// 読み全体の `CommitEvent`（どの文節にも Rewriter と `Literal` がないときだけ）を返す。
    /// 選択が範囲外なら `OutOfRange`。学習の規則は ADR 0006 §6.1
    pub fn commit(&self, choices: &[SegmentChoice]) -> Result<SegmentCommit, SegmentError> {
        let spans = &self.layout.spans;
        if choices.len() != spans.len() {
            return Err(SegmentError::OutOfRange);
        }
        let chosen: Vec<Candidate> = choices
            .iter()
            .enumerate()
            .map(|(seg, choice)| self.chosen(seg, choice))
            .collect::<Result<_, _>>()?;
        let surface: String = chosen.iter().map(|c| c.surface.as_str()).collect();
        let reading: String = spans.iter().map(|s| s.reading.as_str()).collect();
        let mut commit = SegmentCommit {
            surface,
            events: Vec::new(),
            history: Vec::new(),
        };
        if self.whole.ctx.secure || chosen.is_empty() {
            return Ok(commit);
        }
        // 再変換は確定履歴に積まない（選択の位置が文書の途中でありうる。§9.7）
        if self.origin == SegmentedOrigin::Convert {
            commit.history.push(Commit {
                reading: reading.clone(),
                surface: commit.surface.clone(),
            });
        }
        // 再変換で何も変えていなければ、学習しない
        let untouched = self.origin == SegmentedOrigin::Reconvert
            && self.layout.rev == 0
            && choices
                .iter()
                .all(|c| matches!(c, SegmentChoice::Candidate(0)));
        if !untouched {
            commit.events = self.events(&reading, choices, &chosen);
        }
        Ok(commit)
    }

    /// 学習のイベント（ADR 0006 §6.1）。先に読み全体、続いて変えた文節
    fn events(
        &self,
        reading: &str,
        choices: &[SegmentChoice],
        chosen: &[Candidate],
    ) -> Vec<CommitEvent> {
        let has_literal = choices
            .iter()
            .any(|c| matches!(c, SegmentChoice::Literal(_)))
            || chosen.iter().any(is_literal);
        let mut events = Vec::new();
        if chosen.len() == 1 {
            // 1 文節は今の `Conversion::commit` と同じ記録（Rewriter の候補も書式で覚える）
            if !has_literal {
                events.push(CommitEvent {
                    reading: reading.to_owned(),
                    candidate: chosen[0].clone(),
                    analysis: Arc::clone(&self.whole.analysis),
                });
            }
            return events;
        }
        let has_rewriter = chosen
            .iter()
            .any(|c| matches!(c.origin, CandidateOrigin::Rewriter { .. }));
        if !has_literal && !has_rewriter {
            let segments: Vec<Segment> = chosen.iter().flat_map(|c| c.segments.clone()).collect();
            let mut candidate = head_of(&segments, CandidateOrigin::Dictionary);
            candidate.base_cost = chosen
                .iter()
                .fold(0i32, |sum, c| sum.saturating_add(c.base_cost));
            candidate.score = candidate.base_cost;
            events.push(CommitEvent {
                reading: reading.to_owned(),
                candidate,
                analysis: Arc::clone(&self.whole.analysis),
            });
        }
        // 変えた文節。左文脈は、確定する左の文節の語
        let mut left_words: Vec<Segment> = Vec::new();
        for (seg, (choice, candidate)) in choices.iter().zip(chosen).enumerate() {
            let changed = matches!(choice, SegmentChoice::Candidate(index) if *index != 0);
            let span_reading = &self.layout.spans[seg].reading;
            if changed && !is_literal(candidate) && !span_reading.is_empty() {
                let analysis = self.whole.left_context(&left_words).1;
                events.push(CommitEvent {
                    reading: span_reading.clone(),
                    candidate: candidate.clone(),
                    analysis,
                });
            }
            left_words.extend(candidate.segments.iter().cloned());
        }
        events
    }
}

impl Converter {
    /// `convert_until` の結果を区切る（ADR 0006 §2）。先行計算はこれをキャッシュする。
    /// 区切りは Ranker が 1 位にした候補（`generated()[0]`）の語から作るので、表示は変わらない。
    /// 1 位が辞書の候補でない（かな、Rewriter）なら読み全体を 1 文節にする
    pub fn segment(&self, whole: Conversion) -> Segmented {
        let heads = match whole.generated().first() {
            None => Vec::new(),
            Some(top) if top.origin != CandidateOrigin::Dictionary => vec![top.clone()],
            Some(top) => {
                let groups = group_words(&top.segments, &*self.grammar);
                if groups.len() <= 1 {
                    vec![top.clone()]
                } else {
                    groups
                        .into_iter()
                        .map(|g| head_of(&top.segments[g], CandidateOrigin::Dictionary))
                        .collect()
                }
            }
        };
        let layout = layout_of(0, &heads);
        let lists = heads.iter().map(|_| None).collect();
        Segmented {
            whole,
            layout,
            lists,
            origin: SegmentedOrigin::Convert,
            heads,
        }
    }

    /// 区切り直し（§4）。フォーカス中の文節 `index` を `delta` 文字ぶん伸縮する（縮める = 負）。
    /// 動かせないとき（1 文字を縮める、最後の文節を伸ばす、読めない部分 `Literal` の文節）は何も変えずに `Ok`。
    /// 変わったら `layout.rev` が 1 上がり、`index` 以降の候補の列は作り直す（`index` の 1 ページ目は
    /// この呼び出しで作る。`deadline` は LM を待たないなら `None`）。
    /// 読めない部分（`Literal` の文節）は壁になり、その右の文節は動かさない
    pub fn resize(
        &self,
        seg: &mut Segmented,
        index: usize,
        delta: i32,
        left: &[SegmentChoice],
        deadline: Option<Instant>,
    ) -> Result<(), SegmentError> {
        let count = seg.layout.spans.len();
        if index >= count {
            return Err(SegmentError::OutOfRange);
        }
        if delta == 0 || is_literal(&seg.heads[index]) {
            return Ok(());
        }
        // index を含む「読める文節の連なり」は [index, wall)。Literal の文節が壁
        let wall = (index + 1..count)
            .find(|&j| is_literal(&seg.heads[j]))
            .unwrap_or(count);
        let start = seg.layout.spans[index].start;
        let block_end = seg.layout.spans[wall - 1].end;
        let new_len = i64::try_from(seg.layout.spans[index].end - start).unwrap_or(i64::MAX)
            + i64::from(delta);
        let Ok(new_len) = usize::try_from(new_len) else {
            return Ok(());
        };
        if new_len < 1 || start + new_len > block_end {
            return Ok(());
        }
        let left_words = seg.left_words(index, left)?;
        let (ctx, analysis) = seg.whole.left_context(&left_words);
        let key: String = seg.layout.spans[index..wall]
            .iter()
            .map(|s| s.reading.as_str())
            .collect();
        let boundaries: Vec<u32> = if start + new_len < block_end {
            vec![u32::try_from(new_len).unwrap_or(u32::MAX)]
        } else {
            Vec::new()
        };
        let tail = seg.whole.derive(
            &key,
            ctx.clone(),
            Arc::clone(&analysis),
            left_id_of_first_word(seg.heads.get(wall)),
            &boundaries,
            None,
            deadline,
        );
        let words = self.resized_words(&tail, &key, new_len);
        let (first, rest) = words.split_at(words_within(&words, new_len));

        let mut heads: Vec<Candidate> = seg.heads[..index].to_vec();
        heads.push(head_of(first, CandidateOrigin::Dictionary));
        for group in group_words(rest, &*self.grammar) {
            heads.push(head_of(&rest[group], CandidateOrigin::Dictionary));
        }
        let rebuilt = heads.len();
        heads.extend(seg.heads[wall..].iter().cloned());

        // 候補の列: index より前と壁の右は残し、index は今作り、間は遅延して作る
        let first_list = seg.whole.derive(
            &reading_of(first),
            ctx,
            analysis,
            left_id_of_first_word(heads.get(index + 1)),
            &[],
            Some(heads[index].clone()),
            deadline,
        );
        let mut lists = std::mem::take(&mut seg.lists);
        let kept_right = lists.split_off(wall);
        lists.truncate(index);
        lists.push(Some(first_list));
        lists.extend((index + 1..rebuilt).map(|_| None));
        lists.extend(kept_right);

        seg.layout = layout_of(seg.layout.rev.wrapping_add(1), &heads);
        seg.heads = heads;
        seg.lists = lists;
        Ok(())
    }

    /// 区切り直しの変換の 1 位（辞書の候補）の語。読みの `boundary` 文字目で分けられないときは
    /// 1 文字ずつの語にする（起こらないはずの守り）
    fn resized_words(&self, tail: &Conversion, key: &str, boundary: usize) -> Vec<Segment> {
        let top = tail
            .generated()
            .iter()
            .find(|c| c.origin == CandidateOrigin::Dictionary);
        if let Some(top) = top {
            let split = words_within(&top.segments, boundary);
            let consumed: usize = top.segments[..split]
                .iter()
                .map(|w| w.reading.chars().count())
                .sum();
            if consumed == boundary {
                return top.segments.clone();
            }
        }
        let unk = self.grammar.unknown();
        key.chars()
            .map(|c| Segment {
                reading: c.to_string(),
                surface: c.to_string(),
                word: None,
                left_id: unk.left_id,
                right_id: unk.right_id,
                pos_id: unk.pos_id,
            })
            .collect()
    }
}
