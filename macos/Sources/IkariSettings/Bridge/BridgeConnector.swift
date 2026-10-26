// 殻のソケットへのつなぎ方（ADR 0005 §6.3）と、つながらないときの再試行の方針。
import Foundation
import IkariBridge

/// 輸送を作る。本物は Unix ドメインソケット、`--fake-bridge` とテストはメモリ上の対。
protocol BridgeConnecting: Sendable {
    func makeTransport() throws -> any BridgeTransport
}

enum SocketConnectError: Error, Equatable {
    /// ソケットのパスが `sockaddr_un` に入らない
    case pathTooLong
    /// 殻が待っていない（ソケットがない、または接続を断られた）
    case notListening
    case io(Int32)
}

/// 殻が `~/Library/Application Support/ikari-ime/run/bridge.sock` で待つ（docs/phase2-contract.md §5.1）。
struct SocketConnector: BridgeConnecting {
    var socketPath: String

    init(socketPath: String = SocketConnector.defaultSocketPath()) {
        self.socketPath = socketPath
    }

    static func defaultSocketPath(home: URL = FileManager.default.homeDirectoryForCurrentUser) -> String {
        home.appendingPathComponent("Library/Application Support/ikari-ime", isDirectory: true)
            .appendingPathComponent(BridgeProtocol.socketRelativePath).path
    }

    func makeTransport() throws -> any BridgeTransport {
        let descriptor = socket(AF_UNIX, SOCK_STREAM, 0)
        guard descriptor >= 0 else { throw SocketConnectError.io(errno) }
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let capacity = MemoryLayout.size(ofValue: address.sun_path)
        let bytes = Array(socketPath.utf8)
        guard bytes.count < capacity else {
            Darwin.close(descriptor)
            throw SocketConnectError.pathTooLong
        }
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            raw.copyBytes(from: bytes)
            raw[bytes.count] = 0
        }
        let length = socklen_t(MemoryLayout<sockaddr_un>.size)
        let result = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.connect(descriptor, $0, length) }
        }
        guard result == 0 else {
            let code = errno
            Darwin.close(descriptor)
            if code == ENOENT || code == ECONNREFUSED { throw SocketConnectError.notListening }
            throw SocketConnectError.io(code)
        }
        return FileDescriptorTransport(descriptor: descriptor)
    }
}

/// 自動の再接続の方針: 5 秒おきに 1 分まで（ADR 0005 §6.3「切れたとき」）。その後は手動の「再接続」。
enum ReconnectPolicy {
    static let interval: Duration = .seconds(5)
    static let window: Duration = .seconds(60)
    /// 最初の試行のあとに追加で試す回数（5 秒 × 12 = 60 秒）
    static var maxAutomaticRetries: Int { Int(window / interval) }

    /// `attempt` 回目（0 = 最初の試行）の失敗のあと、自動で再試行するか
    static func shouldRetry(afterFailedAttempt attempt: Int) -> Bool {
        attempt < maxAutomaticRetries
    }
}
