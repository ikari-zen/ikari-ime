// 郵便番号の問い合わせの返事の検証と、`XPCEngineClient.postalLookup`（ADR 0032 §9.1）。
import Foundation
import IkariProto
import IkariShellCore
import Testing

@testable import IkariXPC

private func reply(_ addresses: [String]) -> Envelope {
    var message = Ikari_Engine_V1_PostalLookupReply()
    message.addresses = addresses
    var env = Envelope()
    env.schemaVersion = 1
    env.postalLookupReply = message
    return env
}

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

@Suite struct PostalLookupCodecTests {
    @Test func decodesAddressesInOrder() throws {
        let list = ["東京都千代田区千代田", "東京都千代田区丸の内"]
        #expect(try EnvelopeCodec.postalLookup(reply(list)) == list)
    }

    @Test func anEmptyListIsFine() throws {
        #expect(try EnvelopeCodec.postalLookup(reply([])).isEmpty)
    }

    @Test func rejectsEmptyOrTooLongAddresses() {
        let bad: [[String]] = [
            [""],
            ["東京都", ""],
            [String(repeating: "あ", count: EnvelopeCodec.maxPostalAddressCharacters + 1)],
        ]
        for list in bad {
            #expect(throws: EngineClientError.invalidReply) { try EnvelopeCodec.postalLookup(reply(list)) }
        }
    }

    @Test func rejectsTooManyAddresses() {
        let list = Array(repeating: "東京都", count: EnvelopeCodec.maxPostalAddresses + 1)
        #expect(throws: EngineClientError.invalidReply) { try EnvelopeCodec.postalLookup(reply(list)) }
    }

    @Test func acceptsTheLimits() throws {
        let list = Array(
            repeating: String(repeating: "あ", count: EnvelopeCodec.maxPostalAddressCharacters),
            count: EnvelopeCodec.maxPostalAddresses)
        #expect(try EnvelopeCodec.postalLookup(reply(list)).count == EnvelopeCodec.maxPostalAddresses)
    }

    @Test func rejectsTheWrongReplyAndMapsErrors() {
        var ack = Envelope()
        ack.ack = Ikari_Engine_V1_Ack()
        #expect(throws: EngineClientError.invalidReply) { try EnvelopeCodec.postalLookup(ack) }

        var failure = Envelope()
        var error = Ikari_Engine_V1_ErrorReply()
        error.code = .badRequest
        failure.error = error
        #expect(throws: EngineClientError.engine(.badRequest)) { try EnvelopeCodec.postalLookup(failure) }
    }

    @Test func clientSendsTheCodeAndDecodesTheReply() throws {
        let stub = StubTransport { request in
            guard case .postalLookup(let r)? = request.body, r.code == "1000001" else { return nil }
            return reply(["東京都千代田区千代田"])
        }
        let client = XPCEngineClient(transportFactory: { _ in stub }, now: { Date() })
        #expect(try client.postalLookup(code: "1000001", timeout: .seconds(1)) == ["東京都千代田区千代田"])
        #expect(stub.requests.count == 1)
        #expect(stub.requests[0].schemaVersion == 1)
    }

    @Test func clientFailsWithoutBlockingWhenTheEngineDoesNotAnswer() {
        let stub = StubTransport { _ in nil }
        let client = XPCEngineClient(transportFactory: { _ in stub }, now: { Date() })
        #expect(throws: EngineClientError.timeout) {
            try client.postalLookup(code: "1000001", timeout: .milliseconds(1))
        }
    }
}
