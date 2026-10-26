// 「郵便番号」の欄（ADR 0032）のモデル
import Foundation
import IkariShellCore
import Testing

@testable import IkariSettings

@Suite @MainActor struct PostalModelTests {
    @Test func startsWithTheDefaults() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        #expect(model.values.postalJapanese)
        #expect(!model.values.postalEnglish)
        #expect(!model.values.postalTePrefix)
    }

    @Test func togglesGoThroughTheShell() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        await model.setPostalTePrefix(true)
        await model.setPostalJapanese(false)
        await model.setPostalEnglish(true)
        #expect(model.errorMessage == nil)
        #expect(model.values.postalTePrefix && !model.values.postalJapanese && model.values.postalEnglish)
        let settings = harness.shell.state.snapshot.settings
        #expect(settings["postal.te_prefix"] == "on")
        #expect(settings["postal.japanese"] == "off")
        #expect(settings["postal.english"] == "on")
        await model.setPostalTePrefix(false)
        #expect(harness.shell.state.snapshot.settings["postal.te_prefix"] == "off")
    }
}
