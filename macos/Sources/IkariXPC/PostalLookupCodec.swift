// 英語のモードの郵便番号の問い合わせ（ADR 0032 §9.1）の返事の検証。
import Foundation
import IkariProto
import IkariShellCore

extension EnvelopeCodec {
    /// 返事の検証の上限。1 つの郵便番号に結びつく町字は多くても数十件、住所は都道府県から町字までで数十文字。
    /// 余裕を持たせ、壊れた返事だけを弾く
    static let maxPostalAddresses = 256
    static let maxPostalAddressCharacters = 256

    /// `postal_lookup_reply` を検証して住所の配列にする。件数と、住所ごとの空でないこと・長さを確かめ、外れたら信用しない
    static func postalLookup(_ env: Envelope) throws -> [String] {
        try throwIfError(env)
        guard case .postalLookupReply(let reply)? = env.body else { throw EngineClientError.invalidReply }
        guard reply.addresses.count <= maxPostalAddresses else { throw EngineClientError.invalidReply }
        for address in reply.addresses {
            guard !address.isEmpty, address.count <= maxPostalAddressCharacters else {
                throw EngineClientError.invalidReply
            }
        }
        return reply.addresses
    }
}
