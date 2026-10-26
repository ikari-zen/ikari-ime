// settings.tsv の engine.* と、設定アプリ・メニューとの受け渡し（ADR 0005 §7.3、§7.1）。
import Testing

@testable import IkariShellCore

@Suite struct SettingsEngineKeysTests {
    @Test func engineKeysDefaultToLmOnAndFiveMinutes() {
        let parsed = Settings.parse(tsv: "")
        #expect(parsed.settings.engineLM)
        #expect(parsed.settings.engineIdleUnloadSecs == 300)
    }

    @Test func parsesEngineKeys() {
        let parsed = Settings.parse(tsv: "engine.lm\toff\nengine.idle_unload_secs\t0\n")
        #expect(!parsed.settings.engineLM)
        #expect(parsed.settings.engineIdleUnloadSecs == 0)
        #expect(parsed.warnings.isEmpty)
    }

    @Test(arguments: ["engine.lm\ttrue", "engine.lm\t", "engine.idle_unload_secs\t-1", "engine.idle_unload_secs\tabc"])
    func invalidEngineValuesWarnAndKeepTheDefault(line: String) {
        let parsed = Settings.parse(tsv: line)
        #expect(parsed.settings == .default)
        #expect(parsed.warnings.count == 1)
        // 警告に値の文字列を含めない
        #expect(!parsed.warnings[0].contains("abc"))
    }

    @Test func otherEngineKeysAreIgnoredWithoutWarnings() {
        let parsed = Settings.parse(tsv: "engine.future_key\tx\n")
        #expect(parsed.settings == .default)
        #expect(parsed.warnings.isEmpty)
    }

    @Test func engineSettingsDifferOnlyForEngineKeys() {
        let base = Settings.default
        #expect(!base.engineSettingsDiffer(from: base))
        var other = base
        other.pageSize = 3
        other.keymapPreset = .msIme
        other.inputMethod = .kana
        #expect(!base.engineSettingsDiffer(from: other))
        other.engineLM = false
        #expect(base.engineSettingsDiffer(from: other))
        var idle = base
        idle.engineIdleUnloadSecs = 60
        #expect(base.engineSettingsDiffer(from: idle))
    }

    @Test func effectiveEntriesListEveryKnownKeyInAFixedOrder() {
        var settings = Settings(inputMethod: .kana, keymapPreset: .msIme, pagingWrap: false, pageSize: 5, engineLM: false)
        settings.keymapOverrides[.toHiragana] = "ctrl+h"
        settings.keymapOverrides[.toHalfwidthAlnum] = Settings.noneValue
        let entries = settings.effectiveEntries.map { "\($0.key)=\($0.value)" }
        #expect(
            entries == [
                "input.method=kana", "keymap.preset=ms-ime",
                "keymap.convert.hiragana=ctrl+h", "keymap.convert.halfwidth_alnum=none",
                "paging.wrap=false", "paging.size=5", "engine.lm=off", "engine.idle_unload_secs=300",
                "context.shared_exclude=" + Settings.defaultContextSharedExclude.joined(separator: ","),
                "emoji.shortcode=off",
                "emoji.shortcode_exclude=" + Settings.defaultEmojiShortcodeExclude.joined(separator: ","),
                "emoji.small_letters=off",
                "postal.japanese=on", "postal.english=off", "postal.te_prefix=off",
                "unicode.notation=off", "date.weekday_dates=off",
            ])
    }

    @Test func effectiveEntriesRoundTripThroughParse() {
        var settings = Settings(
            inputMethod: .kana, keymapPreset: .msIme, pagingWrap: false, pageSize: 2, engineLM: false, engineIdleUnloadSecs: 0)
        settings.keymapOverrides[.toFullwidthKatakana] = "ctrl+q"
        let tsv = settings.effectiveEntries.map { "\($0.key)\t\($0.value)" }.joined(separator: "\n")
        let parsed = Settings.parse(tsv: tsv)
        #expect(parsed.settings == settings)
        #expect(parsed.warnings.isEmpty)
    }

    @Test func acceptableKeysAndValues() {
        #expect(Settings.isAcceptable(key: "keymap.preset", value: "ms-ime"))
        #expect(Settings.isAcceptable(key: "keymap.convert.hiragana", value: "ctrl+j"))
        #expect(Settings.isAcceptable(key: "keymap.convert.hiragana", value: "none"))
        #expect(Settings.isAcceptable(key: "engine.lm", value: "off"))
        #expect(Settings.isAcceptable(key: "paging.size", value: "9"))
        #expect(!Settings.isAcceptable(key: "paging.size", value: "10"))
        #expect(!Settings.isAcceptable(key: "keymap.preset", value: "emacs"))
        #expect(!Settings.isAcceptable(key: "keymap.convert.unknown", value: "ctrl+j"))
        #expect(!Settings.isAcceptable(key: "engine.future_key", value: "x"))
        #expect(!Settings.isAcceptable(key: "unknown", value: "x"))
        #expect(!Settings.isAcceptable(key: "", value: "x"))
    }

    @Test func knownKeysAreTheDocumentedOnes() {
        for key in ["input.method", "keymap.preset", "paging.wrap", "paging.size", "engine.lm", "engine.idle_unload_secs"] {
            #expect(Settings.isKnownKey(key), "\(key)")
        }
        for action in KeymapAction.allCases { #expect(Settings.isKnownKey("keymap.\(action.rawValue)")) }
        for key in ["keymap.", "keymap.nope", "engine.", "engine.other", "paging", ""] {
            #expect(!Settings.isKnownKey(key), "\(key)")
        }
    }

    @Test func rejectsKeysAndValuesThatCouldInjectAnotherLine() {
        #expect(!Settings.isAcceptable(key: "paging.wrap", value: "true\nengine.lm\toff"))
        #expect(!Settings.isAcceptable(key: "paging.wrap", value: "true\r"))
        #expect(!Settings.isAcceptable(key: "paging.wrap\tx", value: "true"))
        #expect(!Settings.isAcceptable(key: "paging.wrap\n", value: "true"))
    }
}
