// 殻のエントリポイント（ADR 0004 §6.2）。IMKServer を起動して、イベントループを回す。
//
// X（バンドルの組み立て）が Info.plist に入れるキー（ADR 0004 §6.1、docs/phase2-contract.md §1）:
//   CFBundleIdentifier                 = io.github.ikari-zen.inputmethod.ikari
//   CFBundleName                       = ikari
//   InputMethodConnectionName          = io.github.ikari-zen.inputmethod.ikari_Connection   （IkariIdentifiers.inputMethodConnection）
//   InputMethodServerControllerClass   = IkariInputController                      （@objc(IkariInputController) で名前を固定している）
//   LSBackgroundOnly                   = YES
//   LSMinimumSystemVersion             = 26.0
//   TISIntendedLanguage                = ja
//   tsInputMethodCharacterRepertoireKey = [Hira, Kana, Latn]
//   ComponentInputModeDict             = 入力モード1つ（.Japanese「あ」）。アイコン付き（ADR 0005 §15.2-1）
//   IkariSigningMode                   = team | adhoc                              （AppContext が XPC の署名要件に使う）
// Resources には macos/Resources/keymap/*.tsv を `keymap/` ディレクトリごと、アイコンの PDF を入れる。
import AppKit
import Foundation
import IkariXPC
import InputMethodKit

guard let bundleIdentifier = Bundle.main.bundleIdentifier else {
    FileHandle.standardError.write(Data("ikari: no bundle identifier (not running from the app bundle)\n".utf8))
    exit(EXIT_FAILURE)
}

// 接続名の衝突は IMKServer が nil を返さないので、作る前に確かめる
let nameAlreadyRegistered = ServerStartup.isConnectionNameRegistered(IkariIdentifiers.inputMethodConnection)
// サーバを生存させておく（解放されると接続を受けられなくなる）
let server = IMKServer(name: IkariIdentifiers.inputMethodConnection, bundleIdentifier: bundleIdentifier)
guard
    ServerStartup.evaluate(
        server: server, nameAlreadyRegistered: nameAlreadyRegistered, reporter: SystemStartupFailureReporter())
        == .proceed
else {
    exit(EXIT_FAILURE)
}

// 設定アプリとのブリッジ（Unix ソケット）を待ち始める。作れなくても入力は止めない
AppContext.shared.startBridge()

// 正常な終わりを印に残す（ADR 0051。判断は Rust。ここは終了の通知を渡すだけ）
final class TerminationNotifier: NSObject, NSApplicationDelegate {
    func applicationWillTerminate(_ notification: Notification) { AppContext.shared.driver.noteExit() }
}

let application = NSApplication.shared
let terminationNotifier = TerminationNotifier()
application.delegate = terminationNotifier
application.setActivationPolicy(.accessory)
application.run()
