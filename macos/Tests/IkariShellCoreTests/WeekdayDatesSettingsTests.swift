// 曜日の日付の設定（`date.weekday_dates`。既定オフ）
import Testing

@testable import IkariShellCore

@Suite struct WeekdayDatesSettingsTests {
    @Test func defaultIsOff() {
        #expect(!Settings.default.dateWeekdayDates)
    }

    @Test func parsesOnAndOff() {
        let on = Settings.parse(tsv: "date.weekday_dates\ton\n")
        #expect(on.warnings.isEmpty)
        #expect(on.settings.dateWeekdayDates)
        let off = Settings.parse(tsv: "date.weekday_dates\toff\n")
        #expect(off.warnings.isEmpty)
        #expect(!off.settings.dateWeekdayDates)
    }

    @Test func badValueKeepsTheDefaultAndWarns() {
        let parsed = Settings.parse(tsv: "date.weekday_dates\ttrue\n")
        #expect(parsed.warnings.count == 1)
        #expect(!parsed.settings.dateWeekdayDates)
    }

    @Test func keyIsWritableAndShownInTheSnapshot() {
        #expect(Settings.isAcceptable(key: "date.weekday_dates", value: "on"))
        #expect(!Settings.isAcceptable(key: "date.weekday_dates", value: "yes"))
        var s = Settings.default
        s.dateWeekdayDates = true
        let entries = Dictionary(uniqueKeysWithValues: s.effectiveEntries.map { ($0.key, $0.value) })
        #expect(entries["date.weekday_dates"] == "on")
    }

    @Test func engineRestartsWhenItChanges() {
        let base = Settings.default
        var changed = base
        changed.dateWeekdayDates = true
        #expect(base.engineSettingsDiffer(from: changed))
    }
}
