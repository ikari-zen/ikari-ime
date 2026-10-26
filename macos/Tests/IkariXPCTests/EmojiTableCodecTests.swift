// 絵文字の表の返事の検証と、`XPCEngineClient.emojiTable`（ADR 0027 §9.2、§9.4）。
import Foundation
import IkariProto
import IkariShellCore
import Testing

@testable import IkariXPC

private func reply(_ pairs: [(String, String)], label: String = "label-1") -> Envelope {
    var message = Ikari_Engine_V1_EmojiTableReply()
    message.entries = pairs.map { name, emoji in
        var entry = Ikari_Engine_V1_EmojiEntry()
        entry.name = name
        entry.emoji = emoji
        return entry
    }
    message.dataLabel = label
    var env = Envelope()
    env.schemaVersion = 1
    env.emojiTableReply = message
    return env
}

/// 1 回の要求に決まった返事を返す、最小のトランスポート
private final class StubTransport: EngineTransport, @unchecked Sendable {
    private let lock = NSLock()
    private var received: [Envelope] = []
    private let respond: @Sendable (Envelope) -> Envelope?
    let processIdentifier: Int32 = 1

    init(respond: @escaping @Sendable (Envelope) -> Envelope?) { self.respond = respond }

    var requests: [Envelope] { lock.withLock { received } }

    func request(_ data: Data, timeout: Duration) throws -> Data {
        let env = try Envelope(serializedBytes: data)
        lock.withLock { received.append(env) }
        guard var answer = respond(env) else { throw EngineClientError.timeout }
        answer.requestID = env.requestID
        return try answer.serializedData()
    }

    func post(_ data: Data) {}
    func close() {}
}

@Suite struct EmojiTableCodecTests {
    @Test func decodesEntriesAndTheLabel() throws {
        let info = try EnvelopeCodec.emojiTable(reply([("+1", "👍"), ("eyes", "👀"), ("tada", "🎉")]))
        #expect(info.entries == [
            ShortcodeEntry(name: "+1", emoji: "👍"), ShortcodeEntry(name: "eyes", emoji: "👀"),
            ShortcodeEntry(name: "tada", emoji: "🎉"),
        ])
        #expect(info.dataLabel == "label-1")
    }

    @Test func anEmptyTableIsFine() throws {
        // データがないとき、エンジンは空の表を返す
        let info = try EnvelopeCodec.emojiTable(reply([], label: ""))
        #expect(info == .empty)
    }

    @Test func sequencesWithManyScalarsAreAccepted() throws {
        let family = "👨‍👩‍👧‍👦"
        let info = try EnvelopeCodec.emojiTable(reply([("family", family)]))
        #expect(info.entries.first?.emoji == family)
    }

    @Test func rejectsEntriesThatBreakTheContract() {
        // 昇順でない、重複、空の名前や絵文字、長すぎる名前
        let bad: [[(String, String)]] = [
            [("b", "🅱"), ("a", "🅰")],
            [("a", "🅰"), ("a", "🅰")],
            [("", "🅰")],
            [("a", "")],
            [(String(repeating: "a", count: EnvelopeCodec.maxEmojiNameCharacters + 1), "🅰")],
        ]
        for pairs in bad {
            #expect(throws: EngineClientError.invalidReply) { try EnvelopeCodec.emojiTable(reply(pairs)) }
        }
    }

    @Test func rejectsTooManyEntries() {
        let pairs = (0...EnvelopeCodec.maxEmojiEntries).map { (String(format: "n%05d", $0), "🅰") }
        #expect(throws: EngineClientError.invalidReply) { try EnvelopeCodec.emojiTable(reply(pairs)) }
    }

    @Test func rejectsTheWrongReplyAndMapsErrors() {
        var ack = Envelope()
        ack.ack = Ikari_Engine_V1_Ack()
        #expect(throws: EngineClientError.invalidReply) { try EnvelopeCodec.emojiTable(ack) }

        var failure = Envelope()
        var error = Ikari_Engine_V1_ErrorReply()
        error.code = .badRequest
        failure.error = error
        #expect(throws: EngineClientError.engine(.badRequest)) { try EnvelopeCodec.emojiTable(failure) }
    }

    @Test func clientAsksForTheTableAndDecodesTheReply() throws {
        let stub = StubTransport { request in
            guard case .emojiTable? = request.body else { return nil }
            return reply([("eyes", "👀")], label: "v7")
        }
        let client = XPCEngineClient(transportFactory: { _ in stub }, now: { Date() })
        let info = try client.emojiTable(timeout: .seconds(1))
        #expect(info.entries == [ShortcodeEntry(name: "eyes", emoji: "👀")])
        #expect(info.dataLabel == "v7")
        #expect(stub.requests.count == 1)
        #expect(stub.requests[0].schemaVersion == 1)
    }

    @Test func clientFailsWithoutBlockingWhenTheEngineDoesNotAnswer() {
        let stub = StubTransport { _ in nil }
        let client = XPCEngineClient(transportFactory: { _ in stub }, now: { Date() })
        #expect(throws: EngineClientError.timeout) { try client.emojiTable(timeout: .milliseconds(1)) }
    }
}
