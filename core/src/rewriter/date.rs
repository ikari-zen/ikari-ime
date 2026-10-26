//! 日付と時刻（ADR 0002 §12.2）。

use super::era;
use super::{LocalDateTime, RewriteCandidate, RewriteInput, Rewriter};

// 書式ID（固定。学習が書式IDで行われるため、番号を変えない）
const FORMAT_SLASH: u16 = 1; // 2026/10/11
const FORMAT_KANJI_FULL: u16 = 2; // 2026年10月11日
const FORMAT_KANJI_SHORT: u16 = 3; // 10月11日
const FORMAT_REIWA_FULL: u16 = 4; // 令和8年10月11日
const FORMAT_ISO: u16 = 5; // 2026-10-11
const FORMAT_SLASH_WEEKDAY: u16 = 6; // 10/11(日)
const FORMAT_YEAR: u16 = 11; // 2026年
const FORMAT_REIWA_YEAR: u16 = 12; // 令和8年
const FORMAT_TIME_COLON: u16 = 21; // 14:05
const FORMAT_TIME_KANJI: u16 = 22; // 14時05分
const FORMAT_TIME_AMPM: u16 = 23; // 午後2時5分

/// 令和元年の初日
const REIWA_START: (i32, u8, u8) = (2019, 5, 1);
/// 令和の年 = 西暦 - REIWA_YEAR_OFFSET
const REIWA_YEAR_OFFSET: i32 = 2018;
const REIWA_FIRST_YEAR_LABEL: &str = "元";

const WEEKDAY_LABELS: [char; 7] = ['月', '火', '水', '木', '金', '土', '日'];
const DAYS_PER_WEEK: i64 = 7;
const NOON_HOUR: u8 = 12;

/// 読みが指すもの
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    /// 今日から何日ずらすか
    Day(i64),
    /// 今年から何年ずらすか
    Year(i32),
    Time,
}

/// 発火する読みと、その対象
const TRIGGERS: &[(&str, Target)] = &[
    ("きょう", Target::Day(0)),
    ("ほんじつ", Target::Day(0)),
    ("あした", Target::Day(1)),
    ("あす", Target::Day(1)),
    ("きのう", Target::Day(-1)),
    ("あさって", Target::Day(2)),
    ("おととい", Target::Day(-2)),
    ("ことし", Target::Year(0)),
    ("らいねん", Target::Year(1)),
    ("きょねん", Target::Year(-1)),
    ("いま", Target::Time),
];

#[derive(Clone, Copy, Debug, Default)]
pub struct DateRewriter;

impl DateRewriter {
    pub fn new() -> Self {
        Self
    }
}

impl Rewriter for DateRewriter {
    fn name(&self) -> &'static str {
        "date"
    }

    fn rewrite(&self, input: &RewriteInput<'_>, out: &mut Vec<RewriteCandidate>) {
        let Some(&(_, target)) = TRIGGERS.iter().find(|(r, _)| *r == input.reading) else {
            rewrite_era(input.reading, out);
            return;
        };
        let formats = match target {
            Target::Day(offset) => day_formats(input.now, offset),
            Target::Year(offset) => year_formats(input.now.year.saturating_add(offset)),
            Target::Time => time_formats(input.now),
        };
        out.extend(
            formats
                .into_iter()
                .map(|(format, surface)| RewriteCandidate::new(surface, format)),
        );
    }
}

/// 元号 <-> 西暦（年だけ）。読み全体が `<元号><数字>ねん` か `<西暦>ねん` のときだけ働く
fn rewrite_era(reading: &str, out: &mut Vec<RewriteCandidate>) {
    if let Some((format, surface)) = era::era_to_western(reading) {
        out.push(RewriteCandidate::new(surface, format));
    } else if let Some(year) = era::western_year(reading) {
        out.extend(
            year_formats(year)
                .into_iter()
                .map(|(format, surface)| RewriteCandidate::new(surface, format)),
        );
    }
}

pub(super) fn day_formats(now: LocalDateTime, offset: i64) -> Vec<(u16, String)> {
    let Some((y, m, d, weekday)) = shift_days(now, offset) else {
        return Vec::new();
    };
    let mut v = vec![
        (FORMAT_SLASH, format!("{y}/{m}/{d}")),
        (FORMAT_KANJI_FULL, format!("{y}年{m}月{d}日")),
        (FORMAT_KANJI_SHORT, format!("{m}月{d}日")),
    ];
    if let Some(label) = reiwa_label(y, m, d) {
        v.push((FORMAT_REIWA_FULL, format!("{label}{m}月{d}日")));
    }
    v.push((FORMAT_ISO, format!("{y}-{m:02}-{d:02}")));
    v.push((
        FORMAT_SLASH_WEEKDAY,
        format!("{m}/{d}({})", WEEKDAY_LABELS[weekday]),
    ));
    v
}

fn year_formats(year: i32) -> Vec<(u16, String)> {
    let mut v = vec![(FORMAT_YEAR, format!("{year}年"))];
    // 年だけの表記は、その年の末日で令和かどうかを決める（2019年は令和元年）
    if let Some(label) = reiwa_label(year, 12, 31) {
        v.push((FORMAT_REIWA_YEAR, label));
    }
    // 令和より前の元号。境の年は両方（新しい元号が先）
    v.extend(era::old_era_labels(year));
    v
}

fn time_formats(now: LocalDateTime) -> Vec<(u16, String)> {
    let (h, min) = (now.hour, now.minute);
    let (ampm, h12) = if h >= NOON_HOUR {
        ("午後", h - NOON_HOUR)
    } else {
        ("午前", h)
    };
    vec![
        (FORMAT_TIME_COLON, format!("{h}:{min:02}")),
        (FORMAT_TIME_KANJI, format!("{h}時{min:02}分")),
        (FORMAT_TIME_AMPM, format!("{ampm}{h12}時{min}分")),
    ]
}

/// `令和8年` のような年号つきの年。令和より前なら None。
fn reiwa_label(y: i32, m: u8, d: u8) -> Option<String> {
    if (y, m, d) < REIWA_START {
        return None;
    }
    let n = y - REIWA_YEAR_OFFSET;
    Some(if n == 1 {
        format!("令和{REIWA_FIRST_YEAR_LABEL}年")
    } else {
        format!("令和{n}年")
    })
}

/// `now` の日付を `offset` 日ずらす。(年, 月, 日, 曜日 0=月)。範囲外や不正な日付なら None。
fn shift_days(now: LocalDateTime, offset: i64) -> Option<(i32, u8, u8, usize)> {
    if !(1..=12).contains(&now.month) || now.day == 0 || now.weekday > 6 {
        return None;
    }
    let days = days_from_civil(now.year, now.month, now.day).checked_add(offset)?;
    let (y, m, d) = civil_from_days(days);
    let weekday = (i64::from(now.weekday) + offset).rem_euclid(DAYS_PER_WEEK) as usize;
    Some((y, m, d, weekday))
}

// 暦の計算は H. Hinnant の civil_from_days / days_from_civil（1970-01-01 = 0）

fn days_from_civil(y: i32, m: u8, d: u8) -> i64 {
    let (y, m, d) = (i64::from(y), i64::from(m), i64::from(d));
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i32, u8, u8) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;

    fn at(year: i32, month: u8, day: u8, weekday: u8) -> LocalDateTime {
        LocalDateTime {
            year,
            month,
            day,
            hour: 14,
            minute: 5,
            weekday,
        }
    }

    fn run(reading: &str, now: LocalDateTime) -> Vec<(u16, String)> {
        let ctx = Context::new();
        let input = RewriteInput {
            reading,
            context: &ctx,
            now,
        };
        let mut out = Vec::new();
        DateRewriter::new().rewrite(&input, &mut out);
        out.into_iter().map(|c| (c.format, c.surface)).collect()
    }

    fn surfaces(v: &[(u16, String)]) -> Vec<&str> {
        v.iter().map(|(_, s)| s.as_str()).collect()
    }

    // 2026-10-11 は日曜日
    const SUNDAY: u8 = 6;

    #[test]
    fn today_has_all_six_formats_in_order() {
        let v = run("きょう", at(2026, 10, 11, SUNDAY));
        assert_eq!(
            v,
            vec![
                (1, "2026/10/11".to_owned()),
                (2, "2026年10月11日".to_owned()),
                (3, "10月11日".to_owned()),
                (4, "令和8年10月11日".to_owned()),
                (5, "2026-10-11".to_owned()),
                (6, "10/11(日)".to_owned()),
            ]
        );
    }

    #[test]
    fn synonyms_of_today() {
        let now = at(2026, 10, 11, SUNDAY);
        assert_eq!(run("ほんじつ", now), run("きょう", now));
        assert_eq!(run("あす", now), run("あした", now));
    }

    #[test]
    fn relative_days_shift_date_and_weekday() {
        let now = at(2026, 10, 11, SUNDAY);
        assert_eq!(surfaces(&run("あした", now))[0], "2026/10/12");
        assert_eq!(surfaces(&run("あした", now))[5], "10/12(月)");
        assert_eq!(surfaces(&run("きのう", now))[0], "2026/10/10");
        assert_eq!(surfaces(&run("きのう", now))[5], "10/10(土)");
        assert_eq!(surfaces(&run("あさって", now))[0], "2026/10/13");
        assert_eq!(surfaces(&run("おととい", now))[0], "2026/10/9");
    }

    #[test]
    fn month_and_year_boundaries() {
        // 大晦日 -> 元日
        let nye = at(2026, 12, 31, 3);
        let v = run("あした", nye);
        assert_eq!(surfaces(&v)[0], "2027/1/1");
        assert_eq!(surfaces(&v)[4], "2027-01-01");
        assert_eq!(surfaces(&v)[5], "1/1(金)");
        // 元日 -> 前年末
        let v = run("きのう", at(2027, 1, 1, 4));
        assert_eq!(surfaces(&v)[0], "2026/12/31");
        // 月末
        assert_eq!(surfaces(&run("あした", at(2026, 4, 30, 3)))[0], "2026/5/1");
        assert_eq!(
            surfaces(&run("おととい", at(2026, 3, 1, 6)))[0],
            "2026/2/27"
        );
    }

    #[test]
    fn leap_year_february() {
        assert_eq!(surfaces(&run("あした", at(2028, 2, 28, 0)))[0], "2028/2/29");
        assert_eq!(surfaces(&run("あした", at(2028, 2, 29, 1)))[0], "2028/3/1");
        // 2100 年は閏年ではない
        assert_eq!(surfaces(&run("あした", at(2100, 2, 28, 6)))[0], "2100/3/1");
        // 2000 年は閏年
        assert_eq!(surfaces(&run("あした", at(2000, 2, 28, 0)))[0], "2000/2/29");
    }

    #[test]
    fn weekday_wraps_around() {
        // 月曜の「きのう」は日曜
        let v = run("きのう", at(2026, 10, 12, 0));
        assert_eq!(surfaces(&v)[5], "10/11(日)");
        // 日曜の「あさって」は火曜
        let v = run("あさって", at(2026, 10, 11, SUNDAY));
        assert_eq!(surfaces(&v)[5], "10/13(火)");
    }

    #[test]
    fn reiwa_boundary_is_2019_05_01() {
        // 令和元年 5/1 は令和
        let v = run("きょう", at(2019, 5, 1, 2));
        assert_eq!(surfaces(&v)[3], "令和元年5月1日");
        // 前日 4/30 は令和ではないので、書式4を出さない
        let v = run("きょう", at(2019, 4, 30, 1));
        assert!(v.iter().all(|(f, _)| *f != FORMAT_REIWA_FULL), "{v:?}");
        assert_eq!(v.len(), 5);
        // 「あした」で境界をまたぐ
        let v = run("あした", at(2019, 4, 30, 1));
        assert_eq!(surfaces(&v)[3], "令和元年5月1日");
        // 令和2年
        let v = run("きょう", at(2020, 1, 1, 2));
        assert_eq!(surfaces(&v)[3], "令和2年1月1日");
    }

    #[test]
    fn padded_and_unpadded_numbers() {
        let v = run("きょう", at(2026, 1, 5, 0));
        assert_eq!(surfaces(&v)[0], "2026/1/5");
        assert_eq!(surfaces(&v)[2], "1月5日");
        assert_eq!(surfaces(&v)[4], "2026-01-05");
    }

    #[test]
    fn year_readings() {
        let now = at(2026, 10, 11, SUNDAY);
        assert_eq!(
            run("ことし", now),
            vec![(11, "2026年".to_owned()), (12, "令和8年".to_owned())]
        );
        assert_eq!(
            run("らいねん", now),
            vec![(11, "2027年".to_owned()), (12, "令和9年".to_owned())]
        );
        assert_eq!(
            run("きょねん", now),
            vec![(11, "2025年".to_owned()), (12, "令和7年".to_owned())]
        );
    }

    #[test]
    fn year_around_reiwa_start() {
        let v = run("ことし", at(2019, 6, 1, 5));
        assert_eq!(v[1].1, "令和元年");
        let v = run("きょねん", at(2019, 6, 1, 5));
        assert_eq!(surfaces(&v), vec!["2018年", "平成30年"]);
    }

    #[test]
    fn time_formats_afternoon_morning_noon_midnight() {
        let mut now = at(2026, 10, 11, SUNDAY);
        let v = run("いま", now);
        assert_eq!(
            v,
            vec![
                (21, "14:05".to_owned()),
                (22, "14時05分".to_owned()),
                (23, "午後2時5分".to_owned()),
            ]
        );
        now.hour = 0;
        now.minute = 0;
        assert_eq!(
            surfaces(&run("いま", now)),
            vec!["0:00", "0時00分", "午前0時0分"]
        );
        now.hour = 12;
        now.minute = 30;
        assert_eq!(
            surfaces(&run("いま", now)),
            vec!["12:30", "12時30分", "午後0時30分"]
        );
        now.hour = 23;
        now.minute = 59;
        assert_eq!(surfaces(&run("いま", now))[2], "午後11時59分");
        now.hour = 11;
        assert_eq!(surfaces(&run("いま", now))[2], "午前11時59分");
    }

    #[test]
    fn unrelated_readings_produce_nothing() {
        let now = at(2026, 10, 11, SUNDAY);
        for r in [
            "",
            "きょ",
            "きょうは",
            "今日",
            "あしたは",
            "いまい",
            "ひんし",
        ] {
            assert!(run(r, now).is_empty(), "{r}");
        }
    }

    #[test]
    fn invalid_clock_values_do_not_panic() {
        let bad = LocalDateTime {
            year: 2026,
            month: 13,
            day: 0,
            hour: 0,
            minute: 0,
            weekday: 9,
        };
        assert!(run("きょう", bad).is_empty());
        let extreme = at(i32::MAX, 12, 31, 0);
        let _ = run("あした", extreme);
        let _ = run("ことし", extreme);
        let _ = run("らいねん", extreme);
    }

    #[test]
    fn civil_round_trip() {
        for days in [-800_000_i64, -1, 0, 1, 11_000, 20_372, 2_932_896] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(2026, 10, 11), 20_737);
    }

    fn era(reading: &str) -> Vec<String> {
        run(reading, at(2026, 10, 11, SUNDAY))
            .into_iter()
            .map(|(_, s)| s)
            .collect()
    }

    #[test]
    fn era_to_western_years() {
        assert_eq!(era("れいわ8ねん"), ["2026年"]);
        assert_eq!(era("へいせい23ねん"), ["2011年"]);
        assert_eq!(era("しょうわ64ねん"), ["1989年"]);
        assert_eq!(era("たいしょう15ねん"), ["1926年"]);
        assert_eq!(era("めいじ45ねん"), ["1912年"]);
        assert_eq!(era("めいじ1ねん"), ["1868年"]);
        assert_eq!(era("へいせいがんねん"), ["1989年"]);
        assert_eq!(era("れいわがんねん"), ["2019年"]);
        assert_eq!(
            run("れいわ8ねん", at(2026, 10, 11, SUNDAY))[0].0,
            era::FORMAT_ERA_TO_WESTERN
        );
    }

    #[test]
    fn era_to_western_fullwidth_digits() {
        assert_eq!(era("れいわ８ねん"), ["2026年"]);
        assert_eq!(era("へいせい２３ねん"), ["2011年"]);
    }

    #[test]
    fn era_to_western_rejects_out_of_range_and_malformed() {
        for r in [
            "へいせい32ねん", // 平成は31年まで
            "しょうわ65ねん",
            "たいしょう16ねん",
            "めいじ46ねん",
            "れいわ0ねん",
            "れいわねん",      // 数字なし
            "れいわ12345ねん", // 桁が多い
            "れいわ8",         // ねん なし
            "れいわ8ねんど",
            "れいわ8ねんに",
            "へいせいがん",
            "れいわがん1ねん",
            "れいわ8ねん ",
            "れいわ-1ねん",
        ] {
            assert!(run(r, at(2026, 10, 11, SUNDAY)).is_empty(), "{r}");
        }
    }

    #[test]
    fn western_to_era_plain() {
        assert_eq!(era("2011ねん"), ["2011年", "平成23年"]);
        assert_eq!(era("2026ねん"), ["2026年", "令和8年"]);
        assert_eq!(era("1950ねん"), ["1950年", "昭和25年"]);
        assert_eq!(era("1920ねん"), ["1920年", "大正9年"]);
        assert_eq!(era("1900ねん"), ["1900年", "明治33年"]);
        assert_eq!(era("２０１１ねん"), ["2011年", "平成23年"]);
    }

    #[test]
    fn western_to_era_boundary_years_give_both() {
        // 年末の元号が先
        assert_eq!(era("2019ねん"), ["2019年", "令和元年", "平成31年"]);
        assert_eq!(era("1989ねん"), ["1989年", "平成元年", "昭和64年"]);
        assert_eq!(era("1926ねん"), ["1926年", "昭和元年", "大正15年"]);
        assert_eq!(era("1912ねん"), ["1912年", "大正元年", "明治45年"]);
        // 1868 は明治元年だけ（それより前は元号を出さない）
        assert_eq!(era("1868ねん"), ["1868年", "明治元年"]);
        assert_eq!(era("1867ねん"), ["1867年"]);
    }

    #[test]
    fn western_to_era_rejects_malformed() {
        for r in [
            "201ねん",
            "2011",
            "2011ねんど",
            "20111ねん",
            "ねん",
            "20a1ねん",
        ] {
            let v = era(r);
            // 3桁の 201 は年として読めるが元号は出ない
            assert!(
                v.iter().all(|s| !s.contains('平') && !s.contains('令')),
                "{r}"
            );
            if r != "201ねん" {
                assert!(v.is_empty(), "{r}: {v:?}");
            }
        }
    }

    #[test]
    fn era_format_ids_are_stable() {
        assert_eq!(
            [era::FORMAT_ERA_TO_WESTERN, era::FORMAT_OLD_ERA_YEAR],
            [13, 14]
        );
    }

    #[test]
    fn format_ids_are_stable() {
        // 学習が書式IDで行われるので、値を変えてはいけない
        assert_eq!(
            [
                FORMAT_SLASH,
                FORMAT_KANJI_FULL,
                FORMAT_KANJI_SHORT,
                FORMAT_REIWA_FULL,
                FORMAT_ISO,
                FORMAT_SLASH_WEEKDAY,
                FORMAT_YEAR,
                FORMAT_REIWA_YEAR,
                FORMAT_TIME_COLON,
                FORMAT_TIME_KANJI,
                FORMAT_TIME_AMPM,
            ],
            [1, 2, 3, 4, 5, 6, 11, 12, 21, 22, 23]
        );
    }
}
