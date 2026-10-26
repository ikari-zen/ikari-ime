//! 曜日の読みに、その曜日の直近の日付の候補を足す（設定 `date.weekday_dates`。既定 off。#265）。
//!
//! 直近は、今日を含めて次のその曜日（今日が月曜なら「げつようび」は今日）。
//! 書式は「きょう」の候補と同じ組（`date::day_formats`。書式IDも同じ）。
//! 読み全体が曜日の語に一致したときだけ働く。補完はしない。

use super::date::day_formats;
use super::{RewriteCandidate, RewriteInput, Rewriter};

/// 曜日の読み。添字が `LocalDateTime::weekday`（0=月）に対応する
const WEEKDAY_READINGS: [&str; 7] = [
    "げつようび",
    "かようび",
    "すいようび",
    "もくようび",
    "きんようび",
    "どようび",
    "にちようび",
];
const DAYS_PER_WEEK: i64 = 7;

/// 設定から決まる動作（`date.weekday_dates`）
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DateOptions {
    /// `date.weekday_dates`（既定 off）
    pub weekday_dates: bool,
}

pub struct WeekdayRewriter {
    options: DateOptions,
}

impl WeekdayRewriter {
    pub fn new(options: DateOptions) -> Self {
        Self { options }
    }
}

impl Rewriter for WeekdayRewriter {
    fn name(&self) -> &'static str {
        "weekday"
    }

    fn rewrite(&self, input: &RewriteInput<'_>, out: &mut Vec<RewriteCandidate>) {
        if !self.options.weekday_dates {
            return;
        }
        let Some(target) = WEEKDAY_READINGS.iter().position(|r| *r == input.reading) else {
            return;
        };
        // 不正な時計の値は day_formats が空を返す
        let offset = (target as i64 - i64::from(input.now.weekday)).rem_euclid(DAYS_PER_WEEK);
        out.extend(
            day_formats(input.now, offset)
                .into_iter()
                .map(|(format, surface)| RewriteCandidate::new(surface, format)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use crate::rewriter::LocalDateTime;

    fn at(year: i32, month: u8, day: u8, weekday: u8) -> LocalDateTime {
        LocalDateTime {
            year,
            month,
            day,
            hour: 9,
            minute: 0,
            weekday,
        }
    }

    fn run_with(on: bool, reading: &str, now: LocalDateTime) -> Vec<(u16, String)> {
        let ctx = Context::new();
        let input = RewriteInput {
            reading,
            context: &ctx,
            now,
        };
        let mut out = Vec::new();
        WeekdayRewriter::new(DateOptions { weekday_dates: on }).rewrite(&input, &mut out);
        out.into_iter().map(|c| (c.format, c.surface)).collect()
    }

    fn first(reading: &str, now: LocalDateTime) -> String {
        run_with(true, reading, now)
            .into_iter()
            .next()
            .map(|(_, s)| s)
            .unwrap_or_default()
    }

    // 2026-10-26 は月曜日
    const MONDAY: u8 = 0;

    #[test]
    fn today_is_included() {
        let v = run_with(true, "げつようび", at(2026, 10, 26, MONDAY));
        assert_eq!(
            v,
            vec![
                (1, "2026/10/26".to_owned()),
                (2, "2026年10月26日".to_owned()),
                (3, "10月26日".to_owned()),
                (4, "令和8年10月26日".to_owned()),
                (5, "2026-10-26".to_owned()),
                (6, "10/26(月)".to_owned()),
            ]
        );
    }

    #[test]
    fn day_before_and_after() {
        // 前日（日曜）なら翌日
        assert_eq!(first("げつようび", at(2026, 10, 25, 6)), "2026/10/26");
        // 翌日（火曜）なら6日後
        assert_eq!(first("げつようび", at(2026, 10, 27, 1)), "2026/11/2");
    }

    #[test]
    fn all_seven_weekdays_from_monday() {
        let now = at(2026, 10, 26, MONDAY);
        let expected = [
            ("げつようび", "2026/10/26", "10/26(月)"),
            ("かようび", "2026/10/27", "10/27(火)"),
            ("すいようび", "2026/10/28", "10/28(水)"),
            ("もくようび", "2026/10/29", "10/29(木)"),
            ("きんようび", "2026/10/30", "10/30(金)"),
            ("どようび", "2026/10/31", "10/31(土)"),
            ("にちようび", "2026/11/1", "11/1(日)"),
        ];
        for (reading, slash, wd) in expected {
            let v = run_with(true, reading, now);
            assert_eq!(v[0].1, slash, "{reading}");
            assert_eq!(v[5].1, wd, "{reading}");
        }
    }

    #[test]
    fn all_seven_weekdays_from_sunday() {
        // 日曜（2026-10-25）から。にちようびは今日、げつようびは翌日
        let now = at(2026, 10, 25, 6);
        assert_eq!(first("にちようび", now), "2026/10/25");
        assert_eq!(first("げつようび", now), "2026/10/26");
        assert_eq!(first("かようび", now), "2026/10/27");
        assert_eq!(first("すいようび", now), "2026/10/28");
        assert_eq!(first("もくようび", now), "2026/10/29");
        assert_eq!(first("きんようび", now), "2026/10/30");
        assert_eq!(first("どようび", now), "2026/10/31");
    }

    #[test]
    fn crosses_month_and_year_end() {
        // 2026-12-31 は木曜。げつようびは 2027-01-04
        let v = run_with(true, "げつようび", at(2026, 12, 31, 3));
        assert_eq!(v[0].1, "2027/1/4");
        assert_eq!(v[4].1, "2027-01-04");
        assert_eq!(v[5].1, "1/4(月)");
        // 2026-04-30（木）の金曜は 5/1
        assert_eq!(first("きんようび", at(2026, 4, 30, 3)), "2026/5/1");
        // 閏日（2028-02-28 は月曜）の火曜
        assert_eq!(first("かようび", at(2028, 2, 28, 0)), "2028/2/29");
    }

    #[test]
    fn off_gives_nothing() {
        assert!(!DateOptions::default().weekday_dates);
        assert!(run_with(false, "げつようび", at(2026, 10, 26, MONDAY)).is_empty());
    }

    #[test]
    fn non_weekday_readings_give_nothing() {
        let now = at(2026, 10, 26, MONDAY);
        for r in [
            "",
            "げつ",
            "げつよう",
            "げつようびは",
            "げつようびび",
            "月曜日",
            "きょう",
            "ようび",
            "げつようび ",
        ] {
            assert!(run_with(true, r, now).is_empty(), "{r}");
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
        assert!(run_with(true, "げつようび", bad).is_empty());
        let _ = run_with(true, "げつようび", at(i32::MAX, 12, 31, 0));
    }
}
