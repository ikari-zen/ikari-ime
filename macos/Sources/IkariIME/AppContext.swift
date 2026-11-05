// プロセス全体で共有するもの（shell-core の口、エンジンへの XPC、設定アプリとのブリッジ、画面）。
// 設定、エンジンの状態、メニューの中身、ブリッジの判断は shell-core が持つ（ADR 0041）。ここは組み立てるだけ。
import AppKit
import Carbon
import Foundation
import IkariXPC
import os

final class AppContext: @unchecked Sendable {
    static let shared = AppContext()

    /// ログには文字列（読み、候補、文脈、辞書の語、分野の名前）を決して書かない（NFR-19）。種類と件数だけ
    static let log = Logger(subsystem: IkariIdentifiers.shell, category: "shell")

    static let userDirectoryName = "ikari-ime"
    static let settingsFileName = "settings.tsv"
    /// 診断ログのディレクトリ名（エンジンの `LOG_DIR_NAME` と同じ。ADR 0034）
    static let logDirectoryName = "logs"

    let driver: ShellDriver
    /// 設定アプリとのブリッジ（ADR 0005 §6.3）。`startBridge()` で待ち始める
    let bridge: BridgeServer
    let userDirectory: URL
    let signing: SigningMode
    /// 候補ウィンドウと設定アプリ（主スレッド）。`driver.ui` は弱い参照なので、ここで持つ
    private let ui: PanelUI

    private init() {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent("Library/Application Support")
        let directory = support.appendingPathComponent(Self.userDirectoryName, isDirectory: true)
        userDirectory = directory
        signing = BuildSigning.current()

        // 落ちた通知は XPC のキューから来る。shell を作り直しても届くよう、箱を通す
        let downBox = DownNotifierBox()
        let connections = XPCEngineConnections(signing: signing) { connection, kind in
            downBox.engineDown(connection: connection, kind: kind == .interrupted ? .interrupted : .invalidated)
        }
        let shellVersion = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? ""
        driver = ShellDriver(
            config: .init(
                shellVersion: shellVersion,
                settingsPath: directory.appendingPathComponent(Self.settingsFileName).path,
                resourcesDir: Bundle.main.resourcePath ?? "",
                diagDir: directory.appendingPathComponent(Self.logDirectoryName, isDirectory: true).path),
            engine: XPCEngineHost(connections: connections),
            downNotifier: downBox,
            isSecure: { IsSecureEventInputEnabled() })
        ui = MainActor.assumeIsolated { PanelUI() }
        driver.ui = ui

        bridge = BridgeServer(directory: directory, signing: signing)
        let driver = driver
        bridge.onFrame = { connection, frame in driver.bridgeFrame(frame, connection: connection) }
        bridge.onClosed = { connection in driver.bridgeClosed(connection: connection) }
        driver.bridge = bridge
    }

    /// 設定アプリとのブリッジを始める（起動時に 1 回）
    func startBridge() {
        bridge.start()
    }
}
