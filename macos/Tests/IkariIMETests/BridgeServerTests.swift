// IkariIME の BridgeServer（輸送）と、shell-core のブリッジの判断（ADR 0005 §6.3、§7、ADR 0041 L-E）を、本物どうしでつなぐ試験。
// 実際の Unix ソケットで、設定アプリ役のクライアントをつなぐ。枠は `ShellDriver` から shell-core（FFI）に渡る。
// 署名の確認は `peerValidator` で差し替える以外は本物（`realPeerCheckSeesThisProcess` は本物の `LOCAL_PEERTOKEN` と SecCode を使う）。
// shell-ffi の XCFramework がないビルドでは飛ばす。
import Darwin
import Foundation
import Security
import IkariBridge
import IkariProto
import IkariShellABI
import IkariXPC
import Testing

@testable import IkariIME

#if IKARI_HAS_SHELL_FFI

/// 替え玉のエンジン（shell-core のホストの口 `engine_*`）。管理の要求には分野 1 つのプロファイルで答える
private final class StubEngine: EngineHost, @unchecked Sendable {
    let lock = NSLock()
    private var _relayed: [Ikari_Engine_V1_Envelope] = []
    private var _hold: DispatchSemaphore?
    /// 中継された要求（`Hello` などの殻の要求は数えない）
    var relayed: [Ikari_Engine_V1_Envelope] { lock.withLock { _relayed } }
    /// 非 nil なら、管理の要求はこの合図が出るまで止まる（同時数の試験用）
    var hold: DispatchSemaphore? {
        get { lock.withLock { _hold } }
        set { lock.withLock { _hold = newValue } }
    }

    func connect() -> UInt64 { 1 }
    func request(_ connection: UInt64, _ bytes: Data, timeoutMs: UInt32) -> ShellTransportResult {
        guard let request = try? Ikari_Engine_V1_Envelope(serializedBytes: bytes) else { return .unavailable }
        switch request.body {
        case .getProfile?, .commit?:
            lock.withLock { _relayed.append(request) }
            hold?.wait()
            var reply = Ikari_Engine_V1_Envelope()
            reply.schemaVersion = request.schemaVersion
            reply.requestID = request.requestID
            var profile = Ikari_Engine_V1_ProfileReply()
            var d = Ikari_Engine_V1_ProfileDomainState()
            d.id = "it"
            d.name = "IT"
            d.weight = .profileWeightOften
            profile.domains = [d]
            reply.profileReply = profile
            return (try? reply.serializedData()).map { .ok($0) } ?? .unavailable
        default:
            return .unavailable
        }
    }
    func post(_ connection: UInt64, _ bytes: Data) {}
    func close(_ connection: UInt64) {}
    func pid(_ connection: UInt64) -> Int32 { 0 }
}

private func connectClient(_ path: String) throws -> FileDescriptorTransport {
    let fd = socket(AF_UNIX, SOCK_STREAM, 0)
    var address = sockaddr_un()
    address.sun_family = sa_family_t(AF_UNIX)
    let bytes = Array(path.utf8CString)
    withUnsafeMutableBytes(of: &address.sun_path) { raw in
        for (i, b) in bytes.enumerated() { raw[i] = UInt8(bitPattern: b) }
    }
    let rc = withUnsafePointer(to: &address) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
    }
    guard rc == 0 else { throw BridgeTransportError.io(errno) }
    return FileDescriptorTransport(descriptor: fd)
}

/// 中継の同時数の上限（shell-core の `bridge::relay_gate::MAX_CONCURRENT_RELAYS`）
private let maxConcurrentRelays = 4

/// 返事を待つ上限。閉じられたときは待たずに戻るので、長くしても遅くならない（負荷が高くても落ちないように）
private let replyTimeoutSeconds: Double = 15

private func recv(_ t: FileDescriptorTransport, seconds: Double = replyTimeoutSeconds) -> Frame? {
    let sem = DispatchSemaphore(value: 0)
    nonisolated(unsafe) var result: Frame?
    Thread { result = try? t.receive(); sem.signal() }.start()
    if sem.wait(timeout: .now() + seconds) == .timedOut { t.close(); return nil }
    return result
}

private func helloFrame(version: UInt32 = 1) -> Frame {
    var hello = Ikari_Bridge_V1_BridgeHello()
    hello.bridgeVersion = version
    hello.appVersion = "t"
    var f = Frame()
    f.requestID = 1
    f.hello = hello
    return f
}

/// 殻（shell-core）とブリッジを本物でつなぐ。shell-core は主スレッドの代わりに直列のキューで回す
private struct Rig {
    let dir: URL
    let engine = StubEngine()
    let queue = DispatchQueue(label: "ikari.test.shell")
    let driver: ShellDriver
    let server: BridgeServer
    /// 切れた接続の番号
    let closed = LockedBox<[UInt64]>([])
    var settingsFile: URL { dir.appendingPathComponent("settings.tsv") }

    init(validator: (@Sendable (Int32) -> Bool)? = { _ in true }, initial: String = "# note\nengine.lm\ton\n") throws {
        dir = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("ikari-bridge-\(UUID().uuidString.prefix(6))")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try initial.write(to: dir.appendingPathComponent("settings.tsv"), atomically: true, encoding: .utf8)
        driver = ShellDriver(
            config: .init(shellVersion: "t", settingsPath: dir.appendingPathComponent("settings.tsv").path, resourcesDir: ""),
            engine: engine, queue: queue)
        server = BridgeServer(directory: dir, signing: .adhoc, peerValidator: validator)
        let driver = driver
        let closed = closed
        server.onFrame = { connection, frame in driver.bridgeFrame(frame, connection: connection) }
        server.onClosed = { connection in
            closed.set(closed.get() + [connection])
            driver.bridgeClosed(connection: connection)
        }
        driver.bridge = server
        #expect(server.start())
    }

    /// shell-core のキューの上でイベントを渡す
    func dispatch(_ event: Ikari_Shell_V1_ShellEvent) {
        queue.sync { _ = driver.dispatch(event, session: nil) }
    }

    func openTab(_ tab: Ikari_Shell_V1_SettingsTab) {
        var open = Ikari_Shell_V1_OpenSettings()
        open.tab = tab
        var command = Ikari_Shell_V1_MenuCommand()
        command.openSettings = open
        var event = Ikari_Shell_V1_ShellEvent()
        event.menuCommand = command
        dispatch(event)
    }
}

@Suite(.serialized) struct BridgeServerTests {
    @Test func permissionsAndHandshake() throws {
        let rig = try Rig()
        defer { rig.server.stop() }
        let runDir = rig.dir.appendingPathComponent("run").path
        let dirMode = try FileManager.default.attributesOfItem(atPath: runDir)[.posixPermissions] as? Int
        let sockMode = try FileManager.default.attributesOfItem(atPath: rig.server.socketPath)[.posixPermissions] as? Int
        #expect(dirMode == 0o700)
        #expect(sockMode == 0o600)

        let c = try connectClient(rig.server.socketPath)
        try c.send(helloFrame())
        let reply = try #require(recv(c))
        guard case .helloReply(let r)? = reply.body else { Issue.record("no hello reply"); return }
        #expect(r.bridgeVersion == 1 && r.shellVersion == "t")
        c.close()
    }

    @Test func versionMismatchIsRefusedAndClosed() throws {
        let rig = try Rig()
        defer { rig.server.stop() }
        let c = try connectClient(rig.server.socketPath)
        try c.send(helloFrame(version: 99))
        let reply = try #require(recv(c))
        guard case .error(let e)? = reply.body else { Issue.record("no error"); return }
        #expect(e.code == .versionMismatch)
        #expect(recv(c) == nil)  // 閉じられた
    }

    @Test func settingsGetAndUpdateKeepComments() throws {
        let rig = try Rig()
        defer { rig.server.stop() }
        let c = try connectClient(rig.server.socketPath)
        try c.send(helloFrame()); _ = recv(c)

        var get = Frame(); get.requestID = 7; get.getSettings = Ikari_Bridge_V1_GetSettings()
        try c.send(get)
        let snap = try #require(recv(c))
        #expect(snap.requestID == 7)
        guard case .settings(let s)? = snap.body else { Issue.record("no snapshot"); return }
        #expect(s.effective.contains { $0.key == "paging.size" && $0.value == "3" })

        var update = Ikari_Bridge_V1_UpdateSettings()
        var e = Ikari_Bridge_V1_SettingEntry(); e.key = "paging.size"; e.value = "4"
        update.set = [e]
        var f = Frame(); f.requestID = 8; f.updateSettings = update
        try c.send(f)
        // 返事と settings_changed の通知が来る（順は問わない）
        var gotSnapshot = false, gotEvent = false
        for _ in 0..<2 {
            let frame = try #require(recv(c))
            switch frame.body {
            case .settings(let s2)?:
                gotSnapshot = frame.requestID == 8 && s2.effective.contains { $0.key == "paging.size" && $0.value == "4" }
            case .event(let ev)?:
                if case .settingsChanged? = ev.kind { gotEvent = true }
            default: break
            }
        }
        #expect(gotSnapshot && gotEvent)
        let text = try String(contentsOf: rig.settingsFile, encoding: .utf8)
        #expect(text == "# note\nengine.lm\ton\npaging.size\t4\n")
        c.close()
    }

    @Test func invalidSettingIsRejectedWithTheKeyOnlyAndNothingIsWritten() throws {
        let rig = try Rig()
        defer { rig.server.stop() }
        let c = try connectClient(rig.server.socketPath)
        try c.send(helloFrame()); _ = recv(c)
        var update = Ikari_Bridge_V1_UpdateSettings()
        var e = Ikari_Bridge_V1_SettingEntry(); e.key = "paging.size"; e.value = "99"
        update.set = [e]
        var f = Frame(); f.requestID = 9; f.updateSettings = update
        try c.send(f)
        let reply = try #require(recv(c))
        guard case .error(let err)? = reply.body else { Issue.record("no error"); return }
        #expect(err.code == .invalidSetting && err.key == "paging.size")
        #expect(try String(contentsOf: rig.settingsFile, encoding: .utf8) == "# note\nengine.lm\ton\n")
        c.close()
    }

    @Test func engineRelayAllowsAdminAndRefusesCommit() throws {
        let rig = try Rig()
        defer { rig.server.stop() }
        let c = try connectClient(rig.server.socketPath)
        try c.send(helloFrame()); _ = recv(c)

        var env = Ikari_Engine_V1_Envelope(); env.schemaVersion = 1; env.getProfile = Ikari_Engine_V1_ProfileGet()
        var f = Frame(); f.requestID = 21; f.engine = env
        try c.send(f)
        let reply = try #require(recv(c))
        #expect(reply.requestID == 21)
        guard case .engine(let out)? = reply.body, case .profileReply(let p)? = out.body else { Issue.record("no relay reply"); return }
        #expect(p.domains.first?.id == "it")

        var bad = Ikari_Engine_V1_Envelope(); bad.schemaVersion = 1
        var commit = Ikari_Engine_V1_Commit(); commit.conversionID = 1; bad.commit = commit
        var g = Frame(); g.requestID = 22; g.engine = bad
        try c.send(g)
        let refused = try #require(recv(c))
        guard case .error(let e)? = refused.body else { Issue.record("no error"); return }
        #expect(e.code == .notAllowed && refused.requestID == 22)
        #expect(rig.engine.relayed.count == 1)  // Commit は流れていない
        c.close()
    }

    @Test func relayFloodIsRefusedBeyondTheLimitWithoutQueueing() throws {
        let rig = try Rig()
        defer { rig.server.stop() }
        let hold = DispatchSemaphore(value: 0)
        rig.engine.hold = hold
        let c = try connectClient(rig.server.socketPath)
        try c.send(helloFrame()); _ = recv(c)

        let limit = maxConcurrentRelays
        let flood = limit + 20
        for i in 0..<flood {
            var env = Ikari_Engine_V1_Envelope(); env.schemaVersion = 1; env.getProfile = Ikari_Engine_V1_ProfileGet()
            var f = Frame(); f.requestID = UInt64(100 + i); f.engine = env
            try c.send(f)
        }
        // 上限を超えた分は、中継が止まっている間にも、すぐ busy 相当で返る
        var refused = 0
        for _ in 0..<(flood - limit) {
            let r = try #require(recv(c, seconds: 5))
            guard case .error(let e)? = r.body else { Issue.record("expected an error"); break }
            #expect(e.code == .engineUnavailable)
            refused += 1
        }
        #expect(refused == flood - limit)
        #expect(rig.engine.relayed.count <= limit)

        // 止めを外すと、受け付けた分だけが返る。その後は、また受け付ける
        for _ in 0..<limit { hold.signal() }
        for _ in 0..<limit {
            let r = try #require(recv(c))
            guard case .engine? = r.body else { Issue.record("expected a relay reply"); break }
        }
        rig.engine.hold = nil
        var env = Ikari_Engine_V1_Envelope(); env.schemaVersion = 1; env.getProfile = Ikari_Engine_V1_ProfileGet()
        var f = Frame(); f.requestID = 999; f.engine = env
        try c.send(f)
        let again = try #require(recv(c))
        guard case .engine? = again.body else { Issue.record("not accepted after release"); return }
        c.close()
    }

    @Test func secondConnectionIsRejectedWhileOneIsActive() throws {
        let rig = try Rig()
        defer { rig.server.stop() }
        let first = try connectClient(rig.server.socketPath)
        try first.send(helloFrame()); _ = recv(first)
        let second = try connectClient(rig.server.socketPath)
        try? second.send(helloFrame())
        #expect(recv(second) == nil)
        // 最初の接続は生きている
        var get = Frame(); get.requestID = 3; get.getSettings = Ikari_Bridge_V1_GetSettings()
        try first.send(get)
        #expect(recv(first) != nil)
        first.close()
    }

    @Test func aRejectedPeerIsClosedBeforeAnyFrame() throws {
        let rig = try Rig(validator: { _ in false })
        defer { rig.server.stop() }
        let c = try connectClient(rig.server.socketPath)
        try? c.send(helloFrame())
        #expect(recv(c) == nil)
    }

    @Test func reconnectsAfterTheClientLeaves() throws {
        let rig = try Rig()
        defer { rig.server.stop() }
        let a = try connectClient(rig.server.socketPath)
        try a.send(helloFrame()); _ = recv(a)
        a.close()
        // サーバーが前の接続を片づけるまでは断られる。固定の時間は待たず、つながるまで繰り返す
        var reply: Frame?
        let deadline = Date().addingTimeInterval(replyTimeoutSeconds)
        while reply == nil && Date() < deadline {
            let b = try connectClient(rig.server.socketPath)
            try? b.send(helloFrame())
            reply = recv(b, seconds: 2)
            b.close()
            if reply == nil { Thread.sleep(forTimeInterval: 0.05) }
        }
        #expect(reply != nil)
    }

    @Test func openTabEventReachesTheConnectedApp() throws {
        let rig = try Rig()
        defer { rig.server.stop() }
        rig.openTab(.userDictionary)  // つながっていない（送る先がない）
        let c = try connectClient(rig.server.socketPath)
        try c.send(helloFrame()); _ = recv(c)
        // 握手は shell-core が返事を作った時点で済んでいるので、続けて送れる
        rig.openTab(.userDictionary)
        let frame = try #require(recv(c))
        guard case .event(let ev)? = frame.body, case .openTab(let t)? = ev.kind else { Issue.record("no event"); return }
        #expect(t.tab == "dictionary")
        c.close()
    }

    /// 設定アプリが読まなくなっても、通知を送る側（main、入力の経路）は止まらず、殻は接続を閉じる（issue #44）
    @Test func aPeerThatStopsReadingDoesNotBlockNotificationsAndIsDisconnected() throws {
        let rig = try Rig()
        defer { rig.server.stop() }
        let c = try connectClient(rig.server.socketPath)
        defer { c.close() }
        try c.send(helloFrame()); _ = recv(c)
        // 以後、c は読まない
        // 送信バッファが埋まって時間切れになり、殻が切るまで送り続ける。止まったら失敗にする（固まらないように別のスレッドで）
        let disconnected = LockedBox<Bool>(false)
        let slowest = LockedBox<Duration>(.zero)
        let done = DispatchSemaphore(value: 0)
        let closed = rig.closed
        nonisolated(unsafe) let sender = rig
        Thread {
            let clock = ContinuousClock()
            let deadline = Date().addingTimeInterval(replyTimeoutSeconds)
            while closed.get().isEmpty && Date() < deadline {
                for _ in 0..<200 {
                    let elapsed = clock.measure { sender.openTab(.userDictionary) }
                    if elapsed > slowest.get() { slowest.set(elapsed) }
                }
            }
            disconnected.set(!closed.get().isEmpty)
            done.signal()
        }.start()
        #expect(done.wait(timeout: .now() + replyTimeoutSeconds * 2) == .success)  // 送る側が止まらない
        #expect(disconnected.get())  // 殻が接続を閉じた
        #expect(slowest.get() < .milliseconds(500))  // 呼び出し元は書き込みを待たない
    }

    @Test func handEditIsNoticedByReloadAndNotifiesTheApp() throws {
        let rig = try Rig()
        defer { rig.server.stop() }
        let c = try connectClient(rig.server.socketPath)
        try c.send(helloFrame()); _ = recv(c)
        try "paging.size\t4\n".write(to: rig.settingsFile, atomically: true, encoding: .utf8)
        var reload = Ikari_Shell_V1_ShellEvent()
        reload.settingsFileChanged = Ikari_Shell_V1_Empty()
        rig.dispatch(reload)
        let frame = try #require(recv(c))
        guard case .event(let ev)? = frame.body, case .settingsChanged? = ev.kind else { Issue.record("no event"); return }
        // 変わっていなければ知らせない（次に届くのは、頼んだ設定の一式）
        rig.dispatch(reload)
        var get = Frame(); get.requestID = 5; get.getSettings = Ikari_Bridge_V1_GetSettings()
        try c.send(get)
        let snapshot = try #require(recv(c))
        guard case .settings(let s)? = snapshot.body else { Issue.record("expected the snapshot first"); return }
        #expect(s.effective.contains { $0.key == "paging.size" && $0.value == "4" })
        c.close()
    }

    @Test func realPeerCheckSeesThisProcess() throws {
        // 本物の接続（listen / connect / accept）の accept 側の記述子で確かめる
        let path = NSTemporaryDirectory() + "pc-\(UUID().uuidString.prefix(6)).sock"
        let listener = socket(AF_UNIX, SOCK_STREAM, 0)
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8CString)
        withUnsafeMutableBytes(of: &address.sun_path) { raw in for (i, b) in bytes.enumerated() { raw[i] = UInt8(bitPattern: b) } }
        _ = withUnsafePointer(to: &address) { $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(listener, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) } }
        listen(listener, 1)
        defer { close(listener); unlink(path) }
        let client = try connectClient(path)
        defer { client.close() }
        let accepted = accept(listener, nil, nil)
        #expect(accepted >= 0)
        defer { close(accepted) }

        var token = audit_token_t()
        var length = socklen_t(MemoryLayout<audit_token_t>.size)
        let rc = getsockopt(accepted, 0, 0x006, &token, &length)
        #expect(rc == 0)
        var code: SecCode?
        let attrs = [kSecGuestAttributeAudit: withUnsafeBytes(of: &token) { Data($0) }] as CFDictionary
        #expect(SecCodeCopyGuestWithAttributes(nil, attrs, SecCSFlags(), &code) == errSecSuccess)
        var staticCode: SecStaticCode?
        if let code { SecCodeCopyStaticCode(code, SecCSFlags(), &staticCode) }
        var info: CFDictionary?
        if let staticCode { SecCodeCopySigningInformation(staticCode, SecCSFlags(rawValue: kSecCSSigningInformation), &info) }

        let id = (info as? [String: Any])?["identifier"] as? String ?? "none"
        #expect(BridgeServer.peerSatisfies(requirement: "identifier \"\(id)\"", descriptor: accepted))
        #expect(!BridgeServer.peerSatisfies(requirement: "identifier \"io.github.ikari-zen.inputmethod.ikari.settings\"", descriptor: accepted))
        #expect(!BridgeServer.peerSatisfies(requirement: SigningRequirements.settingsApp(mode: .team) ?? "", descriptor: accepted))
        #expect(
            !BridgeServer.peerSatisfies(
                requirement: SigningRequirements.settingsApp(mode: .adhoc, cdhash: Data([1, 2, 3])) ?? "",
                descriptor: accepted))
    }
}

#endif

final class LockedBox<T: Sendable>: @unchecked Sendable {
    private let lock = NSLock()
    private var value: T
    init(_ v: T) { value = v }
    func set(_ v: T) { lock.withLock { value = v } }
    func get() -> T { lock.withLock { value } }
}


@Suite struct RunDirectoryCheckTests {
    private func makeTemp() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("ikari-rundir-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    @Test func acceptsOwnDirectory() throws {
        let root = try makeTemp()
        defer { try? FileManager.default.removeItem(at: root) }
        #expect(BridgeServer.isOwnedRealDirectory(root.path))
    }

    @Test func rejectsSymlinkToDirectory() throws {
        let root = try makeTemp()
        defer { try? FileManager.default.removeItem(at: root) }
        let link = root.appendingPathComponent("link")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: root)
        #expect(!BridgeServer.isOwnedRealDirectory(link.path))
    }

    @Test func rejectsRegularFileAndMissing() throws {
        let root = try makeTemp()
        defer { try? FileManager.default.removeItem(at: root) }
        let file = root.appendingPathComponent("file")
        FileManager.default.createFile(atPath: file.path, contents: Data())
        #expect(!BridgeServer.isOwnedRealDirectory(file.path))
        #expect(!BridgeServer.isOwnedRealDirectory(root.appendingPathComponent("none").path))
    }
}
