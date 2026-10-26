// XPC の下の層（ADR 0004 §5.1）。メッセージの意味は知らず、バイト列を運ぶだけ。
// テストでは `EngineTransport` をモックに差し替えて、XPCEngineClient の再接続や休止の処理を確かめる。
import Foundation
import IkariShellCore

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
    private let connection: NSXPCConnection

    init(serviceName: String, requirement: String, onDown: @escaping @Sendable (TransportDown) -> Void) {
        connection = NSXPCConnection(serviceName: serviceName)
        connection.remoteObjectInterface = NSXPCInterface(with: IkariEngineXPC.self)
        connection.setCodeSigningRequirement(requirement)
        connection.interruptionHandler = { onDown(.interrupted) }
        connection.invalidationHandler = { onDown(.invalidated) }
        connection.resume()
    }

    var processIdentifier: Int32 { connection.processIdentifier }

    func request(_ data: Data, timeout: Duration) throws -> Data {
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
        let proxy = connection.remoteObjectProxyWithErrorHandler { _ in }
        (proxy as? IkariEngineXPC)?.handle(data) { _ in }
    }

    func close() {
        connection.invalidate()
    }
}
