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
    @Test func nilServerLogsAndNotifiesOnce() {
        let spy = SpyReporter()
        #expect(ServerStartup.checkServer(nil, reporter: spy) == false)
        #expect(spy.logs == [ServerStartup.logMessage])
        #expect(spy.notices.count == 1)
        #expect(spy.notices.first?.title == ServerStartup.noticeTitle)
    }

    @Test func createdServerReportsNothing() {
        let spy = SpyReporter()
        #expect(ServerStartup.checkServer(NSObject(), reporter: spy) == true)
        #expect(spy.logs.isEmpty)
        #expect(spy.notices.isEmpty)
    }
}
