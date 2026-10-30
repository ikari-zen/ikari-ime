//! `settings.tsv` の書き換え（今の `SettingsFile.update`。ADR 0005 §7.1）。純関数。

use super::SettingsEntry;
use super::text::{has_forbidden, is_blank_line, trim_blank};

const SEPARATOR: char = '\t';
const COMMENT_PREFIX: char = '#';

/// 今のファイルの文字列から、該当するキーの行の値だけを差し替えた文字列を返す。
///
/// - `set` のキーが既にあれば、その行の値だけを差し替える（同じキーが複数あれば最後の行）。ないキーは末尾に足す（`set` の順）。
/// - `reset` のキーの行は消す（既定に戻す）。同じキーが複数あれば、すべて消す（古い行の値が表に出ないように）。
///   同じキーが `set` と `reset` の両方にあるときは、`reset` を先に行う（結果は `set` の値の 1 行）。
/// - 注釈、空行、未知のキー、ほかのキーの行は、中身も順もそのまま残す（`engine.*` を含む）。
/// - 区切りの tab や改行を含むキー・値は、別の行を混ぜられるので無視する（ファイルに書かない）。
/// - 結果の検証（警告が増えたら書かない）は呼び出す側（`rewrite`）が行う。
pub(super) fn update_text(text: &str, set: &[SettingsEntry], reset: &[String]) -> String {
    // 改行の流儀を保つ。最後の要素は、末尾が改行で終わるときだけ空文字列になる
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();

    for key in reset.iter().filter(|key| is_writable_key(key)) {
        lines.retain(|line| key_of(line) != Some(key.as_str()));
    }

    let mut appended: Vec<String> = Vec::new();
    for entry in set
        .iter()
        .filter(|entry| is_writable_key(&entry.key) && is_writable_value(&entry.value))
    {
        let new_line =
            |carriage: &str| format!("{}{SEPARATOR}{}{carriage}", entry.key, entry.value);
        if let Some(index) = lines
            .iter()
            .rposition(|line| key_of(line) == Some(entry.key.as_str()))
        {
            // 行末の CR（CRLF のファイル）は残す
            let carriage = if lines[index].ends_with('\r') {
                "\r"
            } else {
                ""
            };
            lines[index] = new_line(carriage);
        } else if let Some(index) = appended
            .iter()
            .rposition(|line| key_of(line) == Some(entry.key.as_str()))
        {
            appended[index] = new_line("");
        } else {
            appended.push(new_line(""));
        }
    }
    if appended.is_empty() {
        return lines.join("\n");
    }

    // 末尾が改行で終わっていれば最後の要素は空。足す前に取り除き、足したあとで改行を付ける
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.extend(appended);
    lines.join("\n") + "\n"
}

/// `parse_tsv` と同じ規則で、行のキーを取り出す。注釈、空行、tab のない行は `None`
fn key_of(raw_line: &str) -> Option<&str> {
    let line = raw_line.trim_matches('\r');
    if is_blank_line(line) || line.starts_with(COMMENT_PREFIX) {
        return None;
    }
    let (key, _) = line.split_once(SEPARATOR)?;
    Some(trim_blank(key))
}

fn is_writable_key(key: &str) -> bool {
    !key.is_empty()
        && !key.starts_with(COMMENT_PREFIX)
        && !has_forbidden(key)
        && key == trim_blank(key)
}

fn is_writable_value(value: &str) -> bool {
    !has_forbidden(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(text: &str, set: &[(&str, &str)], reset: &[&str]) -> String {
        let set: Vec<SettingsEntry> = set
            .iter()
            .map(|(k, v)| SettingsEntry {
                key: (*k).into(),
                value: (*v).into(),
            })
            .collect();
        let reset: Vec<String> = reset.iter().map(|k| (*k).to_string()).collect();
        update_text(text, &set, &reset)
    }

    #[test]
    fn replaces_only_the_value_and_keeps_comments_unknown_keys_and_order() {
        let text = "# my notes\ninput.method\tromaji\n\n# lm\nengine.lm\ton\nfuture.key\tkeep\npaging.size\t5\n";
        assert_eq!(
            update(text, &[("input.method", "kana")], &[]),
            "# my notes\ninput.method\tkana\n\n# lm\nengine.lm\ton\nfuture.key\tkeep\npaging.size\t5\n"
        );
    }

    #[test]
    fn replaces_the_last_line_when_the_key_repeats() {
        let text = "paging.size\t3\npaging.wrap\ttrue\npaging.size\t4\n";
        assert_eq!(
            update(text, &[("paging.size", "7")], &[]),
            "paging.size\t3\npaging.wrap\ttrue\npaging.size\t7\n"
        );
    }

    #[test]
    fn appends_missing_keys_in_the_given_order() {
        assert_eq!(
            update(
                "# c\ninput.method\tkana\n",
                &[("paging.size", "2"), ("paging.wrap", "false")],
                &[]
            ),
            "# c\ninput.method\tkana\npaging.size\t2\npaging.wrap\tfalse\n"
        );
    }

    #[test]
    fn appends_to_a_file_without_a_trailing_newline() {
        assert_eq!(
            update("input.method\tkana", &[("paging.size", "2")], &[]),
            "input.method\tkana\npaging.size\t2\n"
        );
    }

    #[test]
    fn appends_to_an_empty_file() {
        assert_eq!(
            update("", &[("input.method", "kana")], &[]),
            "input.method\tkana\n"
        );
    }

    #[test]
    fn keeps_the_trailing_newline_state_when_nothing_is_appended() {
        assert_eq!(
            update("paging.size\t3", &[("paging.size", "4")], &[]),
            "paging.size\t4"
        );
        assert_eq!(
            update("paging.size\t3\n", &[("paging.size", "4")], &[]),
            "paging.size\t4\n"
        );
    }

    #[test]
    fn reset_removes_every_line_of_the_key_only() {
        let text = "# keep\npaging.size\t3\ninput.method\tkana\npaging.size\t4\n";
        assert_eq!(
            update(text, &[], &["paging.size"]),
            "# keep\ninput.method\tkana\n"
        );
    }

    #[test]
    fn resetting_a_missing_key_changes_nothing() {
        let text = "input.method\tkana\n";
        assert_eq!(update(text, &[], &["paging.size"]), text);
    }

    #[test]
    fn a_key_in_both_set_and_reset_ends_up_with_the_set_value() {
        assert_eq!(
            update(
                "paging.size\t3\npaging.size\t4\n",
                &[("paging.size", "9")],
                &["paging.size"]
            ),
            "paging.size\t9\n"
        );
    }

    #[test]
    fn a_repeated_key_in_set_keeps_the_last_value_once() {
        assert_eq!(
            update("", &[("paging.size", "2"), ("paging.size", "3")], &[]),
            "paging.size\t3\n"
        );
    }

    #[test]
    fn commented_out_keys_are_not_treated_as_lines() {
        assert_eq!(
            update("#paging.size\t3\n", &[("paging.size", "5")], &[]),
            "#paging.size\t3\npaging.size\t5\n"
        );
    }

    #[test]
    fn preserves_crlf_line_endings() {
        assert_eq!(
            update(
                "input.method\tromaji\r\npaging.size\t3\r\n",
                &[("input.method", "kana")],
                &[]
            ),
            "input.method\tkana\r\npaging.size\t3\r\n"
        );
    }

    #[test]
    fn ignores_keys_and_values_that_could_inject_lines() {
        let text = "input.method\tkana\n";
        assert_eq!(
            update(text, &[("paging.size", "3\nengine.lm\toff")], &[]),
            text
        );
        assert_eq!(update(text, &[("paging.size\tx", "3")], &[]), text);
        assert_eq!(update(text, &[("", "3")], &[]), text);
        assert_eq!(update(text, &[("#x", "3")], &[]), text);
        assert_eq!(update(text, &[], &["input.method\n"]), text);
    }

    #[test]
    fn unchanged_input_is_returned_as_is() {
        let text = "# only comments\n\n";
        assert_eq!(update(text, &[], &[]), text);
    }

    #[test]
    fn the_result_parses_back_to_the_requested_values() {
        use crate::settings::validate;
        use crate::types::KeymapPreset;
        let text = "# notes\nkeymap.preset\tmacos\nengine.lm\ton\n";
        let result = update(
            text,
            &[
                ("keymap.preset", "ms-ime"),
                ("keymap.convert.hiragana", "none"),
                ("engine.lm", "off"),
            ],
            &[],
        );
        let parsed = validate(&result).unwrap();
        assert!(parsed.issues.is_empty());
        assert_eq!(parsed.snapshot.keymap_preset, KeymapPreset::MsIme);
        assert_eq!(
            parsed.snapshot.keymap_overrides.get("convert.hiragana"),
            Some(&"none".to_string())
        );
        assert!(!parsed.snapshot.engine_lm);
    }
}
