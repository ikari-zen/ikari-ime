//! 設定（ADR 0041 §2.3。レーン L-C）。`settings.tsv` の解釈と検証、書き換え、既定値。
//! 殻と設定アプリ（別のプロセス）の両方が使う。設定アプリは `ikari_settings_validate`・`ikari_settings_rewrite`（§4.4）から呼ぶ。
//!
//! **契約**: `validate` と `rewrite` のシグネチャ、`SettingsChanges`・`SettingsValidation`・`SettingsError` は M1-a で固めた。

use serde::{Deserialize, Serialize};

use crate::types::{SettingsIssue, SettingsSnapshot};

/// `settings.tsv` を読んだ結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsValidation {
    /// 不正な行を飛ばし、既定値で埋めた設定の一式
    pub snapshot: SettingsSnapshot,
    pub issues: Vec<SettingsIssue>,
}

/// 書き換えの 1 件（`key<TAB>value`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsEntry {
    pub key: String,
    pub value: String,
}

/// 書き換えの中身（今の `SettingsFile.update(text:set:reset:)` の引数）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SettingsChanges {
    /// 値を差し替える・足す（この順）
    #[serde(default)]
    pub set: Vec<SettingsEntry>,
    /// 行を消して既定に戻す（`set` より先に行う）
    #[serde(default)]
    pub reset: Vec<String>,
}

/// 設定の処理の誤り。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsError {
    /// 書き換えると警告が増える（書かない）。増えた警告を返す
    IssuesAdded { issues: Vec<SettingsIssue> },
    /// M1-a の stub だけが返す。L-C が実装したら消す
    Unimplemented,
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IssuesAdded { issues } => write!(f, "rewrite adds {} issue(s)", issues.len()),
            Self::Unimplemented => f.write_str("settings: not implemented yet"),
        }
    }
}

impl std::error::Error for SettingsError {}

/// `settings.tsv` の文字列を読み、設定の一式と警告を返す（今の `Settings.parse(tsv:)`）。
pub fn validate(_tsv: &str) -> Result<SettingsValidation, SettingsError> {
    Err(SettingsError::Unimplemented)
}

/// 今のファイルの文字列に `changes` を当てた文字列を返す（今の `SettingsFile.update` と、呼び出す側の検証）。
/// 注釈、空行、未知のキー、ほかのキーの行は、中身も順もそのまま残す。警告が増えるなら `IssuesAdded`。
pub fn rewrite(_file: &str, _changes: &SettingsChanges) -> Result<String, SettingsError> {
    Err(SettingsError::Unimplemented)
}
