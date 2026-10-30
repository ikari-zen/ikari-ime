// 設定アプリ側のブリッジ（ADR 0005 §6.3）。`BridgeTransport` の上で、要求と返事を request_id で対応づける。
// 本物のソケットでも、テストのソケット対（FakeShell が相手）でも同じコードが動く。
// 設定アプリは通信のコードを持たない（NFR-17）: ここは殻の Unix ソケットの記述子を読み書きするだけ。
import Foundation
import IkariBridge
import IkariProto
import IkariXPC

typealias EngineBody = EngineEnvelope.OneOf_Body
typealias SettingEntry = Ikari_Bridge_V1_SettingEntry

enum BridgeEvent: Equatable, Sendable {
    /// メニューや手の編集で設定が変わった。取り直す
    case settingsChanged
    /// 殻が指定したタブを開く（`SettingsTab` の rawValue）
    case openTab(String)
    /// 接続が切れた
    case disconnected
}

enum BridgeClientError: Error, Equatable {
    case disconnected
    /// `BridgeHello` の版が合わない（入れ替えた直後に古い殻が動いている）
    case versionMismatch
    case timeout
    case bridge(BridgeErrorCode, key: String)
    case engine(Ikari_Engine_V1_ErrorCode)
    /// 送る前に弾いた（管理の要求は 256 KiB、インポートは 64 MiB まで）
    case tooLarge
    case unexpectedReply
}

/// 画面のモデルが使う口。本物は `BridgeClient`。
protocol SettingsBridge: Sendable {
    var shellVersion: String { get }
    /// すべての既知のキーの、今効いている値
    func getSettings() async throws -> [String: String]
    /// 反映後の値を返す
    func updateSettings(set: [SettingEntry], reset: [String]) async throws -> [String: String]
    /// 管理の要求（または `Hello`）を送り、返事の本体を返す。`ErrorReply` は `BridgeClientError.engine` で投げる
    func engine(_ body: EngineBody) async throws -> EngineBody
}

enum AdminLimits {
    /// 管理の要求の上限（インポート以外）。エンジンの `MAX_REQUEST_BYTES` と同じ（ADR 0005 §8.3）
    static let maxRequestBytes = 256 * 1024
}

final class BridgeClient: SettingsBridge, @unchecked Sendable {
    /// 殻の待ちの上限（3 秒 / 30 秒）に足す余裕。殻が先に諦めて `ENGINE_UNAVAILABLE` を返せるようにする
    static let replyMargin: Duration = .seconds(5)
    /// 殻に対する要求（設定の取得と更新、Hello）の待ちの上限
    static let shellTimeout: Duration = .seconds(5)
    /// `Envelope.schema_version`（docs/phase2-contract.md §5.2。追加だけなので 1 のまま）
    private static let engineSchemaVersion: UInt32 = 1

    private struct Pending {
        let continuation: CheckedContinuation<Frame, Error>
        var timeout: Task<Void, Never>?
    }

    private let transport: any BridgeTransport
    private let sendQueue = DispatchQueue(label: "\(Identity.productID).settings.bridge.send")
    private let lock = NSLock()
    private var pending: [UInt64: Pending] = [:]
    private var nextID: UInt64 = 1
    private var closed = false
    private var storedShellVersion = ""
    private let eventContinuation: AsyncStream<BridgeEvent>.Continuation
    /// 殻からの通知と、切断。1 つの購読者だけが読む
    let events: AsyncStream<BridgeEvent>

    var shellVersion: String {
        lock.lock()
        defer { lock.unlock() }
        return storedShellVersion
    }

    /// 接続して `BridgeHello` を交わす。版が合わなければ `versionMismatch`。
    static func connect(transport: any BridgeTransport, appVersion: String) async throws -> BridgeClient {
        let client = BridgeClient(transport: transport)
        client.startReading()
        do {
            let reply = try await client.request(timeout: shellTimeout) { frame in
                frame = BridgeHandshake.hello(requestID: frame.requestID, appVersion: appVersion)
            }
            guard case .helloReply(let hello)? = reply.body else { throw BridgeClientError.unexpectedReply }
            client.setShellVersion(hello.shellVersion)
            return client
        } catch {
            client.close()
            throw error
        }
    }

    private init(transport: any BridgeTransport) {
        self.transport = transport
        (events, eventContinuation) = AsyncStream.makeStream(of: BridgeEvent.self, bufferingPolicy: .unbounded)
    }

    func close() {
        transport.close()
        failAll(BridgeClientError.disconnected)
    }

    // MARK: SettingsBridge

    func getSettings() async throws -> [String: String] {
        let reply = try await request(timeout: Self.shellTimeout) { $0.getSettings = Ikari_Bridge_V1_GetSettings() }
        return try Self.snapshot(from: reply)
    }

    func updateSettings(set: [SettingEntry], reset: [String]) async throws -> [String: String] {
        let reply = try await request(timeout: Self.shellTimeout) { frame in
            var update = Ikari_Bridge_V1_UpdateSettings()
            update.set = set
            update.reset = reset
            frame.updateSettings = update
        }
        return try Self.snapshot(from: reply)
    }

    func engine(_ body: EngineBody) async throws -> EngineBody {
        var envelope = EngineEnvelope()
        envelope.schemaVersion = Self.engineSchemaVersion
        envelope.body = body
        if !Self.isImport(body), envelope.serializedByteCount > AdminLimits.maxRequestBytes {
            throw BridgeClientError.tooLarge
        }
        let timeout = RelayPolicy.timeout(for: body) + Self.replyMargin
        let reply = try await request(timeout: timeout) { frame in
            envelope.requestID = frame.requestID
            frame.engine = envelope
        }
        guard case .engine(let answer)? = reply.body, let answerBody = answer.body else {
            throw BridgeClientError.unexpectedReply
        }
        if case .error(let error) = answerBody { throw BridgeClientError.engine(error.code) }
        return answerBody
    }

    // MARK: 内部

    private static func isImport(_ body: EngineBody) -> Bool {
        if case .importData = body { return true }
        return false
    }

    private static func snapshot(from frame: Frame) throws -> [String: String] {
        guard case .settings(let snapshot)? = frame.body else { throw BridgeClientError.unexpectedReply }
        var values: [String: String] = [:]
        for entry in snapshot.effective { values[entry.key] = entry.value }
        return values
    }

    private func setShellVersion(_ version: String) {
        lock.lock()
        storedShellVersion = version
        lock.unlock()
    }

    /// 要求を送って返事を待つ。`build` は `requestID` を入れた空の枠を受け取り、本体を入れる。
    /// 殻の `BridgeError` は `BridgeClientError` で投げる
    private func request(timeout: Duration, _ build: (inout Frame) -> Void) async throws -> Frame {
        let id = allocateID()
        var frame = Frame()
        frame.requestID = id
        build(&frame)
        let outgoing = frame
        let reply: Frame = try await withCheckedThrowingContinuation { continuation in
            lock.lock()
            if closed {
                lock.unlock()
                continuation.resume(throwing: BridgeClientError.disconnected)
                return
            }
            let timer = Task { [weak self] in
                try? await Task.sleep(for: timeout)
                self?.fail(id, BridgeClientError.timeout)
            }
            pending[id] = Pending(continuation: continuation, timeout: timer)
            lock.unlock()
            let transport = self.transport
            sendQueue.async { [weak self] in
                do {
                    try transport.send(outgoing)
                } catch BridgeTransportError.frame {
                    self?.fail(id, BridgeClientError.tooLarge)
                } catch {
                    self?.fail(id, BridgeClientError.disconnected)
                }
            }
        }
        if case .error(let error)? = reply.body {
            if error.code == .versionMismatch { throw BridgeClientError.versionMismatch }
            throw BridgeClientError.bridge(error.code, key: error.key)
        }
        return reply
    }

    private func allocateID() -> UInt64 {
        lock.lock()
        defer { lock.unlock() }
        defer { nextID += 1 }
        return nextID
    }

    private func take(_ id: UInt64) -> Pending? {
        lock.lock()
        defer { lock.unlock() }
        return pending.removeValue(forKey: id)
    }

    private func fail(_ id: UInt64, _ error: Error) {
        guard let entry = take(id) else { return }
        entry.timeout?.cancel()
        entry.continuation.resume(throwing: error)
    }

    private func failAll(_ error: Error) {
        lock.lock()
        let wasClosed = closed
        closed = true
        let all = pending
        pending = [:]
        lock.unlock()
        for entry in all.values {
            entry.timeout?.cancel()
            entry.continuation.resume(throwing: error)
        }
        if !wasClosed {
            eventContinuation.yield(.disconnected)
            eventContinuation.finish()
        }
    }

    private func startReading() {
        let transport = self.transport
        let thread = Thread { [weak self] in
            while true {
                do {
                    let frame = try transport.receive()
                    guard let self else { return }
                    self.dispatch(frame)
                } catch {
                    self?.failAll(BridgeClientError.disconnected)
                    return
                }
            }
        }
        thread.name = "ikari.settings.bridge.receive"
        thread.start()
    }

    private func dispatch(_ frame: Frame) {
        if case .event(let event)? = frame.body {
            switch event.kind {
            case .settingsChanged?: eventContinuation.yield(.settingsChanged)
            case .openTab(let open)?: eventContinuation.yield(.openTab(open.tab))
            case nil: break
            }
            return
        }
        guard let entry = take(frame.requestID) else { return }
        entry.timeout?.cancel()
        entry.continuation.resume(returning: frame)
    }
}

extension EngineEnvelope {
    fileprivate var serializedByteCount: Int { (try? serializedData().count) ?? Int.max }
}
