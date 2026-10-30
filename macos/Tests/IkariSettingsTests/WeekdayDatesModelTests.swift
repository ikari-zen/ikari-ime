// 設定の処理は shell-core（ikari_settings_*）を通すので、XCFramework のあるビルドだけで流す（ADR 0041 §2.3）
#if IKARI_HAS_SHELL_FFI
// 「曜日の日付」の欄のモデル
import Foundation
import IkariShellCore
import Testing

@testable import IkariSettings

@Suite @MainActor struct WeekdayDatesModelTests {
    @Test func startsOffAndTogglesGoThroughTheShell() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        #expect(!model.values.dateWeekdayDates)
        await model.setDateWeekdayDates(true)
        #expect(model.errorMessage == nil)
        #expect(model.values.dateWeekdayDates)
        #expect(harness.shell.state.snapshot.settings["date.weekday_dates"] == "on")
        await model.setDateWeekdayDates(false)
        #expect(harness.shell.state.snapshot.settings["date.weekday_dates"] == "off")
    }
}

#endif
