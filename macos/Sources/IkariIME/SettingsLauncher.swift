// 設定アプリ（ikari.app/Contents/Helpers/IkariSettings.app）を LaunchServices で起動する（ADR 0005 §6.2）。
// IME のプロセスでウィンドウを開かない（アクティブ化で入力中の文字列が確定される。§6.1）。
// すでに動いていれば、LaunchServices が前面に出す（引数は届かないので、開くタブはブリッジの `OpenTab` でも伝える）。
import AppKit
import Foundation
import IkariProto

enum SettingsLauncher {
    /// `ikari.app` の中の設定アプリの場所
    static let appRelativePath = "Contents/Helpers/IkariSettings.app"
    /// 起動時に開くタブを渡す引数: `--tab <名前>`（設定アプリの `SettingsTab.rawValue`）
    static let tabArgument = "--tab"

    /// 設定アプリの `SettingsTab.rawValue`（ブリッジの `OpenTab.tab` と同じ名前）。前回のタブなら nil
    static func argument(for tab: Ikari_Shell_V1_SettingsTab) -> String? {
        switch tab {
        case .input: "input"
        case .conversion: "conversion"
        case .userDictionary: "dictionary"
        case .learning: "learning"
        case .about: "about"
        case .unspecified, .UNRECOGNIZED: nil
        }
    }

    static func appURL(bundle: Bundle = .main) -> URL {
        bundle.bundleURL.appendingPathComponent(appRelativePath, isDirectory: true)
    }

    /// 設定アプリを前面に開く。`.unspecified` なら前回のタブ（既定は先頭）
    @MainActor
    static func open(tab: Ikari_Shell_V1_SettingsTab) {
        let url = appURL()
        guard FileManager.default.fileExists(atPath: url.path) else {
            AppContext.log.error("settings app is missing from the bundle")
            return
        }
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = true
        if let name = argument(for: tab) { configuration.arguments = [tabArgument, name] }
        NSWorkspace.shared.openApplication(at: url, configuration: configuration) { _, error in
            if let error {
                AppContext.log.error("opening the settings app failed: \((error as NSError).code)")
            }
        }
    }
}
