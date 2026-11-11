// IMKServer が作れなかったときの知らせ（ログと通知が 1 回ずつ）。実際の IMKServer は作らない。
import Foundation
import Testing

@testable import IkariIME

private final class SpyReporter: StartupFailureReporting {
    var logs: [String] = []
    var notices: [(title: String, body: String)] = []
    func log(_ message: String) { logs.append(message) }
    func notifyUser(title: String, body: String) { notices.append((title, body)) }
}

@Suite struct ServerStartupTests {
    @Test func nilServerLogsNotifiesAndTerminates() {
        let spy = SpyReporter()
        #expect(ServerStartup.evaluate(server: nil, nameAlreadyRegistered: false, reporter: spy) == .terminate)
        #expect(spy.logs == [ServerStartup.logMessage])
        #expect(spy.notices.count == 1)
        #expect(spy.notices.first?.body == ServerStartup.noticeBody)
    }

    @Test func nilServerWithConflictNotifiesOnlyOnce() {
        let spy = SpyReporter()
        #expect(ServerStartup.evaluate(server: nil, nameAlreadyRegistered: true, reporter: spy) == .terminate)
        #expect(spy.logs.count == 1)
        #expect(spy.notices.count == 1)
    }

    @Test func conflictLogsAndNotifiesOnceButKeepsRunning() {
        let spy = SpyReporter()
        #expect(ServerStartup.evaluate(server: NSObject(), nameAlreadyRegistered: true, reporter: spy) == .proceed)
        #expect(spy.logs == [ServerStartup.conflictLogMessage])
        #expect(spy.notices.count == 1)
        #expect(spy.notices.first?.body == ServerStartup.conflictNoticeBody)
    }

    @Test func noConflictAndCreatedReportsNothing() {
        let spy = SpyReporter()
        #expect(ServerStartup.evaluate(server: NSObject(), nameAlreadyRegistered: false, reporter: spy) == .proceed)
        #expect(spy.logs.isEmpty)
        #expect(spy.notices.isEmpty)
    }

    @Test func lookupFindsARegisteredNameAndNotAnUnregisteredOne() {
        #expect(ServerStartup.isConnectionNameRegistered("com.apple.cfprefsd.daemon"))
        #expect(!ServerStartup.isConnectionNameRegistered("io.github.ikari-zen.test.no-such-name-\(UUID().uuidString)"))
    }
}

@Suite struct LearningQuarantineNoticeTests {
    @Test func presentLogsAndNotifiesOnce() {
        let spy = SpyReporter()
        LearningQuarantineNotice.present(reporter: spy)
        #expect(spy.logs == [LearningQuarantineNotice.logMessage])
        #expect(spy.notices.count == 1)
        #expect(spy.notices.first?.title == "学習データを読み込めませんでした")
        #expect(spy.notices.first?.body.contains("learning.tsv.bad") == true)
    }
}
