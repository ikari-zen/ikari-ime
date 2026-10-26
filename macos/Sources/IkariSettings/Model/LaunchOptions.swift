// 起動の引数（ADR 0005 §6.2「引数で開くタブ」）。殻が `NSWorkspace.openApplication` の configuration.arguments で渡す。
import Foundation
import IkariShellCore

struct LaunchOptions: Equatable {
    /// 開くタブ（`SettingsTab.rawValue`）。指定がなければ先頭
    var tab: SettingsTab?
    /// 本物の殻の代わりに、メモリ上の偽の殻とエンジンにつなぐ（画面の確認用）
    var fakeBridge = false

    static let tabFlag = "--tab"
    static let fakeBridgeFlag = "--fake-bridge"

    /// `--tab dictionary` と `--tab=dictionary` の両方を読む。知らないタブ名と引数は無視する
    static func parse(_ arguments: [String]) -> LaunchOptions {
        var options = LaunchOptions()
        var remaining = arguments[...]
        while let argument = remaining.popFirst() {
            if argument == fakeBridgeFlag {
                options.fakeBridge = true
            } else if argument == tabFlag {
                if let name = remaining.first, let tab = SettingsTab(rawValue: name) {
                    options.tab = tab
                    remaining.removeFirst()
                }
            } else if argument.hasPrefix(tabFlag + "="), let tab = SettingsTab(rawValue: String(argument.dropFirst(tabFlag.count + 1))) {
                options.tab = tab
            }
        }
        return options
    }
}
