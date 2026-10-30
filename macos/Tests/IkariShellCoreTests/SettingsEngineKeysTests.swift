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

    @Test(arguments: ["engine.lm\ttrue", "engine.lm\t", "engine.idle_unload_secs\t-1", "engine.idle_unload_secs\tabc",
            "engine.lm_deadline_ms\t-1", "engine.lm_deadline_ms\tabc", "engine.convert_wait_ms\t-1", "engine.convert_wait_ms\tabc",
        ])
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

    @Test func timeoutKeysDefaultToTheCurrentConstantsAndParse() {
        let none = Settings.parse(tsv: "").settings
        #expect(none.engineLMDeadlineMs == 300)
        #expect(none.engineConvertWaitMs == 30)
        let parsed = Settings.parse(tsv: "engine.lm_deadline_ms\t800\nengine.convert_wait_ms\t0\n")
        #expect(parsed.settings.engineLMDeadlineMs == 800)
        #expect(parsed.settings.engineConvertWaitMs == 0)
        #expect(parsed.warnings.isEmpty)
    }
}
