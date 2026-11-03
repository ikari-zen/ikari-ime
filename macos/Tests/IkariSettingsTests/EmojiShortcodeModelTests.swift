// 設定の処理は shell-core（ikari_settings_*）を通すので、XCFramework のあるビルドだけで流す（ADR 0041 §2.3）
#if IKARI_HAS_SHELL_FFI
// 「絵文字のショートコード」の欄（ADR 0027 §8.1）のモデル。
import Foundation
import Testing

@testable import IkariSettings

@Suite @MainActor struct EmojiShortcodeModelTests {
    @Test func startsWithTheDefaults() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        #expect(!model.values.emojiShortcode)
        #expect(!SettingsValues.defaults.emojiShortcodeExclude.isEmpty)
        #expect(model.values.emojiShortcodeExclude == SettingsValues.defaults.emojiShortcodeExclude)
    }

    @Test func toggleGoesThroughTheShell() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        await model.setEmojiShortcode(true)
        #expect(model.errorMessage == nil)
        #expect(model.values.emojiShortcode)
        #expect(harness.shell.state.snapshot.settings["emoji.shortcode"] == "on")
        await model.setEmojiShortcode(false)
        #expect(!model.values.emojiShortcode)
        #expect(harness.shell.state.snapshot.settings["emoji.shortcode"] == "off")
    }

    @Test func addRemoveAndResetTheExcludedApps() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)

        await model.addEmojiShortcodeExclude("  com.example.Slack ")
        #expect(model.errorMessage == nil)
        #expect(model.values.emojiShortcodeExclude.last == "com.example.Slack")
        #expect(harness.shell.state.snapshot.settings["emoji.shortcode_exclude"]?.hasSuffix(",com.example.Slack") == true)

        await model.removeEmojiShortcodeExclude("com.apple.Terminal")
        #expect(!model.values.emojiShortcodeExclude.contains("com.apple.Terminal"))

        await model.resetEmojiShortcodeExclude()
        #expect(model.values.emojiShortcodeExclude == SettingsValues.defaults.emojiShortcodeExclude)
    }

    @Test func removingEveryEntryStoresAnEmptyList() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        for id in SettingsValues.defaults.emojiShortcodeExclude { await model.removeEmojiShortcodeExclude(id) }
        #expect(model.values.emojiShortcodeExclude.isEmpty)
        #expect(harness.shell.state.snapshot.settings["emoji.shortcode_exclude"] == "")
    }

    @Test func invalidOrDuplicateIDsAreNotAdded() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        await model.addEmojiShortcodeExclude("com.a b")
        #expect(model.errorMessage != nil)
        model.dismissError()
        await model.addEmojiShortcodeExclude("com.apple.Terminal")
        #expect(model.errorMessage == nil)
        #expect(model.values.emojiShortcodeExclude == SettingsValues.defaults.emojiShortcodeExclude)
    }
}

#endif
