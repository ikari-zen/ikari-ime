// 確信が高い変換で候補一覧を隠す設定（`candidate.hide_when_confident`。既定オン。#69）
import Testing

@testable import IkariShellCore

@Suite struct CandidateHideSettingsTests {
    @Test func defaultIsOn() {
        #expect(Settings.default.candidateHideWhenConfident)
    }

    @Test func parsesOnAndOff() {
        let off = Settings.parse(tsv: "candidate.hide_when_confident\toff\n")
        #expect(off.warnings.isEmpty)
        #expect(!off.settings.candidateHideWhenConfident)
        let on = Settings.parse(tsv: "candidate.hide_when_confident\ton\n")
        #expect(on.warnings.isEmpty)
        #expect(on.settings.candidateHideWhenConfident)
        // 値が不正なら警告して既定（オン）のまま
        let bad = Settings.parse(tsv: "candidate.hide_when_confident\ttrue\n")
        #expect(bad.warnings.count == 1)
        #expect(bad.settings.candidateHideWhenConfident)
    }
}
