import Foundation
import Testing
@preconcurrency import XPC

@testable import IkariXPC

/// 檻役の口の仲立ち（ADR 0077 §3.5）。実際の XPC サービスには触れず、取得を差し替えて確かめる
@Suite struct LMCageBrokerTests {
    /// 取得の呼び出し回数と、`onLost` を取っておく
    final class FakeSource: @unchecked Sendable {
        private let lock = NSLock()
        private(set) var fetchCount = 0
        private(set) var closeCount = 0
        private var lostHandlers: [@Sendable () -> Void] = []
        var succeeds = true

        var fetcher: LMCageBroker.Fetcher {
            { [self] onLost in
                lock.lock()
                defer { lock.unlock() }
                fetchCount += 1
                guard succeeds else { return nil }
                lostHandlers.append(onLost)
                // 匿名の口の受け手（檻役の `listener` と同じ作り）
                let connection = xpc_connection_create(nil, nil)
                xpc_connection_set_event_handler(connection) { _ in }
                xpc_connection_resume(connection)
                return LMCageBroker.Fetched(
                    endpoint: xpc_endpoint_create(connection),
                    close: { [self] in
                        lock.lock()
                        closeCount += 1
                        lock.unlock()
                    })
            }
        }

        func loseLatest() {
            lock.lock()
            let handler = lostHandlers.last
            lock.unlock()
            handler?()
        }
    }

    final class Clock: @unchecked Sendable {
        var date = Date(timeIntervalSince1970: 1_000)
    }

    @Test func fetchesOnceAndKeepsTheGeneration() {
        let source = FakeSource()
        let broker = LMCageBroker(fetcher: source.fetcher)
        let first = broker.current()
        let second = broker.current()
        #expect(first?.generation == second?.generation)
        #expect(source.fetchCount == 1)
    }

    @Test func lostConnectionAdvancesTheGenerationOnTheNextFetch() throws {
        let source = FakeSource()
        let broker = LMCageBroker(fetcher: source.fetcher)
        let first = try #require(broker.current())
        source.loseLatest()
        #expect(broker.currentGeneration == nil)
        #expect(source.closeCount == 1)
        let second = try #require(broker.current())
        #expect(second.generation > first.generation)
        #expect(source.fetchCount == 2)
    }

    @Test func failedFetchIsNotRetriedUntilTheIntervalPasses() {
        let source = FakeSource()
        source.succeeds = false
        let clock = Clock()
        let broker = LMCageBroker(fetcher: source.fetcher, now: { clock.date })
        #expect(broker.current() == nil)
        #expect(broker.current() == nil)
        #expect(source.fetchCount == 1)
        clock.date.addTimeInterval(LMCageBroker.retryInterval + 1)
        source.succeeds = true
        #expect(broker.current() != nil)
        #expect(source.fetchCount == 2)
    }

    @Test func engineInterfaceDeclaresTheEndpointArgument() {
        // 宣言が不正なら、ここで落ちる（NSXPCInterface は、selector の引数の型を実行時に検査する）
        let interface = IkariEngineXPCInterface.make()
        #expect(NSStringFromProtocol(interface.protocol) == "IkariEngineXPC")
    }
}
