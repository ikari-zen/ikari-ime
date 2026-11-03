import Foundation
import IkariBridge
import IkariProto
import IkariXPC
import Testing

@testable import IkariSettings

/// 本物の shell-core（XCFramework）にリンクしているか。スタブ（FFI なし）のときは設定の検証が本物と違う（#165）
private let linksShellFFI: Bool = {
    #if IKARI_HAS_SHELL_FFI
        true
    #else
        false
    #endif
}()

@Suite struct BridgeClientTests {
    @Test func handshakeReturnsTheShellVersion() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        #expect(harness.client.shellVersion == "0.0.0-fake")
    }

    @Test func aMismatchedBridgeVersionIsReportedAndClosesTheClient() async throws {
        await #expect(throws: BridgeClientError.versionMismatch) {
            _ = try await Harness.make(acceptedBridgeVersion: BridgeProtocol.version + 1)
        }
    }

    @Test(.enabled(if: linksShellFFI, "スタブ（XCFramework なし）では設定の挙動が本物と違うので飛ばす（#165）"))
    func getsAndUpdatesSettings() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let initial = try await harness.client.getSettings()
        #expect(initial["input.method"] == "romaji")
        #expect(initial["paging.size"] == "3")

        let updated = try await harness.client.updateSettings(
            set: [entry("paging.size", "5"), entry("keymap.convert.hiragana", "ctrl+u")], reset: [])
        #expect(updated["paging.size"] == "5")
        #expect(updated["keymap.convert.hiragana"] == "ctrl+u")

        let reset = try await harness.client.updateSettings(set: [], reset: ["paging.size", "keymap.convert.hiragana"])
        #expect(reset["paging.size"] == "3")
        #expect(reset["keymap.convert.hiragana"] == nil)
    }

    @Test func anInvalidSettingIsRejectedWithItsKey() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        await #expect(throws: BridgeClientError.bridge(.invalidSetting, key: "paging.size")) {
            _ = try await harness.client.updateSettings(set: [entry("paging.size", "99")], reset: [])
        }
        // 何も変わっていない
        #expect(try await harness.client.getSettings()["paging.size"] == "3")
    }

    @Test func relaysAnAdminRequestAndReturnsTheReplyBody() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let body = try await harness.client.engine(.learningStatus(Ikari_Engine_V1_LearningStatus()))
        guard case .learningStatusReply(let reply) = body else {
            Issue.record("unexpected reply \(body)")
            return
        }
        #expect(reply.maxRows == 200_000)
    }

    @Test func anEngineErrorReplyBecomesAnError() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        harness.shell.state.withLock { $0.failNextEngineRequest = .badRequest }
        await #expect(throws: BridgeClientError.engine(.badRequest)) {
            _ = try await harness.client.engine(.listUserDict(Ikari_Engine_V1_UserDictList()))
        }
    }

    @Test func theShellRefusesWhatTheAllowListDoesNotCover() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        await #expect(throws: BridgeClientError.bridge(.notAllowed, key: "")) {
            _ = try await harness.client.engine(.commit(Ikari_Engine_V1_Commit()))
        }
    }

    @Test func anEngineThatIsDownBecomesEngineUnavailable() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        harness.shell.state.withLock { $0.engineUnavailable = true }
        await #expect(throws: BridgeClientError.bridge(.engineUnavailable, key: "")) {
            _ = try await harness.client.engine(.getProfile(Ikari_Engine_V1_ProfileGet()))
        }
    }

    @Test func aRequestOverTheAdminLimitIsRefusedBeforeSending() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        var add = Ikari_Engine_V1_UserDictAdd()
        add.entry = userEntry(reading: "あ", surface: String(repeating: "あ", count: AdminLimits.maxRequestBytes))
        await #expect(throws: BridgeClientError.tooLarge) {
            _ = try await harness.client.engine(.addUserDict(add))
        }
        #expect(harness.shell.state.snapshot.requestLog.isEmpty)
    }

    @Test func importMayBeLargerThanTheAdminLimit() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        var request = Ikari_Engine_V1_ImportData()
        request.dryRun = true
        request.content = Data(repeating: 0x41, count: AdminLimits.maxRequestBytes * 4)
        let body = try await harness.client.engine(.importData(request))
        guard case .importReply(let reply) = body else {
            Issue.record("unexpected reply")
            return
        }
        // 偽のエンジンは中身を読んで「書き出しではない」と返す（送れたことが大事）
        #expect(reply.issue.kind == .notIkariExport)
    }

    @Test func eventsFromTheShellAreDelivered() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        var iterator = harness.client.events.makeAsyncIterator()
        harness.shell.notifyOpenTab(.userDictionary)
        harness.shell.notifySettingsChanged()
        #expect(await iterator.next() == .openTab("dictionary"))
        #expect(await iterator.next() == .settingsChanged)
    }

    @Test func aStoppedShellEndsTheClientAndFailsNewRequests() async throws {
        let harness = try await Harness.make()
        var iterator = harness.client.events.makeAsyncIterator()
        harness.shell.stop()
        #expect(await iterator.next() == .disconnected)
        await #expect(throws: BridgeClientError.disconnected) {
            _ = try await harness.client.getSettings()
        }
    }

    @Test func concurrentRequestsGetTheirOwnReplies() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        try await withThrowingTaskGroup(of: Bool.self) { group in
            for index in 0..<20 {
                group.addTask {
                    if index.isMultiple(of: 2) {
                        return try await harness.client.getSettings()["input.method"] == "romaji"
                    }
                    if case .profileReply = try await harness.client.engine(.getProfile(Ikari_Engine_V1_ProfileGet())) {
                        return true
                    }
                    return false
                }
            }
            for try await ok in group { #expect(ok) }
        }
    }
}

@Suite struct ReconnectPolicyTests {
    @Test func retriesEveryFiveSecondsForAMinute() {
        #expect(ReconnectPolicy.maxAutomaticRetries == 12)
        #expect(ReconnectPolicy.shouldRetry(afterFailedAttempt: 0))
        #expect(ReconnectPolicy.shouldRetry(afterFailedAttempt: 11))
        #expect(!ReconnectPolicy.shouldRetry(afterFailedAttempt: 12))
    }

    @Test func socketPathIsUnderApplicationSupport() {
        let path = SocketConnector.defaultSocketPath(home: URL(fileURLWithPath: "/Users/someone"))
        #expect(path == "/Users/someone/Library/Application Support/ikari-ime/run/bridge.sock")
    }

    @Test func aMissingSocketMeansTheShellIsNotListening() {
        let connector = SocketConnector(socketPath: "/nonexistent-ikari-test/bridge.sock")
        #expect(throws: SocketConnectError.notListening) { _ = try connector.makeTransport() }
    }

    @Test func aTooLongPathIsRejected() {
        let connector = SocketConnector(socketPath: "/" + String(repeating: "a", count: 200))
        #expect(throws: SocketConnectError.pathTooLong) { _ = try connector.makeTransport() }
    }

    @Test func anUntrustedPeerIsCutWithoutSendingAnything() throws {
        let path = "/tmp/ikari-t\(getpid())-\(UInt32.random(in: 0...99999)).sock"
        let listener = try makeListener(path: path)
        defer { Darwin.close(listener); unlink(path) }
        let connector = SocketConnector(socketPath: path, peerValidator: { _ in false })
        #expect(throws: SocketConnectError.untrustedPeer) { _ = try connector.makeTransport() }
        // 殻側から見ると、何も受け取らずに閉じられている（EOF）
        let accepted = accept(listener, nil, nil)
        #expect(accepted >= 0)
        defer { Darwin.close(accepted) }
        var byte: UInt8 = 0
        #expect(read(accepted, &byte, 1) == 0)
    }

    @Test func aTrustedPeerGetsATransport() throws {
        let path = "/tmp/ikari-t\(getpid())-\(UInt32.random(in: 0...99999)).sock"
        let listener = try makeListener(path: path)
        defer { Darwin.close(listener); unlink(path) }
        let connector = SocketConnector(socketPath: path, peerValidator: { _ in true })
        let transport = try connector.makeTransport()
        transport.close()
    }

    @Test func theRealCheckRejectsAPeerThatIsNotTheShell() throws {
        // テストの実行ファイルは殻ではないので、既定の確認は必ず断る
        let path = "/tmp/ikari-t\(getpid())-\(UInt32.random(in: 0...99999)).sock"
        let listener = try makeListener(path: path)
        defer { Darwin.close(listener); unlink(path) }
        let connector = SocketConnector(socketPath: path, signing: .team)
        #expect(throws: SocketConnectError.untrustedPeer) { _ = try connector.makeTransport() }
    }

    private func makeListener(path: String) throws -> Int32 {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            raw.copyBytes(from: bytes)
            raw[bytes.count] = 0
        }
        let length = socklen_t(MemoryLayout<sockaddr_un>.size)
        let rc = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.bind(fd, $0, length) }
        }
        guard rc == 0, listen(fd, 1) == 0 else { Darwin.close(fd); throw SocketConnectError.io(errno) }
        return fd
    }
}
