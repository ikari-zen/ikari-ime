// 絵文字の表（ADR 0027 §9.2）の Envelope と EmojiTableInfo の変換。契約（B47 の契約コミット）。
// 要求の組み立ては済み。返事の検証と変換（上限の検査を含む）はレーン S が埋める。
import Foundation
import IkariProto
import IkariShellCore

extension EnvelopeCodec {
    /// 返事の検証の上限（emoji-data は約 2,000 組。余裕を持たせて壊れた返事だけを弾く）
    static let maxEmojiEntries = 8_192
    /// 名前の長さの上限（gemoji の最長は 40 文字）と、絵文字の Unicode スカラーの数の上限（ZWJ の連なりを含む）
    static let maxEmojiNameCharacters = 64
    static let maxEmojiScalars = 32

    static func emojiTableRequest(requestID: UInt64) -> Envelope {
        envelope(requestID: requestID) { $0.emojiTable = Ikari_Engine_V1_EmojiTableRequest() }
    }

    /// `emoji_table_reply` を検証して `EmojiTableInfo` にする。件数、名前と絵文字の長さ、
    /// 名前が UTF-8 のバイト順で真に昇順（二分探索の前提。重複もなし）であることを確かめ、外れたら信用しない
    static func emojiTable(_ env: Envelope) throws -> EmojiTableInfo {
        try throwIfError(env)
        guard case .emojiTableReply(let reply)? = env.body else { throw EngineClientError.invalidReply }
        guard reply.entries.count <= maxEmojiEntries, reply.dataLabel.count <= maxTextCharacters else {
            throw EngineClientError.invalidReply
        }
        var entries: [ShortcodeEntry] = []
        entries.reserveCapacity(reply.entries.count)
        var previous: String?
        for entry in reply.entries {
            guard !entry.name.isEmpty, entry.name.count <= maxEmojiNameCharacters,
                !entry.emoji.isEmpty, entry.emoji.unicodeScalars.count <= maxEmojiScalars
            else { throw EngineClientError.invalidReply }
            if let previous, !previous.utf8.lexicographicallyPrecedes(entry.name.utf8) {
                throw EngineClientError.invalidReply
            }
            previous = entry.name
            entries.append(ShortcodeEntry(name: entry.name, emoji: entry.emoji))
        }
        return EmojiTableInfo(entries: entries, dataLabel: reply.dataLabel)
    }
}
