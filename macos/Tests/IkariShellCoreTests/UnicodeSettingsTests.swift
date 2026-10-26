// Unicode 表記の設定（`unicode.notation`。既定オフ）
import Testing

@testable import IkariShellCore

@Suite struct UnicodeSettingsTests {
    @Test func defaultIsOff() {
        #expect(!Settings.default.unicodeNotation)
    }

    @Test func parsesOnAndOff() {
        let on = Settings.parse(tsv: "unicode.notation\ton\n")
        #expect(on.warnings.isEmpty)
        #expect(on.settings.unicodeNotation)
        let off = Settings.parse(tsv: "unicode.notation\toff\n")
        #expect(off.warnings.isEmpty)
        #expect(!off.settings.unicodeNotation)
    }

    @Test func badValueKeepsTheDefaultAndWarns() {
        let parsed = Settings.parse(tsv: "unicode.notation\ttrue\n")
        #expect(parsed.warnings.count == 1)
        #expect(!parsed.settings.unicodeNotation)
    }

    @Test func keyIsWritableAndShownInTheSnapshot() {
        #expect(Settings.isAcceptable(key: "unicode.notation", value: "on"))
        #expect(!Settings.isAcceptable(key: "unicode.notation", value: "yes"))
        var s = Settings.default
        s.unicodeNotation = true
        let entries = Dictionary(uniqueKeysWithValues: s.effectiveEntries.map { ($0.key, $0.value) })
        #expect(entries["unicode.notation"] == "on")
    }

    @Test func engineRestartsWhenItChanges() {
        let base = Settings.default
        var changed = base
        changed.unicodeNotation = true
        #expect(base.engineSettingsDiffer(from: changed))
    }
}
