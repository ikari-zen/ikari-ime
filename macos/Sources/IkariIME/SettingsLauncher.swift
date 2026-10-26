// 設定アプリ（ikari.app/Contents/Helpers/IkariSettings.app）を LaunchServices で起動する（ADR 0005 §6.2）。
// IME のプロセスでウィンドウを開かない（アクティブ化で入力中の文字列が確定される。§6.1）。
// すでに動いていれば、LaunchServices が前面に出す（引数は届かないので、開くタブはブリッジの `OpenTab` でも伝える）。
import AppKit
import Foundation
import IkariShellCore

enum SettingsLauncher {
    /// `ikari.app` の中の設定アプリの場所
    static let appRelativePath = "Contents/Helpers/IkariSettings.app"
    /// 起動時に開くタブを渡す引数: `--tab <SettingsTab.rawValue>`（レーン U の設定アプリが読む）
    static let tabArgument = "--tab"

    static func appURL(bundle: Bundle = .main) -> URL {
        bundle.bundleURL.appendingPathComponent(appRelativePath, isDirectory: true)
    }

    /// 設定アプリを前面に開く。`tab == nil` なら前回のタブ（既定は先頭）
    @MainActor
    static func open(tab: SettingsTab?) {
        let url = appURL()
        guard FileManager.default.fileExists(atPath: url.path) else {
            AppContext.log.error("settings app is missing from the bundle")
            return
        }
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = true
        if let tab { configuration.arguments = [tabArgument, tab.rawValue] }
        NSWorkspace.shared.openApplication(at: url, configuration: configuration) { _, error in
            if let error {
                AppContext.log.error("opening the settings app failed: \((error as NSError).code)")
            }
        }
    }
}
