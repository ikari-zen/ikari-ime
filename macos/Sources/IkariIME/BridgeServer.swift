// 設定アプリとのブリッジの殻側（ADR 0005 §6.3、docs/phase2-contract.md §5.1）。
// 殻が Unix ドメインソケット（`run/bridge.sock`。`run/` は 0700、ソケットは 0600）で待ち、受けた接続の相手を
// `LOCAL_PEERTOKEN` の audit token から確かめる（S0-5 で動作を確認済み）。同時に受ける接続は 1 つだけ。
// 枠の読み書きと中継の許可リストは `IkariBridge`（契約。ここでは変えない）を使う。
//
// ログには、種類とエラーコードだけを書く（読み、表記、辞書の語、設定の値は書かない。NFR-19）。
//
// 注意: Package.swift の `IkariIME` ターゲットが `IkariBridge` と `IkariProto` に依存し、`IKARI_HAS_BRIDGE` を定義していること
// が前提（契約ファイルなのでレーン S3 は変えない。変更案はレーン S3 の最終報告に書いた）。
// 定義がないビルドでは、何もしない代わりの実装になる（`#else`）。`canImport(IkariBridge)` は使えない:
// ビルドの状態によってモジュールが見えてしまい、リンクだけが失敗する。
#if IKARI_HAS_BRIDGE
import Darwin
import Foundation
import IkariBridge
import IkariProto
import IkariShellCore
import IkariXPC
import Security

final class BridgeServer: @unchecked Sendable {
    /// `sys/un.h` の `SOL_LOCAL` と `LOCAL_PEERTOKEN`（マクロの取り込みに頼らない）
    private static let solLocal: Int32 = 0
    private static let localPeerToken: Int32 = 0x006
    private static let directoryPermissions: mode_t = 0o700
    private static let socketPermissions: mode_t = 0o600
    /// 設定アプリへの書き込みが進まない時間の上限。超えたら接続を閉じる（設定アプリはつなぎ直す。issue #44）。
    /// 暫定値（掃引していない）。相手が一時停止したときに、送る側のスレッドと `sendLock` を待たせる上限になる
    static let sendTimeout: Duration = .seconds(2)

    private let socketURL: URL
    private let store: SettingsStore
    private let engine: any EngineClient
    private let shellVersion: String
    private let peerValidator: @Sendable (Int32) -> Bool

    private let lock = NSLock()
    private var listener: Int32 = -1
    private var connection: BridgeTransport?
    /// `BridgeHello` を交わし終えた接続だけに、殻からの通知（`Event`）を送る
    private var connectionReady = false

    /// 殻からの通知は、このキューで順に送る。呼び出し元（main、入力の経路）は、相手が読まなくても止まらない
    private let eventQueue = DispatchQueue(label: "\(Identity.productID).bridge-event", qos: .utility)

    /// 中継は別のキューで並行に動かす（インポートで長く待つ間も、設定の取得や更新を止めない）
    private let relayQueue = DispatchQueue(
        label: "\(Identity.productID).bridge-relay", qos: .utility, attributes: .concurrent)

    /// `peerValidator` は接続したソケットの記述子を受け、相手が設定アプリなら真。既定は署名の確認
    /// （テストだけが差し替える）
    init(
        directory: URL, store: SettingsStore, engine: any EngineClient, signing: SigningMode, shellVersion: String,
        peerValidator: (@Sendable (Int32) -> Bool)? = nil
    ) {
        socketURL = directory.appendingPathComponent(BridgeProtocol.socketRelativePath)
        self.store = store
        self.engine = engine
        self.shellVersion = shellVersion
        let requirement = SigningRequirements.settingsApp(mode: signing)
        self.peerValidator = peerValidator ?? { Self.peerSatisfies(requirement: requirement, descriptor: $0) }
    }

    var socketPath: String { socketURL.path }

    // MARK: 起動と停止

    /// ソケットを作って待ち始める。作れなければログに残して何もしない（入力は止めない）
    @discardableResult
    func start() -> Bool {
        guard let descriptor = makeListener() else { return false }
        lock.lock()
        listener = descriptor
        lock.unlock()
        let thread = Thread { [self] in acceptLoop(descriptor) }
        thread.name = "ikari.bridge.accept"
        thread.qualityOfService = .utility
        thread.start()
        AppContext.log.info("bridge: listening")
        return true
    }

    func stop() {
        lock.lock()
        let descriptor = listener
        listener = -1
        let active = connection
        lock.unlock()
        if descriptor >= 0 {
            _ = Darwin.shutdown(descriptor, SHUT_RDWR)
            Darwin.close(descriptor)
        }
        active?.close()
        unlink(socketURL.path)
    }

    // MARK: 殻からの通知

    /// 設定が変わった（メニュー、設定アプリ、手の編集）。つながっている設定アプリは取り直す
    func notifySettingsChanged() {
        sendEvent { $0.settingsChanged = Ikari_Bridge_V1_SettingsChanged() }
    }

    /// すでに動いている設定アプリに、開くタブを伝える（起動の引数は動作中のアプリには届かない）。つながっていなければ false。
    /// 送るのは別のキューなので、true は「送る順番に入れた」の意味（届いたことまでは分からない）
    @discardableResult
    func sendOpenTab(_ tab: SettingsTab) -> Bool {
        sendEvent {
            var open = Ikari_Bridge_V1_OpenTab()
            open.tab = tab.rawValue
            $0.openTab = open
        }
    }

    /// 通知を `eventQueue` に入れて、すぐ戻る。書き込みは `sendTimeout` で打ち切られ、そのときは接続が閉じる
    @discardableResult
    private func sendEvent(_ build: (inout Ikari_Bridge_V1_Event) -> Void) -> Bool {
        lock.lock()
        let target = connectionReady ? connection : nil
        lock.unlock()
        guard let target else { return false }
        var event = Ikari_Bridge_V1_Event()
        build(&event)
        var frame = Frame()
        frame.requestID = 0
        frame.event = event
        eventQueue.async { [self] in send(frame, on: target) }
        return true
    }

    // MARK: ソケット

    private func makeListener() -> Int32? {
        let directory = socketURL.deletingLastPathComponent()
        do {
            try FileManager.default.createDirectory(
                at: directory, withIntermediateDirectories: true,
                attributes: [.posixPermissions: Int(Self.directoryPermissions)])
        } catch {
            AppContext.log.error("bridge: cannot create the run directory")
            return nil
        }
        // すでにあるディレクトリの権限も 0700 にそろえる
        chmod(directory.path, Self.directoryPermissions)
        unlink(socketURL.path)

        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let path = Array(socketURL.path.utf8CString)
        let capacity = MemoryLayout.size(ofValue: address.sun_path)
        guard path.count <= capacity else {
            AppContext.log.error("bridge: socket path is too long")
            return nil
        }
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            for (index, byte) in path.enumerated() { raw[index] = UInt8(bitPattern: byte) }
        }

        let descriptor = socket(AF_UNIX, SOCK_STREAM, 0)
        guard descriptor >= 0 else {
            AppContext.log.error("bridge: socket() failed: \(errno)")
            return nil
        }
        _ = fcntl(descriptor, F_SETFD, FD_CLOEXEC)
        let bound = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(descriptor, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard bound == 0 else {
            AppContext.log.error("bridge: bind() failed: \(errno)")
            Darwin.close(descriptor)
            return nil
        }
        // `run/` が 0700 なので、権限を絞るまでの間も他のユーザーは入れない
        chmod(socketURL.path, Self.socketPermissions)
        guard listen(descriptor, 1) == 0 else {
            AppContext.log.error("bridge: listen() failed: \(errno)")
            Darwin.close(descriptor)
            unlink(socketURL.path)
            return nil
        }
        return descriptor
    }

    private func acceptLoop(_ descriptor: Int32) {
        while true {
            let client = accept(descriptor, nil, nil)
            if client < 0 {
                if errno == EINTR { continue }
                // 停止（記述子を閉じた）か、致命的なエラー
                lock.lock()
                let stopped = listener != descriptor
                lock.unlock()
                if !stopped { AppContext.log.error("bridge: accept() failed: \(errno)") }
                return
            }
            admit(client)
        }
    }

    /// 相手を確かめ、すでに接続があれば断る。通れば、専用のスレッドで受け付ける
    private func admit(_ client: Int32) {
        guard Self.peerHasSameUser(client), peerValidator(client) else {
            AppContext.log.error("bridge: rejected a peer (signature)")
            Darwin.close(client)
            return
        }
        let transport = FileDescriptorTransport(descriptor: client, sendTimeout: Self.sendTimeout)
        lock.lock()
        guard connection == nil else {
            lock.unlock()
            AppContext.log.error("bridge: rejected a second connection")
            transport.close()
            return
        }
        connection = transport
        connectionReady = false
        lock.unlock()

        let thread = Thread { [self] in serve(transport) }
        thread.name = "ikari.bridge.serve"
        thread.qualityOfService = .utility
        thread.start()
    }

    // MARK: 相手の確認

    private static func peerHasSameUser(_ descriptor: Int32) -> Bool {
        var uid: uid_t = 0
        var gid: gid_t = 0
        return getpeereid(descriptor, &uid, &gid) == 0 && uid == geteuid()
    }

    /// `getsockopt(LOCAL_PEERTOKEN)` の audit token から、相手のコードが署名要件を満たすか
    static func peerSatisfies(requirement text: String, descriptor: Int32) -> Bool {
        var token = audit_token_t()
        var length = socklen_t(MemoryLayout<audit_token_t>.size)
        guard getsockopt(descriptor, solLocal, localPeerToken, &token, &length) == 0 else { return false }
        let tokenData = withUnsafeBytes(of: &token) { Data($0) }

        var code: SecCode?
        let attributes = [kSecGuestAttributeAudit: tokenData] as CFDictionary
        guard SecCodeCopyGuestWithAttributes(nil, attributes, SecCSFlags(), &code) == errSecSuccess, let code else {
            return false
        }
        var requirement: SecRequirement?
        guard SecRequirementCreateWithString(text as CFString, SecCSFlags(), &requirement) == errSecSuccess,
            let requirement
        else { return false }
        return SecCodeCheckValidity(code, SecCSFlags(), requirement) == errSecSuccess
    }

    // MARK: 接続ごとの処理

    private func serve(_ transport: BridgeTransport) {
        defer {
            transport.close()
            lock.lock()
            connection = nil
            connectionReady = false
            lock.unlock()
            AppContext.log.info("bridge: disconnected")
        }
        // 最初の枠は BridgeHello。合わなければ VERSION_MISMATCH を返して閉じる
        guard let first = try? transport.receive(), case .hello(let hello)? = first.body else { return }
        let reply = BridgeHandshake.reply(to: hello, requestID: first.requestID, shellVersion: shellVersion)
        guard (try? transport.send(reply)) != nil else { return }
        if case .error? = reply.body { return }
        lock.lock()
        connectionReady = true
        lock.unlock()
        AppContext.log.info("bridge: connected")

        while true {
            do {
                handle(try transport.receive(), on: transport)
            } catch BridgeTransportError.frame {
                AppContext.log.error("bridge: broken frame")
                return
            } catch {
                return
            }
        }
    }

    private func handle(_ frame: Frame, on transport: BridgeTransport) {
        if let rejection = RelayPolicy.rejection(for: frame) {
            send(rejection, on: transport)
            return
        }
        switch frame.body {
        case .getSettings?:
            send(snapshotFrame(requestID: frame.requestID, settings: store.settings), on: transport)
        case .updateSettings(let update)?:
            send(apply(update, requestID: frame.requestID), on: transport)
        case .engine(let envelope)?:
            relayQueue.async { [self] in relay(envelope, requestID: frame.requestID, on: transport) }
        default:
            // 設定アプリから送ってよいのは、上の要求だけ（二度目の Hello、殻からの通知や返事の種類は断る）
            send(.error(requestID: frame.requestID, code: .notAllowed), on: transport)
        }
    }

    private func apply(_ update: Ikari_Bridge_V1_UpdateSettings, requestID: UInt64) -> Frame {
        do {
            let snapshot = try store.update(
                set: update.set.map { (key: $0.key, value: $0.value) }, reset: update.reset)
            return snapshotFrame(requestID: requestID, settings: snapshot.settings)
        } catch SettingsStore.UpdateError.invalid(let key) {
            // key は不正だった設定のキー名だけ（値は入れない）
            return .error(requestID: requestID, code: .invalidSetting, key: key)
        } catch {
            return .error(requestID: requestID, code: .writeFailed)
        }
    }

    private func snapshotFrame(requestID: UInt64, settings: Settings) -> Frame {
        var snapshot = Ikari_Bridge_V1_SettingsSnapshot()
        snapshot.effective = settings.effectiveEntries.map { entry in
            var item = Ikari_Bridge_V1_SettingEntry()
            item.key = entry.key
            item.value = entry.value
            return item
        }
        var frame = Frame()
        frame.requestID = requestID
        frame.settings = snapshot
        return frame
    }

    /// 管理の要求をエンジンへそのまま流し、返事をそのまま返す（種類の許可リストは `handle` で確認済み。中身は解釈しない）
    private func relay(_ envelope: EngineEnvelope, requestID: UInt64, on transport: BridgeTransport) {
        let timeout = RelayPolicy.timeout(for: envelope.body)
        do {
            let request = try envelope.serializedData()
            let replyBytes = try engine.relay(request, timeout: timeout)
            let reply = try EngineEnvelope(serializedBytes: replyBytes)
            var frame = Frame()
            frame.requestID = requestID
            frame.engine = reply
            send(frame, on: transport)
        } catch {
            AppContext.log.error("bridge: relay failed")
            send(.error(requestID: requestID, code: .engineUnavailable), on: transport)
        }
    }

    private func send(_ frame: Frame, on transport: BridgeTransport) {
        do {
            try transport.send(frame)
        } catch BridgeTransportError.frame(let error) {
            // 返事が大きすぎる（エクスポートが上限を超えた）など。壊れた枠は送らず、理由だけ返す
            AppContext.log.error("bridge: cannot send a frame")
            if case .tooLarge = error { _ = try? transport.send(.error(requestID: frame.requestID, code: .tooLarge)) }
        } catch BridgeTransportError.io(let code) where code == ETIMEDOUT {
            // 相手が読まない（`sendTimeout`）。輸送が閉じたので、serve の受信が終わり、接続を片づける
            AppContext.log.error("bridge: send timed out; closing the connection")
            transport.close()
        } catch {
            transport.close()
        }
    }
}
#else
import Foundation
import IkariShellCore
import IkariXPC

/// `IKARI_HAS_BRIDGE` がないビルドでは、ブリッジを持たない（設定アプリはつながらない）。Package.swift に依存と定義を足すと有効になる
final class BridgeServer: @unchecked Sendable {
    init(
        directory: URL, store: SettingsStore, engine: any EngineClient, signing: SigningMode, shellVersion: String,
        peerValidator: (@Sendable (Int32) -> Bool)? = nil
    ) {}

    @discardableResult
    func start() -> Bool {
        AppContext.log.error("bridge: IkariBridge is not linked; the settings app cannot connect")
        return false
    }

    func stop() {}
    func notifySettingsChanged() {}
    @discardableResult
    func sendOpenTab(_ tab: SettingsTab) -> Bool { false }
}
#endif
