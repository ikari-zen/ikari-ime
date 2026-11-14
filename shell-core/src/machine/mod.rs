//! 入力の状態機械の本体（今の `InputStateMachine.swift`・`InputState.swift`。ADR 0004 §6.3。レーン L-A2）。
//!
//! OS にも UI にも依存しない。殻の判断（`controller::Shell`）が、キーを `KeyInput` にして `handle` に渡し、
//! 返ってきた `ShellAction` の列を OS 側へ返す。周辺テキストと選択範囲は、要るときだけ `TextAccess` から取る
//! （今の `@autoclosure` と同じ遅延）。エンジンは `MachineEngine`（本番は `EngineLink`）。
//!
//! 振る舞いは Swift 版と同じにする（共通のシナリオ `shell-core/scenarios/*.json` と、このモジュールのテストで縛る）。

mod clause_delete;
mod composer;
mod developer_note;
mod engine;
mod state;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use ikari_proto::v1;
use unicode_segmentation::UnicodeSegmentation;

pub use composer::{Composer, RomajiSource, convert_to_script};
pub use engine::MachineEngine;
pub use state::{
    ConversionOrigin, ConversionPage, ConvertingSegment, ConvertingState, InputState,
    SegmentChoiceValue, SegmentLayoutInfo, SegmentPage, SegmentSummary,
};

use crate::diag::{
    DiagCommitInfo, DiagEnd, DiagEpisode, DiagFinishedEpisode, DiagMode, DiagModeVia, DiagPickKind,
    DiagPrev, DiagStart, mode_switch_event,
};
use crate::english::{InputMode, is_printable};
use crate::host::{Clock, TextAccess};
use crate::keymap::{KeyChord, Keymap, KeymapAction};
use crate::panel::{AnchorSource, PanelSide};
use crate::settings::ENGINE_PAGE_SIZE;
use crate::shortcode::{
    CANDIDATE_PAGE_SIZE, ShortcodeAfterCommit, ShortcodeChoosing, ShortcodeCommit,
    ShortcodeMachine, ShortcodeMode, ShortcodeOutcome, ShortcodePhase, ShortcodeTable,
};
use crate::timeouts;
use crate::types::{
    CandidateItem, CandidateKind, CandidatePanelModel, KeyInput, KeyKind, MarkedText, SelectedText,
    SettingsSnapshot, ShellAction, SurroundingText, Utf16Len, Utf16Range,
};
use clause_delete::{DeletePreview, UndoStep};
use state::{to_u32, to_usize};

/// エンジンが受け付ける読みの長さの上限（超えたら殻のフォールバックの候補を出す）。
pub const MAX_READING_CHARACTERS: usize = 256;
/// 再変換の対象の長さの上限（文字数。ADR 0006 §9.2）。
pub const MAX_RECONVERT_CHARACTERS: usize = 64;
/// 周辺テキストをエンジンに渡す長さ（左は末尾、右は先頭の文字数。`SurroundingText.maxCharacters`）。
pub const MAX_CONTEXT_CHARACTERS: usize = 2000;
/// 合成の最初の何打鍵目まで、左文脈を読み直すか（1 打鍵目を含む）。暫定。#1（値は計測で決める）
pub const CONTEXT_REFRESH_KEYSTROKES: u32 = 3;

const TRAILING_N_PENDING: &str = "n";
const JIS_DIGIT_1: u8 = 0;
const JIS_DIGIT_9: u8 = 8;
const FALLBACK_SCRIPTS: [KeymapAction; 3] = [
    KeymapAction::ToHiragana,
    KeymapAction::ToFullwidthKatakana,
    KeymapAction::ToHalfwidthKatakana,
];

/// エンジンに渡す周辺テキスト（Swift の `SurroundingText`）。`available == false` は取れなかった。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContextText {
    pub left: String,
    pub right: String,
    pub available: bool,
}

impl ContextText {
    pub fn unavailable() -> Self {
        Self::default()
    }

    /// ホストの生の値から。取れない、または「左が空で文書の長さも 0」なら取れていない（今の `ClientAccess.isAvailable`。ADR 0015 D2）。
    pub fn from_host(raw: Option<SurroundingText>) -> Self {
        match raw {
            Some(raw) if is_available(&raw.left, raw.document_length) => Self {
                left: raw.left,
                right: raw.right,
                available: true,
            },
            _ => Self::unavailable(),
        }
    }

    /// エンジンに渡す長さに切る。左は末尾、右は先頭の `MAX_CONTEXT_CHARACTERS` 文字（見た目の文字）。
    pub fn truncated(self) -> Self {
        Self {
            left: suffix_graphemes(&self.left, MAX_CONTEXT_CHARACTERS),
            right: prefix_graphemes(&self.right, MAX_CONTEXT_CHARACTERS),
            available: self.available,
        }
    }

    fn to_proto(&self) -> v1::SurroundingText {
        v1::SurroundingText {
            left: self.left.clone(),
            right: self.right.clone(),
            available: self.available,
        }
    }
}

/// 周辺テキストを「取れた」と言ってよいか。左が空で文書の長さも 0 なら取れていない（今の `ClientAccess.isAvailable`）。
pub fn is_available(left: &str, document_length: Utf16Len) -> bool {
    !(left.is_empty() && document_length == Utf16Len::ZERO)
}

/// エンジンへ知らせるセッション（`BeginSession`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub session_id: u64,
    pub client_bundle_id: String,
    pub secure: bool,
}

impl SessionInfo {
    fn to_proto(&self) -> v1::BeginSession {
        v1::BeginSession {
            session_id: self.session_id,
            client_bundle_id: self.client_bundle_id.clone(),
            secure: self.secure,
        }
    }
}

/// converting に入る前の状態（読みに戻るときの行き先）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadingOrigin {
    Composing,
    Ascii,
}

/// 変換 1 回ぶんの、エンジンとの付き合い方（ADR 0006）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct SegmentMode {
    /// 区切りを返すエンジンの変換。Shift+←/→ が効く
    can_segment: bool,
    /// ページ送りと確定を文節の API で行う
    use_segment_api: bool,
    /// 区切り直しをした
    layout_changed: bool,
}

/// 再変換の BackSpace で読みの合成に戻ったあとも、Esc で元の文字列に戻せるように覚えておく（ADR 0006 §9.6）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct RestoreTarget {
    original: String,
    replacing: Utf16Range,
}

/// 選択範囲を返さない `TextAccess`（ショートコードの打ち直しで、キーを流し直すとき）。
struct WithoutSelection<'a>(&'a mut dyn TextAccess);

impl TextAccess for WithoutSelection<'_> {
    fn surrounding(&mut self) -> Option<SurroundingText> {
        self.0.surrounding()
    }

    fn selection(&mut self) -> Option<SelectedText> {
        None
    }
}

/// 入力の状態機械。
pub struct InputStateMachine {
    state: InputState,
    engine: Arc<dyn MachineEngine>,
    composer: Composer,
    keymap: Arc<Keymap>,
    settings: Arc<SettingsSnapshot>,
    session: SessionInfo,
    /// `BeginSession` を送ってから `EndSession` を送るまでの間か（#315）
    session_begun: bool,
    clock: Arc<dyn Clock>,

    /// 直近に取った周辺テキスト。secure のときは使わない
    cached_context: Option<ContextText>,
    context_rev: u64,
    /// いまの合成の、文字を打った回数（#1）
    composing_keystrokes: u32,
    last_prefetched_reading: Option<String>,

    reading_origin: ReadingOrigin,
    /// asciiRun の先頭に残したかな
    ascii_prefix: String,
    mode: SegmentMode,
    restore_target: Option<RestoreTarget>,
    /// 文節ごとの削除のプレビューのモード（ADR 0050 §3.4）
    delete_preview: Option<DeletePreview>,
    /// 文節削除の取り消しの覚え（ADR 0053 §3）。末尾が直前の削除
    clause_undo: Vec<UndoStep>,
    shortcode: ShortcodeMachine,
    /// ショートコードの打ち直しで、キーを流し直している間
    replaying_shortcode: bool,
    /// いま処理しているキーがキーリピートか
    input_is_repeat: bool,

    /// 候補ウィンドウがアンカーの上下どちらにあるか（ADR 0014）
    panel_side: PanelSide,

    /// いまの合成 1 回の数（ADR 0034 §4.2）。secure のセッションでは作らない
    episode: Option<DiagEpisode>,
    next_episode_start: DiagStart,
    current_key: Option<KeyKind>,
    /// 外から確定させられた理由。キーによる確定のときは `None`
    external_end: Option<DiagEnd>,
}

impl InputStateMachine {
    pub fn new(
        engine: Arc<dyn MachineEngine>,
        composer: Composer,
        keymap: Arc<Keymap>,
        settings: Arc<SettingsSnapshot>,
        session: SessionInfo,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let shortcode =
            ShortcodeMachine::new(Arc::new(ShortcodeTable::empty()), shortcode_mode(&settings));
        Self {
            state: InputState::Idle,
            engine,
            composer,
            keymap,
            settings,
            session,
            session_begun: false,
            clock,
            cached_context: None,
            context_rev: 0,
            composing_keystrokes: 0,
            last_prefetched_reading: None,
            reading_origin: ReadingOrigin::Composing,
            ascii_prefix: String::new(),
            mode: SegmentMode::default(),
            restore_target: None,
            delete_preview: None,
            clause_undo: Vec::new(),
            shortcode,
            replaying_shortcode: false,
            input_is_repeat: false,
            panel_side: PanelSide::Below,
            episode: None,
            next_episode_start: DiagStart::Typing,
            current_key: None,
            external_end: None,
        }
    }

    pub fn state(&self) -> &InputState {
        &self.state
    }

    pub fn session(&self) -> &SessionInfo {
        &self.session
    }

    pub fn settings(&self) -> &SettingsSnapshot {
        &self.settings
    }

    pub fn keymap(&self) -> &Arc<Keymap> {
        &self.keymap
    }

    pub fn is_idle(&self) -> bool {
        self.state == InputState::Idle
    }

    /// ショートコードの名前の表を渡す。表が空のうちは、ショートコードに入らない。
    pub fn set_shortcode_table(&mut self, table: Arc<ShortcodeTable>) {
        self.shortcode.table = table;
    }

    // MARK: - セッション

    /// 入力モードが有効になったとき。エンジンへセッションの開始を知らせる。
    pub fn activate(&mut self) {
        self.clause_undo.clear();
        self.begin_session();
    }

    /// 入力モードが無効になるとき。先に `commit_composition` を実行すること。
    pub fn deactivate(&mut self) {
        self.clause_undo.clear();
        self.engine.end_session(self.session.session_id);
        self.session_begun = false;
    }

    /// 打鍵の前に呼ぶ。セッションを始めていないときだけ `BeginSession` を送る（#315）。
    pub fn activate_if_needed(&mut self) {
        if !self.session_begun {
            self.begin_session();
        }
    }

    fn begin_session(&mut self) {
        self.engine.begin_session(self.session.to_proto());
        self.session_begun = true;
    }

    /// 打鍵のたびに secure を渡す。変わったときだけエンジンへ知らせる（NFR-18）。
    pub fn set_secure(&mut self, secure: bool) {
        if secure == self.session.secure {
            return;
        }
        self.session.secure = secure;
        if secure {
            self.delete_preview = None;
            self.clause_undo.clear();
            self.cached_context = None;
            self.episode = None;
        }
        self.begin_session();
    }

    /// 英語のモードの文脈のたまりを、確定履歴へ送る（`reading` は空）。空と secure のときは送らない。
    pub fn record_english_text(&self, surface: &str) {
        if surface.is_empty() || self.session.secure {
            return;
        }
        self.engine.commit_text(v1::CommitText {
            session_id: self.session.session_id,
            reading: String::new(),
            surface: surface.to_string(),
        });
    }

    /// IMK の `commitComposition`。表示中の文字列をそのまま確定する。
    pub fn commit_composition(&mut self, reason: DiagEnd) -> Vec<ShellAction> {
        self.clause_undo.clear();
        if self.state == InputState::Idle {
            // 確定後のプレビュー中の変換中の文字列は、アプリが元の表記のまま確定する（ADR 0050 §3.4）
            self.delete_preview = None;
            return Vec::new();
        }
        self.external_end = Some(reason);
        let actions = self.commit_display();
        self.external_end = None;
        actions
    }

    // MARK: - 設定の反映

    /// 設定の変更を、入力中でないときに反映する（ADR 0005 §7.2）。idle のときだけ反映して true。
    /// `composer` は `input.method` が変わったときだけ渡す。セッションの ID は変えず、`BeginSession` も送り直さない。
    pub fn reconfigure(
        &mut self,
        settings: Arc<SettingsSnapshot>,
        keymap: Arc<Keymap>,
        composer: Option<Composer>,
    ) -> bool {
        if self.state != InputState::Idle {
            return false;
        }
        self.clause_undo.clear();
        self.shortcode.mode = shortcode_mode(&settings);
        self.settings = settings;
        self.keymap = keymap;
        if let Some(composer) = composer {
            self.composer = composer;
        }
        true
    }

    // MARK: - 候補ウィンドウの上下

    pub fn panel_side(&self) -> PanelSide {
        self.panel_side
    }

    /// 候補ウィンドウがアンカーの上下どちらにあるか（殻が配置の結果を通知する。表示前と非表示のときは `Below`）。
    pub fn set_panel_side(&mut self, side: PanelSide) {
        self.panel_side = side;
    }

    /// ↑↓ の向きを入れ替えるか。候補ウィンドウが上にあり、↑↓ が既定の割り当てのときだけ。
    fn reverses_arrow(&self, input: &KeyInput) -> bool {
        if self.panel_side != PanelSide::Above || !matches!(input.key, KeyKind::Up | KeyKind::Down)
        {
            return false;
        }
        self.keymap.action(&KeyChord::plain("down")) == Some(KeymapAction::CandidateNext)
            && self.keymap.action(&KeyChord::plain("up")) == Some(KeymapAction::CandidatePrevious)
    }

    // MARK: - キー入力

    /// キーを処理して、実行すべき動作を返す。
    pub fn handle(&mut self, input: &KeyInput, text: &mut dyn TextAccess) -> Vec<ShellAction> {
        self.input_is_repeat = input.is_repeat;
        // secure の間は機能として OFF（ADR 0026 D5）。表示中なら今の表示のまま確定してから素通し
        if self.session.secure {
            self.episode = None;
            if self.state == InputState::Idle {
                return vec![ShellAction::PassThrough];
            }
            let mut actions = self.commit_display();
            actions.push(ShellAction::PassThrough);
            return actions;
        }
        // 削除の直後だけ取り消せる。削除と取り消しとプレビューの中の操作以外のキーは、覚えを捨てる（ADR 0053 §4）
        if !self.clause_undo.is_empty() && !self.keeps_clause_undo(input) {
            self.clause_undo.clear();
        }
        let key_time = self.now();
        self.current_key = Some(input.key.clone());
        self.count_key(input);
        let actions = self.dispatch(input, text);
        self.begin_episode_if_needed(key_time);
        actions
    }

    /// 取り消しの覚えを残すキーか（文節削除、取り消し、プレビューの中の操作）
    fn keeps_clause_undo(&self, input: &KeyInput) -> bool {
        if self.delete_preview.is_some()
            && matches!(
                input.key,
                KeyKind::Left | KeyKind::Right | KeyKind::Backspace | KeyKind::Escape
            )
        {
            return true;
        }
        matches!(
            self.bound_action(input),
            Some(
                KeymapAction::ClauseDelete
                    | KeymapAction::ClauseDeletePreview
                    | KeymapAction::ClauseUndo
            )
        )
    }

    /// 英語のモードの口が状態機械へ渡すキーか（ADR 0053 §6.4）。プレビューの中のすべてのキー、
    /// `clause.delete`・`clause.delete_preview`、覚えがあるときの `clause.undo`。
    /// 英語のモードの間、状態機械は idle なので、読みの入力中・変換中の段はない
    pub fn handles_clause_key(&self, input: &KeyInput) -> bool {
        if self.delete_preview.is_some() {
            return true;
        }
        match self.bound_action(input) {
            Some(KeymapAction::ClauseDelete | KeymapAction::ClauseDeletePreview) => true,
            Some(KeymapAction::ClauseUndo) => !self.clause_undo.is_empty(),
            _ => false,
        }
    }

    // MARK: 診断ログ（ADR 0034 §4.2）

    fn now(&self) -> u64 {
        u64::try_from(self.clock.now().as_nanos()).unwrap_or(u64::MAX)
    }

    fn count_key(&mut self, input: &KeyInput) {
        if let Some(episode) = &mut self.episode {
            episode.keys += 1;
            if input.key == KeyKind::Backspace {
                episode.bs += 1;
            }
        }
    }

    fn begin_episode_if_needed(&mut self, time: u64) {
        if self.episode.is_some() || self.state == InputState::Idle || self.session.secure {
            return;
        }
        let mode = match self.state {
            InputState::Shortcode(_) | InputState::ShortcodeChoice(_) => DiagMode::ShortcodeJa,
            _ if self.settings.input_method == crate::types::InputMethodKind::Kana => {
                DiagMode::Kana
            }
            _ => DiagMode::Romaji,
        };
        let mut started = DiagEpisode::new(mode, self.next_episode_start, time);
        started.keys = 1;
        self.episode = Some(started);
        self.next_episode_start = DiagStart::Typing;
    }

    /// idle に戻る直前に呼ぶ（`state` はまだ前の状態）。
    fn emit_episode(&mut self, end: DiagEnd, converting: Option<&ConvertingState>) {
        let Some(finished) = self.episode.take() else {
            return;
        };
        if self.session.secure {
            return;
        }
        let prev = end.is_external().then(|| self.previous_kind());
        let current = converting.or(match &self.state {
            InputState::Converting(current) => Some(current),
            _ => None,
        });
        let commit = self.commit_info(end, current);
        let event = finished.event(
            self.session.session_id,
            end,
            prev,
            commit.as_ref(),
            self.now(),
        );
        self.engine.diag(&event);
    }

    /// 候補ウィンドウを出した（殻が位置を取ったあとに呼ぶ）。合成の最初の 1 回だけ覚える。
    pub fn note_panel_shown(&mut self, source: AnchorSource) {
        if let Some(episode) = &mut self.episode {
            episode.note_panel(source);
        }
    }

    /// 英語のモードで終わった合成を送る。secure のときは捨てる。
    pub fn record_english_episode(&self, finished: Option<DiagFinishedEpisode>) {
        if let Some(finished) = finished
            && !self.session.secure
        {
            self.engine.diag(&finished.event(self.session.session_id));
        }
    }

    /// 英語・日本語のモードが替わった（`sh_mode`）。secure のときは送らない。
    pub fn record_mode_switch(&self, from: InputMode, to: InputMode, via_key: bool) {
        if self.session.secure || from == to {
            return;
        }
        let via = if via_key {
            DiagModeVia::Key
        } else {
            DiagModeVia::Os
        };
        self.engine
            .diag(&mode_switch_event(self.session.session_id, from, to, via));
    }

    fn previous_kind(&self) -> DiagPrev {
        match &self.state {
            InputState::Converting(current) => {
                if current.segments.len() > 1 || self.mode.use_segment_api {
                    DiagPrev::Segments
                } else {
                    DiagPrev::Converting
                }
            }
            InputState::AsciiRun { .. } => DiagPrev::AsciiRun,
            InputState::Shortcode(_) => DiagPrev::Shortcode,
            InputState::ShortcodeChoice(_) => DiagPrev::ShortcodeChoice,
            InputState::Idle | InputState::Composing { .. } => DiagPrev::Composing,
        }
    }

    fn commit_info(
        &self,
        end: DiagEnd,
        current: Option<&ConvertingState>,
    ) -> Option<DiagCommitInfo> {
        if end == DiagEnd::ShortcodePick {
            return Some(DiagCommitInfo {
                pick: None,
                pick_kind: Some(self.shortcode.last_pick_kind()),
                segs: None,
                seg_changed: None,
            });
        }
        let current = current?;
        if end != DiagEnd::CommitCand && end != DiagEnd::CommitSegments {
            return None;
        }
        let mut info = DiagCommitInfo {
            pick: None,
            pick_kind: None,
            segs: Some(to_u32(current.segments.len())),
            seg_changed: None,
        };
        if end == DiagEnd::CommitCand {
            info.pick = Some(to_u32(current.candidate_offset + current.selected));
            info.pick_kind = Some(pick_kind(current));
        } else {
            info.seg_changed = Some(to_u32(
                current
                    .segments
                    .iter()
                    .filter(|s| s.literal.is_some() || s.candidate_index() != 0)
                    .count(),
            ));
        }
        Some(info)
    }

    fn episode_mut(&mut self) -> Option<&mut DiagEpisode> {
        self.episode.as_mut()
    }

    fn dispatch(&mut self, input: &KeyInput, text: &mut dyn TextAccess) -> Vec<ShellAction> {
        if self.delete_preview.is_some() {
            return self.handle_delete_preview(input, text);
        }
        if self.is_shortcode_state() {
            // ショートコードの間は、キー割り当てより先に機械へ渡す
            if !self.session.secure {
                return self.handle_shortcode(input, text);
            }
            let mut actions = self.commit_shortcode_display();
            actions.extend(self.dispatch(input, text));
            return actions;
        }
        let mut bound = self.bound_action(input);
        if bound == Some(KeymapAction::ClauseUndo) {
            if let Some(actions) = self.clause_undo_key(text) {
                return actions;
            }
            // 覚えがなければ、割り当てがないのと同じ
            bound = None;
        }
        if let Some(action) = bound {
            if matches!(
                action,
                KeymapAction::ClauseDelete | KeymapAction::ClauseDeletePreview
            ) && let Some(actions) = self.start_clause_delete(action, text)
            {
                return actions;
            }
            if action.is_script_conversion() {
                return if self.state == InputState::Idle {
                    vec![ShellAction::PassThrough]
                } else {
                    self.apply_script(action)
                };
            }
            if action == KeymapAction::Reconvert {
                return if self.state == InputState::Idle {
                    self.start_reconversion(text)
                } else {
                    vec![ShellAction::Consume]
                };
            }
            // 候補と文節の操作は変換中だけ効く
            if matches!(self.state, InputState::Converting(_)) {
                return match action {
                    KeymapAction::CandidateNext | KeymapAction::CandidatePrevious => {
                        if let Some(episode) = self.episode_mut() {
                            episode.cand_moves += 1;
                        }
                        let forward =
                            (action == KeymapAction::CandidateNext) != self.reverses_arrow(input);
                        if forward {
                            self.next_candidate()
                        } else {
                            self.previous_candidate()
                        }
                    }
                    _ => self.segment_action(action),
                };
            }
        }
        match self.state {
            InputState::Idle => self.handle_idle(input, text),
            InputState::Composing { .. } => self.handle_composing(input, text),
            InputState::Converting(_) => self.handle_converting(input, text),
            InputState::AsciiRun { .. } => self.handle_ascii_run(input, text),
            InputState::Shortcode(_) | InputState::ShortcodeChoice(_) => {
                self.handle_shortcode(input, text)
            }
        }
    }

    // MARK: idle

    fn handle_idle(&mut self, input: &KeyInput, text: &mut dyn TextAccess) -> Vec<ShellAction> {
        if has_command_modifiers(input) {
            return vec![ShellAction::PassThrough];
        }
        if let Some(entered) = self.enter_shortcode(input) {
            return entered;
        }
        match &input.key {
            KeyKind::Character { text: typed } => {
                if !is_printable(typed) {
                    return vec![ShellAction::PassThrough];
                }
                if is_shifted_letter(typed, input) {
                    return self.start_ascii_run(typed, text);
                }
                self.composer.clear();
                self.feed(typed);
                self.composing_started(text)
            }
            KeyKind::Jis { code } => {
                self.composer.clear();
                self.composer.push_jis(*code, input.modifiers.shift);
                self.composing_started(text)
            }
            // 取り残された変換中の文字列があれば、確定するだけにしてキーを消費する（ADR 0006 §16.3）
            KeyKind::Enter | KeyKind::Escape => vec![ShellAction::ResolveStrayMarkedText],
            KeyKind::Space
            | KeyKind::Backspace
            | KeyKind::Left
            | KeyKind::Right
            | KeyKind::Up
            | KeyKind::Down
            | KeyKind::Function { .. }
            | KeyKind::Other
            | KeyKind::Eisu
            | KeyKind::Kana => vec![ShellAction::PassThrough],
        }
    }

    /// 合成器が何も受け取らなかった（記号など）ときは、アプリにそのまま渡す
    fn composing_started(&mut self, text: &mut dyn TextAccess) -> Vec<ShellAction> {
        if self.composer.kana().is_empty() && self.composer.pending().is_empty() {
            return vec![ShellAction::PassThrough];
        }
        self.refresh_context(text);
        self.composing_keystrokes = 1;
        self.last_prefetched_reading = None;
        self.refresh_composing()
    }

    /// 合成の最初の数打鍵のあいだ、左文脈を読み直す（#1）
    fn refresh_context_on_early_keystroke(&mut self, text: &mut dyn TextAccess) {
        self.composing_keystrokes += 1;
        if self.composing_keystrokes > CONTEXT_REFRESH_KEYSTROKES || self.session.secure {
            return;
        }
        let before = self.context_rev;
        self.refresh_context(text);
        if self.context_rev != before {
            self.last_prefetched_reading = None;
        }
    }

    // MARK: composing

    fn handle_composing(
        &mut self,
        input: &KeyInput,
        text: &mut dyn TextAccess,
    ) -> Vec<ShellAction> {
        if has_command_modifiers(input) {
            return self.commit_display_then_pass();
        }
        match &input.key {
            KeyKind::Character { text: typed } => {
                if !is_printable(typed) {
                    return vec![ShellAction::Consume];
                }
                if is_shifted_letter(typed, input) {
                    return self.start_ascii_run(typed, text);
                }
                self.feed(typed);
                self.refresh_context_on_early_keystroke(text);
                self.refresh_composing()
            }
            KeyKind::Jis { code } => {
                self.composer.push_jis(*code, input.modifiers.shift);
                self.refresh_context_on_early_keystroke(text);
                self.refresh_composing()
            }
            KeyKind::Space => {
                let reading = self.reading_to_convert();
                self.start_conversion(reading, ReadingOrigin::Composing, text)
            }
            KeyKind::Enter => self.commit_display(),
            KeyKind::Escape => {
                let displayed = Utf16Len::of(&self.composer_display());
                self.composer.clear();
                // 再変換の BackSpace で戻った合成は、元に戻して終える（ADR 0006 §9.6）
                if let Some(target) = self.restore_target.clone() {
                    self.reset_composition(Some(DiagEnd::EscClear), None);
                    return vec![restore_action(
                        &target.original,
                        target.replacing,
                        displayed,
                    )];
                }
                self.reset_composition(Some(DiagEnd::EscClear), None);
                vec![clear_mark()]
            }
            KeyKind::Backspace => {
                if !self.composer.backspace() {
                    self.composer.clear();
                }
                self.refresh_composing()
            }
            KeyKind::Left
            | KeyKind::Right
            | KeyKind::Up
            | KeyKind::Down
            | KeyKind::Function { .. }
            | KeyKind::Other
            | KeyKind::Eisu
            | KeyKind::Kana => vec![ShellAction::Consume],
        }
    }

    fn composer_display(&self) -> String {
        format!("{}{}", self.composer.kana(), self.composer.pending())
    }

    /// 変換する読み（FR-07）。末尾の未確定が「n」1 文字のときだけ「ん」を確定する
    fn reading_to_convert(&mut self) -> String {
        if self.composer.pending() == TRAILING_N_PENDING {
            self.composer.flush();
        }
        self.composer.kana().to_string()
    }

    /// かなと pending から表示を作り、読みが変わったら Prefetch を投げる。空なら idle に戻す
    fn refresh_composing(&mut self) -> Vec<ShellAction> {
        if self.composer.kana().is_empty() && self.composer.pending().is_empty() {
            let end = if self.current_key == Some(KeyKind::Escape) {
                DiagEnd::EscClear
            } else {
                DiagEnd::BsEmpty
            };
            self.reset_composition(Some(end), None);
            return vec![clear_mark()];
        }
        // 合成中なら、今の状態の文字列の領域を使い回す（打鍵ごとに確保しない）。取り出している間、状態は idle に見えるが、
        // 先読みは状態を読まない。最後に必ず `Composing` に戻す
        let (mut kana, mut pending) = match std::mem::take(&mut self.state) {
            InputState::Composing { kana, pending } => (kana, pending),
            _ => (String::new(), String::new()),
        };
        kana.clear();
        kana.push_str(self.composer.kana());
        pending.clear();
        pending.push_str(self.composer.pending());
        self.prefetch_if_changed(&kana);
        let mut shown = String::with_capacity(kana.len() + pending.len());
        shown.push_str(&kana);
        shown.push_str(&pending);
        let marked = MarkedText {
            highlighted_length: Utf16Len::of(&kana),
            ..MarkedText::plain(shown)
        };
        self.state = InputState::Composing { kana, pending };
        vec![ShellAction::SetMarkedText(marked)]
    }

    // MARK: asciiRun

    fn start_ascii_run(&mut self, typed: &str, text: &mut dyn TextAccess) -> Vec<ShellAction> {
        if self.state == InputState::Idle {
            self.composer.clear();
            self.refresh_context(text);
        } else {
            // 合成中のかなは確定させてから英字を積む（n は ん になる）
            self.composer.flush();
        }
        self.ascii_prefix = self.composer.kana().to_string();
        self.last_prefetched_reading = None;
        let run = format!("{}{typed}", self.ascii_prefix);
        self.set_ascii_run(run)
    }

    fn set_ascii_run(&mut self, run: String) -> Vec<ShellAction> {
        let marked = MarkedText {
            highlighted_length: Utf16Len::of(&run),
            ..MarkedText::plain(run.clone())
        };
        self.state = InputState::AsciiRun { text: run };
        vec![ShellAction::SetMarkedText(marked)]
    }

    fn handle_ascii_run(
        &mut self,
        input: &KeyInput,
        text: &mut dyn TextAccess,
    ) -> Vec<ShellAction> {
        let InputState::AsciiRun { text: run } = &self.state else {
            return vec![ShellAction::Consume];
        };
        let run = run.clone();
        if has_command_modifiers(input) {
            return self.commit_display_then_pass();
        }
        match &input.key {
            KeyKind::Character { text: typed } => {
                if !is_printable(typed) {
                    return vec![ShellAction::Consume];
                }
                self.set_ascii_run(format!("{run}{typed}"))
            }
            KeyKind::Space => self.start_conversion(run, ReadingOrigin::Ascii, text),
            KeyKind::Enter => self.commit_display(),
            // 英字の部分を捨てて、残したかなの合成に戻る（かながなければ idle）
            KeyKind::Escape => self.leave_ascii_run(),
            KeyKind::Backspace => {
                let prefix_count = grapheme_count(&self.ascii_prefix);
                if grapheme_count(&run) > prefix_count {
                    let shortened = drop_last_grapheme(&run);
                    if grapheme_count(&shortened) > prefix_count {
                        return self.set_ascii_run(shortened);
                    }
                }
                self.leave_ascii_run()
            }
            KeyKind::Jis { .. }
            | KeyKind::Left
            | KeyKind::Right
            | KeyKind::Up
            | KeyKind::Down
            | KeyKind::Function { .. }
            | KeyKind::Other
            | KeyKind::Eisu
            | KeyKind::Kana => vec![ShellAction::Consume],
        }
    }

    fn leave_ascii_run(&mut self) -> Vec<ShellAction> {
        self.ascii_prefix.clear();
        self.refresh_composing()
    }

    // MARK: converting

    fn converting(&self) -> Option<&ConvertingState> {
        match &self.state {
            InputState::Converting(current) => Some(current),
            _ => None,
        }
    }

    fn handle_converting(
        &mut self,
        input: &KeyInput,
        text: &mut dyn TextAccess,
    ) -> Vec<ShellAction> {
        if self.converting().is_none() {
            return vec![ShellAction::Consume];
        }
        if has_command_modifiers(input) {
            return self.commit_display_then_pass();
        }
        match &input.key {
            KeyKind::Space => {
                // 最初の Space は `start_conversion` が数える。ここへ来るのは 2 回目以降
                if let Some(episode) = self.episode_mut() {
                    episode.spaces += 1;
                    episode.cand_moves += 1;
                }
                if input.modifiers.shift {
                    self.previous_candidate()
                } else {
                    self.next_candidate()
                }
            }
            KeyKind::Enter => self.commit_display(),
            KeyKind::Escape => {
                // 戻す処理は `self` を変えるので、状態は複製して渡す（Esc と Backspace だけ。打鍵ごとには複製しない）
                let Some(current) = self.converting().cloned() else {
                    return vec![ShellAction::Consume];
                };
                if let ConversionOrigin::Reconversion {
                    original,
                    replacing,
                } = &current.origin
                {
                    return self.restore_original(original, *replacing, &current);
                }
                if let Some(episode) = self.episode_mut() {
                    episode.esc_back += 1;
                }
                self.return_to_reading(&current)
            }
            KeyKind::Backspace => {
                let Some(current) = self.converting().cloned() else {
                    return vec![ShellAction::Consume];
                };
                self.return_to_reading(&current)
            }
            KeyKind::Character { text: typed } => {
                if !is_printable(typed) {
                    return vec![ShellAction::Consume];
                }
                if let Some(number) = single_digit(typed) {
                    return self.select_number(number);
                }
                let mut actions = self.commit_display();
                actions.extend(self.handle_idle(input, text));
                actions
            }
            KeyKind::Jis { code } => {
                if (JIS_DIGIT_1..=JIS_DIGIT_9).contains(code) && !input.modifiers.shift {
                    return self.select_number(usize::from(code - JIS_DIGIT_1) + 1);
                }
                let mut actions = self.commit_display();
                actions.extend(self.handle_idle(input, text));
                actions
            }
            // 矢印は `bound_action` が先に引く。ここへ来るのは割り当てが外れているとき
            KeyKind::Left
            | KeyKind::Right
            | KeyKind::Up
            | KeyKind::Down
            | KeyKind::Function { .. }
            | KeyKind::Other
            | KeyKind::Eisu
            | KeyKind::Kana => vec![ShellAction::Consume],
        }
    }

    fn page_size(&self) -> usize {
        to_usize(self.settings.page_size)
    }

    /// 数字キー。1 文節なら、その候補を選んで全体を確定する。2 文節以上なら、フォーカスしている文節の候補を選び、
    /// 次の文節へフォーカスを進める（最後の文節では動かさない。ADR 0006 §16.1-3）
    fn select_number(&mut self, number: usize) -> Vec<ShellAction> {
        if let Some(revealed) = self.reveal_candidates_if_hidden() {
            return revealed;
        }
        let page_size = self.page_size();
        let Some(current) = self.converting() else {
            return vec![ShellAction::Consume];
        };
        if number > current.visible_count(page_size) {
            return vec![ShellAction::Consume];
        }
        let (segment_count, focused) = (current.segments.len(), current.focused);
        if let Some(episode) = self.episode_mut() {
            episode.digit_picks += 1;
        }
        if segment_count <= 1 {
            return self.commit_converting(Some(number - 1));
        }
        let selection = self.select(number - 1);
        if focused + 1 < segment_count
            && let Some(moved) = self.move_focus(1)
        {
            return moved;
        }
        selection
    }

    /// 一覧を隠している変換中なら、一覧を出す（選択は動かさない）。隠していなければ `None`
    fn reveal_candidates_if_hidden(&mut self) -> Option<Vec<ShellAction>> {
        let InputState::Converting(current) = &mut self.state else {
            return None;
        };
        if !current.candidates_hidden {
            return None;
        }
        current.candidates_hidden = false;
        Some(vec![ShellAction::ShowCandidates(panel_model(
            current,
            &self.settings,
        ))])
    }

    fn next_candidate(&mut self) -> Vec<ShellAction> {
        if let Some(revealed) = self.reveal_candidates_if_hidden() {
            return revealed;
        }
        // 状態は複製しない。必要な値だけを先に取り出し、変える段になってから `self.state` を直接書き換える
        let Some(current) = self.converting() else {
            return vec![ShellAction::Consume];
        };
        let Some(segment) = current.segments.get(current.focused) else {
            return vec![ShellAction::Consume];
        };
        let visible = current.visible_count(self.page_size());
        let selected = current.selected;
        let candidate_count = current.candidates.len();
        let focused = current.focused;
        let next_index = current.page_index + 1;
        let cached_pages = segment.pages.len();
        let can_fetch = current.has_next && !current.is_fallback;
        if selected + 1 < visible {
            return self.select(selected + 1);
        }
        // 見せていない候補がページに残っているときは、ページ全体に広げて次の候補を選ぶ（ADR 0014 §2.3 (c)）
        if visible < candidate_count {
            if let InputState::Converting(state) = &mut self.state
                && let Some(segment) = state.focused_segment_mut()
            {
                segment.expanded = true;
            }
            return self.select(selected + 1);
        }
        // ページの最後の候補
        if next_index < cached_pages {
            self.count_page();
            return self.show_page(next_index, 0);
        }
        if can_fetch {
            let page = self
                .converting()
                .and_then(|current| self.fetch_page(current, focused, next_index));
            let Some(page) = page else {
                return vec![ShellAction::Consume];
            };
            let mut position = 0;
            if let InputState::Converting(state) = &mut self.state
                && let Some(segment) = state.focused_segment_mut()
            {
                segment.pages.push(page);
                position = segment.pages.len() - 1;
            }
            self.count_page();
            return self.show_page(position, 0);
        }
        if self.settings.paging_wrap {
            self.count_page();
            return self.show_page(0, 0);
        }
        vec![ShellAction::Consume]
    }

    fn count_page(&mut self) {
        if let Some(episode) = self.episode_mut() {
            episode.pages += 1;
        }
    }

    fn previous_candidate(&mut self) -> Vec<ShellAction> {
        if let Some(revealed) = self.reveal_candidates_if_hidden() {
            return revealed;
        }
        let Some(current) = self.converting() else {
            return vec![ShellAction::Consume];
        };
        if current.selected > 0 {
            let index = current.selected - 1;
            return self.select(index);
        }
        if current.page_index > 0 {
            let previous_count = current
                .segments
                .get(current.focused)
                .and_then(|segment| segment.pages.get(current.page_index - 1))
                .map_or(0, |page| page.candidates.len());
            let position = current.page_index - 1;
            return self.show_page(position, previous_count.saturating_sub(1));
        }
        vec![ShellAction::Consume]
    }

    /// 文節 `segment` の `index` ページ目をエンジンに作らせる。失敗、空の返事は `None`
    fn fetch_page(
        &self,
        current: &ConvertingState,
        segment: usize,
        index: usize,
    ) -> Option<ConversionPage> {
        let reading = &current.segments.get(segment)?.reading;
        let timeout = timeouts::convert_timeout(grapheme_count(reading));
        if self.mode.use_segment_api {
            let request = v1::SegmentPageRequest {
                conversion_id: current.conversion_id,
                layout_rev: current.layout_rev,
                segment: to_u32(segment),
                index: to_u32(index),
                left: choices(&current.segments[..segment]),
            };
            let page = self.engine.segment_page(request, timeout).ok()?;
            let page = SegmentPage::from_reply(&page);
            if page.candidates.is_empty() {
                return None;
            }
            return Some(ConversionPage::from_segment_page(
                &page,
                current.conversion_id,
            ));
        }
        let page = self
            .engine
            .page(current.conversion_id, to_u32(index), timeout)
            .ok()?;
        let page = ConversionPage::from_reply(&page);
        (!page.candidates.is_empty()).then_some(page)
    }

    /// フォーカスしている文節の `index` 番目の候補を選ぶ（同じページの中）
    fn select(&mut self, index: usize) -> Vec<ShellAction> {
        let InputState::Converting(current) = &mut self.state else {
            return vec![ShellAction::Consume];
        };
        let mut had_literal = false;
        if let Some(segment) = current.focused_segment_mut() {
            had_literal = segment.literal.is_some();
            segment.selected = index;
            segment.literal = None;
        }
        current.sync_focused();
        let model = panel_model(current, &self.settings);
        let marked = marked_text(current);
        // 字種変換で候補ウィンドウを閉じていたときは、開き直す
        vec![
            ShellAction::SetMarkedText(marked),
            if had_literal {
                ShellAction::ShowCandidates(model)
            } else {
                ShellAction::MoveSelection(model)
            },
        ]
    }

    /// フォーカスしている文節の、取得済みの `position` 番目のページに移る
    fn show_page(&mut self, position: usize, selected: usize) -> Vec<ShellAction> {
        let InputState::Converting(current) = &mut self.state else {
            return vec![ShellAction::Consume];
        };
        let page_index = current.page_index;
        let mut same_page = false;
        if let Some(segment) = current.focused_segment_mut() {
            same_page = position == page_index && segment.literal.is_none();
            segment.page_index = position;
            segment.selected = selected;
            segment.literal = None;
        }
        current.sync_focused();
        let model = panel_model(current, &self.settings);
        let marked = marked_text(current);
        vec![
            ShellAction::SetMarkedText(marked),
            if same_page {
                ShellAction::MoveSelection(model)
            } else {
                ShellAction::ShowCandidates(model)
            },
        ]
    }

    fn return_to_reading(&mut self, current: &ConvertingState) -> Vec<ShellAction> {
        if let ConversionOrigin::Reconversion {
            original,
            replacing,
        } = &current.origin
        {
            // 推定した読みの合成に戻る。この合成の中の Esc は、元の文字列に戻す（ADR 0006 §9.6）。
            // かな入力は読みを合成器に入れられない。何も変えずに、Esc で元に戻せるまま残す
            if !self.seed_composer(&current.reading) {
                return vec![ShellAction::Consume];
            }
            self.engine.cancel(current.conversion_id);
            self.mode = SegmentMode::default();
            self.restore_target = Some(RestoreTarget {
                original: original.clone(),
                replacing: *replacing,
            });
            self.reading_origin = ReadingOrigin::Composing;
            let mut actions = vec![ShellAction::HideCandidates];
            actions.extend(self.refresh_composing());
            return actions;
        }
        if !current.is_fallback {
            self.engine.cancel(current.conversion_id);
        }
        self.mode = SegmentMode::default();
        let mut actions = vec![ShellAction::HideCandidates];
        match self.reading_origin {
            ReadingOrigin::Composing => actions.extend(self.refresh_composing()),
            ReadingOrigin::Ascii => actions.extend(self.set_ascii_run(current.reading.clone())),
        }
        actions
    }

    /// 合成器に読みを入れ直す。ローマ字の合成器はかなをそのまま受け取る。入らなかった（かな入力）ら false
    fn seed_composer(&mut self, reading: &str) -> bool {
        self.composer.clear();
        self.feed(reading);
        if self.composer.kana() == reading {
            return true;
        }
        self.composer.clear();
        false
    }

    /// 元の文字列を書き戻して終える（再変換の Esc。ADR 0006 §9.5、§16.3）
    fn restore_original(
        &mut self,
        original: &str,
        replacing: Utf16Range,
        current: &ConvertingState,
    ) -> Vec<ShellAction> {
        if !current.is_fallback {
            self.engine.cancel(current.conversion_id);
        }
        let displayed = Utf16Len::of(&marked_text(current).text);
        self.reset_composition(Some(DiagEnd::EscClear), None);
        vec![
            ShellAction::HideCandidates,
            restore_action(original, replacing, displayed),
        ]
    }

    // MARK: 文節の操作（ADR 0006 §8.2）

    fn segment_action(&mut self, action: KeymapAction) -> Vec<ShellAction> {
        let actions = self.perform_segment_action(action);
        if actions != [ShellAction::Consume] {
            // 文節の操作は候補ウィンドウを出す。隠していた印は下ろす
            if let InputState::Converting(current) = &mut self.state {
                current.candidates_hidden = false;
            }
            if let Some(episode) = self.episode_mut() {
                match action {
                    KeymapAction::SegmentFocusLeft | KeymapAction::SegmentFocusRight => {
                        episode.seg_focus += 1;
                    }
                    KeymapAction::SegmentShrink | KeymapAction::SegmentExpand => {
                        episode.seg_resize += 1;
                    }
                    _ => {}
                }
            }
        }
        actions
    }

    fn perform_segment_action(&mut self, action: KeymapAction) -> Vec<ShellAction> {
        let Some(current) = self.converting().cloned() else {
            return vec![ShellAction::Consume];
        };
        if current.is_fallback || !self.mode.can_segment {
            return vec![ShellAction::Consume];
        }
        if let Some(layout) = current.whole_layout.clone() {
            return self.enter_segment_mode(action, current, &layout);
        }
        match action {
            KeymapAction::SegmentFocusLeft => self
                .move_focus(-1)
                .unwrap_or_else(|| vec![ShellAction::Consume]),
            KeymapAction::SegmentFocusRight => self
                .move_focus(1)
                .unwrap_or_else(|| vec![ShellAction::Consume]),
            KeymapAction::SegmentShrink => self.resize_focused(-1),
            KeymapAction::SegmentExpand => self.resize_focused(1),
            _ => vec![ShellAction::Consume],
        }
    }

    /// 全体の候補を出している間に文節の操作のキーを受けたとき、文節のモードに入ってそのキーを実行する（ADR 0006 §16.6）。
    /// 失敗（時間切れ、端など）は、全体の候補のまま何も変えない
    fn enter_segment_mode(
        &mut self,
        action: KeymapAction,
        current: ConvertingState,
        layout: &SegmentLayoutInfo,
    ) -> Vec<ShellAction> {
        let previous_mode = self.mode;
        self.mode.use_segment_api = true;
        let carries_selection = matches!(
            action,
            KeymapAction::SegmentFocusLeft | KeymapAction::SegmentFocusRight
        );
        let Some(next) = self.segment_mode_state(&current, layout, carries_selection) else {
            return self.restore_segment_mode(previous_mode, current);
        };
        self.state = InputState::Converting(next.clone());
        let actions = match action {
            KeymapAction::SegmentFocusLeft => {
                // キャレットは末尾にあったので、末尾の文節に入る（#87）。文節が 1 つなら動かない
                if next.segments.len() > 1 {
                    self.move_focus(-1)
                } else {
                    Some(vec![
                        ShellAction::SetMarkedText(marked_text(&next)),
                        ShellAction::ShowCandidates(self.panel_model(&next)),
                    ])
                }
            }
            KeymapAction::SegmentFocusRight => self.move_focus(1),
            KeymapAction::SegmentShrink | KeymapAction::SegmentExpand => {
                let delta = if action == KeymapAction::SegmentShrink {
                    -1
                } else {
                    1
                };
                let actions = self.resize_focused(delta);
                (actions != [ShellAction::Consume]).then_some(actions)
            }
            _ => None,
        };
        actions.unwrap_or_else(|| self.restore_segment_mode(previous_mode, current))
    }

    fn restore_segment_mode(
        &mut self,
        mode: SegmentMode,
        current: ConvertingState,
    ) -> Vec<ShellAction> {
        self.mode = mode;
        self.state = InputState::Converting(current);
        vec![ShellAction::Consume]
    }

    /// 全体の候補の状態から、文節のモードの状態を作る。`carrying` が真なら、いま表示している表記を保つ
    fn segment_mode_state(
        &self,
        current: &ConvertingState,
        layout: &SegmentLayoutInfo,
        carrying: bool,
    ) -> Option<ConvertingState> {
        let (segments, focused) = converting_segments(layout, Some(&current.reading), true)?;
        let mut next = current.clone();
        next.segments = segments;
        next.focused = focused;
        next.conversion_id = layout.conversion_id;
        next.layout_rev = layout.layout_rev;
        next.whole_layout = None;
        next.page_index = 0;
        next.selected = 0;
        next.candidate_offset = 0;
        if carrying
            && let Some(shown) = current
                .segments
                .first()
                .map(|s| s.displayed_surface().to_string())
            && shown != layout.joined_surface()
        {
            // 作れない表記でも文節のモードには入る。選択は 0 番のまま
            if let Some(picks) = self.segment_choices(&shown, &mut next) {
                for (segment, pick) in next.segments.iter_mut().zip(picks) {
                    segment.selected = pick;
                }
            }
        }
        next.sync_focused();
        Some(next)
    }

    /// 各文節の最初のページの候補から 1 つずつ選んでつなぐと `surface` になる組み合わせを探す（順位の小さいものを優先）
    fn segment_choices(&self, surface: &str, state: &mut ConvertingState) -> Option<Vec<usize>> {
        for index in 0..state.segments.len() {
            if state.segments[index].pages.is_empty() {
                let page = self.fetch_page(state, index, 0)?;
                state.segments[index].pages = vec![page];
            }
        }
        let lists: Vec<Vec<&str>> = state
            .segments
            .iter()
            .map(|segment| {
                segment.pages[0]
                    .candidates
                    .iter()
                    .map(|c| c.surface.as_str())
                    .collect()
            })
            .collect();
        let mut picks = Vec::new();
        search_choices(&lists, 0, surface, &mut picks).then_some(picks)
    }

    /// フォーカスを左右の文節へ移す。端では反対の端へ回る（文節が 1 つなら `None`）。
    /// まだ候補を持たない文節は、先にエンジンから 1 ページ目を取る（失敗したら `None`）
    fn move_focus(&mut self, delta: isize) -> Option<Vec<ShellAction>> {
        let mut current = self.converting()?.clone();
        let count = current.segments.len();
        if count <= 1 {
            return None;
        }
        let count_i = isize::try_from(count).ok()?;
        let focused_i = isize::try_from(current.focused).ok()?;
        let target = usize::try_from(((focused_i + delta) % count_i + count_i) % count_i).ok()?;
        if current.segments[target].pages.is_empty() {
            let page = self.fetch_page(&current, target, 0)?;
            current.segments[target].pages = vec![page];
        }
        current.focused = target;
        current.sync_focused();
        self.state = InputState::Converting(current.clone());
        Some(vec![
            ShellAction::SetMarkedText(marked_text(&current)),
            ShellAction::ShowCandidates(self.panel_model(&current)),
        ])
    }

    /// Shift+←/→。フォーカスしている文節を 1 文字縮める／伸ばす。失敗は、区切りを変えずにいまの表示のまま
    fn resize_focused(&mut self, delta: i32) -> Vec<ShellAction> {
        let Some(current) = self.converting().cloned() else {
            return vec![ShellAction::Consume];
        };
        let Some(segment) = current.segments.get(current.focused) else {
            return vec![ShellAction::Consume];
        };
        let reading_count = grapheme_count(&segment.reading);
        if delta < 0 && reading_count <= 1 {
            return vec![ShellAction::Consume];
        }
        if delta > 0 && current.focused == current.segments.len() - 1 {
            return vec![ShellAction::Consume];
        }
        let request = v1::ResizeSegment {
            conversion_id: current.conversion_id,
            layout_rev: current.layout_rev,
            segment: to_u32(current.focused),
            delta,
            left: choices(&current.segments[..current.focused]),
        };
        let Ok(reply) = self
            .engine
            .resize(request, timeouts::convert_timeout(reading_count))
        else {
            return vec![ShellAction::Consume];
        };
        let layout = SegmentLayoutInfo::from_reply(&reply);
        let Some((built, built_focused)) =
            converting_segments(&layout, Some(&current.reading), true)
        else {
            return vec![ShellAction::Consume];
        };
        // 左の文節は変わっていないはず（変わっていたら信用しない）
        if built_focused < current.focused
            || current.segments[..current.focused]
                .iter()
                .zip(&built)
                .any(|(old, new)| old.reading != new.reading)
        {
            return vec![ShellAction::Consume];
        }
        let mut next = current.clone();
        next.segments = current.segments[..current.focused]
            .iter()
            .cloned()
            .chain(built.into_iter().skip(current.focused))
            .collect();
        next.focused = built_focused;
        next.layout_rev = layout.layout_rev;
        next.conversion_id = layout.conversion_id;
        next.sync_focused();
        self.mode.use_segment_api = true;
        self.mode.layout_changed = true;
        self.state = InputState::Converting(next.clone());
        vec![
            ShellAction::SetMarkedText(marked_text(&next)),
            ShellAction::ShowCandidates(self.panel_model(&next)),
        ]
    }

    // MARK: 変換の開始

    fn start_conversion(
        &mut self,
        reading: String,
        origin: ReadingOrigin,
        text: &mut dyn TextAccess,
    ) -> Vec<ShellAction> {
        if reading.is_empty() {
            return vec![ShellAction::Consume];
        }
        self.reading_origin = origin;
        let reading_count = grapheme_count(&reading);

        let mut page: Option<ConversionPage> = None;
        let mut waited_nanos: Option<u64> = None;
        if reading_count <= MAX_READING_CHARACTERS {
            self.refresh_context(text);
            let available = self
                .cached_context
                .as_ref()
                .is_some_and(|context| context.available);
            if let Some(episode) = self.episode_mut() {
                episode.note_context(available);
            }
            let request = v1::Convert {
                session_id: self.session.session_id,
                reading: reading.clone(),
                context: Some(self.context_for_request().to_proto()),
                context_rev: self.context_rev,
                page_size: ENGINE_PAGE_SIZE,
            };
            let timeout = timeouts::convert_timeout(reading_count);
            let wait_start = self.now();
            // 失敗（落ちた、時間切れ、壊れた返事）は、入力を止めずに殻だけの候補にする（ADR 0004 §7.5）
            if let Ok(reply) = self.engine.convert(request, timeout) {
                let reply = ConversionPage::from_reply(&reply);
                if !reply.candidates.is_empty() {
                    page = Some(reply);
                }
            }
            waited_nanos = Some(self.now().wrapping_sub(wait_start));
        }
        let now = self.now();
        let used_fallback = page.is_none();
        if let Some(episode) = self.episode_mut() {
            episode.note_conversion(to_u32(reading_count), now, waited_nanos, used_fallback);
        }
        match page {
            Some(page) => self.enter_converting(reading, page, false),
            None => {
                let page = fallback_page(&reading);
                self.enter_converting(reading, page, true)
            }
        }
    }

    /// 変換中に入る。いつも読み全体の候補から始める（ADR 0006 §16.6）
    fn enter_converting(
        &mut self,
        reading: String,
        page: ConversionPage,
        is_fallback: bool,
    ) -> Vec<ShellAction> {
        let surface = page
            .candidates
            .first()
            .map(|c| c.surface.clone())
            .unwrap_or_default();
        let mut whole_layout = None;
        let mut layout_rev = 0;
        self.mode = SegmentMode::default();
        if !is_fallback
            && let Some(layout) = &page.layout
            && let Some((built, _)) =
                converting_segments(layout, Some(&reading), layout.segments.len() > 1)
        {
            self.mode.can_segment = true;
            layout_rev = layout.layout_rev;
            // 2 文節以上でも、最初は読み全体の候補を出す。区切りは表示にだけ使う
            if built.len() > 1 {
                whole_layout = Some(layout.clone());
            }
        }
        let confident = page.confident;
        let mut converting = ConvertingState {
            reading: reading.clone(),
            conversion_id: page.conversion_id,
            // `candidates` と `has_next` は、すぐ下の `sync_focused` が現在のページから写す（ここでは複製しない）
            is_fallback,
            layout_rev,
            whole_layout,
            lm_report: if is_fallback {
                None
            } else {
                page.lm_report.clone()
            },
            segments: vec![ConvertingSegment::new(reading, surface, vec![page])],
            ..ConvertingState::default()
        };
        converting.sync_focused();
        // 確信が高い読み全体の変換は、1 位だけを出す（#69）
        if confident
            && self.settings.candidate_hide_when_confident
            && !self.session.secure
            && !is_fallback
            && converting.whole_layout.is_none()
            && !converting.candidates.is_empty()
        {
            converting.candidates_hidden = true;
            let marked = marked_text(&converting);
            self.state = InputState::Converting(converting);
            return vec![ShellAction::SetMarkedText(marked)];
        }
        let actions = vec![
            ShellAction::SetMarkedText(marked_text(&converting)),
            ShellAction::ShowCandidates(self.panel_model(&converting)),
        ];
        self.state = InputState::Converting(converting);
        actions
    }

    /// F6〜F10 とプリセットのキー。
    /// - 合成中、英字、1 文節のままの変換: 読みを字種変換して、候補が 1 つの converting に入る（パネルは出さない）
    /// - 区切りを持つ変換: フォーカスしている文節だけを字種変換して `literal` に置く
    fn apply_script(&mut self, action: KeymapAction) -> Vec<ShellAction> {
        let mut carried_origin = ConversionOrigin::Composing;
        let reading = match &mut self.state {
            InputState::Idle | InputState::Shortcode(_) | InputState::ShortcodeChoice(_) => {
                return vec![ShellAction::PassThrough];
            }
            InputState::Composing { .. } => {
                self.reading_origin = ReadingOrigin::Composing;
                self.reading_to_convert()
            }
            InputState::AsciiRun { text } => {
                let text = text.clone();
                self.reading_origin = ReadingOrigin::Ascii;
                text
            }
            InputState::Converting(current) => {
                if self.mode.use_segment_api && !current.is_fallback {
                    if let Some(segment) = current.focused_segment_mut() {
                        segment.literal = Some(convert_to_script(&segment.reading, action));
                    }
                    let current = current.clone();
                    return vec![
                        ShellAction::HideCandidates,
                        ShellAction::SetMarkedText(marked_text(&current)),
                    ];
                }
                carried_origin = current.origin.clone();
                let (reading, conversion_id, is_fallback) = (
                    current.reading.clone(),
                    current.conversion_id,
                    current.is_fallback,
                );
                if !is_fallback {
                    self.engine.cancel(conversion_id);
                }
                reading
            }
        };
        if reading.is_empty() {
            return vec![ShellAction::Consume];
        }
        let surface = convert_to_script(&reading, action);
        let page = ConversionPage {
            candidates: vec![CandidateItem {
                kind: script_kind(action),
                ..candidate(&surface)
            }],
            ..ConversionPage::default()
        };
        self.mode = SegmentMode::default();
        let mut converting = ConvertingState {
            reading: reading.clone(),
            is_fallback: true,
            segments: vec![ConvertingSegment::new(reading, surface, vec![page])],
            origin: carried_origin,
            ..ConvertingState::default()
        };
        converting.sync_focused();
        let marked = marked_text(&converting);
        self.state = InputState::Converting(converting);
        vec![
            ShellAction::HideCandidates,
            ShellAction::SetMarkedText(marked),
        ]
    }

    // MARK: - 再変換（ADR 0006 §9）

    /// idle で `reconvert` のキーを受けた。断る条件（§9.2）に当たれば何もしない（受け止める）
    fn start_reconversion(&mut self, text: &mut dyn TextAccess) -> Vec<ShellAction> {
        if self.session.secure {
            return vec![ShellAction::Consume];
        }
        let Some(selected) = text.selection().filter(is_reconvertible) else {
            return vec![ShellAction::Consume];
        };
        self.refresh_context(text);
        let Some(context) = self.cached_context.clone().filter(|c| c.available) else {
            return vec![ShellAction::Consume];
        };
        let request = v1::Reconvert {
            session_id: self.session.session_id,
            surface: selected.text.clone(),
            context: Some(context.to_proto()),
            page_size: ENGINE_PAGE_SIZE,
        };
        let timeout = timeouts::convert_timeout(grapheme_count(&selected.text));
        let Ok(reply) = self.engine.reconvert(request, timeout) else {
            return vec![ShellAction::Consume];
        };
        let layout = SegmentLayoutInfo::from_reply(&reply);
        let built = converting_segments(&layout, None, true);
        // 最初の表示は、選択していた文字列と 1 文字も変わらない（食い違う返事は信用せず、文書に触れない）
        let Some((segments, focused)) = built.filter(|_| layout.joined_surface() == selected.text)
        else {
            self.engine.cancel(layout.conversion_id);
            return vec![ShellAction::Consume];
        };

        self.reading_origin = ReadingOrigin::Composing;
        self.next_episode_start = DiagStart::Reconvert;
        self.mode = SegmentMode {
            can_segment: true,
            use_segment_api: true,
            layout_changed: false,
        };
        let mut converting = ConvertingState {
            reading: layout.joined_reading(),
            conversion_id: layout.conversion_id,
            segments,
            focused,
            layout_rev: layout.layout_rev,
            origin: ConversionOrigin::Reconversion {
                original: selected.text.clone(),
                replacing: selected.range,
            },
            ..ConvertingState::default()
        };
        converting.sync_focused();
        let mut marked = marked_text(&converting);
        marked.replacement_range = Some(selected.range);
        let model = self.panel_model(&converting);
        self.state = InputState::Converting(converting);
        vec![
            ShellAction::SetMarkedText(marked),
            ShellAction::ShowCandidates(model),
        ]
    }

    // MARK: - 確定

    fn commit_display_then_pass(&mut self) -> Vec<ShellAction> {
        let mut actions = self.commit_display();
        actions.push(ShellAction::PassThrough);
        actions
    }

    /// 表示中の文字列を確定する。
    fn commit_display(&mut self) -> Vec<ShellAction> {
        match &self.state {
            InputState::Idle => Vec::new(),
            InputState::Shortcode(_) | InputState::ShortcodeChoice(_) => {
                self.commit_shortcode_display()
            }
            InputState::Converting(_) => self.commit_converting(None),
            InputState::Composing { .. } => {
                self.composer.flush();
                let text = self.composer_display();
                self.commit_text(text.clone(), text)
            }
            InputState::AsciiRun { text } => {
                let text = text.clone();
                self.commit_text(text.clone(), text)
            }
        }
    }

    /// 変換中の全体を確定する。`selecting` があれば、フォーカスしている文節にその候補を選んでから確定する（1 文節の数字キー）。
    fn commit_converting(&mut self, selecting: Option<usize>) -> Vec<ShellAction> {
        let Some(mut current) = self.converting().cloned() else {
            return vec![ShellAction::Consume];
        };
        if let Some(index) = selecting {
            if index >= current.candidates.len() {
                return vec![ShellAction::Consume];
            }
            if let Some(segment) = current.focused_segment_mut() {
                segment.selected = index;
                segment.literal = None;
            }
            current.sync_focused();
        }
        let surface: String = current
            .segments
            .iter()
            .map(ConvertingSegment::displayed_surface)
            .collect();
        if !self.session.secure {
            if current.is_fallback {
                self.engine.commit_text(v1::CommitText {
                    session_id: self.session.session_id,
                    reading: current.reading.clone(),
                    surface: surface.clone(),
                });
            } else if !self.mode.use_segment_api {
                self.engine.commit(
                    current.conversion_id,
                    to_u32(current.candidate_offset + current.selected),
                );
            } else if matches!(current.origin, ConversionOrigin::Reconversion { .. })
                && self.is_unchanged(&current)
            {
                self.engine.cancel(current.conversion_id);
            } else {
                self.engine.commit_segments(v1::CommitSegments {
                    conversion_id: current.conversion_id,
                    layout_rev: current.layout_rev,
                    choices: choices(&current.segments),
                });
            }
        }
        let splits_segments = current.segments.len() > 1 || self.mode.use_segment_api;
        let end = self.external_end.unwrap_or(if splits_segments {
            DiagEnd::CommitSegments
        } else {
            DiagEnd::CommitCand
        });
        self.reset_composition(Some(end), Some(&current));
        vec![
            ShellAction::Commit { text: surface },
            ShellAction::HideCandidates,
        ]
    }

    /// 区切りも、すべての文節の選択も、最初のまま
    fn is_unchanged(&self, current: &ConvertingState) -> bool {
        !self.mode.layout_changed
            && current
                .segments
                .iter()
                .all(|s| s.literal.is_none() && s.candidate_index() == 0)
    }

    fn commit_text(&mut self, text: String, reading: String) -> Vec<ShellAction> {
        let end = self
            .external_end
            .unwrap_or(if self.current_key == Some(KeyKind::Enter) {
                DiagEnd::CommitKana
            } else {
                DiagEnd::CommitComposition
            });
        self.reset_composition(Some(end), None);
        if text.is_empty() {
            return vec![clear_mark()];
        }
        if !self.session.secure {
            self.engine.commit_text(v1::CommitText {
                session_id: self.session.session_id,
                reading,
                surface: text.clone(),
            });
        }
        vec![ShellAction::Commit { text }]
    }

    /// `end` は診断ログの終わった理由。`None` なら記録を捨てる（打ち直しなど）
    fn reset_composition(&mut self, end: Option<DiagEnd>, converting: Option<&ConvertingState>) {
        match end {
            Some(end) => self.emit_episode(end, converting),
            None => self.episode = None,
        }
        self.composer.clear();
        self.state = InputState::Idle;
        self.reading_origin = ReadingOrigin::Composing;
        self.ascii_prefix.clear();
        self.mode = SegmentMode::default();
        self.restore_target = None;
        self.delete_preview = None;
        self.last_prefetched_reading = None;
        self.composing_keystrokes = 0;
        // 確定すると左文脈が変わる。次の合成の始めに取り直す
        self.cached_context = None;
    }

    // MARK: - ショートコード（ADR 0027）

    fn is_shortcode_state(&self) -> bool {
        matches!(
            self.state,
            InputState::Shortcode(_) | InputState::ShortcodeChoice(_)
        )
    }

    fn can_enter_shortcode(&self) -> bool {
        self.state == InputState::Idle
            && !self.replaying_shortcode
            && !self.input_is_repeat
            && !self.session.secure
            && self.settings.emoji_shortcode
            && !self
                .settings
                .emoji_shortcode_exclude
                .contains(&self.session.client_bundle_id)
    }

    /// idle で入りのキーを受けたら、`shortcode` に入る
    fn enter_shortcode(&mut self, input: &KeyInput) -> Option<Vec<ShellAction>> {
        if !self.can_enter_shortcode() {
            return None;
        }
        let result = self.shortcode.handle(input, true);
        let (ShortcodeOutcome::Typing { marked, .. }, ShortcodePhase::Typing(typing)) =
            (result.outcome, result.phase)
        else {
            return None;
        };
        self.composer.clear();
        self.state = InputState::Shortcode(typing);
        Some(vec![highlighted(&marked)])
    }

    fn handle_shortcode(
        &mut self,
        input: &KeyInput,
        text: &mut dyn TextAccess,
    ) -> Vec<ShellAction> {
        let previous_page = match &self.state {
            InputState::ShortcodeChoice(before) => Some(before.page()),
            _ => None,
        };
        let was_choice = previous_page.is_some();
        let result = self.shortcode.handle(input, false);
        match result.outcome {
            ShortcodeOutcome::NotHandled => {
                // 機械と状態がずれた（起きないはず）。表示を消して idle に戻し、キーを通常の経路で処理する
                self.reset_composition(None, None);
                let mut actions = vec![clear_mark(), ShellAction::HideCandidates];
                actions.extend(self.dispatch(input, text));
                actions
            }
            ShortcodeOutcome::Typing {
                marked,
                hides_candidates,
            } => {
                let ShortcodePhase::Typing(typing) = result.phase else {
                    return vec![ShellAction::Consume];
                };
                self.state = InputState::Shortcode(typing);
                let mut actions = vec![highlighted(&marked)];
                if hides_candidates {
                    actions.push(ShellAction::HideCandidates);
                }
                actions
            }
            ShortcodeOutcome::Cancelled => {
                // `:` だけで Backspace（Esc は打ち直しになる）
                self.reset_composition(Some(DiagEnd::BsEmpty), None);
                vec![clear_mark()]
            }
            ShortcodeOutcome::ShowChoices(choosing) => {
                let actions = vec![
                    highlighted(choosing.marked()),
                    ShellAction::ShowCandidates(shortcode_panel_model(&choosing)),
                ];
                self.state = InputState::ShortcodeChoice(choosing);
                actions
            }
            ShortcodeOutcome::MoveChoice(choosing) => {
                let same_page = previous_page == Some(choosing.page());
                let model = shortcode_panel_model(&choosing);
                let actions = vec![
                    highlighted(choosing.marked()),
                    if same_page {
                        ShellAction::MoveSelection(model)
                    } else {
                        ShellAction::ShowCandidates(model)
                    },
                ];
                self.state = InputState::ShortcodeChoice(choosing);
                actions
            }
            ShortcodeOutcome::Commit(commit) => {
                let end = if was_choice {
                    DiagEnd::ShortcodePick
                } else {
                    DiagEnd::ShortcodeLiteral
                };
                let after = commit.after.clone();
                let mut actions = self.commit_shortcode_text(&commit, was_choice, end);
                match after {
                    ShortcodeAfterCommit::Consume => {}
                    ShortcodeAfterCommit::PassKey => actions.push(ShellAction::PassThrough),
                    ShortcodeAfterCommit::ContinueWith(key) => {
                        actions.extend(self.handle_idle(&key, text));
                    }
                }
                actions
            }
            // 日本語のモードの Esc だけ、今のキーを流さずに（合成に戻って）消費する
            ShortcodeOutcome::Retype { keys, .. } => {
                self.retype_shortcode(&keys, input.key != KeyKind::Escape, text)
            }
        }
    }

    /// 打ち直し（§3.3）。入ってから受けたキーを、ショートコードを切った idle から順に流す。
    /// 途中のキーの `PassThrough` は捨てる。最後のキーが今のキーなら、その `PassThrough` は残す
    fn retype_shortcode(
        &mut self,
        keys: &[KeyInput],
        includes_current_key: bool,
        text: &mut dyn TextAccess,
    ) -> Vec<ShellAction> {
        self.reset_composition(None, None);
        self.replaying_shortcode = true;
        let mut actions = vec![clear_mark()];
        let mut without_selection = WithoutSelection(text);
        for (index, key) in keys.iter().enumerate() {
            let mut replayed = self.dispatch(key, &mut without_selection);
            if !(includes_current_key && index == keys.len() - 1) {
                replayed.retain(|action| *action != ShellAction::PassThrough);
            }
            actions.extend(replayed);
        }
        self.replaying_shortcode = false;
        actions
    }

    /// 今の表示のまま確定する（モードの切り替え、secure、`deactivateServer`、クリック）
    fn commit_shortcode_display(&mut self) -> Vec<ShellAction> {
        let choosing = matches!(self.state, InputState::ShortcodeChoice(_));
        let end = self.external_end.unwrap_or(DiagEnd::CommitComposition);
        let Some(commit) = self.shortcode.flush() else {
            self.reset_composition(Some(end), None);
            return if choosing {
                vec![clear_mark(), ShellAction::HideCandidates]
            } else {
                vec![clear_mark()]
            };
        };
        self.commit_shortcode_text(&commit, choosing, end)
    }

    /// 確定の文字列を挿入し、確定履歴に送る（読みは `:` + 名前。§3.7）。学習には送らない
    fn commit_shortcode_text(
        &mut self,
        commit: &ShortcodeCommit,
        hides_candidates: bool,
        end: DiagEnd,
    ) -> Vec<ShellAction> {
        self.reset_composition(Some(end), None);
        if !self.session.secure {
            self.engine.commit_text(v1::CommitText {
                session_id: self.session.session_id,
                reading: commit.reading.clone(),
                surface: commit.text.clone(),
            });
        }
        let mut actions = vec![ShellAction::Commit {
            text: commit.text.clone(),
        }];
        if hides_candidates {
            actions.push(ShellAction::HideCandidates);
        }
        actions
    }

    // MARK: - 文脈と先行計算

    fn context_for_request(&self) -> ContextText {
        self.cached_context
            .clone()
            .unwrap_or_else(ContextText::unavailable)
    }

    fn refresh_context(&mut self, text: &mut dyn TextAccess) {
        if self.session.secure {
            self.cached_context = None;
            return;
        }
        let fresh = ContextText::from_host(text.surrounding()).truncated();
        if self.cached_context.as_ref() != Some(&fresh) {
            self.cached_context = Some(fresh);
            self.context_rev = self.context_rev.wrapping_add(1);
        }
    }

    fn prefetch_if_changed(&mut self, reading: &str) {
        // 前回と同じなら、数える前に戻る（前回は上限以内だったので、結果は変わらない）。
        // 上限の判定は、バイト数が上限以下なら書記素の数も上限以下（書記素は 1 バイト以上）なので、走査を省く
        if self.session.secure
            || reading.is_empty()
            || self.last_prefetched_reading.as_deref() == Some(reading)
            || (reading.len() > MAX_READING_CHARACTERS
                && grapheme_count(reading) > MAX_READING_CHARACTERS)
        {
            if reading.is_empty() {
                self.last_prefetched_reading = None;
            }
            return;
        }
        match &mut self.last_prefetched_reading {
            Some(last) => {
                last.clear();
                last.push_str(reading);
            }
            None => self.last_prefetched_reading = Some(reading.to_string()),
        }
        self.engine.prefetch(v1::Prefetch {
            session_id: self.session.session_id,
            reading: reading.to_string(),
            context: Some(self.context_for_request().to_proto()),
            context_rev: self.context_rev,
        });
    }

    // MARK: - 小さな部品

    fn feed(&mut self, text: &str) {
        for character in text.chars() {
            self.composer.push_character(character);
        }
    }

    /// キー割り当て。F キーと、Ctrl / Option / Command を伴う 1 文字は常に引く。←/→/↑/↓ は修飾キーの有無を問わず引く
    fn bound_action(&self, input: &KeyInput) -> Option<KeymapAction> {
        // 引くキーの規則は設定アプリの記録と共有する（`KeyChord::for_lookup`）
        let chord = KeyChord::for_lookup(input).ok()?;
        self.keymap.action(&chord)
    }

    fn panel_model(&self, state: &ConvertingState) -> CandidatePanelModel {
        panel_model(state, &self.settings)
    }
}

// MARK: - 自由関数

/// 1 文節なら全体を太い下線。2 文節以上なら、文節ごとの UTF-16 の長さとフォーカス。
/// 全体の候補を出している間は、表示が最初の区切りの表記のままなら、区切りだけを付ける（フォーカスなし。§16.6）
fn marked_text(state: &ConvertingState) -> MarkedText {
    let surfaces: Vec<&str> = state
        .segments
        .iter()
        .map(ConvertingSegment::displayed_surface)
        .collect();
    if let Some(layout) = &state.whole_layout {
        let surface = surfaces.first().copied().unwrap_or_default();
        if layout.joined_surface() == surface {
            return MarkedText {
                segments: layout
                    .segments
                    .iter()
                    .map(|s| Utf16Len::of(&s.surface))
                    .collect(),
                ..MarkedText::plain(surface)
            };
        }
        return highlighted_text(surface);
    }
    if surfaces.len() <= 1 {
        let surface = surfaces.first().copied().unwrap_or_else(|| {
            state
                .candidates
                .get(state.selected)
                .map_or("", |c| c.surface.as_str())
        });
        return highlighted_text(surface);
    }
    MarkedText {
        segments: surfaces.iter().map(|s| Utf16Len::of(s)).collect(),
        focused: Some(to_u32(state.focused)),
        ..MarkedText::plain(surfaces.concat())
    }
}

/// 候補ウィンドウの中身。`self` を借りずに作れるので、状態を借りたまま呼べる（状態の複製をしない）
fn panel_model(state: &ConvertingState, settings: &SettingsSnapshot) -> CandidatePanelModel {
    let page_size = to_usize(settings.page_size);
    // このあと広がる、またはページが進み得るときは、位置を決める高さをページの上限で見込む（ADR 0014 §2.2）
    let visible = state.visible_count(page_size);
    let may_grow = state.has_next || state.page_index > 0;
    let reserved = if may_grow {
        to_usize(ENGINE_PAGE_SIZE)
    } else {
        state.candidates.len()
    };
    let model = panel(
        state.candidates.iter().take(visible).cloned().collect(),
        state.selected,
        state
            .candidates
            .get(state.selected)
            .map(|c| c.annotation.clone())
            .unwrap_or_default(),
        reserved,
    );
    // 開発者向けの 1 行。オフなら文を作らない（ADR 0061 D6）
    if settings.developer_lm_status
        && let Some(report) = &state.lm_report
    {
        return CandidatePanelModel {
            developer_note: developer_note::lm_status_line(report),
            ..model
        };
    }
    model
}

fn shortcode_mode(settings: &SettingsSnapshot) -> ShortcodeMode {
    if settings.input_method == crate::types::InputMethodKind::Kana {
        ShortcodeMode::KanaJis
    } else {
        ShortcodeMode::Romaji
    }
}

fn pick_kind(current: &ConvertingState) -> DiagPickKind {
    if current.is_fallback {
        return DiagPickKind::Fallback;
    }
    match current.candidates.get(current.selected).map(|c| c.kind) {
        None | Some(CandidateKind::Unspecified | CandidateKind::Dictionary) => DiagPickKind::Dict,
        Some(CandidateKind::Hiragana) => DiagPickKind::Hiragana,
        Some(CandidateKind::Katakana) => DiagPickKind::Katakana,
        Some(CandidateKind::Rewriter) => DiagPickKind::Rewriter,
        Some(CandidateKind::Literal) => DiagPickKind::Literal,
    }
}

/// 文節の選択を、エンジンに送る形にする（取得前の文節は 0 番）
fn choices(segments: &[ConvertingSegment]) -> Vec<v1::SegmentChoice> {
    segments
        .iter()
        .map(|segment| match &segment.literal {
            Some(literal) => SegmentChoiceValue::Literal(literal.clone()),
            None => SegmentChoiceValue::Candidate(segment.candidate_index()),
        })
        .map(|choice| choice.to_proto())
        .collect()
}

/// エンジンの区切りから、文節の列とフォーカスを作る。読みの合計が `expected_reading` と違う、フォーカスが範囲外、
/// （`require_focused_page` のとき）フォーカスの文節の候補がない、のどれかなら信用しない
fn converting_segments(
    layout: &SegmentLayoutInfo,
    expected_reading: Option<&str>,
    require_focused_page: bool,
) -> Option<(Vec<ConvertingSegment>, usize)> {
    if layout.focused >= layout.segments.len() {
        return None;
    }
    if let Some(expected) = expected_reading
        && layout.joined_reading() != expected
    {
        return None;
    }
    let focused_page = layout
        .focused_page
        .as_ref()
        .filter(|page| !page.candidates.is_empty());
    if require_focused_page && focused_page.is_none() {
        return None;
    }
    let focused_pages: Vec<ConversionPage> = focused_page
        .map(|page| {
            vec![ConversionPage::from_segment_page(
                page,
                layout.conversion_id,
            )]
        })
        .unwrap_or_default();
    let segments = layout
        .segments
        .iter()
        .enumerate()
        .map(|(index, summary)| {
            ConvertingSegment::new(
                summary.reading.clone(),
                summary.surface.clone(),
                if index == layout.focused {
                    focused_pages.clone()
                } else {
                    Vec::new()
                },
            )
        })
        .collect();
    Some((segments, layout.focused))
}

fn search_choices(lists: &[Vec<&str>], segment: usize, rest: &str, picks: &mut Vec<usize>) -> bool {
    let Some(list) = lists.get(segment) else {
        return rest.is_empty();
    };
    for (index, candidate) in list.iter().enumerate() {
        if let Some(remaining) = rest.strip_prefix(candidate) {
            picks.push(index);
            if search_choices(lists, segment + 1, remaining, picks) {
                return true;
            }
            picks.pop();
        }
    }
    false
}

fn fallback_page(reading: &str) -> ConversionPage {
    let mut candidates: Vec<CandidateItem> = Vec::new();
    for script in FALLBACK_SCRIPTS {
        let surface = convert_to_script(reading, script);
        if surface.is_empty() || candidates.iter().any(|c| c.surface == surface) {
            continue;
        }
        candidates.push(CandidateItem {
            kind: script_kind(script),
            ..candidate(&surface)
        });
    }
    ConversionPage {
        candidates,
        ..ConversionPage::default()
    }
}

fn script_kind(action: KeymapAction) -> CandidateKind {
    match action {
        KeymapAction::ToHiragana => CandidateKind::Hiragana,
        KeymapAction::ToFullwidthKatakana | KeymapAction::ToHalfwidthKatakana => {
            CandidateKind::Katakana
        }
        _ => CandidateKind::Unspecified,
    }
}

fn candidate(surface: &str) -> CandidateItem {
    CandidateItem {
        surface: surface.to_string(),
        annotation: String::new(),
        kind: CandidateKind::Dictionary,
        label: String::new(),
    }
}

/// §9.2 の断る条件。範囲が正しく、64 文字まで、改行も制御文字だけの文字列でもない
fn is_reconvertible(selected: &SelectedText) -> bool {
    if selected.range.length == Utf16Len::ZERO
        || selected.text.is_empty()
        || grapheme_count(&selected.text) > MAX_RECONVERT_CHARACTERS
        || selected.text.chars().any(is_newline)
    {
        return false;
    }
    !selected.text.chars().all(char::is_control)
}

/// Swift の `Character.isNewline`
fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{000B}' | '\u{000C}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
    )
}

fn shortcode_panel_model(choosing: &ShortcodeChoosing) -> CandidatePanelModel {
    let start = choosing.page() * CANDIDATE_PAGE_SIZE;
    let end = (start + CANDIDATE_PAGE_SIZE).min(choosing.candidates.len());
    let page = choosing.candidates.get(start..end).unwrap_or_default();
    panel(
        page.iter()
            .map(|c| CandidateItem {
                surface: c.surface.clone(),
                annotation: String::new(),
                kind: if c.label.is_none() {
                    CandidateKind::Literal
                } else {
                    CandidateKind::Unspecified
                },
                label: c.label.clone().unwrap_or_default(),
            })
            .collect(),
        choosing.selected - start,
        String::new(),
        choosing.candidates.len().min(CANDIDATE_PAGE_SIZE),
    )
}

/// Swift の `CandidatePanelModel.init`（見込む行は候補の数以上）
fn panel(
    candidates: Vec<CandidateItem>,
    selected: usize,
    annotation: String,
    reserved_rows: usize,
) -> CandidatePanelModel {
    let reserved_rows = to_u32(reserved_rows.max(candidates.len()));
    CandidatePanelModel {
        developer_note: String::new(),
        candidates,
        selected: to_u32(selected),
        annotation,
        reserved_rows,
    }
}

fn has_command_modifiers(input: &KeyInput) -> bool {
    input.modifiers.control || input.modifiers.option || input.modifiers.command
}

fn is_shifted_letter(text: &str, input: &KeyInput) -> bool {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => input.modifiers.shift && c.is_ascii_alphabetic(),
        _ => false,
    }
}

/// 1〜9 の半角数字 1 文字
fn single_digit(text: &str) -> Option<usize> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c @ '1'..='9'), None) => c.to_digit(10).and_then(|d| usize::try_from(d).ok()),
        _ => None,
    }
}

fn clear_mark() -> ShellAction {
    ShellAction::SetMarkedText(MarkedText::plain(""))
}

fn highlighted_text(text: &str) -> MarkedText {
    MarkedText {
        highlighted_length: Utf16Len::of(text),
        ..MarkedText::plain(text)
    }
}

fn highlighted(text: &str) -> ShellAction {
    ShellAction::SetMarkedText(highlighted_text(text))
}

/// 変換中の文字列は、再変換の選択範囲の先頭から始まる。長さは、いま表示している文字列の UTF-16 の長さ
fn restore_action(original: &str, replacing: Utf16Range, displayed: Utf16Len) -> ShellAction {
    ShellAction::Restore {
        text: original.to_string(),
        replacing: Utf16Range {
            location: replacing.location,
            length: displayed,
        },
    }
}

/// 見た目の文字の数（Swift の `String.count`）
pub(crate) fn grapheme_count(text: &str) -> usize {
    text.graphemes(true).count()
}

fn drop_last_grapheme(text: &str) -> String {
    text.grapheme_indices(true)
        .next_back()
        .map_or_else(String::new, |(start, _)| text[..start].to_string())
}

fn suffix_graphemes(text: &str, limit: usize) -> String {
    let count = grapheme_count(text);
    if count <= limit {
        return text.to_string();
    }
    text.grapheme_indices(true)
        .nth(count - limit)
        .map_or_else(String::new, |(start, _)| text[start..].to_string())
}

fn prefix_graphemes(text: &str, limit: usize) -> String {
    match text.grapheme_indices(true).nth(limit) {
        Some((end, _)) => text[..end].to_string(),
        None => text.to_string(),
    }
}
