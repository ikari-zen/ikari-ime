import Foundation
import IkariProto
import IkariShellCore
import Testing

@testable import IkariSettings

/// リポジトリの `macos/Resources/keymap/*.tsv`（build.sh が設定アプリのバンドルに置くものと同じ）
func repositoryPresets() -> KeymapPresetTable {
    let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("Resources/keymap")
    return KeymapPresetTable { preset in
        try? String(contentsOf: root.appendingPathComponent("\(preset.rawValue).tsv"), encoding: .utf8)
    }
}

@MainActor
func makeSettingsModel(_ harness: Harness) async -> SettingsModel {
    let model = SettingsModel(bridge: harness.client, presets: repositoryPresets())
    await model.load()
    return model
}

@Suite @MainActor struct SettingsModelTests {
    @Test func loadsDefaultsAndTheProfile() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        #expect(model.isLoaded)
        #expect(model.values == SettingsValues())
        #expect(model.profile.count == 4)
        #expect(model.profile.first { $0.id == "it" }?.weight == .often)
        #expect(model.profile.first { $0.id == "novel" }?.weight == .off)
    }

    @Test func atokPresetCanBeChosenAndStored() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        await model.setKeymapPreset(.atok)
        #expect(model.errorMessage == nil)
        #expect(model.values.keymapPreset == .atok)
        #expect(harness.shell.state.snapshot.settings["keymap.preset"] == "atok")
    }

    @Test func everyKnownSettingCanBeChangedFromTheModel() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)

        await model.setInputMethod(.kana)
        await model.setKeymapPreset(.msIme)
        await model.setPagingWrap(false)
        await model.setPageSize(5)
        await model.setLMEnabled(false)
        await model.setIdleUnloadSecs(0)

        #expect(model.errorMessage == nil)
        #expect(model.values.inputMethod == .kana)
        #expect(model.values.keymapPreset == .msIme)
        #expect(!model.values.pagingWrap)
        #expect(model.values.pageSize == 5)
        #expect(!model.values.lmEnabled)
        #expect(model.values.idleUnloadSecs == 0)
        let stored = harness.shell.state.snapshot.settings
        #expect(stored["input.method"] == "kana")
        #expect(stored["keymap.preset"] == "ms-ime")
        #expect(stored["paging.wrap"] == "false")
        #expect(stored["paging.size"] == "5")
        #expect(stored["engine.lm"] == "off")
        #expect(stored["engine.idle_unload_secs"] == "0")
    }

    @Test func unchangedValuesAreNotSent() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        await model.setLMEnabled(true)  // 既定のまま。エンジンを起こし直させない
        await model.setInputMethod(.romaji)
        #expect(!harness.shell.state.snapshot.requestLog.contains("engine-restart"))
    }

    @Test func changingTheLMRestartsTheEngineButChangingTheKeymapDoesNot() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        await model.setKeymapPreset(.msIme)
        #expect(!harness.shell.state.snapshot.requestLog.contains("engine-restart"))
        await model.setLMEnabled(false)
        #expect(harness.shell.state.snapshot.requestLog.contains("engine-restart"))
    }

    @Test func lmTimeoutsAreWrittenAndClamped() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        #expect(model.values.lmDeadlineMs == 300)
        #expect(model.values.convertWaitMs == 30)
        await model.setLMDeadlineMs(800)
        await model.setConvertWaitMs(0)
        #expect(model.values.lmDeadlineMs == 800)
        #expect(model.values.convertWaitMs == 0)
        await model.setLMDeadlineMs(1)
        #expect(model.values.lmDeadlineMs == SettingsValues.lmDeadlineRange.lowerBound)
        await model.setLMDeadlineMs(1_000_000)
        #expect(model.values.lmDeadlineMs == SettingsValues.lmDeadlineRange.upperBound)
        await model.setConvertWaitMs(-5)
        #expect(model.values.convertWaitMs == 0)
        await model.setConvertWaitMs(1_000)
        #expect(model.values.convertWaitMs == SettingsValues.convertWaitRange.upperBound)
    }

    @Test func pageSizeAndIdleSecondsAreClamped() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        await model.setPageSize(42)
        await model.setIdleUnloadSecs(-5)
        #expect(model.values.pageSize == 9)
        #expect(model.values.idleUnloadSecs == 0)
        await model.setIdleUnloadSecs(10_000_000)
        #expect(model.values.idleUnloadSecs == SettingsValues.idleUnloadRange.upperBound)
    }

    @Test func recordedKeyBecomesAnOverrideAndPresetRemovesIt() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        let chord = KeyChord(key: "h", modifiers: .control)
        await model.setKeymap(.toHiragana, to: .chord(KeyChordText.string(for: chord)))
        #expect(harness.shell.state.snapshot.settings["keymap.convert.hiragana"] == "ctrl+h")
        let row = try #require(model.keymapRows.first { $0.action == .toHiragana })
        #expect(row.binding == .chord("ctrl+h"))
        #expect(row.keyText == "⌃H")
        #expect(row.presetText == "⌃J")
        #expect(row.isOverridden)

        await model.setKeymap(.toHiragana, to: .preset)
        #expect(harness.shell.state.snapshot.settings["keymap.convert.hiragana"] == nil)
        #expect(model.keymapRows.first { $0.action == .toHiragana }?.keyText == "⌃J")
    }

    @Test func segmentActionsShowTheArrowSymbolsFromThePreset() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        func text(_ action: KeymapAction) -> String? { model.keymapRows.first { $0.action == action }?.keyText }
        #expect(text(.segmentFocusLeft) == "←")
        #expect(text(.segmentFocusRight) == "→")
        #expect(text(.segmentShrink) == "⇧←")
        #expect(text(.segmentExpand) == "⇧→")
        #expect(text(.reconvert) == "⌃⇧R")

        await model.setKeymap(.segmentShrink, to: .chord(KeyChordText.string(for: KeyChord(key: "left", modifiers: [.control, .shift]))))
        #expect(harness.shell.state.snapshot.settings["keymap.segment.shrink"] == "ctrl+shift+left")
        #expect(text(.segmentShrink) == "⌃⇧←")
    }

    @Test func noneRemovesTheAssignment() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        await model.setKeymap(.toFullwidthKatakana, to: .none)
        let row = try #require(model.keymapRows.first { $0.action == .toFullwidthKatakana })
        #expect(row.binding == .none)
        #expect(row.keyText == "なし")
        #expect(harness.shell.state.snapshot.settings["keymap.convert.fullwidth_katakana"] == "none")
    }

    @Test func aKeyAlreadyUsedByAnotherActionIsRefused() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        // macOS 風: ctrl+j は「ひらがなに変換」
        await model.setKeymap(.toFullwidthKatakana, to: .chord("ctrl+j"))
        #expect(model.errorMessage?.contains("ひらがなに変換") == true)
        #expect(harness.shell.state.snapshot.settings["keymap.convert.fullwidth_katakana"] == nil)
        // F キーはどの操作にも残っているので、そこへの割り当ても同じく断る
        model.dismissError()
        await model.setKeymap(.toFullwidthKatakana, to: .chord("f6"))
        #expect(model.errorMessage != nil)
    }

    @Test func aKeyFreedByAnOverrideCanBeReused() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        await model.setKeymap(.toHiragana, to: .chord("ctrl+h"))
        // ctrl+j は空いた
        await model.setKeymap(.toFullwidthKatakana, to: .chord("ctrl+j"))
        #expect(model.errorMessage == nil)
        #expect(model.values.keymapBindings[.toFullwidthKatakana] == .chord("ctrl+j"))
    }

    @Test func anOverrideEqualToThePresetIsShownAsPreset() async throws {
        var state = FakeState()
        state.settings["keymap.convert.hiragana"] = "ctrl+j"  // 殻が「今効いているキー」を返す場合
        let harness = try await Harness.make(state: state)
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        #expect(model.resolvedBinding(for: .toHiragana) == .preset)
        #expect(model.keymapRows.first { $0.action == .toHiragana }?.isOverridden == false)
    }

    @Test func aFailedChangeKeepsTheMessageAndShowsTheRealValue() async throws {
        var state = FakeState()
        state.settings["engine.lm"] = "off"
        let harness = try await Harness.make(state: state)
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        harness.shell.stop()
        await model.setLMEnabled(true)
        #expect(model.errorMessage != nil)
        #expect(!model.values.lmEnabled)
    }

    @Test func profileWeightsAreSentAsAWholeWithoutOffEntries() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        await model.setWeight(.sometimes, forDomain: "infra")
        await model.setWeight(.off, forDomain: "it")
        let stored = harness.shell.state.snapshot.profile
        #expect(stored == ["infra": .profileWeightSometimes])
        #expect(model.profile.first { $0.id == "infra" }?.weight == .sometimes)
        #expect(model.profile.first { $0.id == "it" }?.weight == .off)
    }

    @Test func profileErrorsDoNotBlockTheSettings() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        harness.shell.state.withLock { $0.engineUnavailable = true }
        let model = await makeSettingsModel(harness)
        #expect(model.isLoaded)
        #expect(model.profileMessage != nil)
        #expect(model.profile.isEmpty)
    }

    @Test func aSettingsChangedEventCanBeAppliedByReloading() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        harness.shell.state.withLock { $0.settings["paging.size"] = "4" }  // メニューや手の編集
        await model.reloadSettings()
        #expect(model.values.pageSize == 4)
    }
}

@Suite struct SettingsValuesTests {
    @Test func unknownAndInvalidValuesFallBackToDefaults() {
        let values = SettingsValues(snapshot: [
            "input.method": "dvorak", "keymap.preset": "emacs", "paging.size": "0", "paging.wrap": "maybe",
            "engine.lm": "yes", "engine.idle_unload_secs": "-1", "keymap.convert.hiragana": "ctrl+ctrl+",
            "keymap.unknown": "ctrl+x",
        ])
        #expect(values == SettingsValues())
    }

    @Test func parsesEveryKey() {
        let values = SettingsValues(snapshot: [
            "input.method": "kana", "keymap.preset": "ms-ime", "paging.size": "3", "paging.wrap": "false",
            "engine.lm": "off", "engine.idle_unload_secs": "0", "keymap.convert.hiragana": "none",
            "keymap.convert.fullwidth_alnum": "ctrl+g",
        ])
        #expect(values.inputMethod == .kana)
        #expect(values.keymapPreset == .msIme)
        #expect(values.pageSize == 3)
        #expect(!values.pagingWrap)
        #expect(!values.lmEnabled)
        #expect(values.idleUnloadSecs == 0)
        #expect(values.keymapBindings[.toHiragana] == KeymapBinding.none)
        #expect(values.keymapBindings[.toFullwidthAlnum] == .chord("ctrl+g"))
    }

    // プリセットの表の読み方（F キーが先、既定は文字キー）は shell-core（values.rs の preset_table_* のテスト）。
    // ここは画面に出る「プリセットのキー」が同じ期待になることを、shell-core を通して確かめる
    @Test func presetTextIsTheCharacterKeyOfThePreset() {
        func presetText(_ action: KeymapAction, _ preset: KeymapPreset) -> String? {
            let row = SettingsCore.describe(["keymap.preset": preset.rawValue], presets: repositoryPresets())?
                .rows.first { $0.action == action.rawValue }
            return row?.hasPresetText == true ? row?.presetText : nil
        }
        #expect(presetText(.toHiragana, .macos) == KeyChordText.display(for: KeyChord(key: "j", modifiers: .control)))
        #expect(presetText(.toHiragana, .msIme) == KeyChordText.display(for: KeyChord(key: "u", modifiers: .control)))
        #expect(presetText(.toHalfwidthAlnum, .macos) == nil)  // F10 だけ
        #expect(presetText(.toHalfwidthAlnum, .atok) == KeyChordText.display(for: KeyChord(key: "@", modifiers: .control)))
        #expect(
            presetText(.reconvert, .atok) == KeyChordText.display(for: KeyChord(key: "y", modifiers: [.control, .shift])))
    }
}
