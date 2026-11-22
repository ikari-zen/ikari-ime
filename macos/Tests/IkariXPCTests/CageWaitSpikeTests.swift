// スパイク #536（捨てるコード。マージしない）。殻の輸送（XPCTransport）の要求 1 回が、檻役が応答しないときに
// どれだけ待つかを測る。本物の NSXPC（プロセスの中の匿名の口）と本物の libxpc を使い、エンジンは偽物
// （`attachLMCage` は Rust の `CageSlot::attach_unverified` と同じく、口に `hello` を送って最大 5 秒待ってから true）。
// 流し方: SPIKE536=1 swift test --filter CageWaitSpike
import Foundation
import Testing
@preconcurrency import XPC

@testable import IkariXPC

private let spikeEnabled = ProcessInfo.processInfo.environment["SPIKE536"] == "1"
/// Rust の `HELLO_REPLY_DEADLINE`（lm/src/host/cage.rs）
private let engineHelloDeadline: TimeInterval = 5

/// 放すと libxpc が相手に誤りを返すので、伝言と接続を持ったままにする
private final class Keep: @unchecked Sendable {
    static let shared = Keep()
    private let lock = NSLock()
    private var items: [AnyObject] = []
    func add(_ o: AnyObject) {
        lock.lock()
        items.append(o)
        lock.unlock()
    }
}

/// 応答しない檻役（接続も伝言も受けるが、返事をしない）
private func hungListener() -> xpc_connection_t {
    let listener = xpc_connection_create(nil, nil)
    xpc_connection_set_event_handler(listener) { peer in
        guard xpc_get_type(peer) == XPC_TYPE_CONNECTION else { return }
        xpc_connection_set_event_handler(peer) { message in Keep.shared.add(message) }
        xpc_connection_resume(peer)
        Keep.shared.add(peer)
    }
    xpc_connection_resume(listener)
    return listener
}

/// 偽のエンジン（handle はすぐ返す。attachLMCage は口に hello を送り、返事か 5 秒を待ってから true）
private final class FakeEngine: NSObject, IkariEngineXPC, NSXPCListenerDelegate, @unchecked Sendable {
    let listener = NSXPCListener.anonymous()

    override init() {
        super.init()
        listener.delegate = self
        listener.resume()
    }

    func listener(_ listener: NSXPCListener, shouldAcceptNewConnection c: NSXPCConnection) -> Bool {
        c.exportedInterface = IkariEngineXPCInterface.make()
        c.exportedObject = self
        c.resume()
        return true
    }

    func handle(_ request: Data, reply: @escaping (Data) -> Void) { reply(request) }

    func attachLMCage(_ endpoint: xpc_endpoint_t, reply: @escaping (Bool) -> Void) {
        let conn = xpc_connection_create_from_endpoint(endpoint)
        xpc_connection_set_event_handler(conn) { _ in }
        xpc_connection_resume(conn)
        let msg = xpc_dictionary_create(nil, nil, 0)
        xpc_dictionary_set_string(msg, "op", "hello")
        xpc_dictionary_set_uint64(msg, "v", 1)
        let sem = DispatchSemaphore(value: 0)
        xpc_connection_send_message_with_reply(conn, msg, DispatchQueue.global()) { _ in sem.signal() }
        _ = sem.wait(timeout: .now() + engineHelloDeadline)
        xpc_connection_cancel(conn)
        reply(true)  // FFI は確かめの結果に関わらず true（engine-ffi の attach_lm_cage）
    }
}

private enum Scenario: String, CaseIterable {
    /// 檻役なし（対照）
    case noCage
    /// 名前の口の返事が来ない（口を取る前に止まった・応答しない）
    case fetchStalls
    /// 口は取れたが、その先の檻役が応答しない（口を渡したあとに止まった）
    case endpointHung
    /// 口は取れたが、その先の檻役が落ちている
    case endpointDead
}

private func fetcher(for s: Scenario) -> LMCageBroker.Fetcher {
    switch s {
    case .noCage:
        return { _ in nil }
    case .fetchStalls:
        let l = hungListener()
        let ep = xpc_endpoint_create(l)
        // fetchFromService と同じ送り方と待ち（名前で探す代わりに匿名の口）
        return { _ in
            Keep.shared.add(l)
            let conn = xpc_connection_create_from_endpoint(ep)
            xpc_connection_set_event_handler(conn) { _ in }
            xpc_connection_resume(conn)
            let req = xpc_dictionary_create(nil, nil, 0)
            xpc_dictionary_set_string(req, LMCageNamedWire.keyOp, LMCageNamedWire.opEndpoint)
            xpc_dictionary_set_uint64(req, LMCageNamedWire.keyVersion, LMCageNamedWire.protocolVersion)
            let sem = DispatchSemaphore(value: 0)
            xpc_connection_send_message_with_reply(conn, req, DispatchQueue.global()) { _ in sem.signal() }
            _ = sem.wait(timeout: .now() + LMCageBroker.fetchTimeout.timeInterval)
            xpc_connection_cancel(conn)
            return nil
        }
    case .endpointHung:
        let l = hungListener()
        return { _ in
            Keep.shared.add(l)
            return LMCageBroker.Fetched(endpoint: xpc_endpoint_create(l), close: {})
        }
    case .endpointDead:
        let l = xpc_connection_create(nil, nil)
        xpc_connection_set_event_handler(l) { _ in }
        xpc_connection_resume(l)
        let ep = xpc_endpoint_create(l)
        xpc_connection_cancel(l)
        return { _ in LMCageBroker.Fetched(endpoint: ep, close: {}) }
    }
}

private func ms(_ start: DispatchTime) -> Double {
    Double(DispatchTime.now().uptimeNanoseconds - start.uptimeNanoseconds) / 1e6
}

@Suite(.serialized) struct CageWaitSpikeTests {
    @Test(.enabled(if: spikeEnabled)) func measure() throws {
        let runs = Int(ProcessInfo.processInfo.environment["SPIKE536_RUNS"] ?? "") ?? 3
        let keys = Int(ProcessInfo.processInfo.environment["SPIKE536_KEYS"] ?? "") ?? 40
        let gapMs = 150
        let only = ProcessInfo.processInfo.environment["SPIKE536_ONLY"].map { $0.split(separator: ",").map(String.init) }
        for scenario in Scenario.allCases where only?.contains(scenario.rawValue) ?? true {
            for run in 1...runs {
                let engine = FakeEngine()
                let broker = scenario == .noCage ? nil : LMCageBroker(fetcher: fetcher(for: scenario))
                let transport = XPCTransport(
                    spikeListenerEndpoint: engine.listener.endpoint, cage: broker, onDown: { _ in })
                var lat: [Double] = []
                var failures = 0
                let t0 = DispatchTime.now()
                for i in 0..<keys {
                    let t = DispatchTime.now()
                    if i % 2 == 0 {
                        do { _ = try transport.request(Data([1, 2, 3]), timeout: .seconds(1)) } catch { failures += 1 }
                    } else {
                        transport.post(Data([4]))
                    }
                    lat.append(ms(t))
                    Thread.sleep(forTimeInterval: Double(gapMs) / 1000)
                }
                let total = ms(t0)
                transport.close()
                let slow = lat.enumerated().filter { $0.element > 100 }.map { "#\($0.offset):\(Int($0.element))" }
                let sorted = lat.sorted()
                print(
                    "SPIKE536 \(scenario.rawValue) run=\(run) keys=\(keys) gap=\(gapMs)ms total=\(Int(total))ms "
                        + "max=\(String(format: "%.1f", sorted.last!))ms median=\(String(format: "%.2f", sorted[keys / 2]))ms "
                        + "slow(>100ms)=\(slow.count) failures=\(failures) [\(slow.joined(separator: " "))]")
            }
        }
    }
}
