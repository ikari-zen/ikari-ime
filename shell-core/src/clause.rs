//! 文節ごとの削除の「単位」を決める純粋な関数（ADR 0050 §3.3、§4、§7）。
//!
//! 状態にも OS にも触れない。位置は、断りのない限り書記素（Swift の `String.count` と同じ数え方）の通し番号。
//! 文書の位置（UTF-16）に直すのは `utf16_range` だけ。

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::types::{SurroundingText, Utf16Len, Utf16Range};

/// 文の終わりの記号（ADR 0050 §3.3）
const SENTENCE_TERMINATORS: [char; 7] = ['。', '．', '.', '！', '？', '!', '?'];
/// 文の終わりの記号のあとに続く閉じ括弧。記号と同じ文に含める
const CLOSING_BRACKETS: [char; 4] = ['」', '』', '）', ')'];

/// 確定後の削除を始めない入力先（暫定。ADR 0050 §7 の拒否リストの中身は、実機のスパイク S1 の結果で確定する）。
/// `insertText("", replacementRange:)` が効かない、または違う位置を消すアプリを入れる
pub const CLAUSE_DELETE_DENIED_BUNDLES: [&str; 2] = ["com.apple.Terminal", "com.googlecode.iterm2"];

/// 単位を作る向き
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
}

/// 文節の長さ（書記素）の列から、文節の境界（先頭を 0 とする累積）を作る。最後は全体の長さ。
pub fn boundaries(lengths: impl IntoIterator<Item = usize>) -> Vec<usize> {
    let mut total = 0usize;
    let mut points = vec![0];
    for length in lengths {
        total = total.saturating_add(length);
        points.push(total);
    }
    points
}

/// 削除の単位（ADR 0050 §4）。
///
/// - `len`: 文の長さ。`bounds`: core の文節の境界（昇順でなくても、範囲外があってもよい）。`cursor`: カーソル。
/// - `min`: 最小文字数。`Left` は、カーソルから左へ境界を 1 つずつたどり、`min` に届くか文頭に当たるまで合わせる。
///   `Right` は対称（文末）。カーソルが文節の途中なら、それも境界とみなす（最初の塊は次の境界まで）。
/// - その向きに何もない（`Left` でカーソルが文頭、`Right` で文末）ときは `None`。
pub fn unit_range(
    len: usize,
    bounds: &[usize],
    cursor: usize,
    min: usize,
    dir: Dir,
) -> Option<Range<usize>> {
    let cursor = cursor.min(len);
    let mut points: Vec<usize> = bounds
        .iter()
        .copied()
        .filter(|point| *point <= len)
        .chain([0, len, cursor])
        .collect();
    points.sort_unstable();
    points.dedup();
    match dir {
        Dir::Left => {
            if cursor == 0 {
                return None;
            }
            let mut start = cursor;
            while start > 0 && cursor - start < min {
                start = points
                    .iter()
                    .rev()
                    .find(|point| **point < start)
                    .copied()
                    .unwrap_or(0);
            }
            Some(start..cursor)
        }
        Dir::Right => {
            if cursor >= len {
                return None;
            }
            let mut end = cursor;
            while end < len && end - cursor < min {
                end = points
                    .iter()
                    .find(|point| **point > end)
                    .copied()
                    .unwrap_or(len);
            }
            Some(cursor..end)
        }
    }
}

/// キャレットを含む文を切り出したもの。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sentence {
    pub text: String,
    /// キャレットの位置（書記素）
    pub cursor: usize,
    /// `text` の先頭の、文書の中の位置（UTF-16）
    pub location: Utf16Len,
}

impl Sentence {
    /// `text` の書記素の範囲を、文書の UTF-16 の範囲にする。
    pub fn utf16_range(&self, range: &Range<usize>) -> Utf16Range {
        let before = grapheme_slice(&self.text, 0, range.start);
        let inside = grapheme_slice(&self.text, range.start, range.end);
        Utf16Range {
            location: Utf16Len(
                self.location
                    .get()
                    .saturating_add(Utf16Len::of(&before).get()),
            ),
            length: Utf16Len::of(&inside),
        }
    }

    pub fn slice(&self, range: &Range<usize>) -> String {
        grapheme_slice(&self.text, range.start, range.end)
    }
}

/// 周辺テキストから、キャレットを含む文を切り出す（ADR 0050 §3.3 の 2）。
///
/// 左は直前の文頭まで、右は文末まで。文の終わりの記号（と続く閉じ括弧）と改行が区切り。キャレットの直前が
/// 文の終わりの記号なら、その記号はその前の文に含め、右は空にする。合わせて `max_chars` を超えるときは左を優先して右を削り、
/// それでも左が超えるならキャレットから `max_chars` まで。`caret_location` はキャレットの文書の中の位置（UTF-16）。
pub fn cut_sentence(
    left: &str,
    right: &str,
    caret_location: Utf16Len,
    max_chars: usize,
) -> Sentence {
    let (body, had_tail) = strip_terminator_tail(left);
    let start = sentence_start(body);
    let left_part = &left[start..];
    let right_part = if had_tail {
        ""
    } else {
        &right[..sentence_end(right)]
    };

    let left_count = left_part.graphemes(true).count();
    let left_final = if left_count > max_chars {
        grapheme_slice(left_part, left_count - max_chars, left_count)
    } else {
        left_part.to_string()
    };
    let left_final_count = left_count.min(max_chars);
    let right_final = grapheme_slice(right_part, 0, max_chars - left_final_count);

    Sentence {
        location: Utf16Len(
            caret_location
                .get()
                .saturating_sub(Utf16Len::of(&left_final).get()),
        ),
        text: format!("{left_final}{right_final}"),
        cursor: left_final_count,
    }
}

/// `left` が文書の絶対位置の左と言えるか。位置の検査（ADR 0050 §7）。
/// 左の長さがキャレットの位置と同じ、または、ホストが左を切っている（文書の長さが左と右の和より長い）ときだけ真。
pub fn left_matches_caret(surrounding: &SurroundingText, caret: Utf16Range) -> bool {
    let left = Utf16Len::of(&surrounding.left).get();
    let right = Utf16Len::of(&surrounding.right).get();
    let location = caret.location.get();
    left == location
        || (left < location && surrounding.document_length.get() > left.saturating_add(right))
}

pub fn is_denied(bundle_id: &str) -> bool {
    CLAUSE_DELETE_DENIED_BUNDLES.contains(&bundle_id)
}

fn is_terminator(c: char) -> bool {
    SENTENCE_TERMINATORS.contains(&c)
}

fn is_closing(c: char) -> bool {
    CLOSING_BRACKETS.contains(&c)
}

fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{000B}' | '\u{000C}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
    )
}

/// 末尾の「文の終わりの記号 + 閉じ括弧」を除いたものと、除いたかどうか。
fn strip_terminator_tail(text: &str) -> (&str, bool) {
    let without_closers = text.trim_end_matches(is_closing);
    let without_terminators = without_closers.trim_end_matches(is_terminator);
    if without_terminators.len() < without_closers.len() {
        (without_terminators, true)
    } else {
        (text, false)
    }
}

/// `text` の中で、最後の文頭（バイトの位置）。区切りは改行と、文の終わりの記号の連なり（と続く閉じ括弧）の直後。
fn sentence_start(text: &str) -> usize {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut start = 0;
    let mut index = 0;
    while index < chars.len() {
        let (offset, c) = chars[index];
        if is_newline(c) {
            start = offset + c.len_utf8();
        } else if is_terminator(c) {
            while index + 1 < chars.len()
                && (is_terminator(chars[index + 1].1) || is_closing(chars[index + 1].1))
            {
                index += 1;
            }
            let (last_offset, last) = chars[index];
            start = last_offset + last.len_utf8();
        }
        index += 1;
    }
    start
}

/// `text`（キャレットの右）の中の文末（バイトの位置）。最初の改行の前、または最初の文の終わりの記号の連なりの後。
fn sentence_end(text: &str) -> usize {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut index = 0;
    while index < chars.len() {
        let (offset, c) = chars[index];
        if is_newline(c) {
            return offset;
        }
        if is_terminator(c) {
            while index + 1 < chars.len()
                && (is_terminator(chars[index + 1].1) || is_closing(chars[index + 1].1))
            {
                index += 1;
            }
            let (last_offset, last) = chars[index];
            return last_offset + last.len_utf8();
        }
        index += 1;
    }
    text.len()
}

/// 書記素の範囲 `[start, end)` の文字列。
pub fn grapheme_slice(text: &str, start: usize, end: usize) -> String {
    text.graphemes(true)
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(
        len: usize,
        bounds: &[usize],
        cursor: usize,
        min: usize,
        dir: Dir,
    ) -> Option<Range<usize>> {
        unit_range(len, bounds, cursor, min, dir)
    }

    // 例: 「俺は｜正しい」（区切り 俺は／正しい）
    #[test]
    fn left_at_the_start_of_the_sentence_stops_even_when_short() {
        assert_eq!(unit(5, &[0, 2, 5], 2, 3, Dir::Left), Some(0..2));
    }

    #[test]
    fn left_takes_the_last_clause_when_long_enough() {
        assert_eq!(unit(5, &[0, 2, 5], 5, 3, Dir::Left), Some(2..5));
    }

    // 例: 「今日は雨が｜降る」（区切り 今日は／雨が／降る）
    #[test]
    fn left_merges_with_the_previous_clause_until_the_minimum() {
        assert_eq!(unit(7, &[0, 3, 5, 7], 5, 3, Dir::Left), Some(0..5));
        assert_eq!(unit(7, &[0, 3, 5, 7], 5, 1, Dir::Left), Some(3..5));
        assert_eq!(unit(7, &[0, 3, 5, 7], 5, 2, Dir::Left), Some(3..5));
    }

    // 例: 「正し｜い」（文節の途中）
    #[test]
    fn a_caret_in_the_middle_of_a_clause_takes_up_to_the_next_boundary_first() {
        assert_eq!(unit(3, &[0, 3], 2, 3, Dir::Left), Some(0..2));
        assert_eq!(unit(3, &[0, 3], 2, 3, Dir::Right), Some(2..3));
        // 左に境界があって最小文字数に満たなければ、さらに合わせる
        assert_eq!(unit(8, &[0, 3, 8], 5, 3, Dir::Left), Some(0..5));
        assert_eq!(unit(8, &[0, 3, 8], 5, 2, Dir::Left), Some(3..5));
    }

    #[test]
    fn right_merges_toward_the_end_of_the_sentence() {
        assert_eq!(unit(7, &[0, 3, 5, 7], 0, 3, Dir::Right), Some(0..3));
        assert_eq!(unit(7, &[0, 3, 5, 7], 3, 3, Dir::Right), Some(3..7));
        assert_eq!(unit(7, &[0, 3, 5, 7], 3, 1, Dir::Right), Some(3..5));
    }

    #[test]
    fn nothing_in_that_direction_is_none() {
        assert_eq!(unit(5, &[0, 5], 0, 3, Dir::Left), None);
        assert_eq!(unit(5, &[0, 5], 5, 3, Dir::Right), None);
        assert_eq!(unit(0, &[], 0, 3, Dir::Left), None);
    }

    #[test]
    fn out_of_range_bounds_and_cursor_do_not_panic() {
        assert_eq!(unit(4, &[9, 2], 99, 1, Dir::Left), Some(2..4));
        assert_eq!(unit(4, &[], 2, 10, Dir::Left), Some(0..2));
    }

    #[test]
    fn boundaries_accumulate_lengths() {
        assert_eq!(boundaries([2, 3]), vec![0, 2, 5]);
        assert_eq!(boundaries([]), vec![0]);
    }

    fn cut(left: &str, right: &str) -> Sentence {
        let location = Utf16Len::of(left);
        cut_sentence(left, right, location, 64)
    }

    #[test]
    fn the_sentence_runs_from_the_previous_terminator_to_the_next() {
        let sentence = cut("前の文。俺は", "正しい。次の文");
        assert_eq!(sentence.text, "俺は正しい。");
        assert_eq!(sentence.cursor, 2);
        assert_eq!(sentence.location, Utf16Len(4));
    }

    #[test]
    fn a_caret_right_after_a_terminator_keeps_it_with_the_previous_sentence() {
        let sentence = cut("前の文。お願いしました。", "次の文");
        assert_eq!(sentence.text, "お願いしました。");
        assert_eq!(sentence.cursor, 8);
        assert_eq!(sentence.location, Utf16Len(4));
    }

    #[test]
    fn closing_brackets_after_a_terminator_belong_to_the_same_sentence() {
        let sentence = cut("彼は「行く。」と言った", "。");
        assert_eq!(sentence.text, "と言った。");
        assert_eq!(cut("「行く。」", "").text, "「行く。」");
    }

    #[test]
    fn newlines_split_sentences() {
        let sentence = cut("一行目\n二行目", "続き\n三行目");
        assert_eq!(sentence.text, "二行目続き");
        let after_newline = cut("一行目\n", "二行目");
        assert_eq!(after_newline.cursor, 0);
        assert_eq!(after_newline.text, "二行目");
    }

    #[test]
    fn the_sentence_is_capped_with_the_left_side_first() {
        let sentence = cut_sentence("あいうえお", "かきくけこ", Utf16Len(5), 7);
        assert_eq!(sentence.text, "あいうえおかき");
        let capped_left = cut_sentence("あいうえお", "かきくけこ", Utf16Len(5), 3);
        assert_eq!(capped_left.text, "うえお");
        assert_eq!(capped_left.cursor, 3);
        assert_eq!(capped_left.location, Utf16Len(2));
    }

    #[test]
    fn utf16_ranges_count_surrogate_pairs() {
        let sentence = cut("𠮷野家", "");
        assert_eq!(sentence.cursor, 3);
        assert_eq!(
            sentence.utf16_range(&(1..3)),
            Utf16Range {
                location: Utf16Len(2),
                length: Utf16Len(2)
            }
        );
    }

    #[test]
    fn graphemes_are_counted_as_one() {
        // 「が」を結合文字で書いたもの、絵文字の家族
        let text = "か\u{3099}x";
        assert_eq!(grapheme_slice(text, 0, 1), "か\u{3099}");
        assert_eq!(unit(2, &[0, 2], 2, 3, Dir::Left), Some(0..2));
    }

    #[test]
    fn caret_position_check() {
        let surrounding = |left: &str, document: u32| SurroundingText {
            left: left.into(),
            right: "あ".into(),
            document_length: Utf16Len(document),
            caret: None,
        };
        let caret = |location: u32| Utf16Range {
            location: Utf16Len(location),
            length: Utf16Len::ZERO,
        };
        assert!(left_matches_caret(&surrounding("ab", 3), caret(2)));
        // 相対位置を返すアプリ（左のほうが長い）
        assert!(!left_matches_caret(&surrounding("abc", 4), caret(1)));
        // ホストが左を切っている
        assert!(left_matches_caret(&surrounding("bc", 10), caret(7)));
        // 切っていないのに位置が合わない
        assert!(!left_matches_caret(&surrounding("bc", 3), caret(7)));
    }

    #[test]
    fn the_deny_list_is_checked_by_bundle_id() {
        assert!(is_denied("com.apple.Terminal"));
        assert!(!is_denied("com.apple.TextEdit"));
    }
}
