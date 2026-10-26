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

    @Test func keysAreWritableAndShownInTheSnapshot() {
        for key in ["postal.japanese", "postal.english", "postal.te_prefix"] {
            #expect(Settings.isAcceptable(key: key, value: "on"))
            #expect(!Settings.isAcceptable(key: key, value: "yes"))
        }
        var s = Settings.default
        s.postalTePrefix = true
        let entries = Dictionary(uniqueKeysWithValues: s.effectiveEntries.map { ($0.key, $0.value) })
        #expect(entries["postal.japanese"] == "on")
        #expect(entries["postal.english"] == "off")
        #expect(entries["postal.te_prefix"] == "on")
    }

    @Test func engineRestartsForTheKeysTheEngineReads() {
        let base = Settings.default
        var te = base
        te.postalTePrefix = true
        #expect(base.engineSettingsDiffer(from: te))
        var ja = base
        ja.postalJapanese = false
        #expect(base.engineSettingsDiffer(from: ja))
        // 英語のモードは殻が使うので、エンジンは起こし直さない
        var en = base
        en.postalEnglish = true
        #expect(!base.engineSettingsDiffer(from: en))
    }
}
