// 設定アプリとのブリッジの殻側の輸送（ADR 0005 §6.3、docs/phase2-contract.md §5.1、ADR 0041 §2.2）。
// 殻が Unix ドメインソケット（`run/bridge.sock`。`run/` は 0700、ソケットは 0600）で待ち、受けた接続の相手を
// 同じユーザーかと `LOCAL_PEERTOKEN` の audit token の署名で確かめる。同時に受ける接続は 1 つだけ。
// 枠の中身の判断（最初の枠の照合、設定の取得と書き換え、中継の許可と同時数、通知）は shell-core が持つ。
// ここは、受けた枠を `onFrame` で渡し、shell-core が返した枠を送り、頼まれたら閉じるだけ。
//
// ログには、種類とエラーコードだけを書く（読み、表記、辞書の語、設定の値は書かない。NFR-19）。
import Darwin
import Foundation
import IkariBridge
import IkariProto
import IkariXPC
import Security

final class BridgeServer: @unchecked Sendable {
    /// `sys/un.h` の `SOL_LOCAL` と `LOCAL_PEERTOKEN`（マクロの取り込みに頼らない）
    private static let solLocal: Int32 = 0
    private static let localPeerToken: Int32 = 0x006
    private static let directoryPermissions: mode_t = 0o700
    private static let socketPermissions: mode_t = 0o600
    /// 設定アプリへの書き込みが進まない時間の上限。超えたら接続を閉じる（設定アプリはつなぎ直す。issue #44）。
    /// 暫定値（掃引していない）。相手が一時停止したときに、送る側のスレッドを待たせる上限になる
    static let sendTimeout: Duration = .seconds(2)
    /// 輸送の相対パス（`IkariBridge.BridgeProtocol.socketRelativePath`。shell-core の `SOCKET_RELATIVE_PATH` と同じ）
    static let socketRelativePath = BridgeProtocol.socketRelativePath

    private let socketURL: URL
    private let peerValidator: @Sendable (Int32) -> Bool

    private let lock = NSLock()
    private var listener: Int32 = -1
    private var connection: (id: UInt64, transport: BridgeTransport)?
    private var nextConnectionID: UInt64 = 1

    /// 受けた枠（`Frame` の protobuf の本体）。接続ごとの受信のスレッドから呼ぶ。起動の前に 1 度だけ設定する
    var onFrame: (@Sendable (_ connection: UInt64, _ frame: Data) -> Void)?
    /// 接続が切れた（相手が閉じた、壊れた枠、`close`）。起動の前に 1 度だけ設定する
    var onClosed: (@Sendable (_ connection: UInt64) -> Void)?

    /// 送る枠と閉じる頼みは、このキューで順に行う。呼び出し元（main、入力の経路）は、相手が読まなくても止まらない
    private let sendQueue = DispatchQueue(label: "\(IkariIdentifiers.shell).bridge-send", qos: .utility)

    /// `peerValidator` は接続したソケットの記述子を受け、相手が設定アプリなら真。既定は署名の確認（テストだけが差し替える）
    /// 既定の確認は、署名要件（ad-hoc では同梱の設定アプリの cdhash とパスも。#269）。`shellBundle` は自分（ikari.app）。
    /// 期待を作れない（cdhash を読めない）ときは、すべての接続を断る
    init(
        directory: URL, signing: SigningMode, shellBundle: URL = Bundle.main.bundleURL,
        peerValidator: (@Sendable (Int32) -> Bool)? = nil
    ) {
        socketURL = directory.appendingPathComponent(Self.socketRelativePath)
        if let peerValidator {
            self.peerValidator = peerValidator
        } else if let expectation = PeerExpectation.settingsApp(mode: signing, shellBundle: shellBundle) {
            self.peerValidator = {
                Self.peerSatisfies(
                    requirement: expectation.requirement, expectedPath: expectation.expectedPath, descriptor: $0)
            }
        } else {
            AppContext.log.error("bridge: cannot pin the settings app (code hash unavailable); refusing peers")
            self.peerValidator = { _ in false }
        }
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
        let active = connection?.transport
        lock.unlock()
        if descriptor >= 0 {
            _ = Darwin.shutdown(descriptor, SHUT_RDWR)
            Darwin.close(descriptor)
        }
        active?.close()
        unlink(socketURL.path)
    }

    // MARK: shell-core の動作（BridgeReply、BridgeClose）

    /// 枠（`Frame` の protobuf の本体）を送る。すぐ戻る。接続が替わっていたら捨てる
    func send(_ body: Data, to connectionID: UInt64) {
        guard let transport = transport(connectionID) else { return }
        guard let frame = try? Frame(serializedBytes: body) else {
            AppContext.log.error("bridge: shell returned a broken frame")
            return
        }
        sendQueue.async { Self.send(frame, on: transport) }
    }

    /// 接続を閉じる（送る順番の最後に）
    func close(_ connectionID: UInt64) {
        guard let transport = transport(connectionID) else { return }
        sendQueue.async { transport.close() }
    }

    private func transport(_ connectionID: UInt64) -> BridgeTransport? {
        lock.lock()
        defer { lock.unlock() }
        guard let connection, connection.id == connectionID else { return nil }
        return connection.transport
    }

    // MARK: ソケット

    /// `path` がシンボリックリンクでなく、ディレクトリで、自分の uid のものか(lstat で確かめる)
    static func isOwnedRealDirectory(_ path: String) -> Bool {
        var info = stat()
        guard lstat(path, &info) == 0 else { return false }
        return (info.st_mode & S_IFMT) == S_IFDIR && info.st_uid == getuid()
    }

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
        // 作ったあとに lstat で確かめる(シンボリックリンクを辿って chmod しない)
        guard Self.isOwnedRealDirectory(directory.path) else {
            AppContext.log.error("bridge: the run directory is not a plain directory owned by this user")
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
        let id = nextConnectionID
        nextConnectionID &+= 1
        connection = (id, transport)
        lock.unlock()

        let thread = Thread { [self] in serve(transport, id: id) }
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
    static func peerSatisfies(requirement text: String, expectedPath: URL? = nil, descriptor: Int32) -> Bool {
        PeerSignature.satisfies(requirement: text, expectedPath: expectedPath, descriptor: descriptor)
    }

    // MARK: 接続ごとの受信

    private func serve(_ transport: BridgeTransport, id: UInt64) {
        defer {
            transport.close()
            lock.lock()
            if connection?.id == id { connection = nil }
            lock.unlock()
            onClosed?(id)
            AppContext.log.info("bridge: disconnected")
        }
        AppContext.log.info("bridge: connected")
        while true {
            do {
                let frame = try transport.receive()
                // 受けた枠は shell-core に渡す（直列化は protobuf の本体だけ。長さの接頭辞は輸送のもの）
                guard let body = try? frame.serializedData() else { continue }
                onFrame?(id, body)
            } catch BridgeTransportError.frame {
                AppContext.log.error("bridge: broken frame")
                return
            } catch {
                return
            }
        }
    }

    private static func send(_ frame: Frame, on transport: BridgeTransport) {
        do {
            try transport.send(frame)
        } catch BridgeTransportError.frame(let error) {
            // 返事が大きすぎる（エクスポートが上限を超えた）など。壊れた枠は送らず、理由だけ返す
            AppContext.log.error("bridge: cannot send a frame")
            if case .tooLarge = error { _ = try? transport.send(.error(requestID: frame.requestID, code: .tooLarge)) }
        } catch BridgeTransportError.io(let code) where code == ETIMEDOUT {
            // 相手が読まない（`sendTimeout`）。輸送が閉じたので、受信が終わり、接続を片づける
            AppContext.log.error("bridge: send timed out; closing the connection")
            transport.close()
        } catch {
            transport.close()
        }
    }
}
