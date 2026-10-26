//! 読みの正規化。索引のキーの作り方は builder と core の両方に効く契約（ADR 0001 §3.6）。

use crate::MAX_READING_CHARS;

const KATAKANA_FIRST: char = '\u{30A1}';
const KATAKANA_LAST: char = '\u{30F6}';
const KATAKANA_TO_HIRAGANA_OFFSET: u32 = 0x60;
const KATAKANA_ITERATION: char = '\u{30FD}'; // ヽ
const KATAKANA_VOICED_ITERATION: char = '\u{30FE}'; // ヾ
const HIRAGANA_ITERATION: char = '\u{309D}'; // ゝ
const HIRAGANA_VOICED_ITERATION: char = '\u{309E}'; // ゞ
const HIRAGANA_FIRST: char = '\u{3041}'; // ぁ
const HIRAGANA_LAST: char = '\u{3096}'; // ゖ
const PROLONGED_SOUND_MARK: char = '\u{30FC}'; // ー

const READING_SEPARATORS: [char; 5] = ['\u{30FB}', '\u{FF1D}', '\u{30A0}', '\u{0020}', '\u{3000}'];

/// カタカナ U+30A1..=U+30F6 を -0x60 してひらがなにする（ヴ→ゔ、ヵ→ゕ、ヶ→ゖ を含む）。
/// ヽ→ゝ、ヾ→ゞ。長音記号「ー」(U+30FC) はそのまま。それ以外の文字（記号、ASCII）もそのまま。
/// 全域関数で、失敗しない。クエリ側（core::Converter）でも必ずこれを通す。
pub fn normalize_key(s: &str) -> String {
    s.chars().map(normalize_char).collect()
}

fn normalize_char(c: char) -> char {
    match c {
        KATAKANA_FIRST..=KATAKANA_LAST => {
            char::from_u32(c as u32 - KATAKANA_TO_HIRAGANA_OFFSET).unwrap_or(c)
        }
        KATAKANA_ITERATION => HIRAGANA_ITERATION,
        KATAKANA_VOICED_ITERATION => HIRAGANA_VOICED_ITERATION,
        _ => c,
    }
}

/// 索引に入れられる読みか。1..=MAX_READING_CHARS 文字で、全文字が ぁ..=ゖ、ゝ、ゞ、ー のいずれか。
pub fn is_indexable(s: &str) -> bool {
    let mut count = 0usize;
    for c in s.chars() {
        count += 1;
        if count > MAX_READING_CHARS || !is_indexable_char(c) {
            return false;
        }
    }
    count >= 1
}

fn is_indexable_char(c: char) -> bool {
    matches!(
        c,
        HIRAGANA_FIRST
            ..=HIRAGANA_LAST
                | HIRAGANA_ITERATION
                | HIRAGANA_VOICED_ITERATION
                | PROLONGED_SOUND_MARK
    )
}

/// builder 専用。読みから区切り記号「・」(U+30FB)、「＝」(U+FF1D)、「゠」(U+30A0)、空白(U+0020, U+3000) を取り除く。
pub fn strip_reading_separators(s: &str) -> String {
    s.chars()
        .filter(|c| !READING_SEPARATORS.contains(c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_converts_katakana_range() {
        assert_eq!(normalize_key("ヒンシ"), "ひんし");
        assert_eq!(normalize_key("ァ"), "ぁ");
        assert_eq!(normalize_key("ヴヵヶ"), "ゔゕゖ");
    }

    #[test]
    fn normalize_converts_iteration_marks() {
        assert_eq!(normalize_key("ヽヾ"), "ゝゞ");
    }

    #[test]
    fn normalize_keeps_other_chars() {
        assert_eq!(normalize_key("セブン-イレブンー"), "せぶん-いれぶんー");
        assert_eq!(normalize_key("abc←漢字ひら"), "abc←漢字ひら");
        // U+30F7..=U+30FA（ヷヸヹヺ）は範囲外でそのまま
        assert_eq!(normalize_key("ヷヺ"), "ヷヺ");
        assert_eq!(normalize_key(""), "");
    }

    #[test]
    fn indexable_accepts_hiragana() {
        assert!(is_indexable("ひんし"));
        assert!(is_indexable("らーめん"));
        assert!(is_indexable("ゔぁ"));
        assert!(is_indexable("ゝゞ"));
        assert!(is_indexable("ぁゖ"));
    }

    #[test]
    fn indexable_rejects_others() {
        assert!(!is_indexable(""));
        assert!(!is_indexable("ヒンシ"));
        assert!(!is_indexable("ひんしa"));
        assert!(!is_indexable("ひ・ん"));
        assert!(!is_indexable("ひ ん"));
        assert!(!is_indexable("ゔ\u{3097}"));
    }

    #[test]
    fn indexable_length_boundary() {
        let max = "あ".repeat(MAX_READING_CHARS);
        assert!(is_indexable(&max));
        let over = "あ".repeat(MAX_READING_CHARS + 1);
        assert!(!is_indexable(&over));
    }

    #[test]
    fn strip_removes_separators_only() {
        assert_eq!(strip_reading_separators("キ・タ・ナ・イ"), "キタナイ");
        assert_eq!(strip_reading_separators("a＝b゠c d\u{3000}e"), "abcde");
        assert_eq!(strip_reading_separators("ひんし"), "ひんし");
        assert_eq!(strip_reading_separators(""), "");
    }
}
