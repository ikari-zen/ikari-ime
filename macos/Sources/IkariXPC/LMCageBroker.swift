// 檻役（IkariLMCage.xpc）の匿名の口を、殻からエンジンへ仲立ちする部品（ADR 0077 §3.1、§3.5）。
//
// 判断の置き場所: Swift は輸送だけ。XPC サービスを名前で探せるのはアプリ本体（殻）だけで、エンジンは隣のサービスを
// 探せない（OS の制約。ADR 0072 §13.5）。ここは口を取って運ぶだけで、口の中身（起動、止める、fd）は見ない。
// 口の先が本物の檻役かの確認と、LM の判断（いつ起動するか）は Rust のエンジンが持つ。
import Foundation
@preconcurrency import XPC

/// 檻役の名前の口の取り決め（殻 → 檻役。口をもらうだけ）。匿名の口の中（エンジン ↔ 檻役）は `lm::host::cage::wire`
public enum LMCageNamedWire {
    /// 要求の語を入れるキー
    public static let keyOp = "op"
    /// 「匿名の口をください」
    public static let opEndpoint = "endpoint"
    /// 返事に入る匿名の口（`xpc_endpoint`）のキー
    public static let keyEndpoint = "endpoint"
    /// 取り決めの版を入れるキー（`lm::host::cage::wire::KEY_VERSION`）。檻役は、版が合わない要求に口を返さない
    public static let keyVersion = "v"
    /// 取り決めの版（`lm::host::cage::CAGE_PROTOCOL_VERSION`。判断は Rust 側。ここは同じ値を送るだけ）
    public static let protocolVersion: UInt64 = 1
}

/// 檻役の匿名の口の持ち主。どのスレッドからでも呼べる。
public final class LMCageBroker: @unchecked Sendable {
    /// 取った口と、名前の口の接続を閉じる関数（口を捨てるときに呼ぶ）
    public struct Fetched: @unchecked Sendable {
        public let endpoint: xpc_endpoint_t
        public let close: @Sendable () -> Void

        public init(endpoint: xpc_endpoint_t, close: @escaping @Sendable () -> Void) {
            self.endpoint = endpoint
            self.close = close
        }
    }

    /// 口を取りにいく。`onLost` は、その口の先の檻役が落ちた・切れたときに呼ばれる。取れなければ nil
    public typealias Fetcher = @Sendable (_ onLost: @escaping @Sendable () -> Void) -> Fetched?

    /// 口を取るのを待つ上限（檻役の起動を含む）
    static let fetchTimeout: Duration = .seconds(3)
    /// 取れなかったあと、取り直すまでの最短の間隔（要求のたびに待たない）
    static let retryInterval: TimeInterval = 5

    /// 口を取り直した回数。口が変わったかを、受け取る側が比べる
    public struct Snapshot: @unchecked Sendable {
        public let endpoint: xpc_endpoint_t
        public let generation: UInt64
    }

    private let fetcher: Fetcher
    private let now: @Sendable () -> Date
    private let lock = NSLock()
    private var endpoint: xpc_endpoint_t?
    private var closeConnection: (@Sendable () -> Void)?
    private var generation: UInt64 = 0
    private var lostCount: UInt64 = 0
    private var retryNotBefore: Date?
    /// 取っている間は 1 つだけにする
    private var fetching = false

    public init(fetcher: @escaping Fetcher, now: @escaping @Sendable () -> Date = { Date() }) {
        self.fetcher = fetcher
        self.now = now
    }

    /// 同梱の檻役につなぐ。`signing` で署名要件（相手 = 檻役）が決まる
    public convenience init(serviceName: String = IkariIdentifiers.lmCage, signing: SigningMode) {
        let requirement = SigningRequirements.lmCage(mode: signing)
        self.init(fetcher: { onLost in
            Self.fetchFromService(serviceName: serviceName, requirement: requirement, onLost: onLost)
        })
    }

    /// 今の口。まだ無ければ取りにいく（待つのは `fetchTimeout` まで）。取れなければ nil（LM なしのまま進む）
    public func current() -> Snapshot? {
        lock.lock()
        if let endpoint {
            let snapshot = Snapshot(endpoint: endpoint, generation: generation)
            lock.unlock()
            return snapshot
        }
        if fetching || (retryNotBefore.map { now() < $0 } ?? false) {
            lock.unlock()
            return nil
        }
        fetching = true
        let expected = lostCount
        lock.unlock()

        let fetched = fetcher { [weak self] in self?.lost(generationOf: expected) }

        lock.lock()
        defer { lock.unlock() }
        fetching = false
        guard let fetched else {
            retryNotBefore = now().addingTimeInterval(Self.retryInterval)
            return nil
        }
        // 取っている間に落ちた通知が来ていたら、この口は古い
        guard lostCount == expected else {
            fetched.close()
            retryNotBefore = nil
            return nil
        }
        retryNotBefore = nil
        endpoint = fetched.endpoint
        closeConnection = fetched.close
        generation &+= 1
        return Snapshot(endpoint: fetched.endpoint, generation: generation)
    }

    /// 檻役との接続が切れた。持っている口を捨てる（次の `current()` が取り直す。世代が進むので、受け取る側は渡し直す）
    private func lost(generationOf expected: UInt64) {
        lock.lock()
        defer { lock.unlock() }
        guard lostCount == expected else { return }
        lostCount &+= 1
        endpoint = nil
        let close = closeConnection
        closeConnection = nil
        retryNotBefore = nil
        close?()
    }

    /// 世代の比べ用: 持っている口がいま何世代目か（無ければ nil）
    public var currentGeneration: UInt64? {
        lock.lock()
        defer { lock.unlock() }
        return endpoint == nil ? nil : generation
    }

    // MARK: 実際の取得

    private static func fetchFromService(
        serviceName: String, requirement: String, onLost: @escaping @Sendable () -> Void
    ) -> Fetched? {
        let queue = DispatchQueue(label: "io.github.ikari-zen.lm-cage-broker")
        let connection = xpc_connection_create(serviceName, queue)
        xpc_connection_set_event_handler(connection) { event in
            // 口を取ったあとの切断（檻役が落ちた、切れた）だけを見る
            if xpc_get_type(event) == XPC_TYPE_ERROR { onLost() }
        }
        // 相手が檻役でなければ、返事は落とされる（口をもらう前に確かめる）
        guard xpc_connection_set_peer_code_signing_requirement(connection, requirement) == 0 else {
            xpc_connection_cancel(connection)
            return nil
        }
        xpc_connection_resume(connection)

        let request = xpc_dictionary_create(nil, nil, 0)
        xpc_dictionary_set_string(request, LMCageNamedWire.keyOp, LMCageNamedWire.opEndpoint)
        xpc_dictionary_set_uint64(request, LMCageNamedWire.keyVersion, LMCageNamedWire.protocolVersion)

        let box = EndpointBox()
        xpc_connection_send_message_with_reply(connection, request, queue) { reply in
            if xpc_get_type(reply) == XPC_TYPE_DICTIONARY,
                let value = xpc_dictionary_get_value(reply, LMCageNamedWire.keyEndpoint),
                xpc_get_type(value) == XPC_TYPE_ENDPOINT
            {
                box.set(value)
            }
            box.signal()
        }
        guard box.wait(timeout: fetchTimeout), let endpoint = box.value else {
            xpc_connection_cancel(connection)
            return nil
        }
        // 名前の口は、口をもらったあとも持つ（切れたことに気づくため）。口を捨てるときに閉じる
        return Fetched(endpoint: endpoint, close: { xpc_connection_cancel(connection) })
    }
}

/// 取得の返事を待つ箱
private final class EndpointBox: @unchecked Sendable {
    private let semaphore = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var stored: xpc_endpoint_t?

    var value: xpc_endpoint_t? {
        lock.lock()
        defer { lock.unlock() }
        return stored
    }

    func set(_ endpoint: xpc_object_t) {
        lock.lock()
        stored = endpoint
        lock.unlock()
    }

    func signal() { semaphore.signal() }

    func wait(timeout: Duration) -> Bool {
        semaphore.wait(timeout: .now() + timeout.timeInterval) == .success
    }
}
