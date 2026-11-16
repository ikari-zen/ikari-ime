//! 画面に出す行（設定の一覧 + 今効いている値）。**OS に依存しない**。
//!
//! 画面（C#）は、ここが決めた部品（`widget`）と範囲と選択肢で描くだけで、値の書き方（`on`/`true` など）を知らない。
//!
//! 行の書き出し（UTF-8 の TSV。1 行 1 件。並びは画面での順）。列:
//! 1. キー 2. 部品（`bool`、`int`、`choice`、`text`） 3. 節 4. 画面 5. 値（`bool` は `1`/`0`、`int` は 10 進、
//!    `choice` は選択肢の名前、`text` はそのまま） 6. 下限 7. 上限（`int` だけ。ほかは空） 8. 選択肢（`|` 区切り。
//!    `choice` だけ） 9. 反映（`immediate`、`engine_restart`）
//!
//! `text` の部品（Bundle ID の並び、キー割り当て）は、この段では読むだけ（書き換えは次の段）。

use std::collections::HashMap;
use std::fmt::Write as _;

use ikari_shell_core::settings::{
    Reflect, SettingKind, SettingSpec, VALUE_FALSE, VALUE_OFF, VALUE_ON, VALUE_TRUE, all_specs,
};

/// 画面の部品
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Widget {
    Bool,
    Int,
    Choice,
    /// 読むだけ
    Text,
}

impl Widget {
    pub fn of(kind: &SettingKind) -> Widget {
        match kind {
            SettingKind::Bool => Widget::Bool,
            SettingKind::Int { .. } => Widget::Int,
            SettingKind::Enum { .. } => Widget::Choice,
            SettingKind::BundleIdList | SettingKind::KeyChord => Widget::Text,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Widget::Bool => "bool",
            Widget::Int => "int",
            Widget::Choice => "choice",
            Widget::Text => "text",
        }
    }
}

/// 設定の書き方の真偽を、画面の `1`/`0` にする（読めなければ空）
fn bool_cell(value: &str) -> &'static str {
    match value {
        VALUE_TRUE | VALUE_ON => "1",
        VALUE_FALSE | VALUE_OFF => "0",
        _ => "",
    }
}

/// `effective`（設定の口が返した、今効いている値の一式）から、画面の行を作る。値の無いキーは既定値を出す。
pub fn view_tsv(effective: &[(String, String)]) -> String {
    let values: HashMap<&str, &str> = effective
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let mut out = String::new();
    for spec in all_specs() {
        write_row(&mut out, &spec, values.get(spec.key.as_str()).copied());
    }
    out
}

fn write_row(out: &mut String, spec: &SettingSpec, value: Option<&str>) {
    let widget = Widget::of(&spec.kind);
    let raw = value.or(spec.default.as_deref()).unwrap_or("");
    let value = match widget {
        Widget::Bool => bool_cell(raw),
        _ => raw,
    };
    let (min, max, choices) = match &spec.kind {
        SettingKind::Int { min, max } => (min.to_string(), max.to_string(), String::new()),
        SettingKind::Enum { choices } => (String::new(), String::new(), choices.join("|")),
        _ => (String::new(), String::new(), String::new()),
    };
    let reflect = match spec.reflect {
        Reflect::Immediate => "immediate",
        Reflect::EngineRestart => "engine_restart",
    };
    // 値に区切りの文字が入っていたら、行が壊れないよう空白にする（値は設定の口の検査を通ったもの）
    let value: String = value
        .chars()
        .map(|c| {
            if c == '\t' || c == '\n' || c == '\r' {
                ' '
            } else {
                c
            }
        })
        .collect();
    let _ = writeln!(
        out,
        "{}\t{}\t{}\t{}\t{value}\t{min}\t{max}\t{choices}\t{reflect}",
        spec.key,
        widget.id(),
        spec.section.id(),
        spec.screen().id(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikari_shell_core::settings::{KEY_ENGINE_LM, KEY_PAGING_SIZE, KEY_PAGING_WRAP};

    fn row<'a>(tsv: &'a str, key: &str) -> Vec<&'a str> {
        tsv.lines()
            .map(|l| l.split('\t').collect::<Vec<_>>())
            .find(|c| c[0] == key)
            .unwrap_or_else(|| panic!("row for {key}"))
    }

    #[test]
    fn every_spec_becomes_one_row_of_nine_columns_in_order() {
        let tsv = view_tsv(&[]);
        let keys: Vec<&str> = tsv.lines().map(|l| l.split('\t').next().unwrap()).collect();
        let specs: Vec<String> = all_specs().into_iter().map(|s| s.key).collect();
        assert_eq!(keys, specs);
        assert!(tsv.lines().all(|l| l.split('\t').count() == 9));
    }

    #[test]
    fn values_are_normalised_for_the_widgets() {
        let tsv = view_tsv(&[
            (KEY_PAGING_WRAP.to_owned(), "false".to_owned()),
            (KEY_ENGINE_LM.to_owned(), "on".to_owned()),
            (KEY_PAGING_SIZE.to_owned(), "5".to_owned()),
        ]);
        assert_eq!(
            row(&tsv, KEY_PAGING_WRAP)[1..5],
            ["bool", "candidate", "conversion", "0"]
        );
        assert_eq!(row(&tsv, KEY_ENGINE_LM)[4], "1");
        assert_eq!(row(&tsv, KEY_ENGINE_LM)[8], "engine_restart");
        let size = row(&tsv, KEY_PAGING_SIZE);
        assert_eq!(
            size[1..7],
            ["int", "candidate", "conversion", "5", "1", "9"]
        );
        let method = row(&tsv, "input.method");
        assert_eq!(method[1], "choice");
        assert_eq!(method[7], "romaji|kana");
    }

    #[test]
    fn missing_values_fall_back_to_the_defaults() {
        let tsv = view_tsv(&[]);
        assert_eq!(row(&tsv, KEY_PAGING_WRAP)[4], "1");
        assert_eq!(row(&tsv, KEY_PAGING_SIZE)[4], "3");
    }
}
