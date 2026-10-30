//! 共通のシナリオ（JSON）の形（ADR 0041 §5）。**契約**。説明は `shell-core/scenarios/README.md`。
//!
//! Rust の実行器（`shell-core/tests/scenarios.rs`）、Swift の実行器（`macos/Tests/IkariShellCoreTests/ScenarioRunnerTests.swift`。
//! 移行の間だけ）、性能の計測（`cargo xtask perf shell`。L-P）が同じファイルを読む。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::types::{
    CandidateItem, EngineError, KeyInput, KeyKind, Modifiers, SelectedText, ShellAction,
    ShellEvent, SurroundingText, Utf16Len,
};

/// 1 つの JSON ファイル。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioFile {
    #[serde(default)]
    pub description: String,
    /// ファイルの中の全シナリオの準備の既定。シナリオの `setup` が欄ごとに上書きする
    #[serde(default)]
    pub defaults: Setup,
    pub scenarios: Vec<Scenario>,
}

/// 準備（設定、入力先、替え玉のエンジンの台本、周辺テキスト）。省いた欄は既定（`Setup::resolve`）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Setup {
    /// `settings.tsv` の行（`key<TAB>value`）。省けば空のファイル（既定値）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<Client>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<EngineScript>,
    /// 最初の周辺テキスト。省けば取れない
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surrounding: Option<ScenarioSurrounding>,
}

/// 既定を埋めた準備。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSetup {
    pub settings: Vec<String>,
    pub client: Client,
    pub engine: EngineScript,
    pub surrounding: ScenarioSurrounding,
}

impl Setup {
    /// `self`（ファイルの既定）に `scenario`（シナリオの準備）を欄ごとに重ね、残りを既定で埋める。
    pub fn resolve(&self, scenario: &Setup) -> ResolvedSetup {
        ResolvedSetup {
            settings: scenario
                .settings
                .clone()
                .or_else(|| self.settings.clone())
                .unwrap_or_default(),
            client: scenario
                .client
                .clone()
                .or_else(|| self.client.clone())
                .unwrap_or_default(),
            engine: scenario
                .engine
                .clone()
                .or_else(|| self.engine.clone())
                .unwrap_or_default(),
            surrounding: scenario
                .surrounding
                .clone()
                .or_else(|| self.surrounding.clone())
                .unwrap_or_else(ScenarioSurrounding::unavailable),
        }
    }
}

impl ResolvedSetup {
    /// `settings.tsv` の文字列（行を改行でつなぐ）。
    pub fn settings_tsv(&self) -> String {
        let mut text = self.settings.join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        text
    }
}

/// 入力先のアプリ。実行器は最初に `ShellEvent::Activate` を送り、そこまでのエンジンへの送信は比べない。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Client {
    #[serde(default = "default_bundle_id")]
    pub bundle_id: String,
    /// secure な入力。`press`・`type_text` のキーの `KeyInput::secure` に入る
    #[serde(default)]
    pub secure: bool,
}

impl Default for Client {
    fn default() -> Self {
        Self {
            bundle_id: default_bundle_id(),
            secure: false,
        }
    }
}

fn default_bundle_id() -> String {
    "com.apple.TextEdit".into()
}

/// 周辺テキスト（シナリオの書き方）。`available: false` は「ホストが取れなかった」（`TextAccess::surrounding` が `None`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioSurrounding {
    #[serde(default)]
    pub left: String,
    #[serde(default)]
    pub right: String,
    /// 文書の長さ。省けば `left` と `right` の長さの和
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_length: Option<Utf16Len>,
    #[serde(default = "default_true")]
    pub available: bool,
}

impl ScenarioSurrounding {
    pub fn unavailable() -> Self {
        Self {
            left: String::new(),
            right: String::new(),
            document_length: None,
            available: false,
        }
    }

    /// ホストの口が返す値。
    pub fn to_host(&self) -> Option<SurroundingText> {
        if !self.available {
            return None;
        }
        let document_length = self.document_length.unwrap_or_else(|| {
            Utf16Len(
                Utf16Len::of(&self.left)
                    .get()
                    .saturating_add(Utf16Len::of(&self.right).get()),
            )
        });
        Some(SurroundingText {
            left: self.left.clone(),
            right: self.right.clone(),
            document_length,
        })
    }
}

/// 替え玉のエンジンの台本。
///
/// - `convert` は `pages[0]`、`page`（index = i）は `pages[i]` を返す。ないときは `engine: bad_request`。
/// - `segment_page` は `segment_pages` の中で `segment` と `page_index`（要求の `index`）が合うもの、
///   `resize_segment` は `resizes` の中で `segment` と `delta`（と、書いてあれば `layout_rev`）が合うもの、
///   `reconvert` は `reconvert` を返す。ないときは `engine: bad_request`。
/// - `postal_lookup` は `postal` の番号の住所（なければ空）。`emoji_table` は `emoji_table`。`hello` は成功。
/// - `failures` に書いた要求は、その回だけ失敗させる。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineScript {
    #[serde(default)]
    pub pages: Vec<ScriptPage>,
    #[serde(default)]
    pub failures: Vec<ScriptFailure>,
    /// 文節の候補のページ（L-A2 で足した）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segment_pages: Vec<ScriptSegmentPage>,
    /// 区切り直しの返事（L-A2 で足した）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resizes: Vec<ScriptResize>,
    /// 再変換の返事（L-A2 で足した）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconvert: Option<ScriptLayout>,
    /// 絵文字のショートコードの表（名前の昇順。L-A2 で足した）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emoji_table: Vec<ScriptEmoji>,
    /// 郵便番号 → 住所（L-A2 で足した）
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub postal: std::collections::BTreeMap<String, Vec<String>>,
}

/// 1 ページ分の返事（`ConvertReply`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptPage {
    pub conversion_id: u64,
    pub page_index: u32,
    pub candidates: Vec<CandidateItem>,
    pub has_next: bool,
    #[serde(default)]
    pub lm_applied: bool,
    #[serde(default)]
    pub confident: bool,
    /// 区切り（`ConvertReply.layout`）。`convert` の返事だけに付ける（L-A2 で足した）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<ScriptLayout>,
}

/// 区切り（`SegmentLayout`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptLayout {
    pub conversion_id: u64,
    #[serde(default)]
    pub layout_rev: u32,
    pub segments: Vec<ScriptSegment>,
    #[serde(default)]
    pub focused: u32,
    /// フォーカスしている文節の 1 ページ目（`segment` は `focused`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused_page: Option<ScriptSegmentCandidates>,
}

/// 文節 1 つ（`SegmentInfo`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptSegment {
    pub reading: String,
    pub surface: String,
}

/// 文節の候補の 1 ページの中身（`SegmentPageReply` の `segment` 以外）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptSegmentCandidates {
    #[serde(default)]
    pub page_index: u32,
    pub candidates: Vec<CandidateItem>,
    #[serde(default)]
    pub has_next: bool,
}

/// `segment_page` の返事。`segment` と `page_index` で引く。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptSegmentPage {
    pub segment: u32,
    #[serde(default)]
    pub page_index: u32,
    pub candidates: Vec<CandidateItem>,
    #[serde(default)]
    pub has_next: bool,
}

/// `resize_segment` の返事。`segment` と `delta`（と、書いてあれば `layout_rev`）で引く。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptResize {
    pub segment: u32,
    pub delta: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_rev: Option<u32>,
    pub layout: ScriptLayout,
}

/// 絵文字の表の 1 行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptEmoji {
    pub name: String,
    pub emoji: String,
}

impl EngineScript {
    pub fn segment_page(&self, segment: u32, index: u32) -> Option<&ScriptSegmentPage> {
        self.segment_pages
            .iter()
            .find(|page| page.segment == segment && page.page_index == index)
    }

    pub fn resize(&self, segment: u32, delta: i32, layout_rev: u32) -> Option<&ScriptLayout> {
        self.resizes
            .iter()
            .find(|resize| {
                resize.segment == segment
                    && resize.delta == delta
                    && resize.layout_rev.is_none_or(|rev| rev == layout_rev)
            })
            .map(|resize| &resize.layout)
    }
}

/// `request`（要求の種類。`engine.proto` の `Envelope.body` の欄の名前）の `nth` 回目（1 から）を `error` で失敗させる。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptFailure {
    pub request: String,
    pub nth: u32,
    pub error: EngineError,
}

impl EngineScript {
    /// `request` の `nth` 回目（1 から）を失敗させるなら、その誤り。
    pub fn failure_for(&self, request: &str, nth: u32) -> Option<EngineError> {
        self.failures
            .iter()
            .find(|failure| failure.request == request && failure.nth == nth)
            .map(|failure| failure.error)
    }
}

/// 1 つのシナリオ。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// ファイルの中で一意
    pub name: String,
    /// 移した元のテスト（`ファイル名 型名.関数名`）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub from: String,
    /// Rust の実行器だけで流す理由（殻の判断のうち、Swift では IkariIME（AppKit）にあってテストから呼べないもの。
    /// Swift の実行器は飛ばして数える。L-A2 で足した）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rust_only: Option<String>,
    #[serde(default)]
    pub setup: Setup,
    pub steps: Vec<Step>,
}

/// 手順の 1 つ。`event`、`press`、`type_text`、`host` のどれか 1 つを書く。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// 人のための注釈（比べない）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<ShellEvent>,
    /// キー 1 つ（`parse_press` の書式。例: `space`、`shift+space`、`cmd+c`、`ctrl+j`、`f7`、`jis:22`、`k`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub press: Option<String>,
    /// 文字ごとに修飾なしの `character` のキーを打つ
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_text: Option<String>,
    /// `ShellEvent` で表せない OS とのやりとり（`Shell` の公開の関数。L-A2 で足した）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<HostCall>,
    /// この手順のあとに `Shell::take_mode_request` が返すモード（`japanese`、`english`、`none`。L-A2 で足した）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode_request: Option<String>,
    /// この手順から先の周辺テキスト（次に書くまで続く）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surrounding: Option<ScenarioSurrounding>,
    /// この手順だけの選択範囲（省けば選択なし）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<SelectedText>,
    /// この手順の全イベントが返した動作をつないだ列。順序つきで完全一致。省けば比べない
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<Vec<ShellAction>>,
    /// この手順でエンジンへ送った要求の列（`type` と、書いた欄だけを比べる）。`[]` は「何も送らない」。省けば比べない
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<Vec<Map<String, Value>>>,
}

/// `ShellEvent` で表せない OS とのやりとり（`Shell` の公開の関数。ADR 0041 §4.1 に足す候補）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostCall {
    /// `Shell::input_mode_changed`（macOS の `setValue(_:forTag:)`）
    InputModeChanged { mode_id: String },
    /// `Shell::commit_composition`（IMK の `commitComposition`）
    CommitComposition,
    /// `Shell::absorb_mode_key`（JIS の英数・かなキー）
    AbsorbModeKey,
    /// `Shell::panel_shown`（候補ウィンドウの上下。`above` か `below`）
    PanelShown { side: String },
    /// `Shell::panel_hidden`
    PanelHidden,
}

/// シナリオの誤り（形の誤り）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioError(pub String);

impl std::fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ScenarioError {}

/// 比べるエンジンへの要求の種類（`Envelope.body` の欄の名前）。`diag` と `emoji_table` は比べない（非同期・片道で順が決まらない）。
pub const ENGINE_REQUEST_TYPES: &[&str] = &[
    "hello",
    "begin_session",
    "end_session",
    "prefetch",
    "convert",
    "page",
    "commit",
    "commit_text",
    "cancel",
    "shutdown",
    "segment_page",
    "resize_segment",
    "commit_segments",
    "reconvert",
    "postal_lookup",
    "relay",
];

impl Step {
    /// この手順で送るイベントの列。`secure` は `press`・`type_text` のキーに入れる。`host` の手順は空。
    pub fn events(&self, secure: bool) -> Result<Vec<ShellEvent>, ScenarioError> {
        let given = [
            self.event.is_some(),
            self.press.is_some(),
            self.type_text.is_some(),
            self.host.is_some(),
        ];
        if given.iter().filter(|set| **set).count() != 1 {
            return Err(ScenarioError(
                "a step needs exactly one of event, press, type_text, host".into(),
            ));
        }
        if self.host.is_some() {
            return Ok(Vec::new());
        }
        if let Some(event) = &self.event {
            return Ok(vec![event.clone()]);
        }
        if let Some(spec) = &self.press {
            let mut input = parse_press(spec)?;
            input.secure = secure;
            return Ok(vec![ShellEvent::Key(input)]);
        }
        let text = self.type_text.as_deref().unwrap_or_default();
        if text.is_empty() {
            return Err(ScenarioError("type_text is empty".into()));
        }
        Ok(text
            .chars()
            .map(|character| {
                ShellEvent::Key(KeyInput {
                    key: KeyKind::Character {
                        text: character.to_string(),
                    },
                    modifiers: Modifiers::NONE,
                    is_repeat: false,
                    secure,
                })
            })
            .collect())
    }

    /// `engine` の各要素に、既知の `type` があるか。
    pub fn check_engine_expectations(&self) -> Result<(), ScenarioError> {
        for request in self.engine.iter().flatten() {
            match request.get("type").and_then(Value::as_str) {
                Some(kind) if ENGINE_REQUEST_TYPES.contains(&kind) => {}
                other => {
                    return Err(ScenarioError(format!(
                        "unknown engine request type: {other:?}"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// `press` の書式を読む。`修飾+…+キー`。
///
/// - 修飾: `shift`、`ctrl`（`control`）、`option`（`alt`）、`cmd`（`command`）
/// - キー: `space`、`enter`、`escape`、`backspace`、`left`、`right`、`up`、`down`、`other`、`f1`〜`f20`、
///   `jis:<番号>`、それ以外は 1 文字の `character`（`+` そのものは `+` か `shift++` のように最後に書く）
pub fn parse_press(spec: &str) -> Result<KeyInput, ScenarioError> {
    let (prefix, key) = match spec.strip_suffix("++") {
        Some(rest) => (Some(rest), "+"),
        None if spec == "+" => (None, "+"),
        None => match spec.rsplit_once('+') {
            Some((rest, key)) => (Some(rest), key),
            None => (None, spec),
        },
    };
    let mut modifiers = Modifiers::NONE;
    for name in prefix.into_iter().flat_map(|rest| rest.split('+')) {
        match name {
            "shift" => modifiers.shift = true,
            "ctrl" | "control" => modifiers.control = true,
            "option" | "alt" => modifiers.option = true,
            "cmd" | "command" => modifiers.command = true,
            _ => {
                return Err(ScenarioError(format!(
                    "unknown modifier `{name}` in `{spec}`"
                )));
            }
        }
    }
    let key = match key {
        "space" => KeyKind::Space,
        "enter" => KeyKind::Enter,
        "escape" => KeyKind::Escape,
        "backspace" => KeyKind::Backspace,
        "left" => KeyKind::Left,
        "right" => KeyKind::Right,
        "up" => KeyKind::Up,
        "down" => KeyKind::Down,
        "other" => KeyKind::Other,
        _ => {
            if let Some(code) = key.strip_prefix("jis:") {
                let code = code
                    .parse()
                    .map_err(|_| ScenarioError(format!("bad jis key in `{spec}`")))?;
                KeyKind::Jis { code }
            } else if let Some(number) = function_key_number(key) {
                KeyKind::Function { number }
            } else if key.chars().count() == 1 {
                KeyKind::Character { text: key.into() }
            } else {
                return Err(ScenarioError(format!("unknown key `{key}` in `{spec}`")));
            }
        }
    };
    Ok(KeyInput {
        key,
        modifiers,
        is_repeat: false,
        secure: false,
    })
}

/// `f1`〜`f20` の番号。
fn function_key_number(key: &str) -> Option<u8> {
    const MAX_FUNCTION_KEY: u8 = 20;
    let number: u8 = key.strip_prefix('f')?.parse().ok()?;
    (1..=MAX_FUNCTION_KEY).contains(&number).then_some(number)
}

/// 期待の要求 `expected` の欄（`type` を含む）が、すべて実際の要求 `actual` と等しいか。
/// 入れ子の object は、書いた欄だけを比べる。配列は要素ごとに同じ規則で比べ、長さは一致させる。
pub fn request_matches(expected: &Map<String, Value>, actual: &Map<String, Value>) -> bool {
    expected.iter().all(|(key, value)| {
        actual
            .get(key)
            .is_some_and(|actual| value_matches(value, actual))
    })
}

fn value_matches(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => request_matches(expected, actual),
        (Value::Array(expected), Value::Array(actual)) => {
            expected.len() == actual.len()
                && expected
                    .iter()
                    .zip(actual)
                    .all(|(expected, actual)| value_matches(expected, actual))
        }
        (Value::Number(expected), Value::Number(actual)) => expected.as_f64() == actual.as_f64(),
        _ => expected == actual,
    }
}

/// 置き場所（`shell-core/scenarios`）。
pub fn scenarios_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios")
}

/// `dir` の `*.json` を名前の順に読む。
pub fn load_dir(dir: &Path) -> Result<Vec<(PathBuf, ScenarioFile)>, ScenarioError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| ScenarioError(format!("{}: {error}", dir.display())))?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path)
                .map_err(|error| ScenarioError(format!("{}: {error}", path.display())))?;
            let file = serde_json::from_str(&text)
                .map_err(|error| ScenarioError(format!("{}: {error}", path.display())))?;
            Ok((path, file))
        })
        .collect()
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn press_reads_modifiers_and_named_keys() {
        let input = parse_press("shift+space").unwrap();
        assert_eq!(input.key, KeyKind::Space);
        assert!(input.modifiers.shift);
        assert_eq!(
            parse_press("cmd+c").unwrap().key,
            KeyKind::Character { text: "c".into() }
        );
        assert_eq!(
            parse_press("ctrl+shift+@").unwrap().modifiers,
            Modifiers {
                shift: true,
                control: true,
                ..Modifiers::NONE
            }
        );
        assert_eq!(
            parse_press("f7").unwrap().key,
            KeyKind::Function { number: 7 }
        );
        assert_eq!(
            parse_press("jis:22").unwrap().key,
            KeyKind::Jis { code: 22 }
        );
        assert_eq!(
            parse_press("+").unwrap().key,
            KeyKind::Character { text: "+".into() }
        );
        assert_eq!(
            parse_press("shift++").unwrap().key,
            KeyKind::Character { text: "+".into() }
        );
        assert!(parse_press("hyper+a").is_err());
        assert!(parse_press("f21").is_err());
        assert!(parse_press("ab").is_err());
    }

    #[test]
    fn requests_match_on_the_written_fields_only() {
        let expected: Map<String, Value> = serde_json::from_str(
            r#"{"type":"convert","reading":"ひ","context":{"available":true}}"#,
        )
        .unwrap();
        let actual: Map<String, Value> = serde_json::from_str(
            r#"{"type":"convert","reading":"ひ","session_id":3,"context":{"left":"x","right":"","available":true}}"#,
        )
        .unwrap();
        assert!(request_matches(&expected, &actual));
        let other: Map<String, Value> =
            serde_json::from_str(r#"{"type":"convert","reading":"ひん"}"#).unwrap();
        assert!(!request_matches(&expected, &other));
    }
}
