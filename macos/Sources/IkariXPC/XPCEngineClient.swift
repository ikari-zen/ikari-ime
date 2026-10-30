// エンジンの XPC の型付きの口（ADR 0004 §5.1、§7.5）。**今は smoke（IkariSmoke）だけが使う。**
// IME の殻は shell-core（Rust）がエンジンと話し、Swift は輸送（`XPCEngineConnections`）だけを持つ（ADR 0041）。
// 接続、署名要件、interruption / invalidation の処理、再接続、タイムアウト、
// 60秒に3回落ちたら5分休む、を持つ。メッセージの運搬は EngineTransport に任せる。
// 殻だけが使っていた口（絵文字の表、郵便番号、診断ログ、分野の管理）は M5 で消した。
import Foundation
import IkariProto

public final class XPCEngineClient: @unchecked Sendable {
    /// 落ちた回数がこの数に達したら休む（ADR 0004 §7.5）
    static let crashLimit = 3
    static let crashWindow: TimeInterval = 60
    static let suspension: TimeInterval = 300
    /// 版の不一致のときの Shutdown の返事と、終了の確認を待つ上限
    static let shutdownWait: Duration = .seconds(1)
    /// 中継する要求と返事の上限（エンジンの `MAX_ADMIN_REQUEST_BYTES` と枠の上限と同じ。docs/phase2-contract.md §5）
    static let maxRelayBytes = 64 * 1024 * 1024

    private let makeTransport: EngineTransportFactory
    private let now: @Sendable () -> Date

    /// `condition` が以下のすべての状態を守る。待つ間はロックを持たない
    private let condition = NSCondition()
    private var transport: EngineTransport?
    private var generation = 0
    private var nextRequestID: UInt64 = 1
    private var sessions: [UInt64: SessionInfo] = [:]
    private var shellVersion: String?
    private var needsResync = false
    private var expectingExit = false
    private var downEvents = 0
    private var crashDates: [Date] = []
    private var suspendedUntil: Date?
    private var observedEngineVersion: String?
    /// 同梱の XPC サービスにつなぐ。`signing` で署名要件（相手 = エンジン）が決まる。
    public convenience init(serviceName: String = IkariIdentifiers.engine, signing: SigningMode) {
        let requirement = SigningRequirements.engine(mode: signing)
        self.init(
            transportFactory: { onDown in
                XPCTransport(serviceName: serviceName, requirement: requirement, onDown: onDown)
            },
            now: { Date() }
        )
    }

    init(transportFactory: @escaping EngineTransportFactory, now: @escaping @Sendable () -> Date) {
        self.makeTransport = transportFactory
        self.now = now
    }

    // MARK: 診断（smoke とテストが使う）

    /// 今つながっているエンジンのプロセス ID（つながっていなければ 0）
    public var enginePID: Int32 {
        condition.lock()
        defer { condition.unlock() }
        return transport?.processIdentifier ?? 0
    }

    /// 直近の `Hello` の返事に入っていたエンジンの版
    public var lastEngineVersion: String? {
        condition.lock()
        defer { condition.unlock() }
        return observedEngineVersion
    }

    /// 連続して落ちたため、エンジンを呼ばずに休んでいるか
    public var isSuspended: Bool {
        condition.lock()
        defer { condition.unlock() }
        if let until = suspendedUntil { return now() < until }
        return false
    }

    // MARK: 要求と通知

    public func hello(shellVersion: String, timeout: Duration) throws -> EngineHello {
        condition.lock()
        self.shellVersion = shellVersion
        condition.unlock()

        var restarted = false
        while true {
            let env = try roundTrip(timeout: timeout) { env in
                var hello = Ikari_Engine_V1_Hello()
                hello.shellVersion = shellVersion
                env.hello = hello
            }
            let reply = try EnvelopeCodec.hello(env)
            condition.lock()
            observedEngineVersion = reply.engineVersion
            condition.unlock()
            if reply.engineVersion == shellVersion { return reply }
            // 入れ替えた直後に古いエンジンが残っている。止めて、新しい版を起動し直す（ADR 0004 §5.2）
            guard !restarted else { throw EngineClientError.unavailable }
            restarted = true
            shutdownAndWaitForExit()
        }
    }

    public func beginSession(_ session: SessionInfo) {
        condition.lock()
        sessions[session.sessionID] = session
        condition.unlock()
        post { env in env.beginSession = Self.beginMessage(session) }
    }

    public func endSession(sessionID: UInt64) {
        condition.lock()
        sessions[sessionID] = nil
        condition.unlock()
        post { env in
            var msg = Ikari_Engine_V1_EndSession()
            msg.sessionID = sessionID
            env.endSession = msg
        }
    }

    public func prefetch(_ request: ConversionRequest) {
        guard Self.isValid(request) else { return }
        post { env in
            var msg = Ikari_Engine_V1_Prefetch()
            msg.sessionID = request.sessionID
            msg.reading = request.reading
            msg.context = EnvelopeCodec.surrounding(Self.clipped(request.context))
            msg.contextRev = request.contextRev
            env.prefetch = msg
        }
    }

    public func convert(_ request: ConversionRequest, timeout: Duration) throws -> ConversionPage {
        guard Self.isValid(request) else { throw EngineClientError.engine(.badRequest) }
        let env = try roundTrip(timeout: timeout) { env in
            var msg = Ikari_Engine_V1_Convert()
            msg.sessionID = request.sessionID
            msg.reading = request.reading
            msg.context = EnvelopeCodec.surrounding(Self.clipped(request.context))
            msg.contextRev = request.contextRev
            msg.pageSize = UInt32(request.pageSize)
            env.convert = msg
        }
        return try EnvelopeCodec.page(env, pageReply: false)
    }

    public func page(conversionID: UInt64, index: Int, timeout: Duration) throws -> ConversionPage {
        guard index >= 0, index <= Int(Int32.max) else { throw EngineClientError.engine(.badRequest) }
        let env = try roundTrip(timeout: timeout) { env in
            var msg = Ikari_Engine_V1_PageRequest()
            msg.conversionID = conversionID
            msg.index = UInt32(index)
            env.page = msg
        }
        return try EnvelopeCodec.page(env, pageReply: true)
    }

    // 文節と再変換（ADR 0006 §5.2）。失敗（古いエンジンの BAD_REQUEST を含む）は呼び出す側が「何もしない」にする

    public func segmentPage(
        conversionID: UInt64, layoutRev: UInt32, segment: Int, index: Int, left: [SegmentChoiceValue], timeout: Duration
    ) throws -> SegmentPage {
        guard (0..<EnvelopeCodec.maxSegments).contains(segment), (0...Int(Int32.max)).contains(index),
            EnvelopeCodec.isValid(left)
        else { throw EngineClientError.engine(.badRequest) }
        let env = try roundTrip(timeout: timeout) { env in
            var msg = Ikari_Engine_V1_SegmentPageRequest()
            msg.conversionID = conversionID
            msg.layoutRev = layoutRev
            msg.segment = UInt32(segment)
            msg.index = UInt32(index)
            msg.left = left.map(EnvelopeCodec.choice)
            env.segmentPage = msg
        }
        return try EnvelopeCodec.segmentPageReply(env)
    }

    public func resize(
        conversionID: UInt64, layoutRev: UInt32, segment: Int, delta: Int, left: [SegmentChoiceValue], timeout: Duration
    ) throws -> SegmentLayoutInfo {
        guard (0..<EnvelopeCodec.maxSegments).contains(segment), delta != 0, abs(delta) <= EnvelopeCodec.maxTextCharacters,
            EnvelopeCodec.isValid(left)
        else { throw EngineClientError.engine(.badRequest) }
        let env = try roundTrip(timeout: timeout) { env in
            var msg = Ikari_Engine_V1_ResizeSegment()
            msg.conversionID = conversionID
            msg.layoutRev = layoutRev
            msg.segment = UInt32(segment)
            msg.delta = Int32(delta)
            msg.left = left.map(EnvelopeCodec.choice)
            env.resizeSegment = msg
        }
        return try EnvelopeCodec.segmentLayoutReply(env)
    }

    public func commitSegments(conversionID: UInt64, layoutRev: UInt32, choices: [SegmentChoiceValue]) {
        guard !choices.isEmpty, EnvelopeCodec.isValid(choices) else { return }
        post { env in
            var msg = Ikari_Engine_V1_CommitSegments()
            msg.conversionID = conversionID
            msg.layoutRev = layoutRev
            msg.choices = choices.map(EnvelopeCodec.choice)
            env.commitSegments = msg
        }
    }

    public func reconvert(_ request: ReconvertRequest, timeout: Duration) throws -> SegmentLayoutInfo {
        guard !request.surface.isEmpty, request.surface.count <= Self.maxReconvertCharacters,
            (1...9).contains(request.pageSize)
        else { throw EngineClientError.engine(.badRequest) }
        let env = try roundTrip(timeout: timeout) { env in
            var msg = Ikari_Engine_V1_Reconvert()
            msg.sessionID = request.sessionID
            msg.surface = request.surface
            msg.context = EnvelopeCodec.surrounding(Self.clipped(request.context))
            msg.pageSize = UInt32(request.pageSize)
            env.reconvert = msg
        }
        return try EnvelopeCodec.segmentLayoutReply(env)
    }

    public func commit(conversionID: UInt64, candidateIndex: Int) {
        guard candidateIndex >= 0, candidateIndex <= Int(Int32.max) else { return }
        post { env in
            var msg = Ikari_Engine_V1_Commit()
            msg.conversionID = conversionID
            msg.candidateIndex = UInt32(candidateIndex)
            env.commit = msg
        }
    }

    public func commitText(sessionID: UInt64, reading: String, surface: String) {
        post { env in
            var msg = Ikari_Engine_V1_CommitText()
            msg.sessionID = sessionID
            msg.reading = reading
            msg.surface = surface
            env.commitText = msg
        }
    }

    public func cancel(conversionID: UInt64) {
        post { env in
            var msg = Ikari_Engine_V1_Cancel()
            msg.conversionID = conversionID
            env.cancel = msg
        }
    }

    /// エンジンに保存して終了するよう頼む（返事は最大 1 秒待つ）。終了は異常として数えない。
    public func shutdown(reason: EngineShutdownReason) {
        condition.lock()
        expectingExit = true
        condition.unlock()
        let result = try? roundTrip(timeout: Self.shutdownWait) { $0.shutdown = Self.shutdownMessage(reason) }
        if result == nil {
            condition.lock()
            expectingExit = false
            condition.unlock()
        }
    }

    /// 管理の要求の中継（ADR 0005 §8.3）。`Envelope` のバイト列の `request_id` と `schema_version` を付け直して送り、
    /// 返事のバイト列をそのまま返す。中身は解釈しない（許可リストでの断りは呼び出す側。`IkariBridge.RelayPolicy`）。
    /// 返事が `ErrorReply` でも、投げずにそのまま返す（呼び出す側が種類を見る）。
    public func relay(_ request: Data, timeout: Duration) throws -> Data {
        guard !request.isEmpty, request.count <= Self.maxRelayBytes,
            var envelope = try? Envelope(serializedBytes: request)
        else { throw EngineClientError.engine(.badRequest) }
        let (transport, requestID) = try prepare()
        envelope.schemaVersion = EnvelopeCodec.schemaVersion
        envelope.requestID = requestID
        guard let data = try? envelope.serializedData() else { throw EngineClientError.engine(.badRequest) }
        let reply = try transport.request(data, timeout: timeout)
        guard !reply.isEmpty, reply.count <= Self.maxRelayBytes else { throw EngineClientError.invalidReply }
        return reply
    }

    // MARK: 内部

    private static func shutdownMessage(_ reason: EngineShutdownReason) -> Ikari_Engine_V1_Shutdown {
        var message = Ikari_Engine_V1_Shutdown()
        switch reason {
        case .versionMismatch: message.reason = .versionMismatch
        case .settingsChanged: message.reason = .settingsChanged
        }
        return message
    }

    private static func beginMessage(_ session: SessionInfo) -> Ikari_Engine_V1_BeginSession {
        var msg = Ikari_Engine_V1_BeginSession()
        msg.sessionID = session.sessionID
        msg.clientBundleID = session.clientBundleID
        msg.secure = session.secure
        msg.shareContext = session.shareContext
        return msg
    }

    private static let maxReadingCharacters = 256
    /// 再変換の対象の文字数の上限（ADR 0006 §9.2）
    static let maxReconvertCharacters = 64

    private static func isValid(_ request: ConversionRequest) -> Bool {
        !request.reading.isEmpty
            && request.reading.count <= maxReadingCharacters
            && (1...9).contains(request.pageSize)
    }

    /// 左文脈は末尾 2000 文字に切る（殻の責任だが、ここでも守る）
    private static func clipped(_ text: SurroundingText) -> SurroundingText {
        guard text.left.count > SurroundingText.maxCharacters else { return text }
        var out = text
        out.left = String(text.left.suffix(SurroundingText.maxCharacters))
        return out
    }

    /// 接続を用意して（必要なら再接続と再同期をして）、トランスポートと要求の ID を返す。
    private func prepare() throws -> (EngineTransport, UInt64) {
        condition.lock()
        defer { condition.unlock() }
        if let until = suspendedUntil {
            guard now() >= until else { throw EngineClientError.unavailable }
            suspendedUntil = nil
            crashDates.removeAll()
        }
        let current: EngineTransport
        if let existing = transport {
            current = existing
        } else {
            generation += 1
            let mine = generation
            current = makeTransport { [weak self] down in self?.handleDown(down, generation: mine) }
            transport = current
        }
        if needsResync {
            needsResync = false
            // 落ちたあとに起動し直したエンジンには、セッションの記憶がない。返事は待たずに送り直す
            if let version = shellVersion {
                var hello = Ikari_Engine_V1_Hello()
                hello.shellVersion = version
                current.post(encoded(requestID: takeRequestID()) { $0.hello = hello })
            }
            for session in sessions.values {
                current.post(encoded(requestID: takeRequestID()) { $0.beginSession = Self.beginMessage(session) })
            }
        }
        return (current, takeRequestID())
    }

    /// `condition` を持っているときに呼ぶ
    private func takeRequestID() -> UInt64 {
        defer { nextRequestID &+= 1 }
        return nextRequestID
    }

    private func encoded(requestID: UInt64, _ build: (inout Envelope) -> Void) -> Data {
        EnvelopeCodec.encode(EnvelopeCodec.envelope(requestID: requestID, build))
    }

    private func roundTrip(timeout: Duration, _ build: (inout Envelope) -> Void) throws -> Envelope {
        var envelope = try roundTripOnce(timeout: timeout, build)
        if case .error(let failure)? = envelope.body, failure.code == .unknownSession {
            // 後から開いたエンジン（ロックが空くのを待っていた、など）はセッションを知らない。
            // hello とセッションを送り直して、同じ要求を1回だけ送り直す
            condition.lock()
            needsResync = true
            condition.unlock()
            envelope = try roundTripOnce(timeout: timeout, build)
        }
        return envelope
    }

    private func roundTripOnce(timeout: Duration, _ build: (inout Envelope) -> Void) throws -> Envelope {
        let (transport, requestID) = try prepare()
        let data = encoded(requestID: requestID, build)
        let reply = try transport.request(data, timeout: timeout)
        let envelope = try EnvelopeCodec.decode(reply, expectedRequestID: requestID)
        if case .error(let failure)? = envelope.body, failure.code == .engineNotReady {
            // エンジンがまだ開けていない（ロックを別のプロセスが持っている、など）。あとで開いたエンジンには
            // hello もセッションの記憶もないので、次の要求の前に送り直す
            condition.lock()
            needsResync = true
            condition.unlock()
        }
        return envelope
    }

    /// 返事を待たずに送る。つながらなければ捨てる（入力を止めない）。
    private func post(_ build: (inout Envelope) -> Void) {
        guard let (transport, requestID) = try? prepare() else { return }
        transport.post(encoded(requestID: requestID, build))
    }

    private func handleDown(_ down: TransportDown, generation gen: Int) {
        condition.lock()
        defer { condition.unlock() }
        guard gen == generation else { return }  // 古い接続の通知
        downEvents += 1
        needsResync = true
        if down == .invalidated {
            transport = nil
        }
        if expectingExit {
            expectingExit = false
        } else {
            recordCrash()
        }
        condition.broadcast()
    }

    /// `condition` を持っているときに呼ぶ
    private func recordCrash() {
        let current = now()
        crashDates.append(current)
        crashDates.removeAll { current.timeIntervalSince($0) > Self.crashWindow }
        if crashDates.count >= Self.crashLimit {
            suspendedUntil = current.addingTimeInterval(Self.suspension)
            crashDates.removeAll()
        }
    }

    /// 版の不一致のとき: Shutdown を送り、プロセスが終わる（接続の中断の通知）まで待つ。
    private func shutdownAndWaitForExit() {
        condition.lock()
        let before = downEvents
        expectingExit = true
        condition.unlock()

        let acknowledged =
            (try? roundTrip(timeout: Self.shutdownWait) { $0.shutdown = Self.shutdownMessage(.versionMismatch) }) != nil

        condition.lock()
        defer { condition.unlock() }
        if acknowledged {
            let limit = Date(timeIntervalSinceNow: Self.shutdownWait.timeInterval)
            while downEvents == before && condition.wait(until: limit) {}
        }
        expectingExit = false
    }
}
