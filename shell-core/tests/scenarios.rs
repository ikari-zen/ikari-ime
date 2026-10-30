//! 共通のシナリオ（`shell-core/scenarios/*.json`）の Rust の実行器（ADR 0041 §5）。
//!
//! - `scenarios_are_well_formed`: すべてのファイルが Rust の型で読め、手順の形が正しいこと（いつも流す）。
//! - `scenarios_match_on_rust`: `Shell` に流して期待と比べる（`rust_only` を含む全件）。
//! - `record_expectations`: 期待を書くときの記録（手で流す。README）。

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use ikari_proto::v1;
use ikari_proto::v1::envelope::Body;
use ikari_shell_core::controller::BackgroundMode;
use ikari_shell_core::english::InputMode;
use ikari_shell_core::panel::PanelSide;
use ikari_shell_core::scenario::{
    self, EngineScript, HostCall, ScenarioSurrounding, ScriptLayout, ScriptPage,
    ScriptSegmentCandidates,
};
use ikari_shell_core::{
    CandidateItem, CandidateKind, Clock, EngineError, EngineErrorCode, EngineTransport,
    SelectedText, SettingsSource, Shell, ShellConfig, ShellEvent, SurroundingText, TextAccess,
    TransportError, TransportHandle,
};
use prost::Message;
use serde_json::{Map, Value, json};

// ---------------------------------------------------------------- 形の検査

#[test]
fn scenarios_are_well_formed() {
    let files = scenario::load_dir(&scenario::scenarios_dir()).expect("scenarios load");
    assert!(!files.is_empty(), "no scenario files");
    for (path, file) in &files {
        let mut names = std::collections::HashSet::new();
        for scenario in &file.scenarios {
            let at = format!("{} / {}", display(path), scenario.name);
            assert!(names.insert(&scenario.name), "{at}: duplicate name");
            assert!(!scenario.steps.is_empty(), "{at}: no steps");
            let setup = file.defaults.resolve(&scenario.setup);
            for (index, step) in scenario.steps.iter().enumerate() {
                step.events(setup.client.secure)
                    .unwrap_or_else(|error| panic!("{at} step {}: {error}", index + 1));
                step.check_engine_expectations()
                    .unwrap_or_else(|error| panic!("{at} step {}: {error}", index + 1));
            }
        }
    }
}

// ---------------------------------------------------------------- 実行

#[test]
fn scenarios_match_on_rust() {
    let files = scenario::load_dir(&scenario::scenarios_dir()).expect("scenarios load");
    let mut failures = Vec::new();
    let mut count = 0;
    for (path, file) in &files {
        for scenario in &file.scenarios {
            count += 1;
            let setup = file.defaults.resolve(&scenario.setup);
            if let Err(message) = run(&setup, scenario, None) {
                failures.push(format!("{} / {}: {message}", display(path), scenario.name));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {count} scenarios failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// 期待を書くための記録（ADR 0041 §5。今の振る舞いを JSON に固めるとき）。
///
/// `IKARI_SCENARIO_RECORD_IN` の JSON（`{"<ファイル名>": ScenarioFile}`）の全シナリオを流し、手順ごとの動作とエンジンへの送信を
/// `IKARI_SCENARIO_RECORD_OUT` に書く。記録した期待は、読んで意図どおりかを確かめる（README）。
#[test]
#[ignore = "期待を書くときだけ手で流す"]
fn record_expectations() {
    let (Ok(input), Ok(output)) = (
        std::env::var("IKARI_SCENARIO_RECORD_IN"),
        std::env::var("IKARI_SCENARIO_RECORD_OUT"),
    ) else {
        return;
    };
    let text = std::fs::read_to_string(input).expect("record input");
    let files: std::collections::BTreeMap<String, scenario::ScenarioFile> =
        serde_json::from_str(&text).expect("record input is scenario files");
    let mut recorded = Map::new();
    for (name, file) in &files {
        let mut scenarios = Map::new();
        for scenario in &file.scenarios {
            let setup = file.defaults.resolve(&scenario.setup);
            let mut steps = Vec::new();
            if let Err(message) = run(&setup, scenario, Some(&mut steps)) {
                panic!("{name} / {}: {message}", scenario.name);
            }
            scenarios.insert(scenario.name.clone(), Value::Array(steps));
        }
        recorded.insert(name.clone(), Value::Object(scenarios));
    }
    std::fs::write(output, Value::Object(recorded).to_string()).expect("record output");
}

/// 記録に残さない欄（実行器ごとに値が違う。README）
const UNRECORDED_FIELDS: [&str; 2] = ["session_id", "context_rev"];

fn run(
    setup: &scenario::ResolvedSetup,
    scenario: &scenario::Scenario,
    mut recorder: Option<&mut Vec<Value>>,
) -> Result<(), String> {
    let log = Arc::new(Mutex::new(EngineLog::new(setup.engine.clone())));
    let config = ShellConfig {
        shell_version: "scenario".into(),
        settings: SettingsSource::Inline(setup.settings_tsv()),
        resources_dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("../macos/Resources"),
    };
    let mut shell = Shell::new(
        config,
        Box::new(ScriptedTransport {
            log: Arc::clone(&log),
        }),
        Arc::new(FixedClock),
    )
    .map_err(|error| format!("Shell::new: {error}"))?;
    // エンジンへの非同期の問い合わせ（絵文字の表、Hello、郵便番号）を、その場で流す（結果の順を決める）
    shell.set_background_mode(BackgroundMode::Inline);
    let mut text = ScenarioText {
        surrounding: setup.surrounding.clone(),
        selection: None,
    };

    // 入力先を決める。ここまでの送信（hello、begin_session など）は比べない
    shell.dispatch(
        ShellEvent::Activate {
            bundle_id: setup.client.bundle_id.clone(),
            secure: false,
        },
        &mut text,
    );
    lock(&log).sent.clear();

    for (index, step) in scenario.steps.iter().enumerate() {
        let at = format!("step {}", index + 1);
        if let Some(surrounding) = &step.surrounding {
            text.surrounding = surrounding.clone();
        }
        text.selection = step.selection.clone();
        let events = step
            .events(setup.client.secure)
            .map_err(|error| format!("{at}: {error}"))?;
        let mut actions = Vec::new();
        for event in events {
            actions.extend(shell.dispatch(event, &mut text));
        }
        if let Some(call) = &step.host {
            actions.extend(host_call(&mut shell, call).map_err(|error| format!("{at}: {error}"))?);
        }
        let requested = match shell.take_mode_request() {
            Some(InputMode::Japanese) => "japanese",
            Some(InputMode::English) => "english",
            None => "none",
        };
        if let Some(recorder) = recorder.as_deref_mut() {
            let sent: Vec<Value> = std::mem::take(&mut lock(&log).sent)
                .into_iter()
                .map(|mut request| {
                    for field in UNRECORDED_FIELDS {
                        request.remove(field);
                    }
                    Value::Object(request)
                })
                .collect();
            recorder.push(json!({
                "actions": actions,
                "engine": sent,
                "mode_request": requested,
            }));
            continue;
        }
        if let Some(expected) = &step.mode_request {
            let actual = requested;
            if actual != expected {
                return Err(format!(
                    "{at}: mode request differs: expected {expected}, actual {actual}"
                ));
            }
        }
        if let Some(expected) = &step.expect
            && &actions != expected
        {
            return Err(format!(
                "{at}: actions differ\n  expected: {}\n  actual:   {}",
                to_json(expected),
                to_json(&actions)
            ));
        }
        let sent = std::mem::take(&mut lock(&log).sent);
        if let Some(expected) = &step.engine {
            let matches = expected.len() == sent.len()
                && expected
                    .iter()
                    .zip(&sent)
                    .all(|(expected, actual)| scenario::request_matches(expected, actual));
            if !matches {
                return Err(format!(
                    "{at}: engine requests differ\n  expected: {}\n  actual:   {}",
                    to_json(expected),
                    to_json(&sent)
                ));
            }
        }
    }
    Ok(())
}

/// `ShellEvent` で表せない OS とのやりとり。
fn host_call(
    shell: &mut Shell,
    call: &HostCall,
) -> Result<Vec<ikari_shell_core::ShellAction>, String> {
    Ok(match call {
        HostCall::InputModeChanged { mode_id } => shell.input_mode_changed(mode_id),
        HostCall::CommitComposition => shell.commit_composition(),
        HostCall::AbsorbModeKey => shell.absorb_mode_key(),
        HostCall::PanelShown { side } => {
            let side = match side.as_str() {
                "above" => PanelSide::Above,
                "below" => PanelSide::Below,
                other => return Err(format!("unknown panel side {other}")),
            };
            shell.panel_shown(side, None);
            Vec::new()
        }
        HostCall::PanelHidden => {
            shell.panel_hidden();
            Vec::new()
        }
    })
}

// ---------------------------------------------------------------- ホストの替え玉

struct ScenarioText {
    surrounding: ScenarioSurrounding,
    selection: Option<SelectedText>,
}

impl TextAccess for ScenarioText {
    fn surrounding(&mut self) -> Option<SurroundingText> {
        self.surrounding.to_host()
    }

    fn selection(&mut self) -> Option<SelectedText> {
        self.selection.clone()
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Duration {
        Duration::ZERO
    }
}

/// 替え玉のエンジンが受け取った要求（比べる種類だけ）と、種類ごとの回数。
struct EngineLog {
    script: EngineScript,
    sent: Vec<Map<String, Value>>,
    counts: std::collections::HashMap<String, u32>,
}

impl EngineLog {
    fn new(script: EngineScript) -> Self {
        Self {
            script,
            sent: Vec::new(),
            counts: std::collections::HashMap::new(),
        }
    }

    /// 要求を記録し、台本に失敗があればその誤りを返す。
    fn record(
        &mut self,
        mut request: Map<String, Value>,
        timeout: Option<Duration>,
    ) -> Option<EngineError> {
        let kind = request
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if let Some(timeout) = timeout {
            request.insert("timeout_ms".into(), json!(timeout.as_millis() as u64));
        }
        let nth = self.counts.entry(kind.clone()).or_insert(0);
        *nth += 1;
        let failure = self.script.failure_for(&kind, *nth);
        if scenario::ENGINE_REQUEST_TYPES.contains(&kind.as_str()) {
            self.sent.push(request);
        }
        failure
    }
}

fn lock(log: &Mutex<EngineLog>) -> MutexGuard<'_, EngineLog> {
    log.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `Envelope` を読み、台本どおりに返すトランスポート。
struct ScriptedTransport {
    log: Arc<Mutex<EngineLog>>,
}

impl EngineTransport for ScriptedTransport {
    fn connect(&self) -> Result<TransportHandle, TransportError> {
        Ok(TransportHandle(1))
    }

    fn request(
        &self,
        _handle: TransportHandle,
        bytes: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, TransportError> {
        let envelope = v1::Envelope::decode(bytes).map_err(|_| TransportError::Unavailable)?;
        let body = envelope.body.ok_or(TransportError::Unavailable)?;
        let mut log = lock(&self.log);
        let failure = log.record(describe(&body), Some(timeout));
        let reply = match failure {
            Some(EngineError::Unavailable) => return Err(TransportError::Unavailable),
            Some(EngineError::Timeout) => return Err(TransportError::Timeout),
            Some(EngineError::InvalidReply) => return Ok(vec![0xff, 0xff, 0xff]),
            Some(EngineError::Engine(code)) => error_reply(code),
            None => answer(&log.script, &body),
        };
        Ok(v1::Envelope {
            schema_version: ikari_proto::SCHEMA_VERSION,
            request_id: envelope.request_id,
            body: Some(reply),
        }
        .encode_to_vec())
    }

    fn post(&self, _handle: TransportHandle, bytes: &[u8]) {
        if let Ok(v1::Envelope {
            body: Some(body), ..
        }) = v1::Envelope::decode(bytes)
        {
            lock(&self.log).record(describe(&body), None);
        }
    }

    fn close(&self, _handle: TransportHandle) {}

    fn pid(&self, _handle: TransportHandle) -> Option<i32> {
        Some(1)
    }
}

/// 台本どおりの返事。
fn answer(script: &EngineScript, body: &Body) -> Body {
    match body {
        Body::Hello(_) => Body::HelloReply(v1::HelloReply {
            engine_version: "scenario".into(),
            dict_label: "scenario".into(),
            lm: v1::LmState::LmReady as i32,
        }),
        Body::Convert(_) => match script.pages.first() {
            Some(page) => Body::ConvertReply(convert_reply(page)),
            None => error_reply(EngineErrorCode::BadRequest),
        },
        Body::Page(request) => match script.pages.get(request.index as usize) {
            Some(page) => Body::PageReply(convert_reply(page)),
            None => error_reply(EngineErrorCode::BadRequest),
        },
        Body::SegmentPage(request) => match script.segment_page(request.segment, request.index) {
            Some(page) => Body::SegmentPageReply(segment_page_reply(
                page.segment,
                &ScriptSegmentCandidates {
                    page_index: page.page_index,
                    candidates: page.candidates.clone(),
                    has_next: page.has_next,
                },
            )),
            None => error_reply(EngineErrorCode::BadRequest),
        },
        Body::ResizeSegment(request) => {
            match script.resize(request.segment, request.delta, request.layout_rev) {
                Some(layout) => Body::SegmentLayout(layout_reply(layout)),
                None => error_reply(EngineErrorCode::BadRequest),
            }
        }
        Body::Reconvert(_) => match &script.reconvert {
            Some(layout) => Body::SegmentLayout(layout_reply(layout)),
            None => error_reply(EngineErrorCode::BadRequest),
        },
        Body::PostalLookup(request) => Body::PostalLookupReply(v1::PostalLookupReply {
            addresses: script
                .postal
                .get(&request.code)
                .cloned()
                .unwrap_or_default(),
        }),
        Body::EmojiTable(_) => Body::EmojiTableReply(v1::EmojiTableReply {
            entries: script
                .emoji_table
                .iter()
                .map(|entry| v1::EmojiEntry {
                    name: entry.name.clone(),
                    emoji: entry.emoji.clone(),
                })
                .collect(),
            data_label: if script.emoji_table.is_empty() {
                String::new()
            } else {
                "scenario".into()
            },
        }),
        _ => Body::Ack(v1::Ack {}),
    }
}

fn convert_reply(page: &ScriptPage) -> v1::ConvertReply {
    v1::ConvertReply {
        conversion_id: page.conversion_id,
        page_index: page.page_index,
        candidates: candidates(&page.candidates),
        has_next: page.has_next,
        lm_applied: page.lm_applied,
        confident: page.confident,
        layout: page.layout.as_ref().map(layout_reply),
        ..Default::default()
    }
}

fn candidates(items: &[CandidateItem]) -> Vec<v1::Candidate> {
    items
        .iter()
        .map(|candidate| v1::Candidate {
            surface: candidate.surface.clone(),
            annotation: candidate.annotation.clone(),
            kind: candidate_kind(candidate.kind) as i32,
        })
        .collect()
}

fn segment_page_reply(segment: u32, page: &ScriptSegmentCandidates) -> v1::SegmentPageReply {
    v1::SegmentPageReply {
        segment,
        page_index: page.page_index,
        candidates: candidates(&page.candidates),
        has_next: page.has_next,
        ..Default::default()
    }
}

fn layout_reply(layout: &ScriptLayout) -> v1::SegmentLayout {
    v1::SegmentLayout {
        conversion_id: layout.conversion_id,
        layout_rev: layout.layout_rev,
        segments: layout
            .segments
            .iter()
            .map(|segment| v1::SegmentInfo {
                reading: segment.reading.clone(),
                surface: segment.surface.clone(),
            })
            .collect(),
        focused: layout.focused,
        focused_page: layout
            .focused_page
            .as_ref()
            .map(|page| segment_page_reply(layout.focused, page)),
        engine_micros: 0,
    }
}

fn candidate_kind(kind: CandidateKind) -> v1::CandidateKind {
    match kind {
        CandidateKind::Unspecified => v1::CandidateKind::Unspecified,
        CandidateKind::Dictionary => v1::CandidateKind::Dictionary,
        CandidateKind::Hiragana => v1::CandidateKind::Hiragana,
        CandidateKind::Katakana => v1::CandidateKind::Katakana,
        CandidateKind::Rewriter => v1::CandidateKind::Rewriter,
        CandidateKind::Literal => v1::CandidateKind::Literal,
    }
}

fn error_reply(code: EngineErrorCode) -> Body {
    let code = match code {
        EngineErrorCode::Unspecified => v1::ErrorCode::Unspecified,
        EngineErrorCode::SchemaMismatch => v1::ErrorCode::SchemaMismatch,
        EngineErrorCode::BadRequest => v1::ErrorCode::BadRequest,
        EngineErrorCode::UnknownSession => v1::ErrorCode::UnknownSession,
        EngineErrorCode::UnknownConversion => v1::ErrorCode::UnknownConversion,
        EngineErrorCode::EngineNotReady => v1::ErrorCode::EngineNotReady,
        EngineErrorCode::Internal => v1::ErrorCode::Internal,
        EngineErrorCode::StaleLayout => v1::ErrorCode::StaleLayout,
    };
    Body::Error(v1::ErrorReply {
        code: code as i32,
        message: String::new(),
    })
}

// ---------------------------------------------------------------- 要求を JSON に写す（欄の名前は engine.proto）

fn describe(body: &Body) -> Map<String, Value> {
    let (kind, fields) = match body {
        Body::Hello(m) => ("hello", json!({ "shell_version": m.shell_version })),
        Body::BeginSession(m) => (
            "begin_session",
            json!({
                "session_id": m.session_id, "client_bundle_id": m.client_bundle_id,
                "secure": m.secure, "share_context": m.share_context,
            }),
        ),
        Body::EndSession(m) => ("end_session", json!({ "session_id": m.session_id })),
        Body::Prefetch(m) => (
            "prefetch",
            json!({
                "session_id": m.session_id, "reading": m.reading,
                "context": context(m.context.as_ref()), "context_rev": m.context_rev,
            }),
        ),
        Body::Convert(m) => (
            "convert",
            json!({
                "session_id": m.session_id, "reading": m.reading,
                "context": context(m.context.as_ref()), "context_rev": m.context_rev,
                "page_size": m.page_size,
            }),
        ),
        Body::Page(m) => (
            "page",
            json!({ "conversion_id": m.conversion_id, "index": m.index }),
        ),
        Body::Commit(m) => (
            "commit",
            json!({ "conversion_id": m.conversion_id, "candidate_index": m.candidate_index }),
        ),
        Body::CommitText(m) => (
            "commit_text",
            json!({ "session_id": m.session_id, "reading": m.reading, "surface": m.surface }),
        ),
        Body::Cancel(m) => ("cancel", json!({ "conversion_id": m.conversion_id })),
        Body::Shutdown(m) => {
            let reason = match v1::ShutdownReason::try_from(m.reason) {
                Ok(v1::ShutdownReason::VersionMismatch) => "version_mismatch",
                Ok(v1::ShutdownReason::SettingsChanged) => "settings_changed",
                _ => "unspecified",
            };
            ("shutdown", json!({ "reason": reason }))
        }
        Body::SegmentPage(m) => (
            "segment_page",
            json!({
                "conversion_id": m.conversion_id, "layout_rev": m.layout_rev,
                "segment": m.segment, "index": m.index, "left": choices(&m.left),
            }),
        ),
        Body::ResizeSegment(m) => (
            "resize_segment",
            json!({
                "conversion_id": m.conversion_id, "layout_rev": m.layout_rev,
                "segment": m.segment, "delta": m.delta, "left": choices(&m.left),
            }),
        ),
        Body::CommitSegments(m) => (
            "commit_segments",
            json!({
                "conversion_id": m.conversion_id, "layout_rev": m.layout_rev,
                "choices": choices(&m.choices),
            }),
        ),
        Body::Reconvert(m) => (
            "reconvert",
            json!({
                "session_id": m.session_id, "surface": m.surface,
                "context": context(m.context.as_ref()), "page_size": m.page_size,
            }),
        ),
        Body::PostalLookup(m) => ("postal_lookup", json!({ "code": m.code })),
        Body::Diag(_) => ("diag", json!({})),
        Body::EmojiTable(_) => ("emoji_table", json!({})),
        // 管理の要求（設定アプリからの中継）
        _ => ("relay", json!({})),
    };
    let mut map = Map::new();
    map.insert("type".into(), json!(kind));
    if let Value::Object(fields) = fields {
        map.extend(fields);
    }
    map
}

fn context(context: Option<&v1::SurroundingText>) -> Value {
    let context = context.cloned().unwrap_or_default();
    json!({ "left": context.left, "right": context.right, "available": context.available })
}

fn choices(choices: &[v1::SegmentChoice]) -> Value {
    Value::Array(
        choices
            .iter()
            .map(|choice| {
                if choice.literal.is_empty() {
                    json!({ "candidate_index": choice.candidate_index })
                } else {
                    json!({ "literal": choice.literal })
                }
            })
            .collect(),
    )
}

// ---------------------------------------------------------------- 補助

fn to_json<T: serde::Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|error| format!("<{error}>"))
}

fn display(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}
