// 「小さい文字」の欄のモデル（emoji.small_letters）
import Foundation
import IkariShellCore
import Testing

@testable import IkariSettings

@Suite @MainActor struct SmallLettersModelTests {
    @Test func startsOffAndTogglesGoThroughTheShell() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        #expect(!model.values.emojiSmallLetters)
        await model.setEmojiSmallLetters(true)
        #expect(model.errorMessage == nil)
        #expect(model.values.emojiSmallLetters)
        #expect(harness.shell.state.snapshot.settings["emoji.small_letters"] == "on")
        await model.setEmojiSmallLetters(false)
        #expect(harness.shell.state.snapshot.settings["emoji.small_letters"] == "off")
    }
}
