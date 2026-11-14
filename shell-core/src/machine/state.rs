//! 入力の状態（今の `InputState.swift` と、`EngineClient.swift` の変換の値の型）。
//!
//! エンジンとの境界は `ikari_proto::v1` の型（`engine.rs`）。ここは状態機械の中で持つ形で、返事を受けたときに写す。

use ikari_proto::v1;

use crate::menu::EngineLmState;
use crate::shortcode::{ShortcodeChoosing, ShortcodeTyping};
use crate::types::{
    CandidateItem, CandidateKind, LmDeviceInfo, LmReportInfo, LmVerdictInfo, Utf16Range,
};

/// 変換に入る前の状態。読みに戻る（Esc、BackSpace）ときの行き先（ADR 0006 §8.5、§9.5）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConversionOrigin {
    #[default]
    Composing,
    Ascii,
    /// 確定済みの文字列の再変換。`original` は選択していた文字列、`replacing` はその範囲。
    /// Esc は `original` を書き戻す（`ShellAction::Restore`）
    Reconversion {
        original: String,
        replacing: Utf16Range,
    },
}

/// 1 ページ分の候補（`ConvertReply` の写し）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConversionPage {
    /// エンジンが付ける。殻だけが作った候補では 0
    pub conversion_id: u64,
    pub page_index: usize,
    pub candidates: Vec<CandidateItem>,
    pub has_next: bool,
    pub lm_applied: bool,
    /// 1 位と 2 位の差が閾値以上（#69）。`convert` の返事だけで意味を持つ
    pub confident: bool,
    pub engine_micros: u32,
    /// 区切り（`ConvertReply.layout`）。古いエンジンは `None` で、1 文節として扱う
    pub layout: Option<SegmentLayoutInfo>,
    /// LM の様子（`ConvertReply.lm_report`。ADR 0061）。`convert` の返事だけに付く。古いエンジンは `None`
    pub lm_report: Option<LmReportInfo>,
}

impl LmReportInfo {
    /// `ConvertReply.lm_report` の写し。範囲外・未知の列挙は `Unspecified`（表示だけに使うので、落とさない）
    pub(crate) fn from_reply(reply: &v1::LmReport) -> Self {
        Self {
            verdict: match v1::LmVerdict::try_from(reply.verdict) {
                Ok(v1::LmVerdict::LmApplied) => LmVerdictInfo::Applied,
                Ok(v1::LmVerdict::LmTimedOut) => LmVerdictInfo::TimedOut,
                Ok(v1::LmVerdict::LmNotScored) => LmVerdictInfo::NotScored,
                Ok(v1::LmVerdict::LmPrefetchLate) => LmVerdictInfo::PrefetchLate,
                Ok(v1::LmVerdict::LmNoPrefetch) => LmVerdictInfo::NoPrefetch,
                Ok(v1::LmVerdict::LmSecure) => LmVerdictInfo::Secure,
                Ok(v1::LmVerdict::LmUnavailable) => LmVerdictInfo::Unavailable,
                Ok(v1::LmVerdict::Unspecified) | Err(_) => LmVerdictInfo::Unspecified,
            },
            wait_micros: reply.wait_micros,
            infer_micros: reply.infer_micros,
            lm_deadline_micros: reply.lm_deadline_micros,
            convert_wait_micros: reply.convert_wait_micros,
            device: match v1::LmDevice::try_from(reply.device) {
                Ok(v1::LmDevice::Cpu) => LmDeviceInfo::Cpu,
                Ok(v1::LmDevice::Gpu) => LmDeviceInfo::Gpu,
                Ok(v1::LmDevice::Unspecified) | Err(_) => LmDeviceInfo::Unspecified,
            },
            calls: reply.calls,
            timeouts: reply.timeouts,
            late_hits: reply.late_hits,
            lm_state: match v1::LmState::try_from(reply.lm_state) {
                Ok(v1::LmState::LmDisabled) => Some(EngineLmState::Disabled),
                Ok(v1::LmState::LmLoading) => Some(EngineLmState::Loading),
                Ok(v1::LmState::LmReady) => Some(EngineLmState::Ready),
                Ok(v1::LmState::LmFailed) => Some(EngineLmState::Failed),
                Ok(v1::LmState::Unspecified) | Err(_) => None,
            },
        }
    }
}

impl ConversionPage {
    pub(crate) fn from_reply(reply: &v1::ConvertReply) -> Self {
        Self {
            conversion_id: reply.conversion_id,
            page_index: to_usize(reply.page_index),
            candidates: reply.candidates.iter().map(candidate_item).collect(),
            has_next: reply.has_next,
            lm_applied: reply.lm_applied,
            confident: reply.confident,
            engine_micros: reply.engine_micros,
            layout: reply.layout.as_ref().map(SegmentLayoutInfo::from_reply),
            lm_report: reply.lm_report.as_ref().map(LmReportInfo::from_reply),
        }
    }

    pub(crate) fn from_segment_page(page: &SegmentPage, conversion_id: u64) -> Self {
        Self {
            conversion_id,
            page_index: page.page_index,
            candidates: page.candidates.clone(),
            has_next: page.has_next,
            lm_applied: page.lm_applied,
            confident: false,
            engine_micros: page.engine_micros,
            layout: None,
            lm_report: None,
        }
    }
}

/// 文節 1 つの読みと、候補の 0 番（`SegmentInfo`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentSummary {
    pub reading: String,
    pub surface: String,
}

/// 文節の候補の 1 ページ（`SegmentPageReply`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentPage {
    pub segment: usize,
    pub page_index: usize,
    pub candidates: Vec<CandidateItem>,
    pub has_next: bool,
    pub lm_applied: bool,
    pub engine_micros: u32,
}

impl SegmentPage {
    pub(crate) fn from_reply(reply: &v1::SegmentPageReply) -> Self {
        Self {
            segment: to_usize(reply.segment),
            page_index: to_usize(reply.page_index),
            candidates: reply.candidates.iter().map(candidate_item).collect(),
            has_next: reply.has_next,
            lm_applied: reply.lm_applied,
            engine_micros: reply.engine_micros,
        }
    }
}

/// 区切りと、フォーカスしている文節の 1 ページ目（`SegmentLayout`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentLayoutInfo {
    pub conversion_id: u64,
    pub layout_rev: u32,
    pub segments: Vec<SegmentSummary>,
    pub focused: usize,
    pub focused_page: Option<SegmentPage>,
}

impl SegmentLayoutInfo {
    pub(crate) fn from_reply(reply: &v1::SegmentLayout) -> Self {
        Self {
            conversion_id: reply.conversion_id,
            layout_rev: reply.layout_rev,
            segments: reply
                .segments
                .iter()
                .map(|segment| SegmentSummary {
                    reading: segment.reading.clone(),
                    surface: segment.surface.clone(),
                })
                .collect(),
            focused: to_usize(reply.focused),
            focused_page: reply.focused_page.as_ref().map(SegmentPage::from_reply),
        }
    }

    pub(crate) fn joined_reading(&self) -> String {
        self.segments.iter().map(|s| s.reading.as_str()).collect()
    }

    pub(crate) fn joined_surface(&self) -> String {
        self.segments.iter().map(|s| s.surface.as_str()).collect()
    }
}

/// 文節の選択（`SegmentChoice`）。`Literal` は殻が作った表記で、学習しない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentChoiceValue {
    /// 生成済みの列の中の通し番号（ページをまたぐ）
    Candidate(usize),
    Literal(String),
}

impl SegmentChoiceValue {
    pub(crate) fn to_proto(&self) -> v1::SegmentChoice {
        match self {
            Self::Candidate(index) => v1::SegmentChoice {
                candidate_index: to_u32(*index),
                literal: String::new(),
            },
            Self::Literal(text) => v1::SegmentChoice {
                candidate_index: 0,
                literal: text.clone(),
            },
        }
    }
}

/// 変換中の 1 つの文節（ADR 0006 §8.5）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConvertingSegment {
    pub reading: String,
    /// 候補の 0 番（最初に表示した表記）
    pub surface: String,
    /// 取得済みのページ（古い順）
    pub pages: Vec<ConversionPage>,
    /// 現在のページの番号（`pages` の添字）
    pub page_index: usize,
    /// 現在のページの中の選択位置
    pub selected: usize,
    /// 字種変換で殻が作った表記。あれば `selected` より優先して表示し、`SegmentChoiceValue::Literal` として送る
    pub literal: Option<String>,
    /// 最初のページを全部見せている（ADR 0014 §2.3 (c)）
    pub expanded: bool,
}

impl ConvertingSegment {
    pub fn new(reading: String, surface: String, pages: Vec<ConversionPage>) -> Self {
        Self {
            reading,
            surface,
            pages,
            ..Self::default()
        }
    }

    /// 取得済みの列の中の通し番号（ページをまたぐ）。まだ候補を取っていない文節は 0 番
    pub fn candidate_index(&self) -> usize {
        if self.page_index >= self.pages.len() {
            return 0;
        }
        self.pages[..self.page_index]
            .iter()
            .map(|page| page.candidates.len())
            .sum::<usize>()
            + self.selected
    }

    /// いま表示している表記。`literal` があればそれ、まだ候補を取っていない文節は最初の表記
    pub fn displayed_surface(&self) -> &str {
        if let Some(literal) = &self.literal {
            return literal;
        }
        self.pages
            .get(self.page_index)
            .and_then(|page| page.candidates.get(self.selected))
            .map_or(self.surface.as_str(), |candidate| {
                candidate.surface.as_str()
            })
    }
}

/// 変換中の状態（Swift の `ConvertingState`）。
///
/// `segments` が空でないときは、文節ごとのデータは `segments` が持ち、平らなフィールド
/// （`conversion_id`・`page_index`・`candidates`・`has_next`・`selected`・`candidate_offset`）は
/// フォーカスしている文節の現在のページの表示を写したもの（`sync_focused`）。`reading` は読み全体。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConvertingState {
    pub reading: String,
    /// エンジンの `conversion_id`。殻だけが作った候補のときは 0
    pub conversion_id: u64,
    pub page_index: usize,
    pub candidates: Vec<CandidateItem>,
    pub has_next: bool,
    pub selected: usize,
    pub candidate_offset: usize,
    /// エンジンなし、またはタイムアウトで殻だけが作った候補
    pub is_fallback: bool,
    pub segments: Vec<ConvertingSegment>,
    pub focused: usize,
    /// 区切りの版（`SegmentLayout.layout_rev`）
    pub layout_rev: u32,
    pub origin: ConversionOrigin,
    /// 全体の候補を出している間の、エンジンが返した 2 文節以上の区切り（ADR 0006 §16.6）。表示にだけ使う
    pub whole_layout: Option<SegmentLayoutInfo>,
    /// 確信が高い変換で、1 位だけを出して候補ウィンドウを出していない間（#69）
    pub candidates_hidden: bool,
    /// 最初の変換（Space の最初の返事）の LM の様子。ページ送りや文節の移動のあいだ同じものを持つ（ADR 0061 D7）。表示にだけ使う
    pub lm_report: Option<LmReportInfo>,
}

impl ConvertingState {
    /// 候補ウィンドウに見せる件数（ADR 0014 §2.3 (c)）。
    pub fn visible_count(&self, initial: usize) -> usize {
        let expanded = self.segments.get(self.focused).is_none_or(|s| s.expanded);
        if expanded || self.page_index > 0 {
            return self.candidates.len();
        }
        initial.max(1).min(self.candidates.len())
    }

    /// 平らなフィールドを、フォーカスしている文節の現在のページに合わせる。
    pub fn sync_focused(&mut self) {
        let Some(segment) = self.segments.get(self.focused) else {
            return;
        };
        let Some(page) = segment.pages.get(segment.page_index) else {
            return;
        };
        self.page_index = segment.page_index;
        self.candidates.clone_from(&page.candidates);
        self.has_next = page.has_next;
        self.selected = segment.selected;
        self.candidate_offset = segment.pages[..segment.page_index]
            .iter()
            .map(|page| page.candidates.len())
            .sum();
    }

    /// フォーカスしている文節。範囲外は起きないはずだが、panic しない
    pub(crate) fn focused_segment_mut(&mut self) -> Option<&mut ConvertingSegment> {
        self.segments.get_mut(self.focused)
    }
}

/// 入力の状態（Swift の `InputState`）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum InputState {
    #[default]
    Idle,
    /// `kana` は確定したかな、`pending` は未確定のローマ字。表示は `kana + pending`
    Composing {
        kana: String,
        pending: String,
    },
    Converting(ConvertingState),
    /// Shift＋英字の一時英数
    AsciiRun {
        text: String,
    },
    /// ショートコードを打っている途中（ADR 0027 §3.3）
    Shortcode(ShortcodeTyping),
    /// ショートコードの候補を出している（ADR 0027 §3.4）
    ShortcodeChoice(ShortcodeChoosing),
}

pub(crate) fn candidate_item(candidate: &v1::Candidate) -> CandidateItem {
    CandidateItem {
        surface: candidate.surface.clone(),
        annotation: candidate.annotation.clone(),
        kind: candidate_kind(candidate.kind),
        label: String::new(),
    }
}

fn candidate_kind(kind: i32) -> CandidateKind {
    match v1::CandidateKind::try_from(kind) {
        Ok(v1::CandidateKind::Dictionary) => CandidateKind::Dictionary,
        Ok(v1::CandidateKind::Hiragana) => CandidateKind::Hiragana,
        Ok(v1::CandidateKind::Katakana) => CandidateKind::Katakana,
        Ok(v1::CandidateKind::Rewriter) => CandidateKind::Rewriter,
        Ok(v1::CandidateKind::Literal) => CandidateKind::Literal,
        Ok(v1::CandidateKind::Unspecified) | Err(_) => CandidateKind::Unspecified,
    }
}

pub(crate) fn to_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

pub(crate) fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
