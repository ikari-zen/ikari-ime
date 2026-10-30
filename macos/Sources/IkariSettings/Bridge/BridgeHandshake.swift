// 最初の枠で交わす `BridgeHello` の照合（ADR 0005 §6.3「版の照合」）。設定アプリ（`BridgeClient`）と、
// 偽の殻（`FakeShell`）が使う。本物の殻の照合は shell-core（Rust）が行う（ADR 0041。M5 で IkariBridge から移した）。
import Foundation
import IkariBridge
import IkariProto

/// 最初の枠で交わす `BridgeHello` の照合（ADR 0005 §6.3「版の照合」）。
enum BridgeHandshake {
    /// 殻側: 設定アプリの `BridgeHello` に返事を作る。版が合わなければ `VERSION_MISMATCH` の `BridgeError`
    static func reply(to hello: Ikari_Bridge_V1_BridgeHello, requestID: UInt64, shellVersion: String) -> Frame {
        guard hello.bridgeVersion == BridgeProtocol.version else {
            return .error(requestID: requestID, code: .versionMismatch)
        }
        var reply = Ikari_Bridge_V1_BridgeHelloReply()
        reply.bridgeVersion = BridgeProtocol.version
        reply.shellVersion = shellVersion
        var frame = Frame()
        frame.requestID = requestID
        frame.helloReply = reply
        return frame
    }

    /// 設定アプリ側: 最初に送る枠
    static func hello(requestID: UInt64, appVersion: String) -> Frame {
        var hello = Ikari_Bridge_V1_BridgeHello()
        hello.bridgeVersion = BridgeProtocol.version
        hello.appVersion = appVersion
        var frame = Frame()
        frame.requestID = requestID
        frame.hello = hello
        return frame
    }
}
