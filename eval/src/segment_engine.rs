//! 文節の API の口（ADR 0006 §5.1）。評価の部品（区切りの F1、打鍵数、読みの推定）はこの trait だけに依存する。
//!
//! 本物は `CoreSegmentEngine`（`Converter::segment` / `resize` / `reconvert` を呼ぶ）。
//! 単体テストは trait のモックで通す（コアの本体が入る前でも、ハーネスの計算を確かめられるように）。

use std::time::{Duration, Instant};

use ikari_core::{Context, Converter, SegmentChoice, SegmentError, SegmentLayout, Segmented};

/// 評価で使う、文節の操作の最小の集合。シグネチャは ADR 0006 §5.1 の公開 API に合わせる
pub trait SegmentEngine {
    /// 区切りと文節ごとの候補を持つ 1 回の変換
    type Session;

    /// 読みを変換して区切る（`Converter::convert` + `Converter::segment`）
    fn open(&self, reading: &str, context: &str) -> Result<Self::Session, SegmentError>;

    /// いまの区切り
    fn layout<'a>(&self, session: &'a Self::Session) -> &'a SegmentLayout;

    /// 文節 `seg` の候補の表記を、先頭から `max` 件まで返す（`Segmented::page_until` をたどる）。
    /// `left` は文節 0〜seg-1 の選択（初めて作るときだけ左文脈に使われる）
    fn surfaces(
        &self,
        session: &mut Self::Session,
        seg: usize,
        left: &[SegmentChoice],
        max: usize,
    ) -> Result<Vec<String>, SegmentError>;

    /// 区切り直し（`Converter::resize`）。文節 `index` を `delta` 文字ぶん伸縮する
    fn resize(
        &self,
        session: &mut Self::Session,
        index: usize,
        delta: i32,
        left: &[SegmentChoice],
    ) -> Result<(), SegmentError>;

    /// 今の UI の候補の列。読み全体の候補の表記を、先頭から `max` 件まで返す（`Segmented::whole_mut`）
    fn whole_surfaces(&self, session: &mut Self::Session, max: usize) -> Vec<String>;

    /// 再変換（`Converter::reconvert`）。推定した区切りを返す
    fn reconvert(&self, surface: &str, context: &str) -> Result<SegmentLayout, SegmentError>;
}

/// `Converter` を使う本物の実装
pub struct CoreSegmentEngine<'a> {
    converter: &'a Converter,
    /// 変換 1 回の LM の締め切り（開始からの時間）。None なら `Converter` の既定
    lm_deadline: Option<Duration>,
}

impl<'a> CoreSegmentEngine<'a> {
    pub fn new(converter: &'a Converter, lm_deadline: Option<Duration>) -> Self {
        Self {
            converter,
            lm_deadline,
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.lm_deadline.map(|d| Instant::now() + d)
    }
}

fn context_of(text: &str) -> Context {
    Context::new().with_left_text(text)
}

impl SegmentEngine for CoreSegmentEngine<'_> {
    type Session = Segmented;

    fn open(&self, reading: &str, context: &str) -> Result<Segmented, SegmentError> {
        if reading.is_empty() {
            return Err(SegmentError::Empty);
        }
        let ctx = context_of(context);
        let whole = match self.deadline() {
            Some(d) => self.converter.convert_until(reading, &ctx, Some(d)),
            None => self.converter.convert(reading, &ctx),
        };
        Ok(self.converter.segment(whole))
    }

    fn layout<'a>(&self, session: &'a Segmented) -> &'a SegmentLayout {
        session.layout()
    }

    fn surfaces(
        &self,
        session: &mut Segmented,
        seg: usize,
        left: &[SegmentChoice],
        max: usize,
    ) -> Result<Vec<String>, SegmentError> {
        let mut out = Vec::new();
        let mut index = 0;
        while out.len() < max {
            let Some(page) = session.page_until(seg, index, left, self.deadline())? else {
                break;
            };
            out.extend(page.candidates.iter().map(|c| c.surface.clone()));
            index += 1;
        }
        out.truncate(max);
        Ok(out)
    }

    fn resize(
        &self,
        session: &mut Segmented,
        index: usize,
        delta: i32,
        left: &[SegmentChoice],
    ) -> Result<(), SegmentError> {
        self.converter
            .resize(session, index, delta, left, Some(Instant::now()))
    }

    fn whole_surfaces(&self, session: &mut Segmented, max: usize) -> Vec<String> {
        let whole = session.whole_mut();
        let mut out = Vec::new();
        let mut index = 0;
        while out.len() < max {
            let page = match self.deadline() {
                Some(d) => whole.page_until(index, Some(d)),
                None => whole.page(index),
            };
            let Some(page) = page else {
                break;
            };
            out.extend(page.candidates.iter().map(|c| c.surface.clone()));
            index += 1;
        }
        out.truncate(max);
        out
    }

    fn reconvert(&self, surface: &str, context: &str) -> Result<SegmentLayout, SegmentError> {
        self.converter
            .reconvert(surface, &context_of(context), &[], self.deadline())
            .map(|s| s.layout().clone())
    }
}

#[cfg(test)]
pub(crate) mod mock {
    //! trait のモック。辞書の代わりに、読みごとの区切りと候補を表で持つ

    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    use ikari_core::SegmentSpan;

    use super::*;

    /// 区切り（各文節の読みの長さ）ごとの、文節の候補の表
    #[derive(Default)]
    pub(crate) struct MockEngine {
        /// 読み → 最初の区切り（文節の読み）
        pub initial: HashMap<String, Vec<String>>,
        /// 文節の読み → 候補の表記（0 番が先頭）
        pub candidates: HashMap<String, Vec<String>>,
        /// 読み全体 → 今の UI の候補
        pub whole: HashMap<String, Vec<String>>,
        /// 表記 → 再変換で推定する読み（文節の読みの並び）
        pub reconverted: HashMap<String, Vec<String>>,
        pub resizes: Cell<usize>,
        pub opens: Cell<usize>,
        pub surface_calls: RefCell<Vec<(String, usize)>>,
    }

    pub(crate) struct MockSession {
        pub reading: String,
        /// 文節の読みの並び
        pub segments: Vec<String>,
        pub layout: SegmentLayout,
    }

    impl MockEngine {
        fn candidates_of(&self, reading: &str) -> Vec<String> {
            self.candidates
                .get(reading)
                .cloned()
                .unwrap_or_else(|| vec![reading.to_owned()])
        }

        fn rebuild(&self, segments: &[String]) -> SegmentLayout {
            let mut start = 0;
            let spans = segments
                .iter()
                .map(|r| {
                    let end = start + r.chars().count();
                    let span = SegmentSpan {
                        start,
                        end,
                        reading: r.clone(),
                        surface: self.candidates_of(r)[0].clone(),
                    };
                    start = end;
                    span
                })
                .collect();
            SegmentLayout { rev: 0, spans }
        }
    }

    impl SegmentEngine for MockEngine {
        type Session = MockSession;

        fn open(&self, reading: &str, _context: &str) -> Result<MockSession, SegmentError> {
            self.opens.set(self.opens.get() + 1);
            let segments = self
                .initial
                .get(reading)
                .cloned()
                .ok_or(SegmentError::Empty)?;
            let layout = self.rebuild(&segments);
            Ok(MockSession {
                reading: reading.to_owned(),
                segments,
                layout,
            })
        }

        fn layout<'a>(&self, session: &'a MockSession) -> &'a SegmentLayout {
            &session.layout
        }

        fn surfaces(
            &self,
            session: &mut MockSession,
            seg: usize,
            _left: &[SegmentChoice],
            max: usize,
        ) -> Result<Vec<String>, SegmentError> {
            let reading = session
                .segments
                .get(seg)
                .ok_or(SegmentError::OutOfRange)?
                .clone();
            self.surface_calls.borrow_mut().push((reading.clone(), seg));
            let mut list = self.candidates_of(&reading);
            list.truncate(max);
            Ok(list)
        }

        fn resize(
            &self,
            session: &mut MockSession,
            index: usize,
            delta: i32,
            _left: &[SegmentChoice],
        ) -> Result<(), SegmentError> {
            self.resizes.set(self.resizes.get() + 1);
            if index >= session.segments.len() {
                return Err(SegmentError::OutOfRange);
            }
            // Mozc と同じ: 縮めた分は右の文節へ回し、伸ばした分は右の文節から取る。区切りは文字単位
            let chars: Vec<char> = session.reading.chars().collect();
            let start = session.layout.spans[index].start;
            let end = session.layout.spans[index].end;
            let new_end = if delta < 0 {
                if end - start <= 1 {
                    return Ok(());
                }
                end - 1
            } else {
                if index + 1 >= session.segments.len() {
                    return Ok(());
                }
                end + 1
            };
            let head: Vec<String> = session.segments[..index].to_vec();
            let mut next = head;
            next.push(chars[start..new_end].iter().collect());
            // 右は 1 文節にまとめる（モックの単純化）
            if new_end < chars.len() {
                next.push(chars[new_end..].iter().collect());
            }
            let rev = session.layout.rev + 1;
            session.layout = self.rebuild(&next);
            session.layout.rev = rev;
            session.segments = next;
            Ok(())
        }

        fn whole_surfaces(&self, session: &mut MockSession, max: usize) -> Vec<String> {
            let mut list = self
                .whole
                .get(&session.reading)
                .cloned()
                .unwrap_or_default();
            list.truncate(max);
            list
        }

        fn reconvert(&self, surface: &str, _context: &str) -> Result<SegmentLayout, SegmentError> {
            let segments = self
                .reconverted
                .get(surface)
                .ok_or(SegmentError::Empty)?
                .clone();
            Ok(self.rebuild(&segments))
        }
    }
}
