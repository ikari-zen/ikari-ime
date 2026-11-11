//! 状態機械のテスト。シナリオ（`shell-core/scenarios`）で書けないもの（周辺テキストを読んだ回数、要求の欄、診断ログ、
//! 設定の反映、内部の状態）を、今の Swift のテスト（InputStateMachineTests、EarlyContextRefreshTests、ReconfigureTests、
//! DiagShellEventsTests、ReconversionTests）から写した。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ikari_proto::v1;

use super::clause_delete::committed_step_holds;
use super::*;
use crate::diag::DiagEvent;
use crate::keymap::Keymap;
use crate::types::{InputMethodKind, KeyKind, Modifiers};

/// エンジンへの呼び出しの記録。
#[derive(Debug, Clone, PartialEq)]
enum Call {
    BeginSession(v1::BeginSession),
    EndSession(u64),
    Prefetch(v1::Prefetch),
    Convert(v1::Convert, Duration),
    Page(u64, u32),
    Commit(u64, u32),
    CommitText(v1::CommitText),
    Cancel(u64),
    SegmentPage(v1::SegmentPageRequest),
    Resize(v1::ResizeSegment),
    RemoveSegments(v1::RemoveSegments),
    CommitSegments(v1::CommitSegments),
    Reconvert(v1::Reconvert),
}

#[derive(Default)]
struct FakeEngine {
    calls: Mutex<Vec<Call>>,
    diags: Mutex<Vec<DiagEvent>>,
    convert: Mutex<Option<Result<v1::ConvertReply, EngineError>>>,
    reconvert: Mutex<Option<v1::SegmentLayout>>,
    remove: Mutex<Option<v1::SegmentLayout>>,
}

use crate::types::EngineError;

fn candidates(surfaces: &[&str]) -> Vec<v1::Candidate> {
    surfaces
        .iter()
        .map(|surface| v1::Candidate {
            surface: (*surface).into(),
            annotation: String::new(),
            kind: v1::CandidateKind::Dictionary as i32,
        })
        .collect()
}

fn default_page() -> v1::ConvertReply {
    v1::ConvertReply {
        conversion_id: 7,
        page_index: 0,
        candidates: candidates(&["品詞", "瀕死", "ひんし"]),
        has_next: false,
        ..Default::default()
    }
}

impl FakeEngine {
    fn record(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn prefetches(&self) -> Vec<v1::Prefetch> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Prefetch(request) => Some(request),
                _ => None,
            })
            .collect()
    }

    fn converts(&self) -> Vec<(v1::Convert, Duration)> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Convert(request, timeout) => Some((request, timeout)),
                _ => None,
            })
            .collect()
    }

    fn diags(&self) -> Vec<DiagEvent> {
        self.diags.lock().unwrap().clone()
    }
}

impl MachineEngine for FakeEngine {
    fn begin_session(&self, session: v1::BeginSession) {
        self.record(Call::BeginSession(session));
    }
    fn end_session(&self, session_id: u64) {
        self.record(Call::EndSession(session_id));
    }
    fn prefetch(&self, request: v1::Prefetch) {
        self.record(Call::Prefetch(request));
    }
    fn convert(
        &self,
        request: v1::Convert,
        timeout: Duration,
    ) -> Result<v1::ConvertReply, EngineError> {
        self.record(Call::Convert(request, timeout));
        self.convert
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Ok(default_page()))
    }
    fn page(&self, id: u64, index: u32, _: Duration) -> Result<v1::ConvertReply, EngineError> {
        self.record(Call::Page(id, index));
        Err(EngineError::Timeout)
    }
    fn commit(&self, id: u64, index: u32) {
        self.record(Call::Commit(id, index));
    }
    fn commit_text(&self, request: v1::CommitText) {
        self.record(Call::CommitText(request));
    }
    fn cancel(&self, id: u64) {
        self.record(Call::Cancel(id));
    }
    fn segment_page(
        &self,
        request: v1::SegmentPageRequest,
        _: Duration,
    ) -> Result<v1::SegmentPageReply, EngineError> {
        self.record(Call::SegmentPage(request));
        Err(EngineError::Timeout)
    }
    fn resize(
        &self,
        request: v1::ResizeSegment,
        _: Duration,
    ) -> Result<v1::SegmentLayout, EngineError> {
        self.record(Call::Resize(request));
        Err(EngineError::Timeout)
    }
    fn remove_segments(
        &self,
        request: v1::RemoveSegments,
        _: Duration,
    ) -> Result<v1::SegmentLayout, EngineError> {
        self.record(Call::RemoveSegments(request));
        self.remove
            .lock()
            .unwrap()
            .clone()
            .ok_or(EngineError::Timeout)
    }
    fn commit_segments(&self, request: v1::CommitSegments) {
        self.record(Call::CommitSegments(request));
    }
    fn reconvert(
        &self,
        request: v1::Reconvert,
        _: Duration,
    ) -> Result<v1::SegmentLayout, EngineError> {
        self.record(Call::Reconvert(request));
        self.reconvert
            .lock()
            .unwrap()
            .clone()
            .ok_or(EngineError::Engine(
                crate::types::EngineErrorCode::BadRequest,
            ))
    }
    fn diag(&self, event: &DiagEvent) {
        self.diags.lock().unwrap().push(event.clone());
    }
}

/// 周辺テキストと選択。読んだ回数を数える。
struct CountingText {
    surrounding: Option<SurroundingText>,
    selection: Option<SelectedText>,
    surrounding_reads: usize,
    selection_reads: usize,
}

impl TextAccess for CountingText {
    fn surrounding(&mut self) -> Option<SurroundingText> {
        self.surrounding_reads += 1;
        self.surrounding.clone()
    }
    fn selection(&mut self) -> Option<SelectedText> {
        self.selection_reads += 1;
        self.selection.clone()
    }
}

struct TickClock(Mutex<u64>);

impl Clock for TickClock {
    fn now(&self) -> Duration {
        let mut ticks = self.0.lock().unwrap();
        *ticks += 1_000_000;
        Duration::from_nanos(*ticks)
    }
}

const SESSION_ID: u64 = 42;
const LEFT: &str = "ユーザー辞書に単語を登録するときは";

struct Harness {
    engine: Arc<FakeEngine>,
    machine: InputStateMachine,
    text: CountingText,
}

fn keymap(preset: &str) -> Arc<Keymap> {
    let path = format!(
        "{}/resources/keymap/{preset}.tsv",
        env!("CARGO_MANIFEST_DIR")
    );
    let tsv = std::fs::read_to_string(path).unwrap();
    Arc::new(Keymap::from_settings(&tsv, &Default::default()).unwrap())
}

fn surrounding(left: &str) -> Option<SurroundingText> {
    Some(SurroundingText {
        left: left.into(),
        right: String::new(),
        document_length: Utf16Len::of(left),
        caret: None,
    })
}

impl Harness {
    fn new() -> Self {
        Self::with(SettingsSnapshot::default(), false)
    }

    fn with(settings: SettingsSnapshot, secure: bool) -> Self {
        let engine = Arc::new(FakeEngine::default());
        let composer = Composer::new(settings.input_method, RomajiSource::builtin());
        let machine = InputStateMachine::new(
            engine.clone(),
            composer,
            keymap("macos"),
            Arc::new(settings),
            SessionInfo {
                session_id: SESSION_ID,
                client_bundle_id: "com.apple.TextEdit".into(),
                secure,
            },
            Arc::new(TickClock(Mutex::new(0))),
        );
        Self {
            engine,
            machine,
            text: CountingText {
                surrounding: surrounding(LEFT),
                selection: None,
                surrounding_reads: 0,
                selection_reads: 0,
            },
        }
    }

    fn press_with(&mut self, key: KeyKind, modifiers: Modifiers) -> Vec<ShellAction> {
        let input = KeyInput {
            key,
            modifiers,
            is_repeat: false,
            secure: false,
            layout_text: None,
        };
        self.machine.handle(&input, &mut self.text)
    }

    fn press(&mut self, key: KeyKind) -> Vec<ShellAction> {
        self.press_with(key, Modifiers::NONE)
    }

    fn type_text(&mut self, text: &str) -> Vec<ShellAction> {
        let mut last = Vec::new();
        for c in text.chars() {
            last = self.press(KeyKind::Character {
                text: c.to_string(),
            });
        }
        last
    }

    fn converting(&self) -> Option<&ConvertingState> {
        match self.machine.state() {
            InputState::Converting(state) => Some(state),
            _ => None,
        }
    }
}

fn ch(text: &str) -> KeyKind {
    KeyKind::Character { text: text.into() }
}

fn ctrl_shift() -> Modifiers {
    Modifiers {
        control: true,
        shift: true,
        ..Modifiers::NONE
    }
}

// ---------------------------------------------------------------- 合成（InputStateMachineTests の ComposingTests）

#[test]
fn typing_builds_kana_and_prefetches_only_when_the_reading_changes() {
    let mut h = Harness::new();
    let actions = h.type_text("hinsi");
    assert_eq!(
        h.machine.state(),
        &InputState::Composing {
            kana: "ひんし".into(),
            pending: String::new()
        }
    );
    assert_eq!(
        actions,
        vec![ShellAction::SetMarkedText(MarkedText {
            highlighted_length: Utf16Len(3),
            ..MarkedText::plain("ひんし")
        })]
    );
    let readings: Vec<String> = h
        .engine
        .prefetches()
        .into_iter()
        .map(|p| p.reading)
        .collect();
    assert_eq!(readings, ["ひ", "ひん", "ひんし"]);
}

#[test]
fn enter_commits_the_kana_and_records_history() {
    let mut h = Harness::new();
    h.type_text("hin");
    assert_eq!(
        h.press(KeyKind::Enter),
        vec![ShellAction::Commit {
            text: "ひん".into()
        }]
    );
    assert_eq!(
        h.engine.calls().last(),
        Some(&Call::CommitText(v1::CommitText {
            session_id: SESSION_ID,
            reading: "ひん".into(),
            surface: "ひん".into()
        }))
    );
}

#[test]
fn kana_input_uses_the_jis_key_numbers() {
    let mut h = Harness::with(
        SettingsSnapshot {
            input_method: InputMethodKind::Kana,
            ..SettingsSnapshot::default()
        },
        false,
    );
    h.press(KeyKind::Jis { code: 22 });
    h.press(KeyKind::Jis { code: 32 });
    assert_eq!(
        h.machine.state(),
        &InputState::Composing {
            kana: "ちの".into(),
            pending: String::new()
        }
    );
    let readings: Vec<String> = h
        .engine
        .prefetches()
        .into_iter()
        .map(|p| p.reading)
        .collect();
    assert_eq!(readings, ["ち", "ちの"]);
}

// ---------------------------------------------------------------- 変換（ConversionTests）

#[test]
fn space_converts_with_context_page_size_and_the_lm_budget_timeout() {
    let mut h = Harness::new();
    h.type_text("hinsi");
    h.press(KeyKind::Space);
    let (request, timeout) = h.engine.converts().remove(0);
    assert_eq!(request.reading, "ひんし");
    assert_eq!(request.session_id, SESSION_ID);
    assert_eq!(request.page_size, 9);
    let context = request.context.unwrap();
    assert_eq!(context.left, LEFT);
    assert!(context.available);
    assert_eq!(timeout, timeouts::convert_timeout(3));
}

#[test]
fn surrounding_text_is_read_in_the_early_keystrokes_and_before_convert_only() {
    let mut h = Harness::new();
    h.type_text("hinsi");
    assert_eq!(
        h.text.surrounding_reads,
        CONTEXT_REFRESH_KEYSTROKES as usize
    );
    h.press(KeyKind::Space);
    assert_eq!(
        h.text.surrounding_reads,
        CONTEXT_REFRESH_KEYSTROKES as usize + 1
    );
}

#[test]
fn prefetch_and_convert_share_the_context_revision_when_unchanged() {
    let mut h = Harness::new();
    h.type_text("hi");
    h.press(KeyKind::Space);
    let prefetch = h.engine.prefetches().remove(0);
    let (convert, _) = h.engine.converts().remove(0);
    assert_eq!(prefetch.context_rev, convert.context_rev);
    assert_eq!(prefetch.context, convert.context);
}

#[test]
fn the_context_revision_advances_when_the_text_changed_before_convert() {
    let mut h = Harness::new();
    h.type_text("hi");
    h.text.surrounding = surrounding("別の文");
    h.press(KeyKind::Space);
    let prefetch = h.engine.prefetches().remove(0);
    let (convert, _) = h.engine.converts().remove(0);
    assert_ne!(prefetch.context_rev, convert.context_rev);
    assert_eq!(convert.context.unwrap().left, "別の文");
}

#[test]
fn surrounding_text_is_cut_to_the_engine_limit_in_characters() {
    let mut h = Harness::new();
    let left = format!("{}末尾", "あ".repeat(2500));
    h.text.surrounding = Some(SurroundingText {
        left: left.clone(),
        right: "い".repeat(2500),
        document_length: Utf16Len(5002),
        caret: None,
    });
    h.type_text("hi");
    h.press(KeyKind::Space);
    let context = h.engine.converts().remove(0).0.context.unwrap();
    assert_eq!(context.left.chars().count(), 2000);
    assert!(context.left.ends_with("末尾"));
    assert_eq!(context.right.chars().count(), 2000);
}

#[test]
fn an_empty_left_with_zero_document_length_is_unavailable() {
    let mut h = Harness::new();
    h.text.surrounding = Some(SurroundingText::default());
    h.type_text("hi");
    h.press(KeyKind::Space);
    assert!(!h.engine.converts().remove(0).0.context.unwrap().available);
    // 文書の長さがあれば、左が空でも取れている（文書の先頭）
    let mut h = Harness::new();
    h.text.surrounding = Some(SurroundingText {
        document_length: Utf16Len(5),
        ..SurroundingText::default()
    });
    h.type_text("hi");
    h.press(KeyKind::Space);
    assert!(h.engine.converts().remove(0).0.context.unwrap().available);
}

#[test]
fn a_reading_longer_than_the_limit_uses_the_shell_candidates_without_asking() {
    let mut h = Harness::new();
    h.type_text(&"a".repeat(MAX_READING_CHARACTERS + 1));
    h.press(KeyKind::Space);
    assert!(h.engine.converts().is_empty());
    assert!(h.converting().unwrap().is_fallback);
}

#[test]
fn an_engine_failure_falls_back_to_the_script_candidates() {
    let mut h = Harness::new();
    *h.engine.convert.lock().unwrap() = Some(Err(EngineError::Timeout));
    h.type_text("hinsi");
    let actions = h.press(KeyKind::Space);
    let state = h.converting().unwrap();
    assert!(state.is_fallback);
    let surfaces: Vec<&str> = state
        .candidates
        .iter()
        .map(|c| c.surface.as_str())
        .collect();
    assert_eq!(surfaces, ["ひんし", "ヒンシ", "ﾋﾝｼ"]);
    assert_eq!(actions.len(), 2);
    h.press(KeyKind::Enter);
    assert!(matches!(h.engine.calls().last(), Some(Call::CommitText(_))));
}

// ---------------------------------------------------------------- 左文脈の読み直し（EarlyContextRefreshTests）

#[test]
fn a_grown_left_on_the_second_keystroke_is_sent_with_a_new_revision() {
    let mut h = Harness::new();
    h.text.surrounding = Some(SurroundingText {
        document_length: Utf16Len(1),
        ..SurroundingText::default()
    });
    h.type_text("k");
    h.text.surrounding = surrounding("確定");
    h.type_text("a");
    let requests = h.engine.prefetches();
    assert_eq!(requests.len(), 1);
    let sent = &requests[0];
    assert_eq!(sent.context.as_ref().unwrap().left, "確定");
    assert_eq!(sent.context_rev, 2);
    assert_eq!(sent.reading, "か");
}

#[test]
fn reading_stops_after_the_configured_keystrokes() {
    let mut h = Harness::new();
    h.type_text(&"a".repeat(CONTEXT_REFRESH_KEYSTROKES as usize));
    let reads = h.text.surrounding_reads;
    h.text.surrounding = surrounding("後から");
    h.type_text("aaa");
    assert_eq!(h.text.surrounding_reads, reads);
    assert_ne!(
        h.engine
            .prefetches()
            .last()
            .unwrap()
            .context
            .as_ref()
            .unwrap()
            .left,
        "後から"
    );
}

#[test]
fn the_counter_resets_on_the_next_composition() {
    let mut h = Harness::new();
    h.type_text("aaaa");
    h.press(KeyKind::Enter);
    let reads = h.text.surrounding_reads;
    h.type_text("aa");
    assert_eq!(h.text.surrounding_reads, reads + 2);
}

// ---------------------------------------------------------------- secure（NFR-18）

#[test]
fn secure_never_reads_the_text_and_passes_keys_through() {
    let mut h = Harness::with(SettingsSnapshot::default(), true);
    assert_eq!(h.type_text("aaaa"), vec![ShellAction::PassThrough]);
    assert_eq!(h.text.surrounding_reads, 0);
    assert!(h.engine.prefetches().is_empty());
}

#[test]
fn becoming_secure_while_composing_commits_without_history_and_passes_through() {
    let mut h = Harness::new();
    h.type_text("hi");
    h.machine.set_secure(true);
    let actions = h.press(ch("a"));
    assert_eq!(
        actions,
        vec![
            ShellAction::Commit { text: "ひ".into() },
            ShellAction::PassThrough
        ]
    );
    assert!(
        !h.engine
            .calls()
            .iter()
            .any(|call| matches!(call, Call::CommitText(_)))
    );
    // secure が変わったときだけ BeginSession を送り直す
    let begins = h
        .engine
        .calls()
        .iter()
        .filter(|call| matches!(call, Call::BeginSession(session) if session.secure))
        .count();
    assert_eq!(begins, 1);
    h.machine.set_secure(true);
    assert_eq!(
        h.engine
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::BeginSession(_)))
            .count(),
        1
    );
}

// ---------------------------------------------------------------- セッション（#315）

#[test]
fn activate_if_needed_begins_once_until_deactivate() {
    let mut h = Harness::new();
    h.machine.activate_if_needed();
    h.machine.activate_if_needed();
    h.machine.deactivate();
    h.machine.activate_if_needed();
    let kinds: Vec<&str> = h
        .engine
        .calls()
        .iter()
        .map(|call| match call {
            Call::BeginSession(_) => "begin",
            Call::EndSession(_) => "end",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["begin", "end", "begin"]);
}

// ---------------------------------------------------------------- 設定の反映（ReconfigureTests）

#[test]
fn reconfigure_applies_only_when_idle() {
    let mut h = Harness::new();
    h.type_text("hi");
    let settings = Arc::new(SettingsSnapshot {
        page_size: 5,
        ..SettingsSnapshot::default()
    });
    assert!(
        !h.machine
            .reconfigure(Arc::clone(&settings), keymap("ms-ime"), None)
    );
    assert_eq!(h.machine.settings().page_size, 3);
    h.press(KeyKind::Enter);
    assert!(h.machine.reconfigure(settings, keymap("ms-ime"), None));
    assert_eq!(h.machine.settings().page_size, 5);
    // MS-IME 風の Ctrl+I は全角カタカナ
    h.type_text("hi");
    let actions = h.press_with(
        ch("i"),
        Modifiers {
            control: true,
            ..Modifiers::NONE
        },
    );
    assert_eq!(
        actions.last(),
        Some(&ShellAction::SetMarkedText(MarkedText {
            highlighted_length: Utf16Len(1),
            ..MarkedText::plain("ヒ")
        }))
    );
    // 設定の反映では BeginSession を送り直さない
    assert!(
        !h.engine
            .calls()
            .iter()
            .any(|call| matches!(call, Call::BeginSession(_)))
    );
}

#[test]
fn reconfigure_swaps_the_composer_only_when_one_is_given() {
    let mut h = Harness::new();
    let kana = Arc::new(SettingsSnapshot {
        input_method: InputMethodKind::Kana,
        ..SettingsSnapshot::default()
    });
    assert!(h.machine.reconfigure(
        kana,
        keymap("macos"),
        Some(Composer::new(
            InputMethodKind::Kana,
            RomajiSource::builtin()
        ))
    ));
    assert_eq!(h.press(ch("a")), vec![ShellAction::PassThrough]);
    assert!(!h.press(KeyKind::Jis { code: 22 }).is_empty());
}

// ---------------------------------------------------------------- 再変換（ReconversionStartTests）

fn reconvert_layout() -> v1::SegmentLayout {
    v1::SegmentLayout {
        conversion_id: 21,
        layout_rev: 0,
        segments: vec![v1::SegmentInfo {
            reading: "ひんし".into(),
            surface: "品詞".into(),
        }],
        focused: 0,
        focused_page: Some(v1::SegmentPageReply {
            segment: 0,
            page_index: 0,
            candidates: candidates(&["品詞", "瀕死"]),
            has_next: false,
            ..Default::default()
        }),
        engine_micros: 0,
    }
}

#[test]
fn the_selection_is_only_read_when_the_reconvert_key_is_pressed() {
    let mut h = Harness::new();
    *h.engine.reconvert.lock().unwrap() = Some(reconvert_layout());
    h.text.selection = Some(SelectedText {
        range: Utf16Range {
            location: Utf16Len(10),
            length: Utf16Len(2),
        },
        text: "品詞".into(),
    });
    h.type_text("a");
    h.press(KeyKind::Enter);
    assert_eq!(h.text.selection_reads, 0);
    let actions = h.press_with(ch("r"), ctrl_shift());
    assert_eq!(h.text.selection_reads, 1);
    let Some(ShellAction::SetMarkedText(marked)) = actions.first() else {
        panic!("no marked text");
    };
    assert_eq!(marked.text, "品詞");
    assert_eq!(
        marked.replacement_range,
        Some(Utf16Range {
            location: Utf16Len(10),
            length: Utf16Len(2)
        })
    );
    let Some(Call::Reconvert(request)) = h.engine.calls().last().cloned() else {
        panic!("no reconvert");
    };
    assert_eq!(request.surface, "品詞");
    assert_eq!(request.page_size, 9);
}

#[test]
fn reconversion_refuses_control_only_and_empty_ranges() {
    for (text, length) in [("\u{7}", 1), ("品詞", 0)] {
        let mut h = Harness::new();
        h.text.selection = Some(SelectedText {
            range: Utf16Range {
                location: Utf16Len(0),
                length: Utf16Len(length),
            },
            text: text.into(),
        });
        assert_eq!(
            h.press_with(ch("r"), ctrl_shift()),
            vec![ShellAction::Consume]
        );
        assert!(h.engine.calls().is_empty());
    }
}

// ---------------------------------------------------------------- 診断ログ（DiagShellEventsTests）

#[test]
fn one_episode_is_sent_when_the_composition_ends() {
    let mut h = Harness::new();
    h.type_text("hinsi");
    h.press(KeyKind::Space);
    h.press(KeyKind::Space);
    h.press(KeyKind::Enter);
    let diags = h.engine.diags();
    assert_eq!(diags.len(), 1);
    let event = &diags[0];
    assert_eq!(event.kind, "sh_episode");
    assert_eq!(event.session_id, SESSION_ID);
    assert_eq!(event.tags["end"], "commit_cand");
    assert_eq!(event.tags["mode"], "romaji");
    assert_eq!(event.tags["start"], "typing");
    assert_eq!(event.tags["panel"], "none");
    assert_eq!(event.tags["ctx"], "ok");
    assert_eq!(event.tags["pick_kind"], "dict");
    assert_eq!(event.nums["keys"], 8);
    assert_eq!(event.nums["spaces"], 2);
    assert_eq!(event.nums["pick"], 1);
    assert_eq!(event.nums["keys_to_conv"], 6);
    assert_eq!(event.nums["rlen"], 3);
}

#[test]
fn the_panel_source_is_kept_from_the_first_notice() {
    let mut h = Harness::new();
    h.type_text("hi");
    h.press(KeyKind::Space);
    h.machine.note_panel_shown(AnchorSource::Accessibility);
    h.machine.note_panel_shown(AnchorSource::Client);
    h.press(KeyKind::Enter);
    assert_eq!(h.engine.diags()[0].tags["panel"], "accessibility");
}

#[test]
fn an_external_commit_carries_the_reason_and_the_previous_state() {
    let mut h = Harness::new();
    h.type_text("hi");
    h.press(KeyKind::Space);
    let actions = h.machine.commit_composition(DiagEnd::ModeSwitch);
    assert_eq!(
        actions,
        vec![
            ShellAction::Commit {
                text: "品詞".into()
            },
            ShellAction::HideCandidates
        ]
    );
    let event = &h.engine.diags()[0];
    assert_eq!(event.tags["end"], "mode_switch");
    assert_eq!(event.tags["prev"], "converting");
}

#[test]
fn escape_and_backspace_to_empty_are_separate_ends() {
    let mut h = Harness::new();
    h.type_text("a");
    h.press(KeyKind::Escape);
    h.type_text("a");
    h.press(KeyKind::Backspace);
    let ends: Vec<String> = h
        .engine
        .diags()
        .iter()
        .map(|event| event.tags["end"].clone())
        .collect();
    assert_eq!(ends, ["esc_clear", "bs_empty"]);
}

#[test]
fn secure_sessions_never_send_episodes_or_mode_switches() {
    let mut h = Harness::with(SettingsSnapshot::default(), true);
    h.type_text("a");
    h.machine
        .record_mode_switch(InputMode::Japanese, InputMode::English, true);
    assert!(h.engine.diags().is_empty());
}

#[test]
fn mode_switches_record_direction_and_route() {
    let h = Harness::new();
    h.machine
        .record_mode_switch(InputMode::Japanese, InputMode::English, true);
    h.machine
        .record_mode_switch(InputMode::English, InputMode::English, false);
    let diags = h.engine.diags();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].kind, "sh_mode");
    assert_eq!(diags[0].tags["from"], "ja");
    assert_eq!(diags[0].tags["to"], "en");
    assert_eq!(diags[0].tags["via"], "key");
}

// ---------------------------------------------------------------- 小さな部品

#[test]
fn context_text_follows_the_available_rule() {
    assert!(!ContextText::from_host(None).available);
    assert!(!ContextText::from_host(Some(SurroundingText::default())).available);
    assert!(
        ContextText::from_host(Some(SurroundingText {
            left: "a".into(),
            ..SurroundingText::default()
        }))
        .available
    );
}

#[test]
fn graphemes_are_counted_like_swift_characters() {
    assert_eq!(grapheme_count("が"), 1);
    assert_eq!(grapheme_count("か\u{3099}"), 1);
    assert_eq!(grapheme_count("👨‍👩‍👧"), 1);
    assert_eq!(drop_last_grapheme("ab👍🏽"), "ab");
    assert_eq!(suffix_graphemes("abcde", 2), "de");
    assert_eq!(prefix_graphemes("abcde", 2), "ab");
    assert_eq!(prefix_graphemes("ab", 5), "ab");
}

#[test]
fn reconvertible_selections_follow_section_9_2() {
    let selection = |text: &str| SelectedText {
        range: Utf16Range {
            location: Utf16Len(0),
            length: Utf16Len::of(text),
        },
        text: text.into(),
    };
    assert!(is_reconvertible(&selection("品詞")));
    assert!(is_reconvertible(&selection(&"あ".repeat(64))));
    assert!(!is_reconvertible(&selection(&"あ".repeat(65))));
    assert!(!is_reconvertible(&selection("品\r詞")));
    assert!(!is_reconvertible(&selection("\u{1}\u{2}")));
    assert!(!is_reconvertible(&selection("")));
}

// ---------------------------------------------------------------- 文節削除の元に戻す（ADR 0053）

fn option() -> Modifiers {
    Modifiers {
        option: true,
        ..Modifiers::NONE
    }
}

fn shift_option() -> Modifiers {
    Modifiers {
        shift: true,
        option: true,
        ..Modifiers::NONE
    }
}

fn control() -> Modifiers {
    Modifiers {
        control: true,
        ..Modifiers::NONE
    }
}

/// キャレットが `left` の終わりにある周辺テキスト
fn caret_text(left: &str, right: &str) -> Option<SurroundingText> {
    Some(SurroundingText {
        left: left.into(),
        right: right.into(),
        document_length: Utf16Len(Utf16Len::of(left).get() + Utf16Len::of(right).get()),
        caret: Some(Utf16Range {
            location: Utf16Len::of(left),
            length: Utf16Len::ZERO,
        }),
    })
}

fn layout_of(segments: &[(&str, &str)]) -> v1::SegmentLayout {
    v1::SegmentLayout {
        conversion_id: 31,
        layout_rev: 0,
        segments: segments
            .iter()
            .map(|(reading, surface)| v1::SegmentInfo {
                reading: (*reading).into(),
                surface: (*surface).into(),
            })
            .collect(),
        focused: 0,
        focused_page: None,
        engine_micros: 0,
    }
}

fn two_clauses() -> v1::SegmentLayout {
    layout_of(&[("おれは", "俺は"), ("ただしい", "正しい")])
}

fn committed_step(start: u32, left: &str, right: &str, document_length: Option<u32>) -> UndoStep {
    UndoStep::Committed {
        removed: "正しい".into(),
        start: Utf16Len(start),
        left: left.into(),
        right: right.into(),
        document_length: document_length.map(Utf16Len),
    }
}

/// 「俺は正しい」の末尾で `option+backspace`。覚えが 1 段できる
fn delete_last_clause(h: &mut Harness) {
    h.text.surrounding = caret_text("俺は正しい", "");
    *h.engine.reconvert.lock().unwrap() = Some(two_clauses());
    h.press_with(KeyKind::Backspace, option());
    assert_eq!(h.machine.clause_undo.len(), 1);
}

#[test]
fn a_committed_step_holds_only_while_the_text_is_as_it_was_right_after_the_deletion() {
    let step = committed_step(2, "俺は", "です", Some(4));
    let holds = |surrounding: Option<SurroundingText>| {
        committed_step_holds(&step, &surrounding.expect("surrounding"))
    };
    assert!(holds(caret_text("俺は", "です")));
    // 左右は重なる部分で比べる（文書の端、ホストの切り出し）
    let mut cut = caret_text("は", "で").unwrap();
    cut.caret.as_mut().unwrap().location = Utf16Len(2);
    cut.document_length = Utf16Len(4);
    assert!(holds(Some(cut)));
    // キャレットが動いた、選択になった、取れない
    assert!(!holds(caret_text("俺", "はです")));
    let mut selected = caret_text("俺は", "です").unwrap();
    selected.caret.as_mut().unwrap().length = Utf16Len(1);
    assert!(!holds(Some(selected)));
    let mut no_caret = caret_text("俺は", "です").unwrap();
    no_caret.caret = None;
    assert!(!holds(Some(no_caret)));
    // 前後の文字が違う
    assert!(!holds(caret_text("俺も", "です")));
    assert!(!holds(caret_text("俺は", "だよ")));
    // 文書の長さが違う。取れていない（0）ときは見ない
    let mut longer = caret_text("俺は", "です").unwrap();
    longer.document_length = Utf16Len(9);
    assert!(!holds(Some(longer)));
    let mut unknown = caret_text("俺は", "です").unwrap();
    unknown.document_length = Utf16Len::ZERO;
    assert!(holds(Some(unknown)));
    // 消す前の文書の長さが分からなかった段は、長さを見ない
    let blind = committed_step(2, "俺は", "です", None);
    let mut other = caret_text("俺は", "です").unwrap();
    other.document_length = Utf16Len(99);
    assert!(committed_step_holds(&blind, &other));
    // 確定後の段でなければ検査には通らない
    let composing = UndoStep::Composing {
        kana: String::new(),
        pending: String::new(),
        after: (String::new(), String::new()),
    };
    assert!(!committed_step_holds(&composing, &other));
}

#[test]
fn consecutive_deletions_are_undone_one_step_at_a_time() {
    let mut h = Harness::new();
    delete_last_clause(&mut h);
    h.text.surrounding = caret_text("俺は", "");
    *h.engine.reconvert.lock().unwrap() = Some(layout_of(&[("おれは", "俺は")]));
    assert_eq!(
        h.press_with(KeyKind::Backspace, option()),
        vec![ShellAction::DeleteText {
            range: Utf16Range {
                location: Utf16Len(0),
                length: Utf16Len(2)
            }
        }]
    );
    assert_eq!(h.machine.clause_undo.len(), 2);
    h.text.surrounding = caret_text("", "");
    assert_eq!(
        h.press_with(ch("z"), control()),
        vec![ShellAction::Commit {
            text: "俺は".into()
        }]
    );
    h.text.surrounding = caret_text("俺は", "");
    assert_eq!(
        h.press_with(ch("z"), control()),
        vec![ShellAction::Commit {
            text: "正しい".into()
        }]
    );
    assert!(h.machine.clause_undo.is_empty());
    assert_eq!(
        h.press_with(ch("z"), control()),
        vec![ShellAction::PassThrough]
    );
    // 書き戻しは学習に送らない
    assert!(
        !h.engine
            .calls()
            .iter()
            .any(|call| matches!(call, Call::CommitText(_) | Call::Commit(..)))
    );
}

#[test]
fn a_failed_check_forgets_every_step() {
    let mut h = Harness::new();
    delete_last_clause(&mut h);
    h.text.surrounding = caret_text("俺", "は");
    assert_eq!(
        h.press_with(ch("z"), control()),
        vec![ShellAction::PassThrough]
    );
    assert!(h.machine.clause_undo.is_empty());
}

#[test]
fn the_memory_is_dropped_by_other_keys_focus_secure_and_settings() {
    let mut h = Harness::new();
    delete_last_clause(&mut h);
    h.press_with(
        ch("c"),
        Modifiers {
            command: true,
            ..Modifiers::NONE
        },
    );
    assert!(h.machine.clause_undo.is_empty());
    delete_last_clause(&mut h);
    h.machine.deactivate();
    assert!(h.machine.clause_undo.is_empty());
    delete_last_clause(&mut h);
    h.machine.activate();
    assert!(h.machine.clause_undo.is_empty());
    delete_last_clause(&mut h);
    h.machine.commit_composition(DiagEnd::CommitComposition);
    assert!(h.machine.clause_undo.is_empty());
    delete_last_clause(&mut h);
    h.machine.set_secure(true);
    assert!(h.machine.clause_undo.is_empty());
    h.machine.set_secure(false);
    delete_last_clause(&mut h);
    let settings = Arc::new(SettingsSnapshot::default());
    assert!(h.machine.reconfigure(settings, keymap("macos"), None));
    assert!(h.machine.clause_undo.is_empty());
    // 削除のキーとプレビューの中の操作は、覚えを捨てない
    delete_last_clause(&mut h);
    h.text.surrounding = caret_text("俺は", "");
    *h.engine.reconvert.lock().unwrap() = Some(layout_of(&[("おれは", "俺は")]));
    h.press_with(KeyKind::Backspace, shift_option());
    h.press(KeyKind::Left);
    h.press(KeyKind::Escape);
    assert_eq!(h.machine.clause_undo.len(), 1);
}

#[test]
fn english_mode_keys_reach_the_machine_only_for_clause_deletion_and_a_remembered_undo() {
    let mut h = Harness::new();
    let reaches = |h: &Harness, key: KeyKind, modifiers: Modifiers| {
        h.machine.handles_clause_key(&KeyInput {
            key,
            modifiers,
            is_repeat: false,
            secure: false,
            layout_text: None,
        })
    };
    assert!(reaches(&h, KeyKind::Backspace, option()));
    assert!(reaches(&h, KeyKind::Backspace, shift_option()));
    assert!(!reaches(&h, KeyKind::Backspace, Modifiers::NONE));
    assert!(!reaches(&h, ch("a"), Modifiers::NONE));
    // 覚えがなければ ctrl+z は渡さない
    assert!(!reaches(&h, ch("z"), control()));
    delete_last_clause(&mut h);
    assert!(reaches(&h, ch("z"), control()));
}

#[test]
fn text_before_the_caret_in_ascii_skips_the_engine() {
    let mut h = Harness::new();
    h.text.surrounding = caret_text("今日は meeting ", "");
    *h.engine.reconvert.lock().unwrap() = Some(layout_of(&[
        ("きょうは", "今日は"),
        ("みーてぃんぐ", "meeting"),
    ]));
    assert_eq!(
        h.press_with(KeyKind::Backspace, option()),
        vec![ShellAction::PassThrough]
    );
    assert!(
        h.engine
            .calls()
            .iter()
            .all(|call| !matches!(call, Call::Reconvert(_)))
    );
    assert!(h.machine.clause_undo.is_empty());
}
