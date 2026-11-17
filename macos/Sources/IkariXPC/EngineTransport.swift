// XPC の下の層（ADR 0004 §5.1）。メッセージの意味は知らず、バイト列を運ぶだけ。
// テストでは `EngineTransport` をモックに差し替えて、XPCEngineClient の再接続や休止の処理を確かめる。
import Foundation

/// 接続が使えなくなった理由。
enum TransportDown: Sendable {
    /// 相手のプロセスが落ちた（launchd が次のメッセージで起動し直す。接続は有効なまま）
    case interrupted
    /// 接続そのものが無効になった（作り直しが要る）
    case invalidated
}

protocol EngineTransport: AnyObject, Sendable {
    /// 返事を待つ。`timeout` を超えたら `EngineClientError.timeout`、接続の障害なら `.unavailable`
    func request(_ data: Data, timeout: Duration) throws -> Data
    /// 返事を待たない
    func post(_ data: Data)
    /// 接続を閉じる
    func close()
    /// 相手のプロセス ID（不明なら 0）
    var processIdentifier: Int32 { get }
}

typealias EngineTransportFactory = @Sendable (_ onDown: @escaping @Sendable (TransportDown) -> Void) -> EngineTransport

extension Duration {
    var timeInterval: TimeInterval {
        let (seconds, attoseconds) = components
        return TimeInterval(seconds) + TimeInterval(attoseconds) / 1e18
    }
}

/// 返事の受け渡し（セマフォで待つ側と、XPC のキューで受ける側の間）。
private final class ReplyBox: @unchecked Sendable {
    private let lock = NSLock()
    private var result: Result<Data, EngineClientError>?
    let semaphore = DispatchSemaphore(value: 0)

    func set(_ value: Result<Data, EngineClientError>) {
        lock.lock()
        defer { lock.unlock() }
        guard result == nil else { return }
        result = value
        semaphore.signal()
    }

    func get() -> Result<Data, EngineClientError>? {
        lock.lock()
        defer { lock.unlock() }
        return result
    }
}

/// `NSXPCConnection(serviceName:)` による実装。`setCodeSigningRequirement` を `resume()` の前に呼ぶ。
final class XPCTransport: EngineTransport, @unchecked Sendable {
    /// 口を渡す返事を待つ上限（エンジンの起動を含む）
    static let attachTimeout: Duration = .seconds(5)

    private let connection: NSXPCConnection
    private let cage: LMCageBroker?
    private let attachSerial = NSLock()
    private let attachLock = NSLock()
    /// 最後に渡した口の世代。nil は「まだ渡していない、またはエンジンが入れ替わった（渡し直す）」
    private var attachedGenerationValue: UInt64?

    init(
        serviceName: String, requirement: String, cage: LMCageBroker? = nil,
        onDown: @escaping @Sendable (TransportDown) -> Void
    ) {
        connection = NSXPCConnection(serviceName: serviceName)
        self.cage = cage
        connection.remoteObjectInterface = IkariEngineXPCInterface.make()
        connection.setCodeSigningRequirement(requirement)
        connection.interruptionHandler = { [weak self] in
            // エンジンが落ちた。新しいエンジンは口を持たないので、次の要求の前に渡し直す
            self?.forgetAttachment()
            onDown(.interrupted)
        }
        connection.invalidationHandler = { onDown(.invalidated) }
        connection.resume()
    }

    private func forgetAttachment() { setAttachedGeneration(nil) }

    /// 檻役の口が変わっていた、またはエンジンが入れ替わっていたら、口をエンジンに渡す。
    /// 渡せなくても要求は止めない（エンジンは「口が無い → LM なし」で動く。ADR 0077 §3.3）。
    /// 輸送の規則だけで、いつ LM を使うかの判断は持たない
    private func attachCageIfNeeded() {
        guard let cage else { return }
        // 渡す処理は 1 つずつ。世代の読み書きは `attachLock` だけ（返事を待つ間は持たない。落ちた通知が止まらないように）
        attachSerial.lock()
        defer { attachSerial.unlock() }
        guard let snapshot = cage.current(), snapshot.generation != attachedGeneration() else { return }
        let result = AttachResult()
        let proxy = connection.remoteObjectProxyWithErrorHandler { _ in result.finish(false) }
        guard let engine = proxy as? IkariEngineXPC else { return }
        engine.attachLMCage(snapshot.endpoint) { accepted in result.finish(accepted) }
        if result.wait(timeout: Self.attachTimeout), result.accepted {
            setAttachedGeneration(snapshot.generation)
        }
    }

    private func attachedGeneration() -> UInt64? {
        attachLock.lock()
        defer { attachLock.unlock() }
        return attachedGenerationValue
    }

    private func setAttachedGeneration(_ value: UInt64?) {
        attachLock.lock()
        attachedGenerationValue = value
        attachLock.unlock()
    }

    var processIdentifier: Int32 { connection.processIdentifier }

    func request(_ data: Data, timeout: Duration) throws -> Data {
        attachCageIfNeeded()
        let box = ReplyBox()
        let proxy = connection.remoteObjectProxyWithErrorHandler { _ in
            box.set(.failure(.unavailable))
        }
        guard let engine = proxy as? IkariEngineXPC else { throw EngineClientError.unavailable }
        engine.handle(data) { reply in box.set(.success(reply)) }
        let deadline = DispatchTime.now() + timeout.timeInterval
        if box.semaphore.wait(timeout: deadline) == .timedOut {
            // 遅れて届く返事は捨てる
            box.set(.failure(.timeout))
        }
        switch box.get() {
        case .success(let reply)?: return reply
        case .failure(let error)?: throw error
        case nil: throw EngineClientError.unavailable
        }
    }

    func post(_ data: Data) {
        attachCageIfNeeded()
        let proxy = connection.remoteObjectProxyWithErrorHandler { _ in }
        (proxy as? IkariEngineXPC)?.handle(data) { _ in }
    }

    func close() {
        connection.invalidate()
    }
}

/// 口を渡す返事の待ち
private final class AttachResult: @unchecked Sendable {
    private let semaphore = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var value = false

    var accepted: Bool {
        lock.lock()
        defer { lock.unlock() }
        return value
    }

    func finish(_ accepted: Bool) {
        lock.lock()
        value = accepted
        lock.unlock()
        semaphore.signal()
    }

    func wait(timeout: Duration) -> Bool {
        semaphore.wait(timeout: .now() + timeout.timeInterval) == .success
    }
}
