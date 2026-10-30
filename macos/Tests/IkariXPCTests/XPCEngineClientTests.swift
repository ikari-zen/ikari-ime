import Foundation
import IkariProto
import Testing

@testable import IkariXPC

/// モックのトランスポート。受け取った Envelope を記録し、`respond` が返事を決める。
private final class MockTransport: EngineTransport, @unchecked Sendable {
    typealias Responder = @Sendable (Envelope) throws -> Envelope?

    private let lock = NSLock()
    private var _requests: [Envelope] = []
    private var _posts: [Envelope] = []
    private var _closed = false
    private let respond: Responder
    let onDown: @Sendable (TransportDown) -> Void
    let processIdentifier: Int32

    init(pid: Int32, respond: @escaping Responder, onDown: @escaping @Sendable (TransportDown) -> Void) {
        self.processIdentifier = pid
        self.respond = respond
        self.onDown = onDown
    }

    var requests: [Envelope] { lock.withLock { _requests } }
    var posts: [Envelope] { lock.withLock { _posts } }
    var closed: Bool { lock.withLock { _closed } }

    func request(_ data: Data, timeout: Duration) throws -> Data {
        let env = try Envelope(serializedBytes: data)
        lock.withLock { _requests.append(env) }
        guard var reply = try respond(env) else { throw EngineClientError.timeout }
        if reply.requestID == 0 { reply.requestID = env.requestID }
        return try reply.serializedData()
    }

    func post(_ data: Data) {
        if let env = try? Envelope(serializedBytes: data) { lock.withLock { _posts.append(env) } }
    }

    func close() { lock.withLock { _closed = true } }
}

/// 時計と、作られたトランスポートの記録
private final class Harness: @unchecked Sendable {
    private let lock = NSLock()
    private var _now = Date(timeIntervalSince1970: 1_000_000)
    private(set) var transports: [MockTransport] = []
    var engineVersion = "1.0"
    var respond: MockTransport.Responder?

    var now: Date { lock.withLock { _now } }
    func advance(_ seconds: TimeInterval) { lock.withLock { _now = _now.addingTimeInterval(seconds) } }
    var latest: MockTransport { transports[transports.count - 1] }

    func makeClient() -> XPCEngineClient {
        XPCEngineClient(
            transportFactory: { [self] onDown in
                let version = engineVersion
                let custom = respond
                let transport = MockTransport(
                    pid: Int32(100 + transports.count),
                    respond: custom ?? { Harness.defaultResponse($0, engineVersion: version) },
                    onDown: onDown)
                lock.withLock { transports.append(transport) }
                return transport
            },
            now: { [self] in now }
        )
    }

    static func envelope(_ build: (inout Envelope) -> Void) -> Envelope {
        var env = Envelope()
        env.schemaVersion = 1
        build(&env)
        return env
    }

    static func defaultResponse(_ request: Envelope, engineVersion: String) -> Envelope? {
        switch request.body {
        case .hello?:
            return envelope {
                var reply = Ikari_Engine_V1_HelloReply()
                reply.engineVersion = engineVersion
                reply.dictLabel = "test"
                reply.lm = .lmReady
                $0.helloReply = reply
            }
        case .convert?, .page?:
            return envelope { env in
                var reply = Ikari_Engine_V1_ConvertReply()
                reply.conversionID = 7
                reply.candidates = ["品詞", "瀕死"].map {
                    var c = Ikari_Engine_V1_Candidate()
                    c.surface = $0
                    return c
                }
                reply.hasNext_p = true
                if case .convert? = request.body { env.convertReply = reply } else { env.pageReply = reply }
            }
        default:
            return envelope { $0.ack = Ikari_Engine_V1_Ack() }
        }
    }
}

private let sampleRequest = ConversionRequest(
    sessionID: 1, reading: "ひんし", context: SurroundingText(left: "ユーザー辞書に", available: true), contextRev: 1)

@Suite struct XPCEngineClientTests {
    @Test func helloReturnsTheEngineInformationWhenTheVersionMatches() throws {
        let harness = Harness()
        let client = harness.makeClient()
        let hello = try client.hello(shellVersion: "1.0", timeout: .seconds(1))
        #expect(hello.engineVersion == "1.0")
        #expect(hello.dictLabel == "test")
        #expect(hello.lm == .ready)
        #expect(harness.transports.count == 1)
        #expect(harness.latest.requests.count == 1)
        #expect(harness.latest.requests[0].schemaVersion == 1)
    }

    @Test func versionMismatchSendsShutdownRetriesOnceAndGivesUp() {
        let harness = Harness()
        harness.engineVersion = "0.9"
        let client = harness.makeClient()
        #expect(throws: EngineClientError.unavailable) {
            try client.hello(shellVersion: "1.0", timeout: .seconds(1))
        }
        let kinds = harness.latest.requests.map { env -> String in
            switch env.body {
            case .hello?: "hello"
            case .shutdown?: "shutdown"
            default: "other"
            }
        }
        #expect(kinds == ["hello", "shutdown", "hello"])
        #expect(client.lastEngineVersion == "0.9")
    }

    @Test func convertMapsAndValidatesCandidates() throws {
        let harness = Harness()
        let client = harness.makeClient()
        let page = try client.convert(sampleRequest, timeout: .milliseconds(100))
        #expect(page.conversionID == 7)
        #expect(page.candidates.map(\.surface) == ["品詞", "瀕死"])
        #expect(page.hasNext)
        let sent = harness.latest.requests[0]
        guard case .convert(let convert)? = sent.body else {
            Issue.record("convert was not sent")
            return
        }
        #expect(convert.reading == "ひんし")
        #expect(convert.context.left == "ユーザー辞書に")
        #expect(convert.pageSize == 9)
    }

    @Test func leftContextIsClippedToTheLastTwoThousandCharacters() throws {
        let harness = Harness()
        let client = harness.makeClient()
        var request = sampleRequest
        request.context = SurroundingText(left: String(repeating: "あ", count: 2500) + "終", available: true)
        _ = try client.convert(request, timeout: .milliseconds(100))
        guard case .convert(let convert)? = harness.latest.requests[0].body else { return }
        #expect(convert.context.left.count == 2000)
        #expect(convert.context.left.hasSuffix("終"))
    }

    @Test func invalidRequestsAreRejectedBeforeSending() {
        let harness = Harness()
        let client = harness.makeClient()
        var request = sampleRequest
        request.reading = String(repeating: "あ", count: 257)
        #expect(throws: EngineClientError.engine(.badRequest)) { try client.convert(request, timeout: .seconds(1)) }
        request.reading = ""
        #expect(throws: EngineClientError.engine(.badRequest)) { try client.convert(request, timeout: .seconds(1)) }
        request.reading = "あ"
        request.pageSize = 10
        #expect(throws: EngineClientError.engine(.badRequest)) { try client.convert(request, timeout: .seconds(1)) }
        #expect(harness.transports.isEmpty)
    }

    @Test func tooManyCandidatesIsAnInvalidReply() {
        let harness = Harness()
        harness.respond = { _ in
            Harness.envelope { env in
                var reply = Ikari_Engine_V1_ConvertReply()
                reply.candidates = (0..<200).map { index in
                    var c = Ikari_Engine_V1_Candidate()
                    c.surface = "候補\(index)"
                    return c
                }
                env.convertReply = reply
            }
        }
        let client = harness.makeClient()
        #expect(throws: EngineClientError.invalidReply) { try client.convert(sampleRequest, timeout: .seconds(1)) }
    }

    @Test func aMismatchedRequestIDIsAnInvalidReply() {
        let harness = Harness()
        harness.respond = { _ in
            var env = Harness.envelope { $0.ack = Ikari_Engine_V1_Ack() }
            env.requestID = 999_999
            return env
        }
        let client = harness.makeClient()
        #expect(throws: EngineClientError.invalidReply) { try client.convert(sampleRequest, timeout: .seconds(1)) }
    }

    @Test func errorRepliesBecomeEngineErrors() {
        let harness = Harness()
        harness.respond = { _ in
            Harness.envelope {
                var error = Ikari_Engine_V1_ErrorReply()
                error.code = .engineNotReady
                $0.error = error
            }
        }
        let client = harness.makeClient()
        #expect(throws: EngineClientError.engine(.engineNotReady)) {
            try client.convert(sampleRequest, timeout: .seconds(1))
        }
    }

    @Test func timeoutsPassThrough() {
        let harness = Harness()
        harness.respond = { _ in nil }
        let client = harness.makeClient()
        #expect(throws: EngineClientError.timeout) { try client.convert(sampleRequest, timeout: .milliseconds(1)) }
    }

    @Test func notificationsArePostedWithoutWaiting() {
        let harness = Harness()
        harness.respond = { _ in nil }  // request() は使われない
        let client = harness.makeClient()
        client.prefetch(sampleRequest)
        client.commit(conversionID: 7, candidateIndex: 0)
        client.commitText(sessionID: 1, reading: "か", surface: "か")
        client.cancel(conversionID: 7)
        #expect(harness.latest.posts.count == 4)
        #expect(harness.latest.requests.isEmpty)
    }

    @Test func aCrashResyncsHelloAndSessionsBeforeTheNextRequest() throws {
        let harness = Harness()
        let client = harness.makeClient()
        _ = try client.hello(shellVersion: "1.0", timeout: .seconds(1))
        client.beginSession(SessionInfo(sessionID: 1, clientBundleID: "com.apple.TextEdit", secure: false))
        client.beginSession(SessionInfo(sessionID: 2, clientBundleID: "com.apple.Notes", secure: true))
        client.endSession(sessionID: 2)
        let transport = harness.latest
        let postsBefore = transport.posts.count

        transport.onDown(.interrupted)  // エンジンが落ちた
        _ = try client.convert(sampleRequest, timeout: .seconds(1))

        let resent = Array(transport.posts.dropFirst(postsBefore))
        let kinds = resent.map { env -> String in
            switch env.body {
            case .hello?: "hello"
            case .beginSession(let s)?: "begin\(s.sessionID)"
            default: "other"
            }
        }
        #expect(kinds == ["hello", "begin1"])  // 終わったセッション 2 は送り直さない
        #expect(harness.transports.count == 1)  // 中断では接続を作り直さない
    }

    @Test func anEngineThatOpenedLateGetsHelloAndSessionsBeforeTheNextRequest() throws {
        let harness = Harness()
        let notReady = LockedFlag(true)
        harness.respond = { request in
            guard notReady.value else { return Harness.defaultResponse(request, engineVersion: "1.0") }
            return Harness.envelope {
                var failure = Ikari_Engine_V1_ErrorReply()
                failure.code = .engineNotReady
                $0.error = failure
            }
        }
        let client = harness.makeClient()
        _ = try? client.hello(shellVersion: "1.0", timeout: .seconds(1))
        client.beginSession(SessionInfo(sessionID: 1, clientBundleID: "com.apple.TextEdit", secure: false))
        #expect(throws: EngineClientError.engine(.engineNotReady)) {
            _ = try client.convert(sampleRequest, timeout: .seconds(1))
        }
        // ロックが空いて、エンジンが開けた。開いたエンジンは、先の BeginSession を知らない
        notReady.value = false
        let transport = harness.latest
        let postsBefore = transport.posts.count
        _ = try client.convert(sampleRequest, timeout: .seconds(1))
        let kinds = transport.posts.dropFirst(postsBefore).map { env -> String in
            switch env.body {
            case .hello?: "hello"
            case .beginSession(let s)?: "begin\(s.sessionID)"
            default: "other"
            }
        }
        #expect(kinds == ["hello", "begin1"])
    }

    @Test func unknownSessionResendsHelloAndSessionsThenRetriesOnce() throws {
        // 後から開いたエンジンは、ENGINE_NOT_READY ではなく UNKNOWN_SESSION を返す（実機で起きた）。
        // そのときも hello とセッションを送り直し、同じ要求を1回だけ送り直す
        let harness = Harness()
        let forgotten = LockedFlag(false)
        harness.respond = { request in
            // 最初の Convert だけ UNKNOWN_SESSION。送り直しのあとの Convert は通常どおり
            guard forgotten.value, case .convert? = request.body else {
                return Harness.defaultResponse(request, engineVersion: "1.0")
            }
            forgotten.value = false
            return Harness.envelope {
                var failure = Ikari_Engine_V1_ErrorReply()
                failure.code = .unknownSession
                $0.error = failure
            }
        }
        let client = harness.makeClient()
        _ = try? client.hello(shellVersion: "1.0", timeout: .seconds(1))
        client.beginSession(SessionInfo(sessionID: 1, clientBundleID: "com.apple.TextEdit", secure: false))
        forgotten.value = true  // エンジンが開き直して、セッションを忘れた
        let transport = harness.latest
        let postsBefore = transport.posts.count
        _ = try client.convert(sampleRequest, timeout: .seconds(1))
        let kinds = transport.posts.dropFirst(postsBefore).map { env -> String in
            switch env.body {
            case .hello?: "hello"
            case .beginSession(let s)?: "begin\(s.sessionID)"
            default: "other"
            }
        }
        #expect(kinds == ["hello", "begin1"])
    }

    // MARK: エンジンが起動し直したあとの再同期（#315）

    @Test func aPrefetchAfterACrashResyncsBeforeItIsPosted() throws {
        // 仮説 (c) の否定: 片道の prefetch でも、切断のあとの最初の送信の前に hello とセッションを送り直す
        let harness = Harness()
        let client = harness.makeClient()
        _ = try client.hello(shellVersion: "1.0", timeout: .seconds(1))
        client.beginSession(SessionInfo(sessionID: 1, clientBundleID: "com.apple.TextEdit", secure: false))
        let transport = harness.latest
        let postsBefore = transport.posts.count

        transport.onDown(.interrupted)
        client.prefetch(sampleRequest)
        client.commitText(sessionID: 1, reading: "か", surface: "か")

        #expect(postKinds(transport.posts.dropFirst(postsBefore)) == ["hello", "begin1", "prefetch", "commit_text"])
    }

    @Test func anInvalidatedConnectionResyncsOnTheNewConnection() throws {
        // 仮説 (a) の否定: interruption でなく invalidation で終わっても、新しい接続の最初に送り直す
        let harness = Harness()
        let client = harness.makeClient()
        _ = try client.hello(shellVersion: "1.0", timeout: .seconds(1))
        client.beginSession(SessionInfo(sessionID: 1, clientBundleID: "com.apple.TextEdit", secure: false))
        harness.latest.onDown(.invalidated)

        client.prefetch(sampleRequest)
        _ = try client.convert(sampleRequest, timeout: .seconds(1))

        #expect(harness.transports.count == 2)
        #expect(postKinds(harness.latest.posts[...]) == ["hello", "begin1", "prefetch"])
        #expect(harness.latest.requests.count == 1)  // convert は 1 回で通る（UNKNOWN_SESSION にならない）
    }

    @Test func aSessionThatWasNeverBegunCannotBeResyncedWhichMatchesTheLogs() throws {
        // 実機のログの形（#315）: hello も BeginSession も出ずに、convert が同じ ms に 2 回 UNKNOWN_SESSION になる。
        // 殻が hello も BeginSession も一度も送っていない（activateServer を経ずに打鍵が来た）ときにだけ、この形になる。
        // XPCEngineClient は知らないセッションを送り直せないので、BeginSession を送るのは呼び出す側（InputStateMachine）
        let harness = Harness()
        harness.respond = { request in
            guard case .convert? = request.body else { return Harness.defaultResponse(request, engineVersion: "1.0") }
            return errorReply(.unknownSession)
        }
        let client = harness.makeClient()
        #expect(throws: EngineClientError.engine(.unknownSession)) {
            _ = try client.convert(sampleRequest, timeout: .seconds(1))
        }
        let kinds = harness.latest.requests.map { env -> String in
            if case .convert? = env.body { "convert" } else { "other" }
        }
        #expect(kinds == ["convert", "convert"])
        #expect(harness.latest.posts.isEmpty)
    }

    @Test func invalidationRebuildsTheConnection() throws {
        let harness = Harness()
        let client = harness.makeClient()
        _ = try client.convert(sampleRequest, timeout: .seconds(1))
        harness.latest.onDown(.invalidated)
        _ = try client.convert(sampleRequest, timeout: .seconds(1))
        #expect(harness.transports.count == 2)
    }

    @Test func staleConnectionNotificationsAreIgnored() throws {
        let harness = Harness()
        let client = harness.makeClient()
        _ = try client.convert(sampleRequest, timeout: .seconds(1))
        let first = harness.latest
        first.onDown(.invalidated)
        _ = try client.convert(sampleRequest, timeout: .seconds(1))
        let second = harness.latest
        // 古い接続の遅れた通知は、新しい接続を壊さない
        first.onDown(.invalidated)
        _ = try client.convert(sampleRequest, timeout: .seconds(1))
        #expect(harness.transports.count == 2)
        #expect(second.requests.count == 2)
    }

    @Test func threeCrashesInASuspensionWindowStopCallingTheEngineForFiveMinutes() throws {
        let harness = Harness()
        let client = harness.makeClient()
        _ = try client.convert(sampleRequest, timeout: .seconds(1))
        for _ in 0..<3 {
            harness.latest.onDown(.interrupted)
            harness.advance(10)
        }
        #expect(client.isSuspended)
        #expect(throws: EngineClientError.unavailable) { try client.convert(sampleRequest, timeout: .seconds(1)) }

        harness.advance(XPCEngineClient.suspension + 1)
        #expect(!client.isSuspended)
        _ = try client.convert(sampleRequest, timeout: .seconds(1))
    }

    @Test func crashesSpreadOverMoreThanAMinuteDoNotSuspend() throws {
        let harness = Harness()
        let client = harness.makeClient()
        _ = try client.convert(sampleRequest, timeout: .seconds(1))
        for _ in 0..<5 {
            harness.latest.onDown(.interrupted)
            harness.advance(40)
        }
        #expect(!client.isSuspended)
        _ = try client.convert(sampleRequest, timeout: .seconds(1))
    }

    @Test func shutdownIsNotCountedAsACrash() {
        let harness = Harness()
        let client = harness.makeClient()
        for _ in 0..<4 {
            client.shutdown(reason: .settingsChanged)
            harness.latest.onDown(.interrupted)
        }
        #expect(!client.isSuspended)
    }

    @Test func shutdownCarriesTheReason() {
        let harness = Harness()
        let client = harness.makeClient()
        client.shutdown(reason: .settingsChanged)
        client.shutdown(reason: .versionMismatch)
        let reasons = harness.transports.flatMap(\.requests).compactMap { env -> Ikari_Engine_V1_ShutdownReason? in
            if case .shutdown(let message)? = env.body { message.reason } else { nil }
        }
        #expect(reasons == [.settingsChanged, .versionMismatch])
    }

    @Test func pidIsReportedFromTheTransport() throws {
        let harness = Harness()
        let client = harness.makeClient()
        #expect(client.enginePID == 0)
        _ = try client.convert(sampleRequest, timeout: .seconds(1))
        #expect(client.enginePID == 100)
    }
}

// MARK: 管理の要求の中継（ADR 0005 §8.3）

private func profileRequest() -> Data {
    EnvelopeCodec.encode(EnvelopeCodec.envelope(requestID: 999) { $0.getProfile = Ikari_Engine_V1_ProfileGet() })
}

private func profileReply(_ domains: [(String, String, Ikari_Engine_V1_ProfileWeightKind)]) -> Envelope {
    Harness.envelope { env in
        var reply = Ikari_Engine_V1_ProfileReply()
        reply.domains = domains.map { id, name, weight in
            var state = Ikari_Engine_V1_ProfileDomainState()
            state.id = id
            state.name = name
            state.weight = weight
            return state
        }
        env.profileReply = reply
    }
}

private func errorReply(_ code: Ikari_Engine_V1_ErrorCode) -> Envelope {
    Harness.envelope {
        var error = Ikari_Engine_V1_ErrorReply()
        error.code = code
        $0.error = error
    }
}

/// 片道で送った Envelope の種類（再同期の順を比べる）
private func postKinds(_ posts: ArraySlice<Envelope>) -> [String] {
    posts.map { env -> String in
        switch env.body {
        case .hello?: "hello"
        case .beginSession(let s)?: "begin\(s.sessionID)"
        case .prefetch?: "prefetch"
        case .commitText?: "commit_text"
        default: "other"
        }
    }
}

private final class LockedFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var flag: Bool
    init(_ initial: Bool) { flag = initial }
    var value: Bool {
        get { lock.withLock { flag } }
        set { lock.withLock { flag = newValue } }
    }
}

private final class LockedCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0
    func next() -> Int {
        lock.withLock {
            defer { value += 1 }
            return value
        }
    }
}

@Suite struct RelayTests {
    @Test func relaysTheBytesAndReturnsTheReplyAsIs() throws {
        let harness = Harness()
        harness.respond = { _ in profileReply([("it", "IT", .profileWeightOften)]) }
        let client = harness.makeClient()
        let reply = try client.relay(profileRequest(), timeout: .seconds(1))
        guard case .profileReply(let body)? = try Envelope(serializedBytes: reply).body else {
            Issue.record("expected profile reply")
            return
        }
        #expect(body.domains.map(\.id) == ["it"])
    }

    @Test func reassignsTheRequestIDAndSchemaVersion() throws {
        let harness = Harness()
        harness.respond = { _ in profileReply([]) }
        let client = harness.makeClient()
        _ = try client.relay(profileRequest(), timeout: .seconds(1))
        _ = try client.relay(profileRequest(), timeout: .seconds(1))
        let sent = harness.latest.requests
        #expect(sent.count == 2)
        #expect(sent.allSatisfy { $0.schemaVersion == 1 && $0.requestID != 999 && $0.requestID != 0 })
        #expect(sent[0].requestID != sent[1].requestID)
        guard case .getProfile? = sent[0].body else {
            Issue.record("body must be kept")
            return
        }
    }

    @Test func anErrorReplyIsReturnedNotThrown() throws {
        let harness = Harness()
        harness.respond = { _ in errorReply(.badRequest) }
        let reply = try harness.makeClient().relay(profileRequest(), timeout: .seconds(1))
        guard case .error(let error)? = try Envelope(serializedBytes: reply).body else {
            Issue.record("expected error reply")
            return
        }
        #expect(error.code == .badRequest)
    }

    @Test func timeoutPassesThroughAndTheNextRelayStillWorks() throws {
        let harness = Harness()
        let counter = LockedCounter()
        harness.respond = { _ in counter.next() == 0 ? nil : profileReply([]) }
        let client = harness.makeClient()
        #expect(throws: EngineClientError.timeout) { try client.relay(profileRequest(), timeout: .milliseconds(1)) }
        _ = try client.relay(profileRequest(), timeout: .seconds(1))
    }

    @Test func resyncsAfterACrashAndReconnectsAfterInvalidation() throws {
        let harness = Harness()
        harness.respond = { request in
            if case .hello? = request.body { return Harness.defaultResponse(request, engineVersion: "1.0") }
            return profileReply([])
        }
        let client = harness.makeClient()
        _ = try client.hello(shellVersion: "1.0", timeout: .seconds(1))
        let first = harness.latest
        first.onDown(.interrupted)
        _ = try client.relay(profileRequest(), timeout: .seconds(1))
        // 落ちたあとの最初の要求の前に Hello を送り直している
        #expect(first.posts.contains { if case .hello? = $0.body { true } else { false } })

        first.onDown(.invalidated)
        _ = try client.relay(profileRequest(), timeout: .seconds(1))
        #expect(harness.transports.count == 2)
    }

    @Test func malformedRequestsAreRejectedBeforeSending() {
        let harness = Harness()
        let client = harness.makeClient()
        #expect(throws: EngineClientError.engine(.badRequest)) { try client.relay(Data(), timeout: .seconds(1)) }
        #expect(throws: EngineClientError.engine(.badRequest)) {
            try client.relay(Data([0xFF, 0xFF, 0xFF, 0xFF]), timeout: .seconds(1))
        }
        #expect(harness.transports.allSatisfy { $0.requests.isEmpty })
    }

    @Test func settingsChangedShutdownIsAnExpectedExitSoRepeatedRestartsDoNotSuspend() throws {
        let harness = Harness()
        let client = harness.makeClient()
        for _ in 0..<5 {
            client.shutdown(reason: .settingsChanged)
            harness.latest.onDown(.interrupted)
            _ = try client.hello(shellVersion: "1.0", timeout: .seconds(1))
        }
        #expect(!client.isSuspended)
        #expect(harness.transports.count == 1)
    }
}

// MARK: 文節と再変換（ADR 0006）

private func layoutEnvelope(
    segments: [(String, String)], focused: UInt32 = 0, rev: UInt32 = 0, withPage: Bool = true
) -> Envelope {
    Harness.envelope { env in
        var layout = Ikari_Engine_V1_SegmentLayout()
        layout.conversionID = 9
        layout.layoutRev = rev
        layout.focused = focused
        layout.segments = segments.map {
            var info = Ikari_Engine_V1_SegmentInfo()
            info.reading = $0.0
            info.surface = $0.1
            return info
        }
        if withPage {
            var page = Ikari_Engine_V1_SegmentPageReply()
            page.segment = focused
            page.candidates = [segments[Int(focused)].1, "別"].map {
                var c = Ikari_Engine_V1_Candidate()
                c.surface = $0
                return c
            }
            page.hasNext_p = true
            layout.focusedPage = page
        }
        env.segmentLayout = layout
    }
}

@Suite struct XPCSegmentTests {
    @Test func segmentPageSendsTheLayoutRevisionAndLeftChoicesAndMapsTheReply() throws {
        let harness = Harness()
        harness.respond = { request in
            guard case .segmentPage? = request.body else { return nil }
            return Harness.envelope { env in
                var reply = Ikari_Engine_V1_SegmentPageReply()
                reply.segment = 1
                reply.pageIndex = 2
                var c = Ikari_Engine_V1_Candidate()
                c.surface = "知った"
                c.kind = .literal
                reply.candidates = [c]
                reply.hasNext_p = true
                env.segmentPageReply = reply
            }
        }
        let client = harness.makeClient()
        let page = try client.segmentPage(
            conversionID: 9, layoutRev: 3, segment: 1, index: 2, left: [.candidate(4), .literal("ヒンシ")],
            timeout: .seconds(1))
        #expect(page.segment == 1 && page.pageIndex == 2 && page.hasNext)
        #expect(page.candidates == [CandidateItem(surface: "知った", kind: .literal)])
        guard case .segmentPage(let sent)? = harness.latest.requests[0].body else {
            Issue.record("segment_page was not sent")
            return
        }
        #expect(sent.conversionID == 9 && sent.layoutRev == 3 && sent.segment == 1 && sent.index == 2)
        #expect(sent.left.map(\.candidateIndex) == [4, 0])
        #expect(sent.left.map(\.literal) == ["", "ヒンシ"])
    }

    @Test func resizeSendsASignedDeltaAndReturnsTheNewLayout() throws {
        let harness = Harness()
        harness.respond = { _ in layoutEnvelope(segments: [("りぽじとり", "リポジトリ"), ("めい", "名")], focused: 1, rev: 1) }
        let layout = try harness.makeClient().resize(
            conversionID: 9, layoutRev: 0, segment: 0, delta: -2, left: [], timeout: .seconds(1))
        #expect(layout.layoutRev == 1 && layout.focused == 1 && layout.conversionID == 9)
        #expect(
            layout.segments == [
                SegmentSummary(reading: "りぽじとり", surface: "リポジトリ"), SegmentSummary(reading: "めい", surface: "名"),
            ])
        #expect(layout.focusedPage?.candidates.map(\.surface) == ["名", "別"])
        guard case .resizeSegment(let sent)? = harness.latest.requests[0].body else {
            Issue.record("resize_segment was not sent")
            return
        }
        #expect(sent.delta == -2 && sent.segment == 0 && sent.layoutRev == 0)
    }

    @Test func staleLayoutSurfacesAsItsOwnError() {
        let harness = Harness()
        harness.respond = { _ in
            Harness.envelope {
                var failure = Ikari_Engine_V1_ErrorReply()
                failure.code = .staleLayout
                $0.error = failure
            }
        }
        #expect(throws: EngineClientError.engine(.staleLayout)) {
            try harness.makeClient().resize(
                conversionID: 9, layoutRev: 0, segment: 0, delta: 1, left: [], timeout: .seconds(1))
        }
    }

    @Test func commitSegmentsIsANotificationWithOneChoicePerSegment() {
        let harness = Harness()
        harness.respond = { _ in nil }
        let client = harness.makeClient()
        client.commitSegments(conversionID: 9, layoutRev: 2, choices: [.candidate(0), .candidate(3), .literal("ヒ")])
        #expect(harness.latest.requests.isEmpty)
        guard case .commitSegments(let sent)? = harness.latest.posts.first?.body else {
            Issue.record("commit_segments was not posted")
            return
        }
        #expect(sent.conversionID == 9 && sent.layoutRev == 2)
        #expect(sent.choices.map(\.candidateIndex) == [0, 3, 0])
        #expect(sent.choices.map(\.literal) == ["", "", "ヒ"])
    }

    @Test func reconvertSendsTheSelectionAndLeftContextAndReturnsTheLayout() throws {
        let harness = Harness()
        harness.respond = { _ in layoutEnvelope(segments: [("ひんし", "品詞")]) }
        let layout = try harness.makeClient().reconvert(
            ReconvertRequest(
                sessionID: 5, surface: "品詞", context: SurroundingText(left: "辞書に", right: "を", available: true),
                pageSize: 5),
            timeout: .seconds(1))
        #expect(layout.segments.map(\.surface) == ["品詞"])
        guard case .reconvert(let sent)? = harness.latest.requests[0].body else {
            Issue.record("reconvert was not sent")
            return
        }
        #expect(sent.sessionID == 5 && sent.surface == "品詞" && sent.pageSize == 5)
        #expect(sent.context.left == "辞書に" && sent.context.available)
    }

    @Test func requestsOverTheLimitsAreRefusedBeforeSending() {
        let harness = Harness()
        let client = harness.makeClient()
        let tooLong = String(repeating: "あ", count: 65)
        #expect(throws: EngineClientError.engine(.badRequest)) {
            try client.reconvert(ReconvertRequest(sessionID: 1, surface: tooLong, context: .unavailable), timeout: .seconds(1))
        }
        #expect(throws: EngineClientError.engine(.badRequest)) {
            try client.reconvert(ReconvertRequest(sessionID: 1, surface: "", context: .unavailable), timeout: .seconds(1))
        }
        #expect(throws: EngineClientError.engine(.badRequest)) {
            try client.segmentPage(
                conversionID: 1, layoutRev: 0, segment: 0, index: 0,
                left: [.literal(String(repeating: "あ", count: 257))], timeout: .seconds(1))
        }
        #expect(throws: EngineClientError.engine(.badRequest)) {
            try client.resize(conversionID: 1, layoutRev: 0, segment: 0, delta: 0, left: [], timeout: .seconds(1))
        }
        client.commitSegments(conversionID: 1, layoutRev: 0, choices: [])
        #expect(harness.transports.isEmpty)
    }

    @Test func aBrokenLayoutIsAnInvalidReply() {
        let harness = Harness()
        let client = harness.makeClient()
        // 文節がない
        harness.respond = { _ in Harness.envelope { $0.segmentLayout = Ikari_Engine_V1_SegmentLayout() } }
        #expect(throws: EngineClientError.invalidReply) {
            try client.resize(conversionID: 1, layoutRev: 0, segment: 0, delta: 1, left: [], timeout: .seconds(1))
        }
        // フォーカスが範囲外
        harness.respond = { _ in
            var env = layoutEnvelope(segments: [("あ", "亜")])
            env.segmentLayout.focused = 3
            return env
        }
        #expect(throws: EngineClientError.invalidReply) {
            try client.resize(conversionID: 1, layoutRev: 0, segment: 0, delta: 1, left: [], timeout: .seconds(1))
        }
        // 違う種類の返事
        harness.respond = { _ in Harness.envelope { $0.ack = Ikari_Engine_V1_Ack() } }
        #expect(throws: EngineClientError.invalidReply) {
            try client.segmentPage(conversionID: 1, layoutRev: 0, segment: 0, index: 0, left: [], timeout: .seconds(1))
        }
    }

    @Test func convertReplyCarriesTheLayoutAndAnOldEngineHasNone() throws {
        let harness = Harness()
        harness.respond = { _ in
            Harness.envelope { env in
                var reply = Ikari_Engine_V1_ConvertReply()
                reply.conversionID = 9
                var c = Ikari_Engine_V1_Candidate()
                c.surface = "初めて知った"
                reply.candidates = [c]
                reply.layout = layoutEnvelope(segments: [("はじめて", "初めて"), ("しった", "知った")]).segmentLayout
                env.convertReply = reply
            }
        }
        let page = try harness.makeClient().convert(sampleRequest, timeout: .seconds(1))
        #expect(page.layout?.segments.map(\.reading) == ["はじめて", "しった"])
        #expect(page.layout?.focused == 0)

        let plain = try Harness().makeClient().convert(sampleRequest, timeout: .seconds(1))
        #expect(plain.layout == nil)
    }
}

@Suite struct BuildSigningTests {
    @Test func unknownOrMissingValuesFallBackToTheStrictMode() {
        #expect(BuildSigning.mode(from: nil) == .team)
        #expect(BuildSigning.mode(from: "bogus") == .team)
        #expect(BuildSigning.mode(from: "adhoc") == .adhoc)
        #expect(BuildSigning.mode(from: "team") == .team)
    }
}
