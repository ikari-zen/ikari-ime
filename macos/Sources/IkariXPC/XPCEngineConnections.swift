// shell-core のホストの口（`IkariShellHost` の engine_*。ADR 0041 §4.3）の XPC の実装（L-E）。
// メッセージの意味は知らず、接続ごとの番号（再接続の世代を兼ねる）で `XPCTransport` を引き、バイト列を運ぶだけ。
// 再接続、休止、要求番号、返事の検証は shell-core の `EngineLink` が持つ。
import Foundation

/// 落ちた通知の種類（`ikari_shell_engine_down` の kind）。
public enum EngineConnectionDown: Sendable {
    case interrupted, invalidated
}

/// 要求の失敗（トランスポートの誤りだけ。中身の誤りは shell-core が返事を読んで決める）。
public enum EngineConnectionFailure: Error, Sendable {
    case unavailable, timeout
}

/// 番号つきの XPC の接続の表。どのスレッドからでも並行に呼べる。
public final class XPCEngineConnections: @unchecked Sendable {
    public typealias DownHandler = @Sendable (_ connection: UInt64, _ kind: EngineConnectionDown) -> Void

    private let serviceName: String
    private let requirement: String
    private let onDown: DownHandler
    private let lock = NSLock()
    private var connections: [UInt64: XPCTransport] = [:]
    /// 次の番号。0 は「つながらない」なので使わない。使い回さない
    private var nextID: UInt64 = 1

    /// `onDown` は XPC のキューから呼ばれる（主スレッドが要求の返事を待っている間にも届く）
    public init(serviceName: String = IkariIdentifiers.engine, signing: SigningMode, onDown: @escaping DownHandler) {
        self.serviceName = serviceName
        requirement = SigningRequirements.engine(mode: signing)
        self.onDown = onDown
    }

    /// 新しい接続を作り、番号を返す（`resume` まで済ませる。相手のプロセスは最初の要求で起動する）
    public func connect() -> UInt64 {
        lock.lock()
        let id = nextID
        nextID &+= 1
        lock.unlock()
        let onDown = onDown
        let transport = XPCTransport(serviceName: serviceName, requirement: requirement) { down in
            switch down {
            case .interrupted: onDown(id, .interrupted)
            case .invalidated: onDown(id, .invalidated)
            }
        }
        lock.lock()
        connections[id] = transport
        lock.unlock()
        return id
    }

    /// 返事を同期に待つ
    public func request(_ connection: UInt64, _ data: Data, timeoutMs: UInt32) -> Result<Data, EngineConnectionFailure> {
        guard let transport = transport(connection) else { return .failure(.unavailable) }
        do {
            return .success(try transport.request(data, timeout: .milliseconds(Int64(timeoutMs))))
        } catch EngineClientError.timeout {
            return .failure(.timeout)
        } catch {
            return .failure(.unavailable)
        }
    }

    public func post(_ connection: UInt64, _ data: Data) {
        transport(connection)?.post(data)
    }

    public func close(_ connection: UInt64) {
        lock.lock()
        let transport = connections.removeValue(forKey: connection)
        lock.unlock()
        transport?.close()
    }

    /// 相手のプロセスの番号。分からなければ 0
    public func pid(_ connection: UInt64) -> Int32 {
        transport(connection)?.processIdentifier ?? 0
    }

    private func transport(_ connection: UInt64) -> XPCTransport? {
        lock.lock()
        defer { lock.unlock() }
        return connections[connection]
    }
}
