// XPC サービスのエントリポイント（ADR 0004 §5.1）。
// NSXPCListener.service() のデリゲートで接続ごとに setCodeSigningRequirement を呼び、
// ikari_engine_open を1回だけ呼んで、handle は ikari_engine_handle の結果をそのまま返す。
//
// 中身を解釈しないという原則の唯一の例外: Shutdown（版の不一致のとき、殻が送る）だけは、返事を返したあとに
// エンジンを閉じてプロセスを終える。これをエンジン（Rust）の exit に任せないのは、返事を確実に届けるため。
//
// 相手の署名要件は、ビルド時に build.sh が自分の Info.plist（署名の対象）に書く（`IkariPeerRequirement`）。
// IkariXPC の `SigningRequirements` が唯一の出どころで、書かれていなければ全部の接続を断る。
import Foundation
import IkariProto

#if IKARI_HAS_ENGINE_FFI
import IkariEngineFFI
#endif

/// IkariXPC.IkariEngineXPC と同じ（セレクタと型が一致していれば接続できる。ObjC 名も揃える）
@objc(IkariEngineXPC)
protocol EngineXPC {
    func handle(_ request: Data, reply: @escaping (Data) -> Void)
}

enum ServiceConstants {
    static let requirementKey = "IkariPeerRequirement"
    static let dataDirectoryComponents = ["Library", "Application Support", "ikari-ime"]
    static let defaultIdleUnloadSeconds: UInt32 = 300
    /// 返事が相手に届くのを待ってから終わる
    static let exitDelay: TimeInterval = 0.1
    /// エンジンを開けなかったあと、開き直すまでの最短の間隔（要求のたびに辞書を開かない）
    static let reopenInterval: Duration = .seconds(2)
}

typealias Envelope = Ikari_Engine_V1_Envelope

/// エンジン（Rust）の持ち主。1つだけ作り、すべての接続で共有する。
///
/// 開けなかったとき（別のプロセスが `engine.lock` を持っているなど）は、そのまま諦めずに、要求が来たときに
/// 間を空けて開き直す。ロックが空けば、XPC サービスを作り直さなくても変換できるようになる。
final class EngineHost: @unchecked Sendable {
#if IKARI_HAS_ENGINE_FFI
    private let state = NSLock()
    private var engine: OpaquePointer?
    private var lastOpenAttempt: ContinuousClock.Instant?
    private var closed = false
#endif

    init() {
#if IKARI_HAS_ENGINE_FFI
        _ = currentEngine()
#endif
    }

#if IKARI_HAS_ENGINE_FFI
    /// 開いているエンジン。なければ（前回から `reopenInterval` 以上たっていれば）開き直す
    private func currentEngine() -> OpaquePointer? {
        state.lock()
        defer { state.unlock() }
        if let engine { return engine }
        if closed { return nil }
        let now = ContinuousClock.now
        if let last = lastOpenAttempt, now - last < ServiceConstants.reopenInterval { return nil }
        lastOpenAttempt = now
        engine = openEngine()
        return engine
    }

    private func openEngine() -> OpaquePointer? {
        var config = Ikari_Engine_V1_EngineConfig()
        config.dataDir = Self.dataDirectory()
        config.enableLm = true
        config.idleUnloadSecs = ServiceConstants.defaultIdleUnloadSeconds
        let bytes = (try? config.serializedData()) ?? Data()
        var failure = IkariBuf(ptr: nil, len: 0)
        let opened = bytes.withUnsafeBytes { raw in
            ikari_engine_open(raw.bindMemory(to: UInt8.self).baseAddress, bytes.count, &failure)
        }
        if opened == nil {
            // 文字列は出さない（NFR-19）。ENGINE_NOT_READY を返して殻にフォールバックさせ、次の要求で開き直す
            FileHandle.standardError.write(Data("IkariEngine: engine open failed\n".utf8))
            ikari_buf_free(failure)
        }
        return opened
    }
#endif

    /// 実際のホーム。Sandbox の中では `NSHomeDirectory()` がコンテナを指すので使わない
    static func dataDirectory() -> String {
        var home = NSHomeDirectory()
        if let entry = getpwuid(getuid()), let dir = entry.pointee.pw_dir {
            home = String(cString: dir)
        }
        return ServiceConstants.dataDirectoryComponents.reduce(home) { ($0 as NSString).appendingPathComponent($1) }
    }

    func handle(_ request: Data) -> Data {
#if IKARI_HAS_ENGINE_FFI
        guard let engine = currentEngine() else { return Self.notReady(request) }
        let buffer = request.withUnsafeBytes { raw in
            ikari_engine_handle(engine, raw.bindMemory(to: UInt8.self).baseAddress, request.count)
        }
        defer { ikari_buf_free(buffer) }
        guard let pointer = buffer.ptr else { return Self.notReady(request) }
        return Data(bytes: pointer, count: buffer.len)
#else
        return Self.notReady(request)
#endif
    }

    func close() {
#if IKARI_HAS_ENGINE_FFI
        state.lock()
        let closing = engine
        engine = nil
        closed = true
        state.unlock()
        if let closing { ikari_engine_close(closing) }
#endif
    }

    static func notReady(_ request: Data) -> Data {
        var env = Envelope()
        env.schemaVersion = 1
        env.requestID = (try? Envelope(serializedBytes: request))?.requestID ?? 0
        var error = Ikari_Engine_V1_ErrorReply()
        error.code = .engineNotReady
        error.message = "engine not ready"
        env.error = error
        return (try? env.serializedData()) ?? Data()
    }

    static func isShutdown(_ request: Data) -> Bool {
        if case .shutdown? = (try? Envelope(serializedBytes: request))?.body { return true }
        return false
    }
}

final class EngineService: NSObject, EngineXPC, @unchecked Sendable {
    private let host: EngineHost

    init(host: EngineHost) { self.host = host }

    func handle(_ request: Data, reply: @escaping (Data) -> Void) {
        // 同じ接続の要求は順番に処理する。Convert は締め切りまでここで止まってよい
        let response = host.handle(request)
        reply(response)
        if EngineHost.isShutdown(request) {
            host.close()
            DispatchQueue.global().asyncAfter(deadline: .now() + ServiceConstants.exitDelay) { exit(EXIT_SUCCESS) }
        }
    }
}

final class ListenerDelegate: NSObject, NSXPCListenerDelegate, @unchecked Sendable {
    private let service: EngineService
    private let peerRequirement: String?

    init(service: EngineService, peerRequirement: String?) {
        self.service = service
        self.peerRequirement = peerRequirement
    }

    func listener(_ listener: NSXPCListener, shouldAcceptNewConnection connection: NSXPCConnection) -> Bool {
        // 要件が分からないときは誰も受け付けない
        guard let peerRequirement else { return false }
        connection.setCodeSigningRequirement(peerRequirement)
        connection.exportedInterface = NSXPCInterface(with: EngineXPC.self)
        connection.exportedObject = service
        connection.resume()
        return true
    }
}

let peerRequirement = Bundle.main.object(forInfoDictionaryKey: ServiceConstants.requirementKey) as? String
if peerRequirement == nil {
    FileHandle.standardError.write(Data("IkariEngine: no peer requirement; rejecting every connection\n".utf8))
}
let host = EngineHost()
let delegate = ListenerDelegate(service: EngineService(host: host), peerRequirement: peerRequirement)
let listener = NSXPCListener.service()
listener.delegate = delegate
listener.resume()
