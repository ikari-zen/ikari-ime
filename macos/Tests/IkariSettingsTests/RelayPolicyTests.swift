import Foundation
import IkariProto
import IkariBridge
import Testing

@testable import IkariSettings

private func engineFrame(id: UInt64 = 4, _ build: (inout EngineEnvelope) -> Void) -> Frame {
    var envelope = EngineEnvelope()
    envelope.schemaVersion = 1
    envelope.requestID = id
    build(&envelope)
    var frame = Frame()
    frame.requestID = id
    frame.engine = envelope
    return frame
}

@Suite struct RelayPolicyTests {
    @Test func allowsHelloAndEveryAdminRequest() {
        let allowed: [(inout EngineEnvelope) -> Void] = [
            { $0.hello = Ikari_Engine_V1_Hello() },
            { $0.listUserDict = Ikari_Engine_V1_UserDictList() },
            { $0.addUserDict = Ikari_Engine_V1_UserDictAdd() },
            { $0.removeUserDict = Ikari_Engine_V1_UserDictRemove() },
            { $0.learningStatus = Ikari_Engine_V1_LearningStatus() },
            { $0.resetLearning = Ikari_Engine_V1_LearningReset() },
            { $0.exportData = Ikari_Engine_V1_ExportData() },
            { $0.importData = Ikari_Engine_V1_ImportData() },
            { $0.getProfile = Ikari_Engine_V1_ProfileGet() },
            { $0.setProfile = Ikari_Engine_V1_ProfileSet() },
        ]
        for build in allowed {
            let frame = engineFrame(build)
            #expect(RelayPolicy.rejection(for: frame) == nil, "\(frame)")
        }
    }

    @Test func rejectsTheRequestsThatWouldLetTheSettingsAppTouchTyping() {
        // 設定アプリから Commit などを送って学習を汚す経路を作らない
        let rejected: [(inout EngineEnvelope) -> Void] = [
            { $0.beginSession = Ikari_Engine_V1_BeginSession() },
            { $0.endSession = Ikari_Engine_V1_EndSession() },
            { $0.prefetch = Ikari_Engine_V1_Prefetch() },
            { $0.convert = Ikari_Engine_V1_Convert() },
            { $0.page = Ikari_Engine_V1_PageRequest() },
            { $0.commit = Ikari_Engine_V1_Commit() },
            { $0.commitText = Ikari_Engine_V1_CommitText() },
            { $0.cancel = Ikari_Engine_V1_Cancel() },
            { $0.shutdown = Ikari_Engine_V1_Shutdown() },
            // 文節の編集と再変換（ADR 0006）。新しい 6 種は殻の入力だけが送る
            { $0.segmentPage = Ikari_Engine_V1_SegmentPageRequest() },
            { $0.segmentPageReply = Ikari_Engine_V1_SegmentPageReply() },
            { $0.resizeSegment = Ikari_Engine_V1_ResizeSegment() },
            { $0.segmentLayout = Ikari_Engine_V1_SegmentLayout() },
            { $0.commitSegments = Ikari_Engine_V1_CommitSegments() },
            { $0.reconvert = Ikari_Engine_V1_Reconvert() },
            // 英語のモードの郵便番号（ADR 0032 §9.1）も殻の入力だけが頼む
            { $0.postalLookup = Ikari_Engine_V1_PostalLookupRequest() },
            { $0.postalLookupReply = Ikari_Engine_V1_PostalLookupReply() },
        ]
        for build in rejected {
            let frame = engineFrame(id: 12, build)
            let rejection = RelayPolicy.rejection(for: frame)
            #expect(rejection?.requestID == 12, "\(frame)")
            guard case .error(let error)? = rejection?.body else {
                Issue.record("expected a BridgeError for \(frame)")
                continue
            }
            #expect(error.code == .notAllowed)
            #expect(error.key.isEmpty)
        }
    }

    @Test func rejectsRepliesAndErrorsSentAsRequests() {
        let rejected: [(inout EngineEnvelope) -> Void] = [
            { $0.helloReply = Ikari_Engine_V1_HelloReply() },
            { $0.ack = Ikari_Engine_V1_Ack() },
            { $0.convertReply = Ikari_Engine_V1_ConvertReply() },
            { $0.pageReply = Ikari_Engine_V1_ConvertReply() },
            { $0.error = Ikari_Engine_V1_ErrorReply() },
            { $0.userDictListReply = Ikari_Engine_V1_UserDictListReply() },
            { $0.learningStatusReply = Ikari_Engine_V1_LearningStatusReply() },
            { $0.exportReply = Ikari_Engine_V1_ExportReply() },
            { $0.importReply = Ikari_Engine_V1_ImportReply() },
            { $0.profileReply = Ikari_Engine_V1_ProfileReply() },
        ]
        for build in rejected {
            #expect(RelayPolicy.rejection(for: engineFrame(build)) != nil)
        }
    }

    @Test func rejectsAnEnvelopeWithoutABody() {
        #expect(RelayPolicy.rejection(for: engineFrame { _ in })?.requestID == 4)
        #expect(!RelayPolicy.isAllowed(nil))
    }

    @Test func leavesFramesThatAreNotForTheEngineAlone() {
        var frame = Frame()
        frame.requestID = 1
        frame.getSettings = Ikari_Bridge_V1_GetSettings()
        #expect(RelayPolicy.rejection(for: frame) == nil)
    }

    @Test func transfersGetThirtySecondsAndTheRestThree() {
        #expect(RelayPolicy.timeout(for: .importData(Ikari_Engine_V1_ImportData())) == .seconds(30))
        #expect(RelayPolicy.timeout(for: .exportData(Ikari_Engine_V1_ExportData())) == .seconds(30))
        #expect(RelayPolicy.timeout(for: .listUserDict(Ikari_Engine_V1_UserDictList())) == .seconds(3))
        #expect(RelayPolicy.timeout(for: .hello(Ikari_Engine_V1_Hello())) == .seconds(3))
        #expect(RelayPolicy.timeout(for: nil) == .seconds(3))
    }
}

@Suite struct BridgeHandshakeTests {
    @Test func answersAMatchingVersionWithTheShellVersion() {
        let hello = BridgeHandshake.hello(requestID: 1, appVersion: "0.3.0")
        guard case .hello(let message)? = hello.body else {
            Issue.record("expected a hello")
            return
        }
        #expect(message.bridgeVersion == BridgeProtocol.version)
        let reply = BridgeHandshake.reply(to: message, requestID: hello.requestID, shellVersion: "0.3.0")
        #expect(reply.requestID == 1)
        guard case .helloReply(let body)? = reply.body else {
            Issue.record("expected a hello reply, got \(reply)")
            return
        }
        #expect(body.bridgeVersion == BridgeProtocol.version)
        #expect(body.shellVersion == "0.3.0")
    }

    @Test func answersAVersionMismatchWithAnError() {
        var hello = Ikari_Bridge_V1_BridgeHello()
        hello.bridgeVersion = BridgeProtocol.version + 1
        let reply = BridgeHandshake.reply(to: hello, requestID: 2, shellVersion: "0.3.0")
        #expect(reply == Frame.error(requestID: 2, code: .versionMismatch))
    }

    @Test func errorFramesCarryOnlyTheKeyName() {
        let frame = Frame.error(requestID: 3, code: .invalidSetting, key: "paging.size")
        guard case .error(let error)? = frame.body else {
            Issue.record("expected an error")
            return
        }
        #expect(error.code == .invalidSetting)
        #expect(error.key == "paging.size")
    }
}
