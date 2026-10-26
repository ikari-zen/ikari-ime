import Foundation
import IkariBridge
import IkariProto
import Testing

@testable import IkariSettings

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

    @Test func getsAndUpdatesSettings() async throws {
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
}
