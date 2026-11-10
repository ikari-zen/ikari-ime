//! 公開の型（ADR 0041 §4.1・§4.2・§7.3 M1-a）。**契約**。レーンは変えない（足りなければリードに戻す）。
//!
//! - 文字列は UTF-8。位置と長さは UTF-16 の単位（`Utf16Len`、`Utf16Range`）。`String::len()` を位置に使わない。
//! - serde の形は、シナリオの JSON（`shell-core/scenarios/README.md`）と同じ。列挙は `type`（動作とイベント）か
//!   `kind`（キー）の欄で種類を表し、名前は snake_case。既定値の欄は JSON で省いてよい。
//! - proto（`proto/ikari/shell/v1/shell.proto`）は同じ型を FFI のために写したもの。写しは `shell-ffi` が持つ（M1-b）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- UTF-16

/// UTF-16 の単位の長さ（または文書の先頭からの位置）。`NSRange` と TSF の ACP に合わせる。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Utf16Len(pub u32);

impl Utf16Len {
    pub const ZERO: Self = Self(0);

    /// `text` の UTF-16 の長さ。`u32` を超える長さは `u32::MAX` に丸める。
    pub fn of(text: &str) -> Self {
        Self(u32::try_from(text.encode_utf16().count()).unwrap_or(u32::MAX))
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

/// 文書の中の範囲（UTF-16 の単位。`NSRange` と同じ）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Utf16Range {
    pub location: Utf16Len,
    pub length: Utf16Len,
}

// ---------------------------------------------------------------- キー

/// 修飾キー。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Modifiers {
    #[serde(skip_serializing_if = "is_false")]
    pub shift: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub control: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub option: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub command: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        shift: false,
        control: false,
        option: false,
        command: false,
    };

    pub fn is_empty(&self) -> bool {
        *self == Self::NONE
    }
}

/// キーの種類（Swift の `KeyInput.Key`）。OS の keyCode は OS の入口が写す。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KeyKind {
    /// 文字を生むキー（ローマ字入力）。`text` は修飾キーを適用したあとの文字
    Character {
        text: String,
    },
    /// かな入力用の物理キー（JIS 配列の番号。`ikari_input::JisKey`）
    Jis {
        code: u8,
    },
    Space,
    Enter,
    Escape,
    Backspace,
    Left,
    Right,
    Up,
    Down,
    /// F1〜F20
    Function {
        number: u8,
    },
    Other,
    /// JIS の英数キー（L-E で追加）。OS がモードを切り替えたあと同じキーが届くので、受け止めるだけにする
    Eisu,
    /// JIS のかなキー（L-E で追加）
    Kana,
}

/// 1 回のキーの入力。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyInput {
    pub key: KeyKind,
    #[serde(default, skip_serializing_if = "Modifiers::is_empty")]
    pub modifiers: Modifiers,
    /// キーの自動の繰り返し
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_repeat: bool,
    /// secure な入力（パスワード欄。macOS は `IsSecureEventInputEnabled()`）
    #[serde(default, skip_serializing_if = "is_false")]
    pub secure: bool,
    /// `KeyKind::Jis` のときだけ: OS の配列がそのキーで出す文字（Shift だけを適用。ないキーは空）。L-E で追加。
    /// かな入力でないとき、英語のモード、Ctrl / Option / Command つきのとき、`shell-core` が文字のキーとして読む
    /// （`None` なら、JIS の刻印から読む）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_text: Option<String>,
}

impl KeyInput {
    /// 修飾なし、繰り返しなし、secure でないキー。
    pub fn new(key: KeyKind) -> Self {
        Self {
            key,
            modifiers: Modifiers::NONE,
            is_repeat: false,
            secure: false,
            layout_text: None,
        }
    }
}

// ---------------------------------------------------------------- ホストから受け取るテキスト

/// 周辺テキストの生の値（ホストの口 `TextAccess::surrounding` の返事）。
/// 「取れた」と言ってよいか（左が空で文書の長さも 0 なら取れていない。今の `ClientAccess.isAvailable`）と、
/// エンジンの上限（2000 文字）への切り詰めは `shell-core` が決める。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurroundingText {
    /// キャレット（または変換中の範囲）の左
    #[serde(default)]
    pub left: String,
    /// 右
    #[serde(default)]
    pub right: String,
    /// 文書全体の長さ（ホストが返した値。返さないアプリは 0）
    #[serde(default)]
    pub document_length: Utf16Len,
    /// キャレット（選択）の範囲（UTF-16。`client.selectedRange()`）。取れなければ `None`（ADR 0050 §5.2）。
    /// 文脈（Prefetch・Convert の `SurroundingText`）には送らない
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caret: Option<Utf16Range>,
}

/// 選択範囲とその文字列（再変換の対象。ADR 0006 §9.2）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedText {
    pub range: Utf16Range,
    pub text: String,
}

// ---------------------------------------------------------------- 表示

/// マーク中のテキスト（下線つきの未確定文字列）。Swift の `MarkedText`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkedText {
    pub text: String,
    /// 合成中は `kana` の部分だけを強調する範囲。`segments` が空でないときは使わない
    #[serde(default, skip_serializing_if = "is_zero_len")]
    pub highlighted_length: Utf16Len,
    /// 文節ごとの長さ（先頭から順。合計は `text` の長さ）。空なら文節に分けない
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<Utf16Len>,
    /// フォーカスしている文節の番号。`segments` が空なら `None`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused: Option<u32>,
    /// 再変換を始めるときだけ入れる。置き換える範囲（ADR 0006 §9.5）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement_range: Option<Utf16Range>,
    /// 下線を付けるか。英語のモードのショートコードを打っている途中だけ false（ADR 0027 §9.5）
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub underlined: bool,
    /// フォーカスしている文節（`focused`）を削除の対象として見せる（ADR 0050 §3.4）。既定 false
    #[serde(default, skip_serializing_if = "is_false")]
    pub delete_target: bool,
}

impl MarkedText {
    /// 下線つき、強調なしの文字列。
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            highlighted_length: Utf16Len::ZERO,
            segments: Vec::new(),
            focused: None,
            replacement_range: None,
            underlined: true,
            delete_target: false,
        }
    }
}

/// 候補の種類（`engine.proto` の `CandidateKind`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    Unspecified,
    #[default]
    Dictionary,
    Hiragana,
    Katakana,
    Rewriter,
    /// 再変換で読めなかった部分の、元の表記だけの候補（ADR 0006 §9.3）
    Literal,
}

/// 候補 1 件。
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateItem {
    pub surface: String,
    /// 2 行目に出す注釈
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub annotation: String,
    #[serde(default, skip_serializing_if = "is_dictionary")]
    pub kind: CandidateKind,
    /// 行の右に薄く出す名前（ショートコードの `:eyes:`。ADR 0027 §3.4）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

impl Clone for CandidateItem {
    fn clone(&self) -> Self {
        Self {
            surface: self.surface.clone(),
            annotation: self.annotation.clone(),
            kind: self.kind,
            label: self.label.clone(),
        }
    }

    /// 複製先の文字列の領域を使い回す（`Vec<CandidateItem>::clone_from` が要素ごとに呼ぶ。確保を減らす）
    fn clone_from(&mut self, source: &Self) {
        self.surface.clone_from(&source.surface);
        self.annotation.clone_from(&source.annotation);
        self.kind = source.kind;
        self.label.clone_from(&source.label);
    }
}

/// 候補ウィンドウに出す内容（縦一列）。Swift の `CandidatePanelModel`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidatePanelModel {
    /// 表示する候補（番号は 1 から振る）
    pub candidates: Vec<CandidateItem>,
    pub selected: u32,
    /// 注釈。空なら行を出さない
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub annotation: String,
    /// 位置を決めるときに見込む行の数（ADR 0014 §2.2）。`candidates` の数以上
    pub reserved_rows: u32,
}

// ---------------------------------------------------------------- メニュー

/// キー割り当ての既定の組（`keymap.preset`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum KeymapPreset {
    #[default]
    #[serde(rename = "macos")]
    Macos,
    #[serde(rename = "ms-ime")]
    MsIme,
    #[serde(rename = "atok")]
    Atok,
}

/// 入力方式（`input.method`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMethodKind {
    #[default]
    Romaji,
    Kana,
}

/// 設定アプリのタブ。`None`（JSON では省略）は前回のタブ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsTab {
    Input,
    Conversion,
    /// ブリッジの `OpenTab.tab` では `dictionary`
    #[serde(rename = "dictionary")]
    UserDictionary,
    Learning,
    About,
}

/// 入力メニューの項目を選んだときの動作。Swift の `MenuCommand`。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MenuCommand {
    SelectKeymapPreset {
        preset: KeymapPreset,
    },
    SelectInputMethod {
        method: InputMethodKind,
    },
    /// LM のオン・オフ（`engine.lm`）
    SetLm {
        enabled: bool,
    },
    /// 分野を含める（`often`）・外す
    SetProfileDomain {
        id: String,
        included: bool,
    },
    OpenSettings {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<SettingsTab>,
    },
}

/// 排他の選択の組。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MenuRadioGroup {
    KeymapPreset,
    InputMethod,
}

/// メニューの項目の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MenuItemKind {
    Separator,
    /// チェックのない項目
    Action,
    /// 独立して入り切りする項目
    Toggle,
    /// 排他の選択の 1 つ
    Radio {
        group: MenuRadioGroup,
    },
}

/// メニューの項目。Swift の `MenuItemSpec`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuItem {
    pub kind: MenuItemKind,
    #[serde(default)]
    pub title: String,
    /// `Separator` では `None`、それ以外では必ずある
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<MenuCommand>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub checked: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

/// 入力メニューの中身（平らな列。ADR 0005 §5.2）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuModel {
    pub items: Vec<MenuItem>,
}

// ---------------------------------------------------------------- 識別子

/// タイマーの番号。`shell-core` が `ScheduleTimer` で付け、OS 側は時間が来たら `Timer` のイベントで返す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TimerId(pub u32);

/// 設定アプリとの接続（Bridge）の番号。OS 側が付ける。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConnId(pub u64);

// ---------------------------------------------------------------- イベントと動作

/// OS のイベント（ADR 0041 §4.1）。`Shell::dispatch` の入力。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShellEvent {
    Key(KeyInput),
    /// 入力先のアプリが替わった（IMK の `activateServer`）
    Activate {
        /// 入力先のアプリの bundle ID（Windows は実行ファイルの名前など）
        bundle_id: String,
        /// secure な入力（パスワード欄）。L-E で追加
        #[serde(default, skip_serializing_if = "is_false")]
        secure: bool,
    },
    Deactivate,
    MenuCommand {
        command: MenuCommand,
    },
    /// 設定のファイルが変わったかもしれない（読み直すかは `shell-core` が決める）
    SettingsFileChanged,
    /// `ScheduleTimer` で頼んだ時間が来た
    Timer {
        id: TimerId,
    },
    /// 設定アプリから枠（`FrameCodec` の 1 枠のバイト列）が届いた
    BridgeFrame {
        conn: ConnId,
        frame: Vec<u8>,
    },
    // ---- L-E で追加（M1-a の口で表せなかった OS とのやりとり。`Shell` の同名の公開の関数と同じ）
    /// OS が入力モードを替えた（macOS の `kTextServiceInputModePropertyTag`。値はモードの ID）
    InputModeChanged {
        mode_id: String,
        #[serde(default, skip_serializing_if = "is_false")]
        secure: bool,
    },
    /// 外から確定させられた（IMK の `commitComposition`。クリックなど）
    CommitComposition,
    /// 候補ウィンドウを出した・動かした（OS が位置を決めたあと）。`source` は新しくアンカーを取ったときだけ
    PanelShown {
        side: crate::panel::PanelSide,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<crate::panel::AnchorSource>,
    },
    /// 候補ウィンドウを閉じた（`HideCandidates` は `shell-core` が自分で数えるので、OS 側の都合で閉じたときだけ）
    PanelHidden,
    /// 設定アプリとの接続が切れた
    BridgeClosed {
        conn: ConnId,
    },
}

/// `shell-core` が返す動作（ADR 0041 §4.1）。OS 側は順に実行するだけにする。
/// 戻り値の「キーを食べたか」は、列に `PassThrough` を含むかで決める。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShellAction {
    /// このキーを IME が処理しない（アプリにそのまま渡す）
    PassThrough,
    /// 処理した（表示を変えない）
    Consume,
    SetMarkedText(MarkedText),
    /// テキストを確定してマークを消す
    Commit {
        text: String,
    },
    ShowCandidates(CandidatePanelModel),
    MoveSelection(CandidatePanelModel),
    HideCandidates,
    /// 確定済みの文字列の範囲を消す（ADR 0050 §3.3）。変換中の文字列がないときだけ返す。
    /// macOS は `client.insertText("", replacementRange: range)` を 1 回
    DeleteText {
        range: Utf16Range,
    },
    /// 元の文字列を、`replacing` の範囲を置き換える 1 回の挿入で書き戻す（ADR 0006 §16.3）
    Restore {
        text: String,
        replacing: Utf16Range,
    },
    /// 殻が何も入力していない状態で Enter / Esc を受けた。取り残された変換中の文字列があれば確定し、
    /// なければキーをアプリに渡す（ADR 0006 §16.3）
    ResolveStrayMarkedText,
    /// 読みが変わった（Swift の `KeyAction.prefetch`。エンジンへの Prefetch は `shell-core` が送る）
    Prefetch {
        reading: String,
    },
    /// メニューの中身が変わった。OS 側はこれを持っておき、メニューを開くときに写す
    ShowMenu(MenuModel),
    ScheduleTimer {
        id: TimerId,
        after_ms: u32,
    },
    CancelTimer {
        id: TimerId,
    },
    /// 設定アプリへ枠を返す
    BridgeReply {
        conn: ConnId,
        frame: Vec<u8>,
    },
    OpenSettings {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<SettingsTab>,
    },
    /// OS にモードを選んでもらう（macOS の `client.selectMode`。値はモードの ID）。実際に替わったかは
    /// `InputModeChanged` で知る。L-E で追加（`Shell::take_mode_request` を FFI の口が動作にしたもの）
    SelectMode {
        mode_id: String,
    },
    /// 設定アプリとの接続を閉じる（最初の枠が合わない、壊れた枠）。L-E で追加
    BridgeClose {
        conn: ConnId,
    },
    /// 不正な学習ファイルを退避して空で起動したことを知らせる（#271）。エンジンが Hello の返事で 1 回だけ伝え、
    /// shell-core が 1 回だけ返す。OS 側は、いまの入力処理が終わってから通知を出す
    ShowLearningQuarantineNotice,
}

// ---------------------------------------------------------------- エンジン

/// エンジンの返した誤りの種類（`engine.proto` の `ErrorCode`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineErrorCode {
    Unspecified,
    SchemaMismatch,
    BadRequest,
    UnknownSession,
    UnknownConversion,
    EngineNotReady,
    Internal,
    /// `layout_rev` が古い（ADR 0006 §5.2）
    StaleLayout,
}

/// エンジンへの要求の誤り（4 種。ADR 0041 §4.3）。
/// トランスポートの誤り（`Unavailable`、`Timeout`）はホストの戻り値から、中身の誤りは `shell-core` が返事を読んで決める。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineError {
    /// つながらない（落ちた、再起動中、連続して落ちて休止中）
    Unavailable,
    /// 待ちの上限を超えた（返事は捨てる）
    Timeout,
    /// 返事が壊れている、または候補の数や長さが範囲外
    InvalidReply,
    Engine(EngineErrorCode),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => f.write_str("engine unavailable"),
            Self::Timeout => f.write_str("engine timeout"),
            Self::InvalidReply => f.write_str("invalid engine reply"),
            Self::Engine(code) => write!(f, "engine error: {code:?}"),
        }
    }
}

impl std::error::Error for EngineError {}

// ---------------------------------------------------------------- 設定（ADR 0041 §2.3）

/// 設定の一式（`settings.tsv` を読んだ結果）。Swift の `Settings`。既定値は `settings` モジュールが持つ。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsSnapshot {
    /// `input.method`
    pub input_method: InputMethodKind,
    /// `keymap.preset`
    pub keymap_preset: KeymapPreset,
    /// `keymap.<操作>` → キーの表記、または `none`
    pub keymap_overrides: BTreeMap<String, String>,
    /// `paging.wrap`
    pub paging_wrap: bool,
    /// `paging.size`（1..9）
    pub page_size: u32,
    /// `engine.lm`
    pub engine_lm: bool,
    /// `engine.idle_unload_secs`
    pub engine_idle_unload_secs: u32,
    /// `engine.lm_deadline_ms`
    pub engine_lm_deadline_ms: u32,
    /// `engine.convert_wait_ms`
    pub engine_convert_wait_ms: u32,
    /// `emoji.shortcode`
    pub emoji_shortcode: bool,
    /// `emoji.shortcode_exclude`
    pub emoji_shortcode_exclude: Vec<String>,
    /// `emoji.small_letters`
    pub emoji_small_letters: bool,
    /// `postal.japanese`
    pub postal_japanese: bool,
    /// `postal.english`
    pub postal_english: bool,
    /// `postal.te_prefix`
    pub postal_te_prefix: bool,
    /// `unicode.notation`
    pub unicode_notation: bool,
    /// `date.weekday_dates`
    pub date_weekday_dates: bool,
    /// `candidate.hide_when_confident`
    pub candidate_hide_when_confident: bool,
    /// `clause.min_chars`（1..10。ADR 0050）
    pub clause_min_chars: u32,
}

/// 設定の警告の種類。値の文字列は含めない（ログにユーザーの文字列を残さない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsIssueKind {
    /// `key<TAB>value` の形でない
    Malformed,
    UnknownKey,
    InvalidValue,
}

/// 設定の警告 1 件（行番号は 1 から）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsIssue {
    pub line: u32,
    /// `Malformed` では空
    pub key: String,
    pub kind: SettingsIssueKind,
}

// ---------------------------------------------------------------- serde の補助

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_true(value: &bool) -> bool {
    *value
}

fn default_true() -> bool {
    true
}

fn is_zero_len(value: &Utf16Len) -> bool {
    *value == Utf16Len::ZERO
}

fn is_dictionary(kind: &CandidateKind) -> bool {
    *kind == CandidateKind::Dictionary
}
