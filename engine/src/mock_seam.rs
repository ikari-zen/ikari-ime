//! テスト用の seam のモック（`SegmentSeam` / `SegConv`）。コアの `Segmented` の代わりに、
//! 決まった規則で区切りと候補を作る。**変換の質は確かめない。エンジンの配線（番号、版、締め切り、
//! 上限、ログ、学習の事象の受け渡し）だけを、結果が決まっている相手で確かめるためのもの**。
//! 本物のコアで通すテストは `tests/segments/real_core.rs`。
//!
//! - 区切りは `MockSeam::split(reading, 境界の文字位置)` で決める。決めていない読みは 1 文節
//!   （1 文節のときは読み全体の変換をそのまま使う。ADR 0006 §2.3）。
//! - 複数の文節の候補は `mock_surface(読み, 番号)`（`「モック:読み#番号」`）の 12 件。コアの塊は 9 件。
//!   テストは文字列そのものではなく `mock_surface` で期待値を作る（辞書の語ではないことが見て分かる）。
//! - 呼び出しは `MockSeam::calls` に残す（締め切り、左の選択、塊の番号）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use ikari_core::{
    Candidate, CandidateOrigin, Commit, CommitEvent, Context, Conversion, Converter, SegmentChoice,
    SegmentCommit, SegmentError, SegmentLayout, SegmentSpan, SegmentedOrigin,
};

use crate::segments::{Chunk, SegConv, SegmentSeam};

/// 複数の文節の候補の総数
pub(crate) const MOCK_LIST_LEN: usize = 12;
/// コアの 1 塊の件数
pub(crate) const MOCK_CHUNK: usize = 9;

/// モックが受けた呼び出し 1 件
#[derive(Clone, Debug)]
pub(crate) struct Call {
    pub what: &'static str,
    pub seg: usize,
    pub index: usize,
    /// `left` の長さ（初めて作るときの左の選択の数）
    pub left_len: usize,
    pub left: Vec<SegmentChoice>,
    pub deadline: Option<Instant>,
    pub at: Instant,
}

impl Call {
    /// 待たずに作った（締め切りがない、または呼んだ時刻以前）
    pub fn without_waiting(&self) -> bool {
        self.deadline.is_none_or(|d| d <= self.at)
    }
}

pub(crate) struct MockSeam {
    me: Weak<MockSeam>,
    converter: Arc<Converter>,
    splits: Mutex<HashMap<String, Vec<usize>>>,
    /// 塊を初めて作るときにかかる時間（LM の待ちの代わり）
    delay: Mutex<Duration>,
    calls: Mutex<Vec<Call>>,
    /// 再変換で受けた (表記, 左文脈, 確定履歴)
    reconverted: Mutex<Vec<(String, String, Vec<Commit>)>>,
}

impl MockSeam {
    pub fn new(converter: Arc<Converter>) -> Arc<MockSeam> {
        Arc::new_cyclic(|me| MockSeam {
            me: me.clone(),
            converter,
            splits: Mutex::new(HashMap::new()),
            delay: Mutex::new(Duration::ZERO),
            calls: Mutex::new(Vec::new()),
            reconverted: Mutex::new(Vec::new()),
        })
    }

    /// `reading` を `boundaries`（読みの文字位置）で区切る
    pub fn split(&self, reading: &str, boundaries: &[usize]) {
        self.splits
            .lock()
            .unwrap()
            .insert(reading.to_owned(), boundaries.to_vec());
    }

    pub fn set_delay(&self, delay: Duration) {
        *self.delay.lock().unwrap() = delay;
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    pub fn reconverted(&self) -> Vec<(String, String, Vec<Commit>)> {
        self.reconverted.lock().unwrap().clone()
    }

    fn record(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }

    fn build(&self, mut whole: Conversion, origin: SegmentedOrigin) -> Box<dyn SegConv> {
        // 1 位の表記が要るので、最初のページを作っておく（待たない）
        let _ = whole.page_until(0, None);
        let reading = whole.reading().to_owned();
        let chars: Vec<char> = reading.chars().collect();
        let mut bounds = self
            .splits
            .lock()
            .unwrap()
            .get(&reading)
            .cloned()
            .unwrap_or_default();
        bounds.retain(|b| *b > 0 && *b < chars.len());
        let mut starts = vec![0];
        starts.extend(bounds);
        let mut ends: Vec<usize> = starts[1..].to_vec();
        ends.push(chars.len());
        let synthetic = origin == SegmentedOrigin::Reconvert;
        let top = whole.generated().first().map(|c| c.surface.clone());
        let spans = starts
            .iter()
            .zip(&ends)
            .map(|(s, e)| {
                let piece: String = chars[*s..*e].iter().collect();
                let surface = if synthetic {
                    piece.clone()
                } else if starts.len() == 1 {
                    top.clone().unwrap_or_else(|| piece.clone())
                } else {
                    mock_surface(&piece, 0)
                };
                SegmentSpan {
                    start: *s,
                    end: *e,
                    reading: piece,
                    surface,
                }
            })
            .collect::<Vec<_>>();
        let lists = vec![None; spans.len()];
        Box::new(MockSeg {
            seam: self.me.upgrade().expect("the seam is alive"),
            whole,
            layout: SegmentLayout { rev: 0, spans },
            lists,
            origin,
            synthetic,
        })
    }
}

/// モックの候補の表記の先頭。本物の辞書の語と取り違えないための目印
const MOCK_SURFACE_PREFIX: &str = "モック:";

/// 複数の文節の `j` 番目の候補の表記
pub(crate) fn mock_surface(reading: &str, j: usize) -> String {
    format!("{MOCK_SURFACE_PREFIX}{reading}#{j}")
}

fn mock_candidate(template: &Candidate, surface: String, j: usize) -> Candidate {
    let mut c = template.clone();
    c.surface = surface;
    c.score = i32::try_from(j).unwrap_or(i32::MAX);
    c.origin = CandidateOrigin::Dictionary;
    c
}

// 本物の seam は `Converter` を持つ。モックは `reconvert` の元になる変換をそれで作る
pub(crate) struct MockSeg {
    seam: Arc<MockSeam>,
    whole: Conversion,
    layout: SegmentLayout,
    /// 複数の文節のときの、文節ごとの生成済みの列（1 文節のときは使わない）
    lists: Vec<Option<Vec<Candidate>>>,
    origin: SegmentedOrigin,
    /// 再変換: 1 文節でも合成の列を使い、0 番は元の表記（読みの断片そのもの）にする
    synthetic: bool,
}

impl MockSeg {
    /// 文節（読み `reading`）の `j` 番目の候補の表記
    fn surface_of(&self, reading: &str, j: usize) -> String {
        if self.synthetic && j == 0 {
            reading.to_owned()
        } else {
            mock_surface(reading, j)
        }
    }
}

/// 読めない表記（再変換が `Empty` を返す）
pub(crate) const UNREADABLE: &str = "読めない";

impl SegmentSeam for MockSeam {
    fn segment(&self, whole: Conversion) -> Box<dyn SegConv> {
        self.build(whole, SegmentedOrigin::Convert)
    }

    /// `surface` を読みとして扱う（かなだけで試す）。0 番は元の表記
    fn reconvert(
        &self,
        surface: &str,
        ctx: &Context,
        hint: &[Commit],
        deadline: Option<Instant>,
    ) -> Result<Box<dyn SegConv>, SegmentError> {
        self.record(Call {
            what: "reconvert",
            seg: 0,
            index: 0,
            left_len: 0,
            left: Vec::new(),
            deadline,
            at: Instant::now(),
        });
        self.reconverted.lock().unwrap().push((
            surface.to_owned(),
            ctx.left_text.clone(),
            hint.to_vec(),
        ));
        if surface == UNREADABLE {
            return Err(SegmentError::Empty);
        }
        let whole = self.converter.convert_until(surface, ctx, deadline);
        Ok(self.build(whole, SegmentedOrigin::Reconvert))
    }
}

impl SegConv for MockSeg {
    fn layout(&self) -> &SegmentLayout {
        &self.layout
    }

    fn whole(&self) -> &Conversion {
        &self.whole
    }

    fn whole_mut(&mut self) -> &mut Conversion {
        &mut self.whole
    }

    fn chunk(
        &mut self,
        seg: usize,
        index: usize,
        left: &[SegmentChoice],
        deadline: Option<Instant>,
    ) -> Result<Option<Chunk>, SegmentError> {
        self.seam.record(Call {
            what: "chunk",
            seg,
            index,
            left_len: left.len(),
            left: left.to_vec(),
            deadline,
            at: Instant::now(),
        });
        if seg >= self.layout.spans.len() {
            return Err(SegmentError::OutOfRange);
        }
        if self.layout.spans.len() == 1 && !self.synthetic {
            // 1 文節は読み全体の列そのもの
            return Ok(self.whole.page_until(index, deadline).map(|p| Chunk {
                candidates: p.candidates.to_vec(),
                has_next: p.has_next,
            }));
        }
        if self.lists[seg].is_none() {
            let delay = *self.seam.delay.lock().unwrap();
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
            let template = self.whole.generated()[0].clone();
            let reading = self.layout.spans[seg].reading.clone();
            self.lists[seg] = Some(
                (0..MOCK_LIST_LEN)
                    .map(|j| mock_candidate(&template, self.surface_of(&reading, j), j))
                    .collect(),
            );
        }
        let list = self.lists[seg].as_ref().expect("just made");
        let start = index * MOCK_CHUNK;
        if start >= list.len() {
            return Ok(None);
        }
        let end = (start + MOCK_CHUNK).min(list.len());
        Ok(Some(Chunk {
            candidates: list[start..end].to_vec(),
            has_next: end < list.len(),
        }))
    }

    fn origin(&self) -> SegmentedOrigin {
        self.origin
    }

    /// 読み全体の事象（Literal がなければ）、続けて変えた文節（0 番以外の Candidate）の事象。
    /// secure（`whole.commit(0)` が None）なら空
    fn commit(&self, choices: &[SegmentChoice]) -> Result<SegmentCommit, SegmentError> {
        let spans = &self.layout.spans;
        if choices.len() != spans.len() {
            return Err(SegmentError::OutOfRange);
        }
        let single = spans.len() == 1 && !self.synthetic;
        let mut surfaces = Vec::new();
        for (span, choice) in spans.iter().zip(choices) {
            surfaces.push(match choice {
                SegmentChoice::Candidate(j) if single => self
                    .whole
                    .generated()
                    .get(*j)
                    .ok_or(SegmentError::OutOfRange)?
                    .surface
                    .clone(),
                SegmentChoice::Candidate(j) if *j < MOCK_LIST_LEN => {
                    self.surface_of(&span.reading, *j)
                }
                SegmentChoice::Candidate(_) => return Err(SegmentError::OutOfRange),
                SegmentChoice::Literal(text) => text.clone(),
            });
        }
        let surface: String = surfaces.concat();
        let Some(template) = self.whole.commit(0) else {
            return Ok(SegmentCommit {
                surface,
                events: Vec::new(),
                history: Vec::new(),
            });
        };
        let mut events = Vec::new();
        let any_literal = choices
            .iter()
            .any(|c| matches!(c, SegmentChoice::Literal(_)));
        if !any_literal {
            let mut candidate = template.candidate.clone();
            candidate.surface = surface.clone();
            events.push(CommitEvent {
                reading: self.whole.reading().to_owned(),
                candidate,
                analysis: Arc::clone(&template.analysis),
            });
        }
        if !single {
            for (span, choice) in spans.iter().zip(choices) {
                if let SegmentChoice::Candidate(j) = choice
                    && *j != 0
                {
                    events.push(CommitEvent {
                        reading: span.reading.clone(),
                        candidate: mock_candidate(
                            &template.candidate,
                            self.surface_of(&span.reading, *j),
                            *j,
                        ),
                        analysis: Arc::clone(&template.analysis),
                    });
                }
            }
        }
        Ok(SegmentCommit {
            history: vec![Commit {
                reading: self.whole.reading().to_owned(),
                surface: surface.clone(),
            }],
            surface,
            events,
        })
    }

    /// 文節 `index` を `delta` 文字ぶん伸縮する。0 〜 index-1 は変えず、境界から後ろは 1 文節にする
    /// （本物は規則で分ける）。動かせないときは何もしない
    fn resize(
        &mut self,
        index: usize,
        delta: i32,
        left: &[SegmentChoice],
        deadline: Option<Instant>,
    ) -> Result<(), SegmentError> {
        self.seam.record(Call {
            what: "resize",
            seg: index,
            index: 0,
            left_len: left.len(),
            left: left.to_vec(),
            deadline,
            at: Instant::now(),
        });
        let span = self
            .layout
            .spans
            .get(index)
            .ok_or(SegmentError::OutOfRange)?
            .clone();
        let chars: Vec<char> = self.whole.reading().chars().collect();
        let len = i64::try_from(span.end - span.start).unwrap_or(i64::MAX);
        let new_len = len + i64::from(delta);
        let boundary = i64::try_from(span.start).unwrap_or(i64::MAX) + new_len;
        let total = i64::try_from(chars.len()).unwrap_or(i64::MAX);
        if delta == 0 || new_len < 1 || boundary > total {
            return Ok(());
        }
        let boundary = usize::try_from(boundary).expect("positive");
        let piece = |s: usize, e: usize| -> SegmentSpan {
            let reading: String = chars[s..e].iter().collect();
            SegmentSpan {
                start: s,
                end: e,
                surface: self.surface_of(&reading, 0),
                reading,
            }
        };
        let mut tail = vec![piece(span.start, boundary)];
        if boundary < chars.len() {
            tail.push(piece(boundary, chars.len()));
        }
        self.layout.spans.truncate(index);
        self.layout.spans.extend(tail);
        self.layout.rev += 1;
        self.lists.truncate(index);
        self.lists.resize(self.layout.spans.len(), None);
        Ok(())
    }
}
