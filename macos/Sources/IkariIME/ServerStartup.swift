// IMKServer の起動の判断（接続名の登録に失敗したとき、ログに残して利用者に知らせる）。
// 判断の置き場所: 検知と通知は OS の入口なので Swift。判断は小さな関数に分け、IMKServer を作らずにテストできるようにする。
import AppKit
import Darwin
import Foundation
import os

/// 起動に失敗したことの知らせ先（ログと利用者への通知）
protocol StartupFailureReporting {
    func log(_ message: String)
    func notifyUser(title: String, body: String)
}

enum StartupDecision: Equatable {
    case proceed
    case terminate
}

enum ServerStartup {
    static let logMessage = "failed to start IMKServer (connection name could not be registered)"
    static let noticeTitle = "ikari を起動できませんでした"
    static let noticeBody =
        "ikari が入力の受け付けを始められませんでした。ほかの入力メソッドが同じ名前を使っているか、ikari が正しくインストールされていない可能性があります。ikari を入れ直し、いったんログアウトして、もう一度ログインしてください。"

    static let conflictLogMessage = "connection name was already registered before IMKServer was created"
    static let conflictNoticeBody =
        "ikari が入力の受け付けを始められなかった可能性があります。ほかの入力メソッドが同じ名前を使っているか、前の ikari が残っているかもしれません。いったんログアウトして、もう一度ログインしてください。"

    /// IMKServer を作る前の事前確認。接続名をすでにほかのプロセスが持っていれば true
    /// （衝突しても IMKServer は nil を返さず、IMK が標準エラーに書くだけなので、nil の検査では取れない）
    static func isConnectionNameRegistered(_ name: String) -> Bool {
        // NSMachBootstrapServer は Swift から使えず、bootstrap_look_up はヘッダだけで公開されているので、シンボルを引いて呼ぶ。
        // 引けたら、ほかのプロセスがその名前を持っている
        typealias LookUp = @convention(c) (mach_port_t, UnsafePointer<CChar>, UnsafeMutablePointer<mach_port_t>) -> kern_return_t
        guard let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "bootstrap_look_up") else { return false }  // RTLD_DEFAULT
        let lookUp = unsafeBitCast(symbol, to: LookUp.self)
        var bootstrap: mach_port_t = mach_port_t(MACH_PORT_NULL)
        guard task_get_special_port(mach_task_self_, TASK_BOOTSTRAP_PORT, &bootstrap) == KERN_SUCCESS else { return false }
        var port: mach_port_t = mach_port_t(MACH_PORT_NULL)
        guard lookUp(bootstrap, name, &port) == KERN_SUCCESS else { return false }
        mach_port_deallocate(mach_task_self_, port)
        return true
    }

    /// 作れなかった（nil）ときは、ログと通知のあと終了を返す。
    /// 事前確認で衝突とわかったときは、ログと通知だけ出して続行する（通常の起動での誤検知で IME が止まらないように）
    static func evaluate(
        server: AnyObject?, nameAlreadyRegistered: Bool, reporter: StartupFailureReporting
    ) -> StartupDecision {
        if server == nil {
            reporter.log(logMessage)
            reporter.notifyUser(title: noticeTitle, body: noticeBody)
            return .terminate
        }
        if nameAlreadyRegistered {
            reporter.log(conflictLogMessage)
            reporter.notifyUser(title: noticeTitle, body: conflictNoticeBody)
        }
        return .proceed
    }
}

/// 実機の知らせ先。ログは os.Logger と標準エラー、通知は NSAlert（権限も entitlement も要らない）
struct SystemStartupFailureReporter: StartupFailureReporting {
    func log(_ message: String) {
        AppContext.log.error("\(message, privacy: .public)")
        FileHandle.standardError.write(Data("ikari: \(message)\n".utf8))
    }

    func notifyUser(title: String, body: String) {
        // 起動時の主スレッドから呼ぶ（main.swift）
        MainActor.assumeIsolated {
            let alert = NSAlert()
            alert.alertStyle = .critical
            alert.messageText = title
            alert.informativeText = body
            alert.runModal()
        }
    }
}

/// 不正な学習ファイルを退避して空で起動したことの知らせ（#271）。
/// 判断の置き場所: 出すかどうか（エンジンの Hello の返事で 1 回だけ）は Rust（shell-core）。ここは表示だけ。
enum LearningQuarantineNotice {
    static let logMessage = "learning file was unreadable; moved aside and started with an empty learning"
    static let noticeTitle = "学習データを読み込めませんでした"
    static let noticeBody =
        "学習データを読み込めなかったため、いったん空の状態で起動しました。元のファイルは、学習データと同じフォルダに「learning.tsv.bad」などの名前で残してあります。"

    /// ログと通知を 1 回ずつ出す
    static func present(reporter: StartupFailureReporting) {
        reporter.log(logMessage)
        reporter.notifyUser(title: noticeTitle, body: noticeBody)
    }
}
