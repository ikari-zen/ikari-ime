// 郵便番号の設定（ADR 0032）。`postal.japanese` / `postal.english` / `postal.te_prefix`
import Testing

@testable import IkariShellCore

@Suite struct PostalSettingsTests {
    @Test func defaultsAreJapaneseOnAndTheRestOff() {
        let s = Settings.default
        #expect(s.postalJapanese)
        #expect(!s.postalEnglish)
        #expect(!s.postalTePrefix)
    }

    @Test func parsesTheThreeKeys() {
        let parsed = Settings.parse(tsv: "postal.japanese\toff\npostal.english\ton\npostal.te_prefix\ton\n")
        #expect(parsed.warnings.isEmpty)
        #expect(!parsed.settings.postalJapanese)
        #expect(parsed.settings.postalEnglish)
        #expect(parsed.settings.postalTePrefix)
    }

    @Test func badValuesKeepTheDefaultAndWarn() {
        let parsed = Settings.parse(tsv: "postal.te_prefix\tmaybe\npostal.japanese\ttrue\n")
        #expect(parsed.warnings.count == 2)
        #expect(parsed.settings.postalJapanese)
        #expect(!parsed.settings.postalTePrefix)
    }
}
