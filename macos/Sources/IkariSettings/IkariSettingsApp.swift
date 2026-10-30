// 設定アプリ「ikari 設定」（ADR 0005 §6.2）。Bundle ID io.github.ikari-zen.inputmethod.ikari.settings（IkariIdentifiers.settings）。
// Sandbox なし、Hardened Runtime。エンジンには直接つながない: 殻の Unix ソケット（IkariBridge）だけを使う。
// 開いている間は Dock に出て、最後のウィンドウを閉じたら終了する。
import AppKit
import IkariBridge
import SwiftUI

@main
struct IkariSettingsApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @State private var app: AppModel

    init() {
        let options = LaunchOptions.parse(Array(CommandLine.arguments.dropFirst()))
        let connector: any BridgeConnecting = options.fakeBridge ? FakeBridgeConnector() : SocketConnector()
        _app = State(
            initialValue: AppModel(connector: connector, dialogs: AppKitDialogs(), initialTab: options.tab ?? .input))
    }

    var body: some Scene {
        Window("ikari 設定", id: "main") {
            ContentView(app: app)
                .task { app.start() }
        }
        .windowResizability(.contentMinSize)
        .defaultSize(width: SidebarMetrics.windowDefaultWidth, height: 620)
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // 殻が LaunchServices で起動したとき、前面に出ないことがある（ADR 0005 §6.4 の代わり）
        NSApp.activate()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}

/// `--fake-bridge`: 本物の殻とエンジンなしで画面を確認するための、メモリ上の接続
struct FakeBridgeConnector: BridgeConnecting {
    func makeTransport() throws -> any BridgeTransport {
        let (appSide, shellSide) = try FileDescriptorTransport.socketPair()
        var state = FakeState()
        state.userDict = [
            {
                var entry = UserDictEntry()
                entry.reading = "いかり"
                entry.surface = "錨"
                entry.pos = "名詞"
                entry.comment = "見本"
                return entry
            }()
        ]
        FakeShell(transport: shellSide, state: state).start()
        return appSide
    }
}
