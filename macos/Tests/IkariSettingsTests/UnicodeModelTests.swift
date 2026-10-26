// 「Unicode 表記」の欄のモデル
import Foundation
import IkariShellCore
import Testing

@testable import IkariSettings

@Suite @MainActor struct UnicodeModelTests {
    @Test func startsOffAndTogglesGoThroughTheShell() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        #expect(!model.values.unicodeNotation)
        await model.setUnicodeNotation(true)
        #expect(model.errorMessage == nil)
        #expect(model.values.unicodeNotation)
        #expect(harness.shell.state.snapshot.settings["unicode.notation"] == "on")
        await model.setUnicodeNotation(false)
        #expect(harness.shell.state.snapshot.settings["unicode.notation"] == "off")
    }
}
