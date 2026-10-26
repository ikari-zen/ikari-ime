// 絵文字のショートコードの表（ADR 0027 §9.1）。契約（B47 の契約コミット）。型とシグネチャだけで、中身はレーン S が埋める。
import Foundation

/// ショートコードの名前と絵文字の 1 組（`emoji.tsv` の `s` の行、`EmojiEntry` の写し）
public struct ShortcodeEntry: Sendable, Equatable {
    public var name: String
    public var emoji: String

    public init(name: String, emoji: String) {
        self.name = name
        self.emoji = emoji
    }
}

/// 名前 → 絵文字。名前の昇順（UTF-8 のバイト順。エンジンの返事と同じ）の配列を二分探索する（ADR 0027 §8.2）。純粋。
public struct ShortcodeTable: Sendable, Equatable {
    /// 名前の昇順。エンジンが昇順で返す前提で、`init` は並べ直さない
    public let entries: [ShortcodeEntry]

    /// 空の表（データがない、まだ届いていない）。`ShortcodeMachine` は入らない
    public static let empty = ShortcodeTable(entries: [])

    public init(entries: [ShortcodeEntry]) {
        self.entries = entries
    }

    public var isEmpty: Bool { entries.isEmpty }

    /// 名前が完全に一致する絵文字。名前は emoji-data の中で一意（ADR 0027 §3.5）
    public func exact(_ name: String) -> String? {
        let index = lowerBound(name)
        guard entries.indices.contains(index), entries[index].name == name else { return nil }
        return entries[index].emoji
    }

    /// 名前の前方一致（`name` と同じ名前も含む）。並びは名前の短い順、同じ長さなら名前の順（ADR 0027 §3.5）。
    /// `name` が空なら全件ではなく空（`:` だけでは候補を出さない。§3.3）
    public func prefix(_ name: String) -> [ShortcodeEntry] {
        guard !name.isEmpty else { return [] }
        var matched: [ShortcodeEntry] = []
        var index = lowerBound(name)
        while index < entries.count, entries[index].name.hasPrefix(name) {
            matched.append(entries[index])
            index += 1
        }
        // 元の並び（名前の昇順）は同じ長さの中の順になる。安定ソートではないので、長さと名前の両方で比べる
        return matched.sorted { lhs, rhs in
            let (left, right) = (lhs.name.utf8.count, rhs.name.utf8.count)
            return left != right ? left < right : Self.precedes(lhs.name, rhs.name)
        }
    }

    /// `name` 以上の最初の位置（UTF-8 のバイト順の二分探索）
    private func lowerBound(_ name: String) -> Int {
        var low = 0
        var high = entries.count
        while low < high {
            let middle = (low + high) / 2
            if Self.precedes(entries[middle].name, name) {
                low = middle + 1
            } else {
                high = middle
            }
        }
        return low
    }

    private static func precedes(_ lhs: String, _ rhs: String) -> Bool {
        lhs.utf8.lexicographicallyPrecedes(rhs.utf8)
    }
}
