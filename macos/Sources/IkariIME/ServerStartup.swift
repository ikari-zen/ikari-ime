// IMKServer の起動の判断（接続名の登録に失敗したとき、ログに残して利用者に知らせる）。
// 判断の置き場所: 検知と通知は OS の入口なので Swift。判断は小さな関数に分け、IMKServer を作らずにテストできるようにする。
import AppKit
import Foundation
import os

/// 起動に失敗したことの知らせ先（ログと利用者への通知）
protocol StartupFailureReporting {
    func log(_ message: String)
    func notifyUser(title: String, body: String)
}

enum ServerStartup {
    static let logMessage = "failed to start IMKServer (connection name could not be registered)"
    static let noticeTitle = "ikari を起動できませんでした"
    static let noticeBody =
        "ikari が入力の受け付けを始められませんでした。ほかの入力メソッドが同じ名前を使っているか、ikari が正しくインストールされていない可能性があります。ikari を入れ直し、いったんログアウトして、もう一度ログインしてください。"

    /// 作れたら true。作れなかったらログと通知を 1 回ずつ呼んで false
    static func checkServer(_ server: AnyObject?, reporter: StartupFailureReporting) -> Bool {
        guard server == nil else { return true }
        reporter.log(logMessage)
        reporter.notifyUser(title: noticeTitle, body: noticeBody)
        return false
    }
}

/// 実機の知らせ先。ログは os.Logger と標準エラー、通知は NSAlert（権限も entitlement も要らない）
struct SystemStartupFailureReporter: StartupFailureReporting {
    func log(_ message: String) {
        AppContext.log.error("\(message, privacy: .public)")
        FileHandle.standardError.write(Data("ikari: \(message)\n".utf8))
    }

    func notifyUser(title: String, body: String) {
        let alert = NSAlert()
        alert.alertStyle = .critical
        alert.messageText = title
        alert.informativeText = body
        alert.runModal()
    }
}
