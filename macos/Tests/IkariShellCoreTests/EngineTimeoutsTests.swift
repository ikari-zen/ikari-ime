import Testing

@testable import IkariShellCore

@Suite struct EngineTimeoutsTests {
    @Test func budgetFollowsBaseAndPerCharacterAndCap() {
        #expect(EngineTimeouts.lmBudget(readingCharacterCount: 0) == .milliseconds(30))
        #expect(EngineTimeouts.lmBudget(readingCharacterCount: 3) == .milliseconds(30))
        #expect(EngineTimeouts.lmBudget(readingCharacterCount: 7) == .milliseconds(42))
        #expect(EngineTimeouts.lmBudget(readingCharacterCount: 500) == .milliseconds(300))
        #expect(EngineTimeouts.lmBudget(readingCharacterCount: -1) == .milliseconds(30))
    }

    @Test func adminTimeoutsFollowThePhase3aContract() {
        #expect(EngineTimeouts.admin == .seconds(3))
        #expect(EngineTimeouts.adminTransfer == .seconds(30))
    }

    @Test func convertTimeoutAddsTheIpcMargin() {
        #expect(EngineTimeouts.convertTimeout(readingCharacterCount: 3) == .milliseconds(70))
    }
}
