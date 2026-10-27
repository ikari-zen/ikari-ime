import Foundation
import Testing

@testable import IkariEngineService

/// 解放の記録つきの資源（エンジンの代わり）
private final class FakeEngine: @unchecked Sendable {
    private let lock = NSLock()
    private var _released = false
    private var _usedAfterRelease = false

    var released: Bool { lock.withLock { _released } }
    var usedAfterRelease: Bool { lock.withLock { _usedAfterRelease } }

    func use() { lock.withLock { if _released { _usedAfterRelease = true } } }
    func release() { lock.withLock { _released = true } }
}

/// 一度だけ開く門
private func openedGate(_ engine: FakeEngine) -> EngineGate<FakeEngine> {
    let gate = EngineGate<FakeEngine>()
    if gate.enter(open: { engine }) != nil { gate.leave() }
    return gate
}

private enum Timing {
    /// 処理中のまま close を呼んでから、処理を終えるまでの時間
    static let holdSeconds: TimeInterval = 0.3
    /// close の待ちの上限（処理より十分長い）
    static let longTimeout: TimeInterval = 10
    /// close の待ちの上限（処理より短い）
    static let shortTimeout: TimeInterval = 0.05
}

@Suite struct EngineGateTests {
    @Test func closeWaitsForInFlightRequest() {
        let engine = FakeEngine()
        let gate = openedGate(engine)
        let entered = DispatchSemaphore(value: 0)
        let finish = DispatchSemaphore(value: 0)
        let worker = Thread {
            _ = gate.withResource(open: { nil }) { resource in
                entered.signal()
                finish.wait()
                resource.use()
            }
        }
        worker.start()
        entered.wait()

        let closeStarted = Date()
        DispatchQueue.global().asyncAfter(deadline: .now() + Timing.holdSeconds) {
            // 処理の終わりまで解放されていないこと
            #expect(!engine.released)
            finish.signal()
        }
        let outcome = gate.close(timeout: Timing.longTimeout)
        guard case .release(let closing) = outcome else {
            Issue.record("expected release, got \(outcome)")
            return
        }
        closing.release()
        #expect(Date().timeIntervalSince(closeStarted) >= Timing.holdSeconds * 0.9)
        #expect(!engine.usedAfterRelease)
    }

    @Test func requestAfterCloseIsRejected() {
        let engine = FakeEngine()
        let gate = openedGate(engine)
        guard case .release = gate.close(timeout: Timing.longTimeout) else {
            Issue.record("expected release")
            return
        }
        var opened = false
        let result: Int? = gate.withResource(open: {
            opened = true
            return FakeEngine()
        }) { _ in 1 }
        #expect(result == nil)
        #expect(!opened)  // closed のあとは開き直さない
        #expect(gate.isClosed)
    }

    @Test func closeTimesOutAndLeavesResource() {
        let engine = FakeEngine()
        let gate = openedGate(engine)
        guard gate.enter(open: { nil }) != nil else {
            Issue.record("enter failed")
            return
        }
        // 処理中のまま上限を超えたら、解放せずに残す
        guard case .leaked = gate.close(timeout: Timing.shortTimeout) else {
            Issue.record("expected leaked")
            return
        }
        gate.leave()
        // 処理中がなくなれば、呼び直しで解放できる
        guard case .release = gate.close(timeout: Timing.shortTimeout) else {
            Issue.record("expected release after leave")
            return
        }
        guard case .nothing = gate.close(timeout: Timing.shortTimeout) else {
            Issue.record("expected nothing on second release")
            return
        }
    }

    @Test func failedOpenDoesNotCountAsInFlight() {
        let gate = EngineGate<FakeEngine>()
        #expect(gate.enter(open: { nil }) == nil)
        guard case .nothing = gate.close(timeout: Timing.shortTimeout) else {
            Issue.record("expected nothing")
            return
        }
    }
}
