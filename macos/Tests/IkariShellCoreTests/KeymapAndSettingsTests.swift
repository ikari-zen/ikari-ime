// キー割り当てと settings.tsv の解析（docs/phase2-contract.md §3、§4）。
import Testing

@testable import IkariShellCore

@Suite struct KeyChordParsingTests {
    @Test func parsesValidNotations() {
        let cases: [(String, String, KeyModifiers)] = [
            ("ctrl+j", "j", .control),
            ("f6", "f6", []),
            ("shift+space", "space", .shift),
            ("ctrl+;", ";", .control),
            ("CTRL+J", "j", .control),
            ("ctrl+shift+f10", "f10", [.control, .shift]),
            ("command+option+enter", "enter", [.command, .option]),
            ("escape", "escape", []),
            ("backspace", "backspace", []),
            ("f20", "f20", []),
            ("left", "left", []),
            ("shift+right", "right", .shift),
            ("ctrl+shift+r", "r", [.control, .shift]),
            ("ctrl++", "+", .control),
            ("+", "+", []),
            (" ctrl+k ", "k", .control),
        ]
        for (text, key, modifiers) in cases {
            #expect(KeyChord(parsing: text) == KeyChord(key: key, modifiers: modifiers), "\(text)")
        }
    }

    @Test(arguments: [
        "", " ", "ctrl+", "ctrl", "+j", "meta+j", "ctrl+ctrl+j", "ctrl+jj", "f0", "f21", "f06", "ff", "ctrl+f1x", "ctrl+ ",
        "ctrl + j", "tab",
    ])
    func rejectsInvalidNotations(text: String) {
        #expect(KeyChord(parsing: text) == nil)
    }
}

@Suite struct KeymapTests {
    private func chord(_ text: String) -> KeyChord { KeyChord(parsing: text)! }

    @Test func presetFilesFollowTheContractTable() throws {
        let macos = try Keymap(presetTSV: Harness.presetTSV(.macos), overrides: [:])
        let msIme = try Keymap(presetTSV: Harness.presetTSV(.msIme), overrides: [:])

        let expectations: [(KeymapAction, String, String?, String?)] = [
            (.toHiragana, "f6", "ctrl+j", "ctrl+u"),
            (.toFullwidthKatakana, "f7", "ctrl+k", "ctrl+i"),
            (.toHalfwidthKatakana, "f8", "ctrl+;", "ctrl+o"),
            (.toFullwidthAlnum, "f9", "ctrl+l", "ctrl+p"),
            (.toHalfwidthAlnum, "f10", nil, "ctrl+t"),
        ]
        for (action, function, macosKey, msKey) in expectations {
            #expect(macos.action(for: chord(function)) == action)
            #expect(msIme.action(for: chord(function)) == action)
            if let macosKey { #expect(macos.action(for: chord(macosKey)) == action) }
            if let msKey { #expect(msIme.action(for: chord(msKey)) == action) }
        }
        // 文節の操作と再変換（ADR 0006 §8.1）は両プリセットで同じ
        let segmentExpectations: [(KeymapAction, String)] = [
            (.segmentFocusLeft, "left"), (.segmentFocusRight, "right"),
            (.segmentShrink, "shift+left"), (.segmentExpand, "shift+right"),
            (.reconvert, "ctrl+shift+r"),
            // ↑↓ は候補の軸（ADR 0014 §2.4）
            (.candidateNext, "down"), (.candidatePrevious, "up"),
        ]
        for (action, key) in segmentExpectations {
            #expect(macos.action(for: chord(key)) == action)
            #expect(msIme.action(for: chord(key)) == action)
            #expect(!action.isScriptConversion)
        }
        #expect(macos.bindings.count == 16)
        #expect(msIme.bindings.count == 17)
        // 別のプリセットのキーは効かない
        #expect(macos.action(for: chord("ctrl+u")) == nil)
        #expect(msIme.action(for: chord("ctrl+j")) == nil)
    }

    /// ATOK 風は MS-IME 風を土台に、半角英数（ctrl+@）と再変換（ctrl+shift+y）だけ違う（#62）
    @Test func atokPresetDiffersFromMsImeOnlyInHalfwidthAlnumAndReconvert() throws {
        let atok = try Keymap(presetTSV: Harness.presetTSV(.atok), overrides: [:])
        let msIme = try Keymap(presetTSV: Harness.presetTSV(.msIme), overrides: [:])
        var expected = msIme.bindings
        expected[chord("ctrl+t")] = nil
        expected[chord("ctrl+shift+r")] = nil
        expected[chord("ctrl+@")] = .toHalfwidthAlnum
        // US 配列では @ が Shift+2 なので、ctrl+shift+@ も同じ操作にする
        expected[chord("ctrl+shift+@")] = .toHalfwidthAlnum
        expected[chord("ctrl+shift+y")] = .reconvert
        #expect(atok.bindings == expected)
        #expect(atok.action(for: chord("f10")) == .toHalfwidthAlnum)
        #expect(atok.action(for: chord("ctrl+shift+r")) == nil)
        #expect(atok.action(for: chord("ctrl+t")) == nil)
    }

    @Test func atokPresetSettingRoundTripsAndUnknownValuesStayDefault() {
        #expect(KeymapPreset(rawValue: "atok") == .atok)
        #expect(Settings.parse(tsv: "keymap.preset\tatok").settings.keymapPreset == .atok)
        #expect(Settings.parse(tsv: "keymap.preset\temacs").settings.keymapPreset == .macos)
    }

    @Test func overridesReplaceCharacterKeysButFunctionKeysRemain() throws {
        let tsv = try Harness.presetTSV(.macos)
        let keymap = try Keymap(presetTSV: tsv, overrides: [.toHiragana: chord("ctrl+h")])
        #expect(keymap.action(for: chord("ctrl+h")) == .toHiragana)
        #expect(keymap.action(for: chord("ctrl+j")) == nil)
        #expect(keymap.action(for: chord("f6")) == .toHiragana)
    }

    @Test func settingsNoneRemovesEveryBindingOfTheAction() throws {
        let tsv = try Harness.presetTSV(.macos)
        let settings = Settings(keymapOverrides: [.toHiragana: "none", .toFullwidthKatakana: "ctrl+g"])
        let keymap = try Keymap(presetTSV: tsv, settings: settings)
        #expect(keymap.action(for: chord("ctrl+j")) == nil)
        #expect(keymap.action(for: chord("f6")) == nil)
        #expect(keymap.action(for: chord("ctrl+g")) == .toFullwidthKatakana)
        #expect(keymap.action(for: chord("f7")) == .toFullwidthKatakana)
        #expect(keymap.action(for: chord("ctrl+k")) == nil)
    }

    @Test func settingsWithUnreadableOverrideAreIgnored() throws {
        let tsv = try Harness.presetTSV(.macos)
        let settings = Settings(keymapOverrides: [.toHiragana: "ctrl+ctrl+x"])
        let keymap = try Keymap(presetTSV: tsv, settings: settings)
        #expect(keymap.action(for: chord("ctrl+j")) == .toHiragana)
    }

    @Test func commentsAndBlankLinesInPresetsAreSkipped() throws {
        let keymap = try Keymap(presetTSV: "# c\n\nf6\tconvert.hiragana\r\n  \n", overrides: [:])
        #expect(keymap.bindings.count == 1)
    }

    @Test(arguments: [
        ("f6", 1), ("f6\tnope", 1), ("ctrl+ctrl+j\tconvert.hiragana", 1), ("\nf6\tconvert.hiragana\textra", 2),
        ("f6 convert.hiragana", 1),
    ])
    func brokenPresetLinesThrowWithTheLineNumber(tsv: String, line: Int) {
        #expect(throws: KeymapError.invalidLine(number: line)) {
            try Keymap(presetTSV: tsv, overrides: [:])
        }
    }
}

@Suite struct SettingsParsingTests {
    @Test func emptyTextGivesDefaults() {
        let result = Settings.parse(tsv: "")
        #expect(result.settings == .default)
        #expect(result.warnings.isEmpty)
        #expect(Settings.default.keymapPreset == .macos)
        #expect(Settings.default.pagingWrap)
        #expect(Settings.default.pageSize == 3)
        #expect(Settings.default.inputMethod == .romaji)
    }

    @Test func readsEveryKey() {
        let tsv = """
            # コメント
            input.method\tkana
            keymap.preset\tms-ime
            keymap.convert.hiragana\tctrl+h
            keymap.convert.halfwidth_alnum\tnone
            paging.wrap\tfalse
            paging.size\t5
            engine.lm\toff
            engine.idle_unload_secs\t0

            """
        let result = Settings.parse(tsv: tsv)
        #expect(result.warnings.isEmpty)
        #expect(
            result.settings
                == Settings(
                    inputMethod: .kana,
                    keymapPreset: .msIme,
                    keymapOverrides: [.toHiragana: "ctrl+h", .toHalfwidthAlnum: "none"],
                    pagingWrap: false,
                    pageSize: 5,
                    engineLM: false,
                    engineIdleUnloadSecs: 0))
    }

    @Test func invalidValuesFallBackToDefaultsWithWarningsThatDoNotEchoTheValue() {
        let tsv = """
            input.method\tdvorak
            keymap.preset\tvim
            keymap.convert.hiragana\tctrl+ctrl+j
            paging.wrap\tmaybe
            paging.size\t10
            """
        let result = Settings.parse(tsv: tsv)
        #expect(result.settings == .default)
        #expect(result.warnings.count == 5)
        for warning in result.warnings {
            #expect(!warning.contains("dvorak") && !warning.contains("vim") && !warning.contains("maybe"))
        }
    }

    @Test(arguments: ["0", "-1", "abc", "", "9.5", "１"])
    func pageSizeOutOfRangeIsRejected(value: String) {
        let result = Settings.parse(tsv: "paging.size\t\(value)")
        #expect(result.settings.pageSize == 3)
        #expect(result.warnings.count == 1)
    }

    @Test(arguments: ["1", "9"])
    func pageSizeBoundsAreAccepted(value: String) {
        #expect(Settings.parse(tsv: "paging.size\t\(value)").settings.pageSize == Int(value))
    }

    @Test func unknownKeysAndMalformedLinesAreIgnored() {
        let result = Settings.parse(tsv: "no.such\t1\nkeymap.convert.bogus\tf6\njust-a-key\nkeymap.preset\tmacos")
        #expect(result.settings == .default)
        #expect(result.warnings.count == 3)
    }

    @Test func handlesWindowsLineEndingsAndSurroundingSpaces() {
        let result = Settings.parse(tsv: "paging.wrap\t false \r\npaging.size\t3\r\n")
        #expect(result.settings.pagingWrap == false)
        #expect(result.settings.pageSize == 3)
        #expect(result.warnings.isEmpty)
    }

    @Test func laterLinesWin() {
        #expect(Settings.parse(tsv: "paging.size\t3\npaging.size\t4").settings.pageSize == 4)
    }

    @Test func parsedSettingsDriveTheKeymap() throws {
        let settings = Settings.parse(tsv: "keymap.preset\tms-ime\nkeymap.convert.hiragana\tnone").settings
        let tsv = try Harness.presetTSV(settings.keymapPreset)
        let keymap = try Keymap(presetTSV: tsv, settings: settings)
        #expect(keymap.action(for: KeyChord(key: "u", modifiers: .control)) == nil)
        #expect(keymap.action(for: KeyChord(key: "i", modifiers: .control)) == .toFullwidthKatakana)
    }
}
