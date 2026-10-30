// 設定の処理は shell-core（ikari_settings_*）を通すので、XCFramework のあるビルドだけで流す（ADR 0041 §2.3）
#if IKARI_HAS_SHELL_FFI
// 「確信が高い変換では候補一覧を出さない」のモデル（#69）
import Foundation
import Testing

@testable import IkariSettings

@Suite @MainActor struct CandidateHideModelTests {
    @Test func startsOnAndTogglesGoThroughTheShell() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        #expect(model.values.candidateHideWhenConfident)
        await model.setCandidateHideWhenConfident(false)
        #expect(model.errorMessage == nil)
        #expect(!model.values.candidateHideWhenConfident)
        #expect(harness.shell.state.snapshot.settings["candidate.hide_when_confident"] == "off")
        await model.setCandidateHideWhenConfident(true)
        #expect(harness.shell.state.snapshot.settings["candidate.hide_when_confident"] == "on")
    }
}

#endif
