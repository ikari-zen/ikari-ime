// 英語のモード（ADR 0026 §5）: 打鍵の分類、文脈のたまり、モードの切り替え、Info.plist の必須キー。
import Foundation
import Testing

@testable import IkariShellCore

@Suite struct InputModeTests {
    @Test func modeIDsMapToModes() {
        #expect(InputMode(modeID: "io.github.ikari-zen.inputmethod.ikari.Japanese") == .japanese)
        // 英語のモードは macOS の標準の辞書のキーで通知される（S-A の T1）。入力ソースの ID でも読む
        #expect(InputMode(modeID: "com.apple.inputmethod.Roman") == .english)
        #expect(InputMode(modeID: "io.github.ikari-zen.inputmethod.ikari.English") == .english)
        #expect(InputMode(modeID: "com.apple.inputmethod.Japanese") == nil)
        #expect(InputMode(modeID: "") == nil)
    }

    @Test func modeIDsRoundTripAndToggle() {
        for mode in [InputMode.japanese, .english] { #expect(InputMode(modeID: mode.modeID) == mode) }
        #expect(InputMode.japanese.toggled == .english)
        #expect(InputMode.english.toggled == .japanese)
    }

    @Test func eisuAndKanaKeysAreAbsorbed() {
        #expect(EnglishMode.absorbsKeyCode(102))
        #expect(EnglishMode.absorbsKeyCode(104))
        #expect(!EnglishMode.absorbsKeyCode(0))
        #expect(!EnglishMode.absorbsKeyCode(KeyCodes.space))
    }
}

@Suite struct ModeKeymapTests {
    private func chord(_ text: String) -> KeyChord { KeyChord(parsing: text)! }

    @Test(arguments: [KeymapPreset.macos, .msIme])
    func presetsBindNoModeAction(preset: KeymapPreset) throws {
        let keymap = try Keymap(presetTSV: Harness.presetTSV(preset), overrides: [:])
        #expect(!keymap.bindings.values.contains { $0.isModeSwitch })
        #expect(KeymapAction.allCases.filter(\.isModeSwitch) == [.modeToggle, .modeEnglish, .modeJapanese])
        #expect(KeymapAction.allCases.filter(\.isModeSwitch).allSatisfy { !$0.isScriptConversion })
        #expect(KeymapAction.modeToggle.rawValue == "mode.toggle")
        #expect(KeymapAction.modeEnglish.rawValue == "mode.english")
        #expect(KeymapAction.modeJapanese.rawValue == "mode.japanese")
    }

    @Test func settingsCanBindModeActions() throws {
        let settings = Settings(keymapOverrides: [.modeToggle: "ctrl+;", .modeEnglish: "f13", .modeJapanese: "f14"])
        let keymap = try Keymap(presetTSV: Harness.presetTSV(.macos), settings: settings)
        #expect(keymap.action(for: chord("f13")) == .modeEnglish)
        #expect(keymap.action(for: chord("f14")) == .modeJapanese)
        // 既定の割り当て（半角カタカナ）を奪う
        #expect(keymap.action(for: chord("ctrl+;")) == .modeToggle)
        #expect(Settings.parse(tsv: "keymap.mode.toggle\tf13\n").warnings.isEmpty)
    }

    @Test func modeKeysResolveInBothModes() throws {
        let settings = Settings(keymapOverrides: [.modeToggle: "f13", .modeEnglish: "f14", .modeJapanese: "ctrl+shift+j"])
        let keymap = try Keymap(presetTSV: Harness.presetTSV(.macos), settings: settings)
        let f13 = KeyInput(key: .function(13))
        let f14 = KeyInput(key: .function(14))
        let ctrlShiftJ = KeyInput(key: .character("j"), modifiers: [.control, .shift])
        #expect(EnglishMode.modeSwitchTarget(for: f13, keymap: keymap, current: .japanese) == .english)
        #expect(EnglishMode.modeSwitchTarget(for: f13, keymap: keymap, current: .english) == .japanese)
        #expect(EnglishMode.modeSwitchTarget(for: f14, keymap: keymap, current: .english) == .english)
        #expect(EnglishMode.modeSwitchTarget(for: f14, keymap: keymap, current: .japanese) == .english)
        #expect(EnglishMode.modeSwitchTarget(for: ctrlShiftJ, keymap: keymap, current: .english) == .japanese)
        // 割り当てのないキー、素の文字は何もしない
        #expect(EnglishMode.modeSwitchTarget(for: KeyInput(key: .function(15)), keymap: keymap, current: .english) == nil)
        #expect(EnglishMode.modeSwitchTarget(for: KeyInput(key: .character("j")), keymap: keymap, current: .english) == nil)
        // 既定（割り当てなし）では、どのキーも切り替えにならない
        let plain = try Keymap(presetTSV: Harness.presetTSV(.macos), overrides: [:])
        #expect(EnglishMode.modeSwitchTarget(for: f13, keymap: plain, current: .english) == nil)
    }
}

@Suite struct EnglishClassifyTests {
    private let keymap = Keymap(bindings: [KeyChord(key: "f13"): .modeToggle, KeyChord(key: "f6"): .toHiragana])

    @Test func secureIsPassedThroughBeforeAnythingElse() {
        // secure なら、mode.* のキーも文字も素通し（ADR 0026 D5）
        #expect(
            EnglishMode.classify(input: KeyInput(key: .function(13)), keymap: keymap, secure: true, current: .english)
                == .passThroughSecure)
        #expect(
            EnglishMode.classify(input: KeyInput(key: .character("a")), keymap: keymap, secure: true, current: .english)
                == .passThroughSecure)
    }

    @Test func modeKeySwitches() {
        #expect(
            EnglishMode.classify(input: KeyInput(key: .function(13)), keymap: keymap, secure: false, current: .english)
                == .switchMode(.japanese))
    }

    @Test func otherKeysAreText() {
        for key in [KeyInput.Key.character("a"), .space, .enter, .left, .function(6), .other] {
            #expect(
                EnglishMode.classify(input: KeyInput(key: key), keymap: keymap, secure: false, current: .english) == .text,
                "\(key)")
        }
    }
}

@Suite struct EnglishContextPoolTests {
    private func press(_ mode: inout EnglishMode, _ key: KeyInput.Key, _ modifiers: KeyModifiers = []) -> String? {
        mode.record(KeyInput(key: key, modifiers: modifiers))
    }

    private func type(_ mode: inout EnglishMode, _ text: String) {
        for character in text { #expect(press(&mode, .character(String(character))) == nil) }
    }

    @Test func printableCharactersAccumulateAndSpaceIsIncluded() {
        var mode = EnglishMode()
        type(&mode, "hello")
        #expect(press(&mode, .space) == nil)
        type(&mode, "World")
        #expect(press(&mode, .character("!"), .shift) == nil)
        #expect(mode.pool.text == "hello World!")
        #expect(mode.lastCharacter == "!")
    }

    @Test func shiftedLettersAreTextButChordsAreSeparators() {
        var mode = EnglishMode()
        type(&mode, "ab")
        #expect(press(&mode, .character("C"), .shift) == nil)
        #expect(mode.pool.text == "abC")
        for modifiers: KeyModifiers in [.control, .option, .command, [.shift, .command]] {
            var m = EnglishMode()
            type(&m, "ab")
            #expect(press(&m, .character("c"), modifiers) == "ab", "\(modifiers)")
            #expect(m.pool.isEmpty)
        }
    }

    @Test func backspaceRemovesTheLastCharacter() {
        var mode = EnglishMode()
        type(&mode, "abc")
        #expect(press(&mode, .backspace) == nil)
        #expect(mode.pool.text == "ab")
        // 結合文字や絵文字は見た目の 1 文字ごと
        type(&mode, "👨‍👩‍👧")
        #expect(press(&mode, .backspace) == nil)
        #expect(mode.pool.text == "ab")
        #expect(press(&mode, .backspace, .shift) == nil)
        #expect(mode.pool.text == "a")
        // Option+Backspace は語の削除なので、たまりを区切る
        #expect(press(&mode, .backspace, .option) == "a")
    }

    @Test func backspaceOnAnEmptyPoolDoesNothing() {
        var mode = EnglishMode()
        #expect(press(&mode, .backspace) == nil)
        #expect(mode.pool.isEmpty)
        #expect(mode.lastCharacter == nil)
        #expect(mode.flush() == nil)
    }

    @Test(arguments: [
        KeyInput.Key.enter, .left, .right, .up, .down, .escape, .function(5), .other, .jis(22),
        .character("\t"), .character("\u{19}"), .character("\u{F700}"), .character(""),
    ])
    func separatorsEmitThePoolAsOneItem(key: KeyInput.Key) {
        var mode = EnglishMode()
        type(&mode, "hi there")
        #expect(press(&mode, key) == "hi there")
        #expect(mode.pool.isEmpty)
        // たまりが空のときの区切りは何も出さない（空の履歴を送らない）
        #expect(press(&mode, key) == nil)
    }

    @Test func modeSwitchAndDeactivateFlushThroughFlush() {
        var mode = EnglishMode()
        type(&mode, "abc")
        #expect(mode.flush() == "abc")
        #expect(mode.flush() == nil)
        // ショートコードで確定した文字列（絵文字を含む）もたまりに足す
        #expect(mode.appendCommitted("👀") == nil)
        type(&mode, "x")
        #expect(mode.pool.text == "👀x")
        #expect(mode.lastCharacter == "x")
    }

    @Test func discardDropsWithoutEmitting() {
        var mode = EnglishMode()
        type(&mode, "secret")
        mode.discard()
        #expect(mode.pool.isEmpty)
        #expect(mode.flush() == nil)
    }

    @Test func poolIsEmittedWhenItReachesTheLimit() {
        var mode = EnglishMode()
        let limit = EnglishContextPool.maxScalars
        #expect(limit == 2000)
        for _ in 0..<(limit - 1) { #expect(press(&mode, .character("a")) == nil) }
        #expect(mode.pool.text.count == limit - 1)
        let emitted = press(&mode, .character("a"))
        #expect(emitted?.unicodeScalars.count == limit)
        #expect(mode.pool.isEmpty)
        // 次の 1 文字から新しいたまり
        #expect(press(&mode, .character("b")) == nil)
        #expect(mode.pool.text == "b")
    }

    @Test func limitCountsUnicodeScalarsLikeTheEngine() {
        // 家族の絵文字は 1 文字（Character）だが 5 スカラー。エンジンの上限（スカラー）を超えないよう、スカラーで数える
        var pool = EnglishContextPool()
        let family = "👨‍👩‍👧"
        #expect(family.count == 1 && family.unicodeScalars.count == 5)
        var emitted: String?
        var added = 0
        while emitted == nil {
            emitted = pool.append(family)
            added += 1
        }
        #expect(added == EnglishContextPool.maxScalars / 5)
        #expect(emitted?.unicodeScalars.count == EnglishContextPool.maxScalars)
        // 巨大な 1 回の追加でも、取り出す（呼ぶ側が上限を超えた 1 件を送る。エンジンは 2000 超を断る）ことを確かめておく
        var big = EnglishContextPool()
        #expect(big.append(String(repeating: "x", count: 2500))?.unicodeScalars.count == 2500)
    }

    @Test func emptyAdditionIsIgnored() {
        var pool = EnglishContextPool()
        #expect(pool.append("") == nil)
        #expect(pool.isEmpty)
    }
}

@Suite struct EnglishContextToEngineTests {
    @Test func englishTextIsSentWithAnEmptyReading() throws {
        let h = try Harness()
        h.machine.recordEnglishText("hello world")
        #expect(h.engine.calls == [.commitText(sessionID: Harness.sessionID, reading: "", surface: "hello world")])
    }

    @Test func emptyTextIsNotSent() throws {
        let h = try Harness()
        h.machine.recordEnglishText("")
        #expect(h.engine.calls.isEmpty)
    }

    @Test func secureSessionSendsNothing() throws {
        let h = try Harness(secure: true)
        h.machine.recordEnglishText("password")
        #expect(h.engine.calls.isEmpty)
    }

    @Test func secureSetAfterwardsStopsSending() throws {
        let h = try Harness()
        h.machine.setSecure(true)
        let before = h.engine.calls.count
        h.machine.recordEnglishText("password")
        #expect(h.engine.calls.count == before)
    }

    /// 日本語 → 英語の切り替えのとき、コントローラは `commitComposition()` で今の表示のまま確定する（変換しない）
    @Test func switchingModeCommitsTheDisplayedCompositionAsIs() throws {
        let h = try Harness()
        h.type("ka")
        #expect(h.markedText == "か")
        let actions = h.machine.commitComposition()
        #expect(actions.contains(.commit("か")))
        #expect(h.machine.state == .idle)
        // 変換しない（Convert は送らない）。履歴には読みつきで入る
        #expect(h.engine.convertRequests.isEmpty)
        #expect(h.engine.calls.contains(.commitText(sessionID: Harness.sessionID, reading: "か", surface: "か")))
        // 何もないときは何も起きない（何度呼んでもよい）
        #expect(h.machine.commitComposition().isEmpty)
    }

    @Test func switchingModeKeepsTheSession() throws {
        let h = try Harness()
        h.machine.activate()
        h.machine.recordEnglishText("abc")
        _ = h.machine.commitComposition()
        // セッションの開始を送り直さない（確定履歴を切らさない。ADR 0015 D1）
        #expect(h.engine.count { if case .beginSession = $0 { true } else { false } } == 1)
    }
}

@Suite struct ModeInfoPlistTests {
    private static let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()

    /// テンプレートの置き場所の印（`@IKARI_ID_PREFIX@`）を、build.sh と同じに頭で埋めて読む（ADR 0036 §3.1）
    private static func rendered(_ url: URL) throws -> Data {
        let text = try String(contentsOf: url, encoding: .utf8)
        return Data(text.replacingOccurrences(of: "@IKARI_ID_PREFIX@", with: Identity.idPrefix).utf8)
    }

    private func plist() throws -> [String: Any] {
        let url = Self.root.appendingPathComponent("scripts/templates/ikari.Info.plist")
        let data = try Self.rendered(url)
        return try #require(try PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any])
    }

    private func modes() throws -> [String: [String: Any]] {
        let dict = try #require(try plist()["ComponentInputModeDict"] as? [String: Any])
        return try #require(dict["tsInputModeListKey"] as? [String: [String: Any]])
    }

    @Test func japaneseModeKeepsItsOwnKeyAndGainsTheKanaShortcut() throws {
        let japanese = try #require(try modes()[InputModeID.japanese])
        #expect(japanese["TISInputSourceID"] as? String == InputModeID.japanese)
        #expect(japanese["tsInputModeJISKeyboardShortcutKey"] as? Int == 1)
        #expect(japanese["tsInputModeScriptKey"] as? String == "smJapanese")
        #expect(japanese["tsInputModeIsVisibleKey"] as? Bool == true)
    }

    @Test func englishModeUsesTheStandardKeyAndIsVisibleAndDefaultOn() throws {
        let english = try #require(try modes()[InputModeID.english])
        #expect(InputModeID.english == "com.apple.inputmethod.Roman")
        #expect(english["TISInputSourceID"] as? String == InputModeID.englishInputSource)
        #expect(english["TISIntendedLanguage"] as? String == "en")
        #expect(english["tsInputModeScriptKey"] as? String == "smRoman")
        #expect(english["tsInputModePrimaryInScriptKey"] as? Bool == false)
        #expect(english["tsInputModeJISKeyboardShortcutKey"] as? Int == 3)
        #expect(english["tsInputModeIsVisibleKey"] as? Bool == true)
        // false だと入力ソースの一覧に出ない（S-A）
        #expect(english["tsInputModeDefaultStateKey"] as? Bool == true)
        #expect(english["tsInputModeMenuIconFileKey"] as? String == "ikari-english.pdf")
    }

    @Test func visibleModeOrderIsJapaneseThenEnglish() throws {
        let dict = try #require(try plist()["ComponentInputModeDict"] as? [String: Any])
        #expect(dict["tsVisibleInputModeOrderedArrayKey"] as? [String] == [InputModeID.japanese, InputModeID.english])
    }

    @Test func iconsReferencedByTheModesExist() throws {
        for mode in try modes().values {
            for key in ["tsInputModeMenuIconFileKey", "tsInputModePaletteIconFileKey", "tsInputModeAlternateMenuIconFileKey"] {
                let name = try #require(mode[key] as? String)
                #expect(FileManager.default.fileExists(atPath: Self.root.appendingPathComponent("Resources/icons/\(name)").path), "\(name)")
            }
        }
    }

    /// 表示名は、TISInputSourceID ではなく、モードの辞書のキーで引かれる（S-A の入力ソースの追加）
    @Test(arguments: ["ja", "en"])
    func displayNamesAreKeyedByTheModeDictionaryKey(language: String) throws {
        let url = Self.root.appendingPathComponent("Resources/\(language).lproj/InfoPlist.strings")
        let parsed = try PropertyListSerialization.propertyList(from: Self.rendered(url), format: nil)
        let strings = try #require(parsed as? [String: String])
        #expect(strings[InputModeID.japanese] == "ikari")
        #expect(strings[InputModeID.english] == "ikari (English)")
    }
}
