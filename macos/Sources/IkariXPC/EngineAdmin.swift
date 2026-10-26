// 殻（入力メニュー）が使う、エンジンの管理の要求の型付きの口（ADR 0005 §8.3「殻の口」）。
// 今はプロファイル（分野）だけ。辞書と学習は設定アプリが `BridgeServer` の中継で直接話す。
// 送るのは `EngineClient.relay` で、待ちの上限は `EngineTimeouts.admin`。分野の名前は利用者向けの文字列だが、ログには書かない。
import Foundation
import IkariProto
import IkariShellCore

public struct EngineAdmin: Sendable {
    /// 返事の検証の上限（壊れた返事だけを弾く。同梱のプリセットは数個）
    static let maxDomains = 64
    static let maxTextCharacters = 512

    private let client: any EngineClient

    public init(client: any EngineClient) {
        self.client = client
    }

    /// 同梱のプリセットすべての、今の重み（OFF を含む）
    public func profile() throws -> [ProfileDomain] {
        try roundTrip { $0.getProfile = Ikari_Engine_V1_ProfileGet() }
    }

    /// 分野全体を置き換える（`profile.tsv` に書き、その場で組み立て直される）。OFF の分野は送らない（エンジンの規則）。
    /// 返事は変更後の全体。
    @discardableResult
    public func setProfile(_ domains: [ProfileDomain]) throws -> [ProfileDomain] {
        var message = Ikari_Engine_V1_ProfileSet()
        message.domains = domains.compactMap { domain in
            guard domain.weight != .off else { return nil }
            var state = Ikari_Engine_V1_ProfileDomainState()
            state.id = domain.id
            state.name = domain.name
            state.weight = Self.kind(domain.weight)
            return state
        }
        return try roundTrip { $0.setProfile = message }
    }

    /// 1 つの分野の重みだけを変える。最新の状態を取り直してから変えるので、メニューのキャッシュが古くても他の分野を壊さない。
    /// 知らない `id` なら `EngineClientError.engine(.badRequest)`
    @discardableResult
    public func setWeight(_ weight: ProfileWeight, forDomain id: String) throws -> [ProfileDomain] {
        var domains = try profile()
        guard let index = domains.firstIndex(where: { $0.id == id }) else {
            throw EngineClientError.engine(.badRequest)
        }
        domains[index].weight = weight
        return try setProfile(domains)
    }

    // MARK: 内部

    private func roundTrip(_ build: (inout Envelope) -> Void) throws -> [ProfileDomain] {
        // request_id は relay が付け直す
        let request = EnvelopeCodec.encode(EnvelopeCodec.envelope(requestID: 0, build))
        let reply = try client.relay(request, timeout: EngineTimeouts.admin)
        guard let envelope = try? Envelope(serializedBytes: reply) else { throw EngineClientError.invalidReply }
        return try Self.domains(from: envelope)
    }

    static func domains(from envelope: Envelope) throws -> [ProfileDomain] {
        try EnvelopeCodec.throwIfError(envelope)
        guard case .profileReply(let reply)? = envelope.body else { throw EngineClientError.invalidReply }
        guard reply.domains.count <= maxDomains else { throw EngineClientError.invalidReply }
        return try reply.domains.map { state in
            guard !state.id.isEmpty, state.id.count <= maxTextCharacters, state.name.count <= maxTextCharacters else {
                throw EngineClientError.invalidReply
            }
            return ProfileDomain(id: state.id, name: state.name, weight: weight(state.weight))
        }
    }

    private static func weight(_ kind: Ikari_Engine_V1_ProfileWeightKind) -> ProfileWeight {
        switch kind {
        case .profileWeightSometimes: .sometimes
        case .profileWeightOften: .often
        default: .off
        }
    }

    private static func kind(_ weight: ProfileWeight) -> Ikari_Engine_V1_ProfileWeightKind {
        switch weight {
        case .off: .profileWeightOff
        case .sometimes: .profileWeightSometimes
        case .often: .profileWeightOften
        }
    }
}
