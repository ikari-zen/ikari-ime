//! 画面に出す設定の一覧（ADR 0040 §2.5、W4-s1）。キー、種類、既定値、画面の区分、文言の鍵、反映の扱い。
//!
//! **判断の置き場所: Rust**。macOS と Windows の設定画面は、この一覧に従って画面を作る（何を出すか、どの範囲か、
//! どこに置くか）。文言そのものは OS の画面が持つ（鍵は [`SettingSpec::label_key`]・[`SettingSpec::description_key`]。
//! 多言語化は後）。片方の OS の画面だけに設定を足さない。
//!
//! # 二重の定義を作らない
//! 検証と既定値の本体は `defaults.rs`（定数）、`parse.rs`（検査）、`values.rs`（範囲）のまま。一覧はそれらから作る:
//! 既定値は `effective_entries(default_snapshot())` から引き、範囲と選択肢は同じ定数から引く。
//! 一覧と本体のずれはテストで止める。
//!
//! 値の書き込みは、画面が `SettingsIntent`（`values.rs`）にして `SettingsValues::plan` へ渡す。一覧は「何があるか」を
//! 言い、「どう書くか」は言わない（`on`/`off` と `true`/`false` の違いなどは本体が持つ）。

use std::fmt::Write as _;

/// 画面。`settings.tsv` のキーを持つ画面だけ（ユーザー辞書、学習、このアプリについては、キーがないので一覧の外）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsScreen {
    /// 入力（macOS の `SettingsTab.input`）
    Input,
    /// 変換（macOS の `SettingsTab.conversion`）
    Conversion,
}

impl SettingsScreen {
    /// 書き出しに使う名前（macOS の `SettingsTab` の rawValue と同じ。変えない）
    pub fn id(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Conversion => "conversion",
        }
    }
}

/// 画面の中の節（画面での並びの順）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsSection {
    InputMethod,
    KeymapPreset,
    KeymapActions,
    Candidate,
    ClauseDelete,
    LanguageModel,
    EmojiShortcode,
    Postal,
    UnicodeNotation,
    WeekdayDates,
}

impl SettingsSection {
    /// 節を置く画面
    pub fn screen(self) -> SettingsScreen {
        match self {
            Self::InputMethod | Self::KeymapPreset | Self::KeymapActions => SettingsScreen::Input,
            Self::Candidate
            | Self::ClauseDelete
            | Self::LanguageModel
            | Self::EmojiShortcode
            | Self::Postal
            | Self::UnicodeNotation
            | Self::WeekdayDates => SettingsScreen::Conversion,
        }
    }

    /// 文言の鍵や一覧の書き出しに使う名前（変えない）
    pub fn id(self) -> &'static str {
        match self {
            Self::InputMethod => "input_method",
            Self::KeymapPreset => "keymap_preset",
            Self::KeymapActions => "keymap_actions",
            Self::Candidate => "candidate",
            Self::ClauseDelete => "clause_delete",
            Self::LanguageModel => "language_model",
            Self::EmojiShortcode => "emoji_shortcode",
            Self::Postal => "postal",
            Self::UnicodeNotation => "unicode_notation",
            Self::WeekdayDates => "weekday_dates",
        }
    }

    /// 節の題の文言の鍵。文言は OS の画面が持つ
    pub fn title_key(self) -> String {
        format!("settings.section.{}.title", self.id())
    }
}

/// 値の種類。画面は種類から部品を選ぶ（スイッチ、数の選択、選択肢、リスト、キー入力）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingKind {
    /// オン・オフ
    Bool,
    /// 整数（両端を含む。範囲の外は `SettingsValues::plan` が範囲に丸める）
    Int { min: i64, max: i64 },
    /// 決まった選択肢のどれか（`settings.tsv` の書き方）
    Enum { choices: Vec<&'static str> },
    /// Bundle ID の並び（アプリの除外リスト。追加・削除・既定に戻す）
    BundleIdList,
    /// キー割り当て（`ctrl+j` の表記、`none` で外す、行なしでプリセットのまま）
    KeyChord,
}

/// 変更が効くまでの扱い。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reflect {
    /// 次のキー入力から効く
    Immediate,
    /// エンジンを起こし直す（`engine_settings_differ` と同じ。読み込み中の数秒は辞書だけで変換する）
    EngineRestart,
}

/// 画面に出す設定 1 件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingSpec {
    /// `settings.tsv` のキー（`keymap.<操作>` を含む）
    pub key: String,
    pub kind: SettingKind,
    /// 既定値（`settings.tsv` の書き方）。`None` は「行がない」が既定（キー割り当て: プリセットのまま）
    pub default: Option<String>,
    pub section: SettingsSection,
    pub reflect: Reflect,
}

impl SettingSpec {
    pub fn screen(&self) -> SettingsScreen {
        self.section.screen()
    }

    /// 表示名の文言の鍵。文言は OS の画面が持つ
    pub fn label_key(&self) -> String {
        format!("settings.{}.label", self.key)
    }

    /// 説明の文言の鍵。文言は OS の画面が持つ（説明のない設定は画面が何も出さなくてよい）
    pub fn description_key(&self) -> String {
        format!("settings.{}.description", self.key)
    }
}

/// 画面に出す設定の全部。画面での並びの順（画面ごと、節ごと、節の中）。
pub fn all_specs() -> Vec<SettingSpec> {
    todo!("W4-s1: 次のコミットで一覧を入れる")
}

/// `screen` に出す設定だけ（並びは `all_specs` のまま）。
pub fn specs_for(screen: SettingsScreen) -> Vec<SettingSpec> {
    all_specs()
        .into_iter()
        .filter(|spec| spec.screen() == screen)
        .collect()
}

/// 一覧の書き出し（1 行 1 件、タブ区切り）。両 OS の画面のテストが読む。
/// 列: キー、種類、節、画面、既定値（なしは空）、反映。
pub fn specs_tsv() -> String {
    let mut out = String::new();
    for spec in all_specs() {
        let kind = match &spec.kind {
            SettingKind::Bool => "bool".to_string(),
            SettingKind::Int { min, max } => format!("int:{min}..{max}"),
            SettingKind::Enum { choices } => format!("enum:{}", choices.join("|")),
            SettingKind::BundleIdList => "bundle_id_list".to_string(),
            SettingKind::KeyChord => "key_chord".to_string(),
        };
        let reflect = match spec.reflect {
            Reflect::Immediate => "immediate",
            Reflect::EngineRestart => "engine_restart",
        };
        let _ = writeln!(
            out,
            "{}\t{kind}\t{}\t{}\t{}\t{reflect}",
            spec.key,
            spec.section.id(),
            spec.screen().id(),
            spec.default.as_deref().unwrap_or("")
        );
    }
    out
}
