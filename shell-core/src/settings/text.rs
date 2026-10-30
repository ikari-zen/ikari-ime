//! 文字列の補助。Swift の `trimmingCharacters(in: .whitespaces)`・`isNewline`・`controlCharacters` と同じ範囲を使う。

/// Swift の `CharacterSet.whitespaces`（タブと、Unicode の Zs）。改行は含まない
pub(crate) fn is_blank(c: char) -> bool {
    matches!(
        c,
        '\t' | ' ' | '\u{00A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

/// Swift の `Character.isNewline`（`\r\n` は 1 つ）
fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{000B}' | '\u{000C}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
    )
}

/// `trimmingCharacters(in: .whitespaces)`
pub(crate) fn trim_blank(text: &str) -> &str {
    text.trim_matches(is_blank)
}

/// 空白だけ（または空）か
pub(crate) fn is_blank_line(text: &str) -> bool {
    text.chars().all(is_blank)
}

/// `split(omittingEmptySubsequences: false, whereSeparator: \.isNewline)`。`\r\n` は 1 つの区切り
pub(crate) fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        if !is_newline(c) {
            continue;
        }
        lines.push(&text[start..index]);
        start = index + c.len_utf8();
        if c == '\r'
            && let Some(&(next, '\n')) = chars.peek()
        {
            chars.next();
            start = next + 1;
        }
    }
    lines.push(&text[start..]);
    lines
}

/// 行の区切りや制御文字を含むか（ファイルに別の行を混ぜられる文字）。
/// Swift の `CharacterSet.controlCharacters`（Cc）に改行を足したもの
pub(crate) fn has_forbidden(text: &str) -> bool {
    text.chars().any(|c| c.is_control() || is_newline(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_lines_treats_crlf_as_one_separator() {
        assert_eq!(split_lines("a\r\nb\nc\rd"), vec!["a", "b", "c", "d"]);
        assert_eq!(split_lines("a\n"), vec!["a", ""]);
        assert_eq!(split_lines(""), vec![""]);
    }

    #[test]
    fn trim_blank_keeps_newlines_and_trims_tabs_and_ideographic_space() {
        assert_eq!(trim_blank("\t \u{3000}x\u{3000} "), "x");
        assert!(is_blank_line(" \t"));
        assert!(!is_blank_line("x"));
    }

    #[test]
    fn forbidden_covers_newlines_and_controls() {
        assert!(has_forbidden("a\nb"));
        assert!(has_forbidden("a\tb"));
        assert!(has_forbidden("a\u{2028}b"));
        assert!(!has_forbidden("abc あ"));
    }
}
