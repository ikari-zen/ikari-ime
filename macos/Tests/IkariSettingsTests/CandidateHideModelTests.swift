// 「確信が高い変換では候補一覧を出さない」のモデル（#69）
import Foundation
import IkariShellCore
import Testing

@testable import IkariSettings

@Suite @MainActor struct CandidateHideModelTests {
    @Test func startsOffAndTogglesGoThroughTheShell() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        #expect(!model.values.candidateHideWhenConfident)
        await model.setCandidateHideWhenConfident(true)
        #expect(model.errorMessage == nil)
        #expect(model.values.candidateHideWhenConfident)
        #expect(harness.shell.state.snapshot.settings["candidate.hide_when_confident"] == "on")
        await model.setCandidateHideWhenConfident(false)
        #expect(harness.shell.state.snapshot.settings["candidate.hide_when_confident"] == "off")
    }
}
