//! 診断の数え上げ（今の `DiagEpisode.swift`。ADR 0034。レーン L-A1）。
//!
//! 合成 1 回の `sh_episode` を数える。貯めるのはメモリ上の整数だけで、文字列そのものは持たない（NFR-19）。
//! キーと語は engine の許可リスト（`diag_schema.rs`）に合わせる。
//!
//! 時刻は単調時計のナノ秒（`u64`）で受け取る。時計の読み方（`Clock`）は呼び出し側（L-A2 の `machine`・`controller`）が持つ。

use std::collections::BTreeMap;

use crate::english::InputMode;
use crate::panel::AnchorSource;

/// `sh_episode` の種類（`diag_schema.rs`）。
pub const EPISODE_KIND: &str = "sh_episode";
/// `sh_mode` の種類。
pub const MODE_SWITCH_KIND: &str = "sh_mode";

const NANOS_PER_MILLI: u64 = 1_000_000;
const NANOS_PER_MICRO: u64 = 1_000;

/// 診断の 1 件（`EngineClient.sendDiag` に渡す値。Swift の `DiagEventValue`）。
/// 数とタグの語は許可リストの中だけ。ユーザーの文字列は入れない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagEvent {
    pub session_id: u64,
    pub kind: String,
    pub nums: BTreeMap<String, i64>,
    pub tags: BTreeMap<String, String>,
}

impl DiagEvent {
    pub fn new(session_id: u64, kind: &str) -> Self {
        Self {
            session_id,
            kind: kind.to_string(),
            nums: BTreeMap::new(),
            tags: BTreeMap::new(),
        }
    }
}

/// `end` タグ: 状態が `idle` に戻った理由。
/// `secure` は作らない: secure のセッションでは貯めず送らない（NFR-18）ので、途中で secure になった合成は捨てる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagEnd {
    CommitCand,
    CommitSegments,
    CommitKana,
    EscClear,
    BsEmpty,
    CommitComposition,
    Deactivate,
    ModeSwitch,
    Click,
    ShortcodeLiteral,
    ShortcodePick,
}

impl DiagEnd {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CommitCand => "commit_cand",
            Self::CommitSegments => "commit_segments",
            Self::CommitKana => "commit_kana",
            Self::EscClear => "esc_clear",
            Self::BsEmpty => "bs_empty",
            Self::CommitComposition => "commit_composition",
            Self::Deactivate => "deactivate",
            Self::ModeSwitch => "mode_switch",
            Self::Click => "click",
            Self::ShortcodeLiteral => "shortcode_literal",
            Self::ShortcodePick => "shortcode_pick",
        }
    }

    /// 外から確定させられた理由。`prev` を添える。
    pub fn is_external(self) -> bool {
        matches!(
            self,
            Self::CommitComposition | Self::Deactivate | Self::ModeSwitch | Self::Click
        )
    }
}

/// `prev` タグ: 外から確定させられたときの直前の状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagPrev {
    Composing,
    Converting,
    Segments,
    AsciiRun,
    Shortcode,
    ShortcodeChoice,
}

impl DiagPrev {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Composing => "composing",
            Self::Converting => "converting",
            Self::Segments => "segments",
            Self::AsciiRun => "ascii_run",
            Self::Shortcode => "shortcode",
            Self::ShortcodeChoice => "shortcode_choice",
        }
    }
}

/// `start` タグ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagStart {
    Typing,
    Reconvert,
}

impl DiagStart {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Typing => "typing",
            Self::Reconvert => "reconvert",
        }
    }
}

/// `mode` タグ（`ShortcodeEn` は英語のモード側が数える）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagMode {
    Romaji,
    Kana,
    ShortcodeJa,
    ShortcodeEn,
}

impl DiagMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Romaji => "romaji",
            Self::Kana => "kana",
            Self::ShortcodeJa => "shortcode_ja",
            Self::ShortcodeEn => "shortcode_en",
        }
    }
}

/// `panel` タグ: 候補ウィンドウの位置を最初に何から取ったか（`AnchorSource` と 1 対 1）。出さなかったら `None`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagPanel {
    Client,
    Accessibility,
    LastKnown,
    MouseScreen,
    Absent,
}

impl DiagPanel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Accessibility => "accessibility",
            Self::LastKnown => "last_known",
            Self::MouseScreen => "mouse_screen",
            Self::Absent => "none",
        }
    }
}

impl From<AnchorSource> for DiagPanel {
    fn from(source: AnchorSource) -> Self {
        match source {
            AnchorSource::Client => Self::Client,
            AnchorSource::Accessibility => Self::Accessibility,
            AnchorSource::LastKnown => Self::LastKnown,
            AnchorSource::MouseScreen => Self::MouseScreen,
        }
    }
}

/// `ctx` タグ: 最初の変換のときに、殻が周辺テキストを取れたか。
/// 権限の拒否（`denied`）は殻から見えない（IMK は「取れなかった」としか返さない）ので作らない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagContext {
    Ok,
    Absent,
}

impl DiagContext {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Absent => "none",
        }
    }
}

/// `pick_kind` タグ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagPickKind {
    Emoji,
    SmallLetter,
    Dict,
    Hiragana,
    Katakana,
    Rewriter,
    Literal,
    Fallback,
}

impl DiagPickKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Emoji => "emoji",
            Self::SmallLetter => "small_letter",
            Self::Dict => "dict",
            Self::Hiragana => "hiragana",
            Self::Katakana => "katakana",
            Self::Rewriter => "rewriter",
            Self::Literal => "literal",
            Self::Fallback => "fallback",
        }
    }
}

/// 確定したときの変換中の状態から取る値。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiagCommitInfo {
    pub pick: Option<u32>,
    pub pick_kind: Option<DiagPickKind>,
    pub segs: Option<u32>,
    pub seg_changed: Option<u32>,
}

/// 合成 1 回ぶんの数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagEpisode {
    pub mode: DiagMode,
    pub start: DiagStart,
    /// 最初の打鍵の時刻（単調時計のナノ秒）
    pub started_at: u64,

    pub keys: u32,
    pub keys_to_conv: Option<u32>,
    pub rlen: Option<u32>,
    /// 最初の Space の時刻
    pub converted_at: Option<u64>,
    pub wait_micros: Option<u64>,
    pub spaces: u32,
    pub cand_moves: u32,
    pub pages: u32,
    pub digit_picks: u32,
    pub seg_focus: u32,
    pub seg_resize: u32,
    pub esc_back: u32,
    pub bs: u32,
    pub fallback: u32,
    /// 候補ウィンドウを最初に出したときの位置の取り方。出していなければ `None`（`none` と書く）
    pub panel: Option<DiagPanel>,
    /// 最初の変換のときの周辺テキスト。変換しなければ `None`（書かない）
    pub context: Option<DiagContext>,
}

impl DiagEpisode {
    pub fn new(mode: DiagMode, start: DiagStart, started_at: u64) -> Self {
        Self {
            mode,
            start,
            started_at,
            keys: 0,
            keys_to_conv: None,
            rlen: None,
            converted_at: None,
            wait_micros: None,
            spaces: 0,
            cand_moves: 0,
            pages: 0,
            digit_picks: 0,
            seg_focus: 0,
            seg_resize: 0,
            esc_back: 0,
            bs: 0,
            fallback: 0,
            panel: None,
            context: None,
        }
    }

    /// 変換（Space）に入った。2 回目以降の変換では、最初の値（読みの長さ、打鍵数、待ち時間）を保つ。
    pub fn note_conversion(
        &mut self,
        reading_length: u32,
        now: u64,
        waited_nanos: Option<u64>,
        used_fallback: bool,
    ) {
        self.spaces += 1;
        if used_fallback {
            self.fallback += 1;
        }
        if self.converted_at.is_some() {
            return;
        }
        self.converted_at = Some(now);
        self.keys_to_conv = Some(self.keys);
        self.rlen = Some(reading_length);
        self.wait_micros = waited_nanos.map(|nanos| nanos / NANOS_PER_MICRO);
    }

    /// 候補ウィンドウを出した。最初の 1 回だけ覚える。
    pub fn note_panel(&mut self, source: AnchorSource) {
        if self.panel.is_none() {
            self.panel = Some(DiagPanel::from(source));
        }
    }

    /// 最初の変換のときの周辺テキスト（2 回目以降は見ない）。
    pub fn note_context(&mut self, available: bool) {
        if self.context.is_none() {
            self.context = Some(if available {
                DiagContext::Ok
            } else {
                DiagContext::Absent
            });
        }
    }

    pub fn event(
        &self,
        session_id: u64,
        end: DiagEnd,
        prev: Option<DiagPrev>,
        commit: Option<&DiagCommitInfo>,
        now: u64,
    ) -> DiagEvent {
        let mut event = DiagEvent::new(session_id, EPISODE_KIND);
        let nums = &mut event.nums;
        let mut put = |key: &str, value: i64| {
            nums.insert(key.to_string(), value);
        };
        put("keys", i64::from(self.keys));
        put("dur_ms", millis(self.started_at, now));
        put("spaces", i64::from(self.spaces));
        put("cand_moves", i64::from(self.cand_moves));
        put("pages", i64::from(self.pages));
        put("digit_pick", i64::from(self.digit_picks));
        put("seg_focus", i64::from(self.seg_focus));
        put("seg_resize", i64::from(self.seg_resize));
        put("esc_back", i64::from(self.esc_back));
        put("bs", i64::from(self.bs));
        put("fallback", i64::from(self.fallback));
        if let Some(value) = self.keys_to_conv {
            put("keys_to_conv", i64::from(value));
        }
        if let Some(value) = self.rlen {
            put("rlen", i64::from(value));
        }
        if let Some(converted_at) = self.converted_at {
            put("pick_ms", millis(converted_at, now));
        }
        if let Some(value) = self.wait_micros {
            put("wait_us", i64::try_from(value).unwrap_or(i64::MAX));
        }
        if let Some(commit) = commit {
            if let Some(value) = commit.segs {
                put("segs", i64::from(value));
            }
            if let Some(value) = commit.pick {
                put("pick", i64::from(value));
            }
            if let Some(value) = commit.seg_changed {
                put("seg_changed", i64::from(value));
            }
        }

        let tags = &mut event.tags;
        let mut tag = |key: &str, value: &str| {
            tags.insert(key.to_string(), value.to_string());
        };
        tag("mode", self.mode.as_str());
        tag("start", self.start.as_str());
        tag("end", end.as_str());
        tag("panel", self.panel.unwrap_or(DiagPanel::Absent).as_str());
        if let Some(context) = self.context {
            tag("ctx", context.as_str());
        }
        if let Some(prev) = prev
            && end.is_external()
        {
            tag("prev", prev.as_str());
        }
        if let Some(kind) = commit.and_then(|c| c.pick_kind) {
            tag("pick_kind", kind.as_str());
        }
        event
    }
}

fn millis(start: u64, end: u64) -> i64 {
    let elapsed = end.saturating_sub(start) / NANOS_PER_MILLI;
    i64::try_from(elapsed).unwrap_or(i64::MAX)
}

/// 英語のモードで終わった合成 1 回ぶん。`EnglishMode` が作り、コントローラが状態機械の
/// `record_english_episode` に渡して送る（英語のモードの状態は状態機械の外にある。セッション ID とエンジンは状態機械が持つ）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagFinishedEpisode {
    pub episode: DiagEpisode,
    pub end: DiagEnd,
    pub prev: Option<DiagPrev>,
    pub commit: Option<DiagCommitInfo>,
    pub ended_at: u64,
}

impl DiagFinishedEpisode {
    pub fn event(&self, session_id: u64) -> DiagEvent {
        self.episode.event(
            session_id,
            self.end,
            self.prev,
            self.commit.as_ref(),
            self.ended_at,
        )
    }
}

/// `sh_mode`（ADR 0034 §4.2）: 英語・日本語のモードが替わった。値は固定の語だけ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagModeVia {
    /// ikari のキー割り当て（`mode.*`）で替えた
    Key,
    /// macOS 経由（入力メニュー、英数・かなキー、入力ソースの切り替えキー）。
    /// macOS の通知（`setValue`）の時点では、後者のどれかは分からない（英数・かなキーは通知のあとに届く）
    Os,
}

impl DiagModeVia {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Os => "os",
        }
    }
}

/// `sh_mode` の 1 件を作る。
pub fn mode_switch_event(
    session_id: u64,
    from: InputMode,
    to: InputMode,
    via: DiagModeVia,
) -> DiagEvent {
    let mut event = DiagEvent::new(session_id, MODE_SWITCH_KIND);
    event.tags.insert("from".into(), lang(from).into());
    event.tags.insert("to".into(), lang(to).into());
    event.tags.insert("via".into(), via.as_str().into());
    event
}

fn lang(mode: InputMode) -> &'static str {
    match mode {
        InputMode::Japanese => "ja",
        InputMode::English => "en",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// engine/src/diag_schema.rs の `sh_episode` の許可リスト（数のキーとタグのキー）
    const ALLOWED_NUMS: [&str; 18] = [
        "keys",
        "keys_to_conv",
        "rlen",
        "dur_ms",
        "pick_ms",
        "spaces",
        "cand_moves",
        "pages",
        "pick",
        "digit_pick",
        "segs",
        "seg_focus",
        "seg_resize",
        "seg_changed",
        "esc_back",
        "bs",
        "wait_us",
        "fallback",
    ];
    const ALLOWED_TAGS: [&str; 7] = ["mode", "start", "end", "prev", "pick_kind", "panel", "ctx"];
    const STEP: u64 = 5_000_000;

    fn episode_with_conversion() -> DiagEpisode {
        let mut episode = DiagEpisode::new(DiagMode::Romaji, DiagStart::Typing, STEP);
        episode.keys = 5;
        episode.note_context(true);
        episode.note_conversion(3, 6 * STEP, Some(1_500_000), false);
        episode.keys += 3;
        episode
    }

    #[test]
    fn event_carries_counts_and_allowed_keys_only() {
        let mut episode = episode_with_conversion();
        episode.spaces += 1;
        episode.cand_moves = 1;
        let commit = DiagCommitInfo {
            pick: Some(1),
            pick_kind: Some(DiagPickKind::Dict),
            segs: Some(1),
            seg_changed: None,
        };
        let event = episode.event(7, DiagEnd::CommitCand, None, Some(&commit), 10 * STEP);
        assert_eq!(event.kind, "sh_episode");
        assert_eq!(event.session_id, 7);
        let tags: BTreeMap<&str, &str> = event
            .tags
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            tags,
            BTreeMap::from([
                ("mode", "romaji"),
                ("start", "typing"),
                ("end", "commit_cand"),
                ("pick_kind", "dict"),
                ("panel", "none"),
                ("ctx", "ok"),
            ])
        );
        assert_eq!(event.nums["keys"], 8);
        assert_eq!(event.nums["keys_to_conv"], 5);
        assert_eq!(event.nums["rlen"], 3);
        assert_eq!(event.nums["spaces"], 2);
        assert_eq!(event.nums["cand_moves"], 1);
        assert_eq!(event.nums["pick"], 1);
        assert_eq!(event.nums["segs"], 1);
        assert_eq!(event.nums["pages"], 0);
        assert_eq!(event.nums["bs"], 0);
        assert_eq!(event.nums["fallback"], 0);
        assert_eq!(event.nums["dur_ms"], 45);
        assert_eq!(event.nums["pick_ms"], 20);
        assert_eq!(event.nums["wait_us"], 1500);
        assert!(
            event
                .nums
                .keys()
                .all(|k| ALLOWED_NUMS.contains(&k.as_str()))
        );
        assert!(
            event
                .tags
                .keys()
                .all(|k| ALLOWED_TAGS.contains(&k.as_str()))
        );
    }

    #[test]
    fn without_conversion_the_conversion_values_are_absent() {
        let mut episode = DiagEpisode::new(DiagMode::Kana, DiagStart::Reconvert, 0);
        episode.keys = 2;
        let event = episode.event(1, DiagEnd::CommitKana, None, None, STEP);
        assert!(!event.nums.contains_key("keys_to_conv"));
        assert!(!event.nums.contains_key("pick_ms"));
        assert!(!event.nums.contains_key("wait_us"));
        assert!(!event.nums.contains_key("rlen"));
        assert_eq!(event.nums["spaces"], 0);
        assert_eq!(event.tags["mode"], "kana");
        assert_eq!(event.tags["start"], "reconvert");
        assert!(!event.tags.contains_key("ctx"));
    }

    #[test]
    fn second_conversion_keeps_the_first_values_but_counts_spaces_and_fallback() {
        let mut episode = episode_with_conversion();
        episode.note_conversion(9, 20 * STEP, Some(9_000_000), true);
        assert_eq!(episode.spaces, 2);
        assert_eq!(episode.fallback, 1);
        assert_eq!(episode.rlen, Some(3));
        assert_eq!(episode.keys_to_conv, Some(5));
        assert_eq!(episode.converted_at, Some(6 * STEP));
        assert_eq!(episode.wait_micros, Some(1500));
    }

    #[test]
    fn panel_and_context_keep_the_first_value() {
        let mut episode = DiagEpisode::new(DiagMode::Romaji, DiagStart::Typing, 0);
        episode.note_panel(AnchorSource::Accessibility);
        episode.note_panel(AnchorSource::MouseScreen);
        episode.note_context(false);
        episode.note_context(true);
        let event = episode.event(1, DiagEnd::CommitCand, None, None, 0);
        assert_eq!(event.tags["panel"], "accessibility");
        assert_eq!(event.tags["ctx"], "none");
    }

    #[test]
    fn every_anchor_source_maps_to_an_allowed_word() {
        let words = [
            "client",
            "accessibility",
            "last_known",
            "mouse_screen",
            "none",
        ];
        for source in [
            AnchorSource::Client,
            AnchorSource::Accessibility,
            AnchorSource::LastKnown,
            AnchorSource::MouseScreen,
        ] {
            assert!(words.contains(&DiagPanel::from(source).as_str()));
        }
    }

    #[test]
    fn prev_is_attached_only_for_external_ends() {
        let episode = DiagEpisode::new(DiagMode::Romaji, DiagStart::Typing, 0);
        for (end, expected) in [
            (DiagEnd::Deactivate, Some("composing")),
            (DiagEnd::ModeSwitch, Some("composing")),
            (DiagEnd::Click, Some("composing")),
            (DiagEnd::CommitComposition, Some("composing")),
            (DiagEnd::CommitCand, None),
            (DiagEnd::EscClear, None),
        ] {
            let event = episode.event(1, end, Some(DiagPrev::Composing), None, 0);
            assert_eq!(
                event.tags.get("prev").map(String::as_str),
                expected,
                "{end:?}"
            );
        }
    }

    #[test]
    fn duration_never_goes_negative() {
        let episode = DiagEpisode::new(DiagMode::Romaji, DiagStart::Typing, 100 * STEP);
        let event = episode.event(1, DiagEnd::CommitKana, None, None, STEP);
        assert_eq!(event.nums["dur_ms"], 0);
    }

    #[test]
    fn mode_switch_records_direction_and_route() {
        let key = mode_switch_event(3, InputMode::Japanese, InputMode::English, DiagModeVia::Key);
        assert_eq!(key.kind, "sh_mode");
        assert_eq!(key.session_id, 3);
        assert_eq!(
            key.tags,
            BTreeMap::from([
                ("from".to_string(), "ja".to_string()),
                ("to".to_string(), "en".to_string()),
                ("via".to_string(), "key".to_string()),
            ])
        );
        assert!(key.nums.is_empty());
        let os = mode_switch_event(3, InputMode::English, InputMode::Japanese, DiagModeVia::Os);
        assert_eq!(os.tags["from"], "en");
        assert_eq!(os.tags["via"], "os");
    }

    #[test]
    fn finished_episode_builds_its_event_at_the_end_time() {
        let finished = DiagFinishedEpisode {
            episode: DiagEpisode::new(DiagMode::ShortcodeEn, DiagStart::Typing, 0),
            end: DiagEnd::ShortcodePick,
            prev: None,
            commit: Some(DiagCommitInfo {
                pick_kind: Some(DiagPickKind::Emoji),
                ..DiagCommitInfo::default()
            }),
            ended_at: 3 * STEP,
        };
        let event = finished.event(5);
        assert_eq!(event.nums["dur_ms"], 15);
        assert_eq!(event.tags["pick_kind"], "emoji");
        assert_eq!(event.tags["mode"], "shortcode_en");
    }
}
