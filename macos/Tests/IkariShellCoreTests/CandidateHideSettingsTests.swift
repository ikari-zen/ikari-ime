// 確信が高い変換で候補一覧を隠す設定（`candidate.hide_when_confident`。既定オフ。#69）
import Testing

@testable import IkariShellCore

@Suite struct CandidateHideSettingsTests {
    @Test func defaultIsOff() {
        #expect(!Settings.default.candidateHideWhenConfident)
    }

    @Test func parsesOnAndOff() {
        let on = Settings.parse(tsv: "candidate.hide_when_confident\ton\n")
        #expect(on.warnings.isEmpty)
        #expect(on.settings.candidateHideWhenConfident)
        let bad = Settings.parse(tsv: "candidate.hide_when_confident\ttrue\n")
        #expect(bad.warnings.count == 1)
        #expect(!bad.settings.candidateHideWhenConfident)
    }

    @Test func keyIsWritableAndShownInTheSnapshot() {
        #expect(Settings.isAcceptable(key: "candidate.hide_when_confident", value: "on"))
        #expect(!Settings.isAcceptable(key: "candidate.hide_when_confident", value: "yes"))
        var s = Settings.default
        s.candidateHideWhenConfident = true
        let entries = Dictionary(uniqueKeysWithValues: s.effectiveEntries.map { ($0.key, $0.value) })
        #expect(entries["candidate.hide_when_confident"] == "on")
    }

    @Test func engineIsNotRestarted() {
        var changed = Settings.default
        changed.candidateHideWhenConfident = true
        #expect(!Settings.default.engineSettingsDiffer(from: changed))
    }
}
