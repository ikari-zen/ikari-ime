//! 元号と西暦の相互変換（明治〜令和。年だけ。issue #262）。
//!
//! - 元号 → 西暦: `れいわ8ねん` → `2026年`、`へいせいがんねん` → `1989年`
//! - 西暦 → 元号（令和より前）: `2011ねん` → `平成23年`。境の年は両方の元号を出す
//!   （令和は date.rs の `reiwa_label` が出す）
//!
//! 読み全体が形に合うときだけ働く（補完はしない）。

// 書式ID（固定。date.rs の書式IDと重ならない値を使う）
pub(super) const FORMAT_ERA_TO_WESTERN: u16 = 13; // れいわ8ねん -> 2026年
pub(super) const FORMAT_OLD_ERA_YEAR: u16 = 14; // 2011年 -> 平成23年（令和より前の元号）

pub(super) const YEAR_SUFFIX_READING: &str = "ねん";
const FIRST_YEAR_READING: &str = "がん";
pub(super) const FIRST_YEAR_LABEL: &str = "元";
/// 年の数字の最大桁数
const MAX_YEAR_DIGITS: usize = 4;
const FULLWIDTH_ZERO: u32 = '０' as u32;

struct Era {
    reading: &'static str,
    label: &'static str,
    /// 元年の西暦
    first_year: i32,
    /// 最後の年の西暦（改元の年を含む）。None なら現在も続く
    last_year: Option<i32>,
}

/// 新しい順
const ERAS: &[Era] = &[
    Era {
        reading: "れいわ",
        label: "令和",
        first_year: 2019,
        last_year: None,
    },
    Era {
        reading: "へいせい",
        label: "平成",
        first_year: 1989,
        last_year: Some(2019),
    },
    Era {
        reading: "しょうわ",
        label: "昭和",
        first_year: 1926,
        last_year: Some(1989),
    },
    Era {
        reading: "たいしょう",
        label: "大正",
        first_year: 1912,
        last_year: Some(1926),
    },
    Era {
        reading: "めいじ",
        label: "明治",
        first_year: 1868,
        last_year: Some(1912),
    },
];

/// 令和は date.rs が出すので、ここでは扱わない
const REIWA_INDEX: usize = 0;

/// `<元号の読み><数字|がん>ねん` → `<西暦>年`
pub(super) fn era_to_western(reading: &str) -> Option<(u16, String)> {
    let body = reading.strip_suffix(YEAR_SUFFIX_READING)?;
    let era = ERAS.iter().find(|e| body.starts_with(e.reading))?;
    let rest = &body[era.reading.len()..];
    let n = if rest == FIRST_YEAR_READING {
        1
    } else {
        parse_year_digits(rest)?
    };
    if n < 1 {
        return None;
    }
    let year = era.first_year.checked_add(n - 1)?;
    if era.last_year.is_some_and(|last| year > last) {
        return None;
    }
    Some((FORMAT_ERA_TO_WESTERN, format!("{year}年")))
}

/// 西暦の年が含まれる、令和より前の元号の表記。新しい元号が先（境の年は2つ）。
/// 範囲（1868 より前）の外なら空。
pub(super) fn old_era_labels(year: i32) -> Vec<(u16, String)> {
    ERAS.iter()
        .enumerate()
        .filter(|(i, e)| {
            *i != REIWA_INDEX && year >= e.first_year && e.last_year.is_none_or(|last| year <= last)
        })
        .map(|(_, e)| {
            let n = year - e.first_year + 1;
            let label = if n == 1 {
                format!("{}{FIRST_YEAR_LABEL}年", e.label)
            } else {
                format!("{}{n}年", e.label)
            };
            (FORMAT_OLD_ERA_YEAR, label)
        })
        .collect()
}

/// `<数字>ねん` の数字（半角・全角。1〜MAX_YEAR_DIGITS 桁）
pub(super) fn western_year(reading: &str) -> Option<i32> {
    parse_year_digits(reading.strip_suffix(YEAR_SUFFIX_READING)?)
}

fn parse_year_digits(s: &str) -> Option<i32> {
    let count = s.chars().count();
    if count == 0 || count > MAX_YEAR_DIGITS {
        return None;
    }
    let mut n: i32 = 0;
    for c in s.chars() {
        let d = match c {
            '0'..='9' => c as u32 - '0' as u32,
            '０'..='９' => c as u32 - FULLWIDTH_ZERO,
            _ => return None,
        };
        n = n * 10 + d as i32;
    }
    Some(n)
}
