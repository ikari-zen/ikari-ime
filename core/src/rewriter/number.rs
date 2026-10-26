//! 数字の書式（ADR 0002 §12.2）。

use super::{RewriteCandidate, RewriteInput, Rewriter};

// 書式ID（固定）
const FORMAT_COMMA: u16 = 1; // 1,234
const FORMAT_KANJI: u16 = 2; // 千二百三十四
const FORMAT_FULLWIDTH: u16 = 3; // １２３４
const FORMAT_KANJI_DIGITS: u16 = 4; // 一二三四

const DECIMAL_POINT: char = '.';
const FULLWIDTH_DECIMAL_POINT: char = '．';
const KANJI_DECIMAL_POINT: char = '・';
const COMMA_GROUP: usize = 3;
const FULLWIDTH_ZERO: u32 = '０' as u32;
/// 漢数字に直す整数部の桁数の上限（u128 に収まる 38 桁より小さく、京より上の単位は使わない）
const MAX_KANJI_INTEGER_DIGITS: usize = 20;
/// 数字として扱う読みの長さの上限（病的な入力で重くしない）
const MAX_DIGITS: usize = 64;

const KANJI_DIGITS: [char; 10] = ['〇', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
const SMALL_UNITS: [&str; 4] = ["", "十", "百", "千"];
/// 4桁ごとの大きな単位（添字 0 = 一の位の組）
const BIG_UNITS: [&str; 5] = ["", "万", "億", "兆", "京"];
const KANJI_ZERO_WORD: &str = "零";

#[derive(Clone, Copy, Debug, Default)]
pub struct NumberRewriter;

impl NumberRewriter {
    pub fn new() -> Self {
        Self
    }
}

impl Rewriter for NumberRewriter {
    fn name(&self) -> &'static str {
        "number"
    }

    fn rewrite(&self, input: &RewriteInput<'_>, out: &mut Vec<RewriteCandidate>) {
        let Some(num) = Number::parse(input.reading) else {
            return;
        };
        let mut push = |format: u16, surface: Option<String>| {
            if let Some(surface) = surface
                && !out.iter().any(|c| c.surface == surface)
            {
                out.push(RewriteCandidate::new(surface, format));
            }
        };
        push(FORMAT_COMMA, num.comma());
        push(FORMAT_KANJI, num.kanji());
        push(FORMAT_FULLWIDTH, Some(num.fullwidth()));
        push(FORMAT_KANJI_DIGITS, Some(num.kanji_digits()));
    }
}

/// 数字だけの読み（ASCII か全角。小数点は1つまで）を ASCII に直したもの
struct Number {
    integer: String,
    fraction: Option<String>,
}

impl Number {
    fn parse(reading: &str) -> Option<Self> {
        if reading.is_empty() || reading.chars().count() > MAX_DIGITS {
            return None;
        }
        let mut integer = String::new();
        let mut fraction: Option<String> = None;
        for c in reading.chars() {
            let target = match fraction.as_mut() {
                Some(f) => f,
                None => &mut integer,
            };
            if let Some(d) = ascii_digit(c) {
                target.push(d);
            } else if (c == DECIMAL_POINT || c == FULLWIDTH_DECIMAL_POINT) && fraction.is_none() {
                fraction = Some(String::new());
            } else {
                return None;
            }
        }
        // 「.5」「1.」のように、小数点の前後のどちらかが空なら数字として扱わない
        if integer.is_empty() || fraction.as_deref() == Some("") {
            return None;
        }
        Some(Self { integer, fraction })
    }

    /// 先頭が 0 の整数（`007` など）。桁区切りと位取りの漢数字では意味が変わるので使わない
    fn has_leading_zero(&self) -> bool {
        self.integer.len() > 1 && self.integer.starts_with('0')
    }

    fn comma(&self) -> Option<String> {
        if self.has_leading_zero() {
            return None;
        }
        let mut s = group_digits(&self.integer);
        if let Some(f) = &self.fraction {
            s.push(DECIMAL_POINT);
            s.push_str(f);
        }
        Some(s)
    }

    fn kanji(&self) -> Option<String> {
        if self.has_leading_zero() || self.integer.len() > MAX_KANJI_INTEGER_DIGITS {
            return None;
        }
        let mut s = kanji_integer(&self.integer)?;
        if let Some(f) = &self.fraction {
            s.push(KANJI_DECIMAL_POINT);
            s.extend(f.chars().filter_map(kanji_digit));
        }
        Some(s)
    }

    fn fullwidth(&self) -> String {
        let conv = |c: char| match ascii_digit(c) {
            Some(d) => char::from_u32(FULLWIDTH_ZERO + u32::from(d) - u32::from('0')).unwrap_or(d),
            None => c,
        };
        let mut s: String = self.integer.chars().map(conv).collect();
        if let Some(f) = &self.fraction {
            s.push(FULLWIDTH_DECIMAL_POINT);
            s.extend(f.chars().map(conv));
        }
        s
    }

    fn kanji_digits(&self) -> String {
        let mut s: String = self.integer.chars().filter_map(kanji_digit).collect();
        if let Some(f) = &self.fraction {
            s.push(KANJI_DECIMAL_POINT);
            s.extend(f.chars().filter_map(kanji_digit));
        }
        s
    }
}

/// ASCII か全角の数字を ASCII の数字にする
fn ascii_digit(c: char) -> Option<char> {
    match c {
        '0'..='9' => Some(c),
        '０'..='９' => char::from_u32(c as u32 - FULLWIDTH_ZERO + u32::from('0')),
        _ => None,
    }
}

fn kanji_digit(c: char) -> Option<char> {
    c.to_digit(10).map(|d| KANJI_DIGITS[d as usize])
}

/// `1234567` -> `1,234,567`
fn group_digits(digits: &str) -> String {
    let n = digits.len();
    let mut s = String::with_capacity(n + n / COMMA_GROUP);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (n - i).is_multiple_of(COMMA_GROUP) {
            s.push(',');
        }
        s.push(c);
    }
    s
}

/// 整数（ASCII の数字、先頭の 0 なし）を、位取りの漢数字にする。
/// 千・百・十の前の「一」は省く（`千二百`、`十`）。万以上の単位の前は省かない（`一万`）。
fn kanji_integer(digits: &str) -> Option<String> {
    if digits.chars().all(|c| c == '0') {
        return Some(KANJI_ZERO_WORD.to_owned());
    }
    let chars: Vec<u32> = digits
        .chars()
        .map(|c| c.to_digit(10))
        .collect::<Option<_>>()?;
    // 4桁ずつの組（下位から）
    let groups: Vec<&[u32]> = {
        let mut gs = Vec::new();
        let mut end = chars.len();
        while end > 0 {
            let start = end.saturating_sub(4);
            gs.push(&chars[start..end]);
            end = start;
        }
        gs
    };
    if groups.len() > BIG_UNITS.len() {
        return None;
    }
    let mut s = String::new();
    for (gi, g) in groups.iter().enumerate().rev() {
        let value = g.iter().fold(0u32, |acc, d| acc * 10 + d);
        if value == 0 {
            continue;
        }
        s.push_str(&kanji_group(g));
        s.push_str(BIG_UNITS[gi]);
    }
    Some(s)
}

/// 1〜4桁の組（上位が先）を漢数字にする
fn kanji_group(g: &[u32]) -> String {
    let mut s = String::new();
    let len = g.len();
    for (i, &d) in g.iter().enumerate() {
        if d == 0 {
            continue;
        }
        let unit = len - 1 - i;
        if !(d == 1 && unit > 0) {
            s.push(KANJI_DIGITS[d as usize]);
        }
        s.push_str(SMALL_UNITS[unit]);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use crate::rewriter::LocalDateTime;

    fn run(reading: &str) -> Vec<(u16, String)> {
        let ctx = Context::new();
        let input = RewriteInput {
            reading,
            context: &ctx,
            now: LocalDateTime {
                year: 2026,
                month: 10,
                day: 11,
                hour: 0,
                minute: 0,
                weekday: 6,
            },
        };
        let mut out = Vec::new();
        NumberRewriter::new().rewrite(&input, &mut out);
        out.into_iter().map(|c| (c.format, c.surface)).collect()
    }

    fn kanji(s: &str) -> String {
        kanji_integer(s).unwrap()
    }

    #[test]
    fn spec_example_1234() {
        assert_eq!(
            run("1234"),
            vec![
                (1, "1,234".to_owned()),
                (2, "千二百三十四".to_owned()),
                (3, "１２３４".to_owned()),
                (4, "一二三四".to_owned()),
            ]
        );
    }

    #[test]
    fn comma_grouping_boundaries() {
        let comma = |r: &str| run(r).into_iter().find(|(f, _)| *f == 1).map(|(_, s)| s);
        assert_eq!(comma("0").as_deref(), Some("0"));
        assert_eq!(comma("999").as_deref(), Some("999"));
        assert_eq!(comma("1000").as_deref(), Some("1,000"));
        assert_eq!(comma("12345").as_deref(), Some("12,345"));
        assert_eq!(comma("123456").as_deref(), Some("123,456"));
        assert_eq!(comma("1234567").as_deref(), Some("1,234,567"));
        assert_eq!(comma("1000000").as_deref(), Some("1,000,000"));
    }

    #[test]
    fn kanji_numerals() {
        assert_eq!(kanji("0"), "零");
        assert_eq!(kanji("1"), "一");
        assert_eq!(kanji("10"), "十");
        assert_eq!(kanji("11"), "十一");
        assert_eq!(kanji("20"), "二十");
        assert_eq!(kanji("100"), "百");
        assert_eq!(kanji("101"), "百一");
        assert_eq!(kanji("1000"), "千");
        assert_eq!(kanji("1001"), "千一");
        assert_eq!(kanji("1234"), "千二百三十四");
        assert_eq!(kanji("9999"), "九千九百九十九");
        assert_eq!(kanji("10000"), "一万");
        assert_eq!(kanji("10001"), "一万一");
        assert_eq!(kanji("12345"), "一万二千三百四十五");
        assert_eq!(kanji("100000"), "十万");
        assert_eq!(kanji("110000"), "十一万");
        assert_eq!(kanji("1000000"), "百万");
        assert_eq!(kanji("20000000"), "二千万");
        assert_eq!(kanji("100000000"), "一億");
        assert_eq!(kanji("100010000"), "一億一万");
        assert_eq!(kanji("1000000000000"), "一兆");
        assert_eq!(kanji("10000000000000000"), "一京");
        assert_eq!(kanji("123456789"), "一億二千三百四十五万六千七百八十九");
    }

    #[test]
    fn kanji_skips_too_large() {
        // 京より上の単位は使わない
        let big = format!("1{}", "0".repeat(20));
        let got = run(&big);
        assert!(got.iter().all(|(f, _)| *f != FORMAT_KANJI), "{got:?}");
        assert!(got.iter().any(|(f, _)| *f == FORMAT_COMMA));
        let ok = format!("9{}", "0".repeat(15));
        assert!(run(&ok).iter().any(|(f, _)| *f == FORMAT_KANJI));
    }

    #[test]
    fn fullwidth_input_is_accepted() {
        let got = run("１２３４");
        assert_eq!(got[0], (1, "1,234".to_owned()));
        assert_eq!(got[1], (2, "千二百三十四".to_owned()));
        // 読みと同じ表記も出す（読みと同じ候補の除去は Converter が行う）
        assert_eq!(got[2], (3, "１２３４".to_owned()));
        assert_eq!(got[3], (4, "一二三四".to_owned()));
    }

    #[test]
    fn single_digit_deduplicates_identical_surfaces() {
        // 「5」: 桁区切りは「5」、漢数字は「五」、全角は「５」、位取りは「五」（漢数字と同じなので落ちる）
        assert_eq!(
            run("5"),
            vec![
                (1, "5".to_owned()),
                (2, "五".to_owned()),
                (3, "５".to_owned())
            ]
        );
    }

    #[test]
    fn decimals() {
        assert_eq!(
            run("3.14"),
            vec![
                (1, "3.14".to_owned()),
                (2, "三・一四".to_owned()),
                (3, "３．１４".to_owned()),
            ]
        );
        let got = run("1234.5");
        assert_eq!(got[0].1, "1,234.5");
        assert_eq!(got[1].1, "千二百三十四・五");
        assert_eq!(got[2].1, "１２３４．５");
        assert_eq!(got[3].1, "一二三四・五");
        assert_eq!(run("0.5")[0].1, "0.5");
    }

    #[test]
    fn leading_zeros_skip_grouping_and_numerals() {
        let got = run("007");
        assert_eq!(
            got,
            vec![(3, "００７".to_owned()), (4, "〇〇七".to_owned())]
        );
        // 「0」と「0.5」は先頭の 0 でも通常どおり
        assert!(!run("0").is_empty());
        assert!(run("0.5").iter().any(|(f, _)| *f == FORMAT_COMMA));
    }

    #[test]
    fn rejects_non_numbers() {
        for r in [
            "",
            "a",
            "12a",
            "1.2.3",
            ".5",
            "1.",
            ".",
            "-1",
            "1,234",
            "いち",
            "1 2",
            "１２３abc",
        ] {
            assert!(run(r).is_empty(), "{r:?} -> {:?}", run(r));
        }
        let long = "1".repeat(MAX_DIGITS + 1);
        assert!(run(&long).is_empty());
    }

    #[test]
    fn longest_accepted_input_does_not_panic() {
        let long = "9".repeat(MAX_DIGITS);
        let got = run(&long);
        assert!(got.iter().any(|(f, _)| *f == FORMAT_COMMA));
    }

    #[test]
    fn format_ids_are_stable() {
        assert_eq!(
            [
                FORMAT_COMMA,
                FORMAT_KANJI,
                FORMAT_FULLWIDTH,
                FORMAT_KANJI_DIGITS
            ],
            [1, 2, 3, 4]
        );
    }
}
