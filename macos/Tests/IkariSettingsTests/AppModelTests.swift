// 設定の処理は shell-core（ikari_settings_*）を通すので、XCFramework のあるビルドだけで流す（ADR 0041 §2.3）
#if IKARI_HAS_SHELL_FFI
import Foundation
import IkariBridge
import IkariProto
import Testing

@testable import IkariSettings

/// 偽の殻につなぐ接続元。最初の何回かを失敗させられる
final class MemoryConnector: BridgeConnecting, @unchecked Sendable {
    private struct Inner {
        var failures: [Error] = []
        var alwaysFail: Error?
        var attempts = 0
        var shells: [FakeShell] = []
    }

    private let inner = Locked(Inner())
    private let state: FakeState
    private let acceptedBridgeVersion: UInt32

    init(
        state: FakeState = FakeState(), failures: [Error] = [],
        acceptedBridgeVersion: UInt32 = BridgeProtocol.version
    ) {
        self.state = state
        self.acceptedBridgeVersion = acceptedBridgeVersion
        inner.withLock { $0.failures = failures }
    }

    var attempts: Int { inner.snapshot.attempts }
    var lastShell: FakeShell? { inner.snapshot.shells.last }
    func setAlwaysFail(_ error: Error?) { inner.withLock { $0.alwaysFail = error } }

    func makeTransport() throws -> any BridgeTransport {
        let failure: Error? = inner.withLock { inner in
            inner.attempts += 1
            if !inner.failures.isEmpty { return inner.failures.removeFirst() }
            return inner.alwaysFail
        }
        if let failure { throw failure }
        let (appSide, shellSide) = try FileDescriptorTransport.socketPair()
        let shell = FakeShell(transport: shellSide, state: state, acceptedBridgeVersion: acceptedBridgeVersion)
        shell.start()
        inner.withLock { $0.shells.append(shell) }
        return appSide
    }
}

/// 条件が満たされるまで待つ（上限つき）
@MainActor
func eventually(timeout: Duration = .seconds(5), _ condition: @MainActor () -> Bool) async -> Bool {
    let deadline = ContinuousClock.now + timeout
    while ContinuousClock.now < deadline {
        if condition() { return true }
        try? await Task.sleep(for: .milliseconds(5))
    }
    return condition()
}

@MainActor
private func makeApp(
    _ connector: MemoryConnector, tab: SettingsTab = .input
) -> AppModel {
    AppModel(
        connector: connector, dialogs: ScriptedDialogs(), initialTab: tab, presets: repositoryPresets(),
        dataDirectory: URL(fileURLWithPath: "/nonexistent"), appVersion: "test", sleep: { _ in await Task.yield() })
}

@Suite @MainActor struct AppModelTests {
    @Test func connectsAndLoadsTheSelectedTab() async throws {
        let connector = MemoryConnector()
        let app = makeApp(connector, tab: .userDictionary)
        app.start()
        defer { app.stop() }
        #expect(await eventually { app.phase == .connected })
        let session = try #require(app.session)
        #expect(await eventually { session.dictionary.isLoaded })
        #expect(!session.settings.isLoaded)  // 開いていないタブは、開いたときに読む
    }

    @Test func retriesWhileTheShellIsNotListeningThenConnects() async throws {
        let connector = MemoryConnector(failures: Array(repeating: SocketConnectError.notListening, count: 3))
        let app = makeApp(connector)
        app.start()
        defer { app.stop() }
        #expect(await eventually { app.phase == .connected })
        #expect(connector.attempts == 4)
    }

    @Test func givesUpAfterTheAutomaticRetriesAndOffersManualReconnect() async throws {
        let connector = MemoryConnector()
        connector.setAlwaysFail(SocketConnectError.notListening)
        let app = makeApp(connector)
        app.start()
        defer { app.stop() }
        #expect(await eventually { connector.attempts == 1 + ReconnectPolicy.maxAutomaticRetries && app.phase == .disconnected(.shellNotRunning) })
        #expect(app.session == nil)

        // 殻が戻ったので、手動で再接続する
        connector.setAlwaysFail(nil)
        #expect(await eventually { app.reconnectAvailable })
        app.reconnect()
        #expect(await eventually { app.phase == .connected })
    }

    @Test func aVersionMismatchIsNotRetried() async throws {
        let connector = MemoryConnector(acceptedBridgeVersion: BridgeProtocol.version + 1)
        let app = makeApp(connector)
        app.start()
        defer { app.stop() }
        #expect(await eventually { app.phase == .disconnected(.versionMismatch) })
        #expect(await eventually { app.reconnectAvailable })
        #expect(connector.attempts == 1)
    }

    @Test func aLostConnectionDisablesTheSessionAndAReconnectRestoresIt() async throws {
        let connector = MemoryConnector()
        let app = makeApp(connector)
        app.start()
        defer { app.stop() }
        #expect(await eventually { app.phase == .connected })
        let first = try #require(app.session)

        connector.setAlwaysFail(SocketConnectError.notListening)  // 殻が止まって、しばらく戻らない
        connector.lastShell?.stop()
        #expect(await eventually { app.session == nil && app.reconnectAvailable })
        #expect(app.phase == .disconnected(.shellNotRunning))

        connector.setAlwaysFail(nil)  // 入力ソースを選び直して、殻が戻った
        app.reconnect()
        #expect(await eventually { app.phase == .connected && app.session != nil })
        #expect(app.session !== first)
    }

    @Test func theShellCanSwitchTheTabAndAnUnknownTabIsIgnored() async throws {
        let connector = MemoryConnector()
        let app = makeApp(connector)
        app.start()
        defer { app.stop() }
        #expect(await eventually { app.phase == .connected })
        connector.lastShell?.notifyOpenTab(.learning)
        #expect(await eventually { app.selectedTab == .learning })
        app.openTab(named: "nonexistent")
        #expect(app.selectedTab == .learning)
        app.openTab(named: "about")
        #expect(app.selectedTab == .about)
    }

    @Test func aSettingsChangedEventReloadsTheSettings() async throws {
        let connector = MemoryConnector()
        let app = makeApp(connector, tab: .input)
        app.start()
        defer { app.stop() }
        let session = try await connectedSession(app)
        #expect(await eventually { session.settings.isLoaded })
        connector.lastShell?.state.withLock { $0.settings["paging.size"] = "4" }
        connector.lastShell?.notifySettingsChanged()
        #expect(await eventually { session.settings.values.pageSize == 4 })
    }

    private func connectedSession(_ app: AppModel) async throws -> Session {
        #expect(await eventually { app.session != nil })
        return try #require(app.session)
    }
}

@Suite @MainActor struct AboutModelTests {
    @Test func showsTheVersionsDictionaryAndLMState() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = AboutModel(bridge: harness.client, appVersion: "1.2.3", notices: "notices")
        await model.load()
        #expect(model.appVersion == "1.2.3")
        #expect(model.shellVersion == "0.0.0-fake")
        #expect(model.engineVersion == "0.0.0-fake")
        #expect(model.dictLabel?.contains("SudachiDict Core") == true)
        #expect(model.lmText == "使用中")
        #expect(model.notices == "notices")
    }

    @Test func reflectsTheLMBeingOff() async throws {
        var state = FakeState()
        state.settings["engine.lm"] = "off"
        let harness = try await Harness.make(state: state)
        defer { harness.close() }
        let model = AboutModel(bridge: harness.client, appVersion: "x", notices: "")
        await model.load()
        #expect(model.lmText == "オフ")
    }

    @Test func anEngineProblemIsShownAndThePreviousValuesStay() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = AboutModel(bridge: harness.client, appVersion: "x", notices: "")
        await model.load()
        harness.shell.state.withLock { $0.engineUnavailable = true }
        await model.load()
        #expect(model.message != nil)
        #expect(model.engineVersion == "0.0.0-fake")
    }

    @Test func describesEveryLMState() {
        for state in [Ikari_Engine_V1_LmState.lmDisabled, .lmLoading, .lmReady, .lmFailed, .unspecified] {
            #expect(!LMStatusText.describe(state).isEmpty)
        }
    }

    @Test func missingNoticesAreReportedInsteadOfEmpty() {
        #expect(AboutModel.bundledNotices(Bundle(for: BundleToken.self)) == AboutModel.noticesMissing)
    }
}

private final class BundleToken {}

@Suite struct LaunchOptionsTests {
    @Test func readsTheTabInBothForms() {
        #expect(LaunchOptions.parse(["--tab", "dictionary"]).tab == .userDictionary)
        #expect(LaunchOptions.parse(["--tab=learning"]).tab == .learning)
    }

    @Test func ignoresUnknownTabsAndArguments() {
        #expect(LaunchOptions.parse(["--tab", "display"]).tab == nil)
        #expect(LaunchOptions.parse(["--tab=display", "-NSDocumentRevisionsDebugMode", "YES"]) == LaunchOptions())
        #expect(LaunchOptions.parse(["--tab"]) == LaunchOptions())
    }

    @Test func fakeBridgeFlag() {
        #expect(LaunchOptions.parse(["--fake-bridge", "--tab", "about"]) == LaunchOptions(tab: .about, fakeBridge: true))
    }

    @Test func everyShellTabNameIsAcceptedByTheApp() {
        for tab in SettingsTab.allCases {
            #expect(LaunchOptions.parse(["--tab", tab.rawValue]).tab == tab)
        }
    }
}

#endif
