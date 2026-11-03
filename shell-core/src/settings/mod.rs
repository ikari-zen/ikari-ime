//! 設定（ADR 0041 §2.3。レーン L-C）。`settings.tsv` の解釈と検証、書き換え、既定値。
//! 殻と設定アプリ（別のプロセス）の両方が使う。設定アプリは `ikari_settings_validate`・`ikari_settings_rewrite`（§4.4）から呼ぶ。
//!
//! **契約**: `validate` と `rewrite` のシグネチャ、`SettingsChanges`・`SettingsValidation`・`SettingsError` は M1-a で固めた。
//!
//! # 構成（L-C）
//! - `defaults`: キーの名前、既定値、操作の名前の表
//! - `parse`: `settings.tsv` の解釈（`validate` の中身）、既知のキーと値の検査、`effective_entries`
//! - `file`: 行を保ったままの書き換え（`rewrite` の中身）
//! - `store`: ファイルの読み書き（mtime と大きさで手の編集に気づく。一時ファイル + rename、権限 0600）
//! - `chord`: キーの表記の読み書きと表示
//! - `values`: 設定アプリの Model のうち、検証・整形・既定値・変更の組み立て（`@Observable` の状態は持たない）
//! - `admin`: 管理の要求（属性プロファイル）の組み立て

mod admin;
mod chord;
mod defaults;
mod file;
mod parse;
mod store;
mod text;
mod values;

pub use admin::{
    CONJ_SEPARATOR, DictionaryDraft, DraftProblem, ImportSize, MAX_IMPORT_BYTES, ProfileDomainRow,
    ProfileWeight, dictionary_display_order, dictionary_entry_from_draft, dictionary_entry_matches,
    export_file_name, import_size, pos_display, profile_get_request, profile_rows,
    profile_set_request, profile_weight_from_proto, profile_weight_to_proto,
    sort_dictionary_entries,
};
pub use chord::KeyChord;
pub use defaults::*;
pub use parse::{
    effective_entries, engine_settings_differ, is_acceptable, is_known_key, parse_bundle_id_list,
};
pub use store::{Reload, SettingsChange, SettingsStore, StoreError, Updated};
pub use values::{
    CONVERT_WAIT_RANGE, IDLE_UNLOAD_RANGE, KeymapBinding, KeymapPresetTable, KeymapRowData,
    LM_DEADLINE_RANGE, PlanError, SettingsIntent, SettingsValues,
};

use serde::{Deserialize, Serialize};

use crate::types::{SettingsIssue, SettingsIssueKind, SettingsSnapshot};

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
/// 未知のキーと不正な値は無視して既定値を使い、警告に積む（読み込みで入力を止めない）。空の文字列は既定値の一式。
/// 現在の実装は失敗しない（`Result` は契約のまま）。
pub fn validate(tsv: &str) -> Result<SettingsValidation, SettingsError> {
    Ok(parse::parse_tsv(tsv))
}

/// 今のファイルの文字列に `changes` を当てた文字列を返す（今の `SettingsFile.update` と、呼び出す側の検証）。
/// 注釈、空行、未知のキー、ほかのキーの行は、中身も順もそのまま残す。警告が増えるなら `IssuesAdded`。
///
/// `IssuesAdded` の `issues`:
/// - 書いてはいけない `set`（未知のキー、不正な値、行を混ぜる文字）: 行番号 0 で、キーが既知なら `InvalidValue` とそのキー、
///   未知なら `UnknownKey` とキーなし（空）。値は含めない。全部を集めて返す
/// - 未知の `reset` のキー: 行番号 0、`UnknownKey`、キーなし
/// - 書き換えの結果、警告が増えた: 増えた分（行番号は結果のファイルのもの）
pub fn rewrite(file: &str, changes: &SettingsChanges) -> Result<String, SettingsError> {
    let mut rejected: Vec<SettingsIssue> = Vec::new();
    for entry in &changes.set {
        if !parse::is_acceptable(&entry.key, &entry.value) {
            rejected.push(if parse::is_known_key(&entry.key) {
                rewrite_issue(&entry.key, SettingsIssueKind::InvalidValue)
            } else {
                rewrite_issue("", SettingsIssueKind::UnknownKey)
            });
        }
    }
    for key in &changes.reset {
        if !parse::is_known_key(key) {
            rejected.push(rewrite_issue("", SettingsIssueKind::UnknownKey));
        }
    }
    if !rejected.is_empty() {
        return Err(SettingsError::IssuesAdded { issues: rejected });
    }

    let updated = file::update_text(file, &changes.set, &changes.reset);
    let before = parse::parse_tsv(file).issues;
    let after = parse::parse_tsv(&updated).issues;
    if after.len() > before.len() {
        return Err(SettingsError::IssuesAdded {
            issues: added_issues(&before, after),
        });
    }
    Ok(updated)
}

fn rewrite_issue(key: &str, kind: SettingsIssueKind) -> SettingsIssue {
    SettingsIssue {
        line: 0,
        key: key.to_string(),
        kind,
    }
}

/// `after` のうち、`before` に（キーと種類が同じものとして）なかった分
fn added_issues(before: &[SettingsIssue], after: Vec<SettingsIssue>) -> Vec<SettingsIssue> {
    let mut remaining: Vec<&SettingsIssue> = before.iter().collect();
    after
        .into_iter()
        .filter(|issue| {
            match remaining
                .iter()
                .position(|old| old.key == issue.key && old.kind == issue.kind)
            {
                Some(index) => {
                    remaining.swap_remove(index);
                    false
                }
                None => true,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(set: &[(&str, &str)], reset: &[&str]) -> SettingsChanges {
        SettingsChanges {
            set: set
                .iter()
                .map(|(k, v)| SettingsEntry {
                    key: (*k).into(),
                    value: (*v).into(),
                })
                .collect(),
            reset: reset.iter().map(|k| (*k).to_string()).collect(),
        }
    }

    fn added(result: Result<String, SettingsError>) -> Vec<SettingsIssue> {
        match result {
            Err(SettingsError::IssuesAdded { issues }) => issues,
            other => panic!("expected IssuesAdded, got {other:?}"),
        }
    }

    #[test]
    fn rewrite_replaces_and_resets() {
        let text = "# n
paging.size	3
engine.lm	on
";
        let result = rewrite(text, &change(&[("paging.size", "5")], &["engine.lm"])).unwrap();
        assert_eq!(
            result,
            "# n
paging.size	5
"
        );
        assert_eq!(validate(&result).unwrap().snapshot.page_size, 5);
    }

    #[test]
    fn rewrite_rejects_unknown_keys_bad_values_and_injection() {
        let text = "input.method	kana
";
        let issues = added(rewrite(text, &change(&[("paging.size", "10")], &[])));
        assert_eq!(
            issues,
            vec![SettingsIssue {
                line: 0,
                key: "paging.size".into(),
                kind: SettingsIssueKind::InvalidValue
            }]
        );
        let issues = added(rewrite(text, &change(&[("future.key", "x")], &[])));
        assert_eq!(issues[0].kind, SettingsIssueKind::UnknownKey);
        assert_eq!(issues[0].key, "");
        assert!(issues[0].line == 0);
        added(rewrite(
            text,
            &change(
                &[(
                    "paging.wrap",
                    "true
engine.lm	off",
                )],
                &[],
            ),
        ));
        added(rewrite(text, &change(&[], &["no.such"])));
        // 何も書かれていない
        assert_eq!(rewrite(text, &change(&[], &[])).unwrap(), text);
    }

    #[test]
    fn rewrite_refuses_a_write_that_adds_issues_but_allows_one_that_does_not() {
        // 既にある壊れた行はそのまま残してよい。増やさない限り書ける
        let text = "future.key	x
paging.size	abc
";
        let result = rewrite(text, &change(&[("input.method", "kana")], &[])).unwrap();
        assert_eq!(
            result,
            "future.key	x
paging.size	abc
input.method	kana
"
        );
        // 結果の警告が増えない（既存の警告と同数）
        assert_eq!(validate(&result).unwrap().issues.len(), 2);
    }

    #[test]
    fn rewrite_can_fix_a_broken_line() {
        // 受理される値だけを書くので、警告は増えない。既存の壊れた行を直す側（減る）を確かめる
        let text = "paging.size	abc
";
        let fixed = rewrite(text, &change(&[("paging.size", "4")], &[])).unwrap();
        assert!(validate(&fixed).unwrap().issues.is_empty());
    }

    #[test]
    fn added_issues_ignore_those_already_present() {
        let old = SettingsIssue {
            line: 1,
            key: "a".into(),
            kind: SettingsIssueKind::InvalidValue,
        };
        let moved = SettingsIssue {
            line: 5,
            ..old.clone()
        };
        let new = SettingsIssue {
            line: 6,
            key: "b".into(),
            kind: SettingsIssueKind::UnknownKey,
        };
        assert_eq!(
            added_issues(std::slice::from_ref(&old), vec![moved, new.clone()]),
            vec![new]
        );
    }

    #[test]
    fn validate_never_fails_and_returns_defaults_for_empty_text() {
        let result = validate("").unwrap();
        assert_eq!(result.snapshot, default_snapshot());
        assert!(result.issues.is_empty());
    }
}
