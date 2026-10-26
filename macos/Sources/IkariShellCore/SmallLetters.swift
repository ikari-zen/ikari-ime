// 上付き・下付きの小さい文字の候補（`:m2:` → m²、`:m_2:` → m₂。Mozc の small_letter_rewriter 相当。#264）。
// 純粋。ショートコードの名前（`a-z 0-9 _ + -`）から、閉じの `:` のときに足す候補を作る。絵文字の表に同じ名前があれば、そちらが先
import Foundation

enum SmallLetters {
    /// 名前に対する候補（「そのままの入力」は含まない）。上付き・下付きの文字列、単位の 1 文字の記号の順。なければ空
    static func candidates(for name: String) -> [String] {
        var result: [String] = []
        if let scripted = scripted(name) { result.append(scripted) }
        if let unit = units[name], !result.contains(unit) { result.append(unit) }
        return result
    }

    /// `<英字>?<数字>+` は英字 + 上付きの数字（m2 → m²、x10 → x¹⁰）。`<英字>?_<文字>+` は英字 + 下付き（m_2 → m₂、x_n → xₙ）。
    /// 英字だけ・数字だけの名前の先頭の英字は、そのまま残す。小さい文字にできない文字を含むときは nil
    private static func scripted(_ name: String) -> String? {
        let characters = Array(name)
        let baseEnd = characters.firstIndex { !$0.isLetter } ?? characters.count
        let base = String(characters[..<baseEnd])
        var tail = Array(characters[baseEnd...])
        guard !tail.isEmpty else { return nil }
        let table: [Character: Character]
        if tail.first == "_" {
            tail.removeFirst()
            guard !tail.isEmpty else { return nil }
            table = subscripts
        } else {
            guard tail.allSatisfy(\.isNumber) else { return nil }
            table = superscripts
        }
        var converted = ""
        for character in tail {
            guard let small = table[character] else { return nil }
            converted.append(small)
        }
        return base + converted
    }

    private static let superscripts: [Character: Character] = [
        "0": "⁰", "1": "¹", "2": "²", "3": "³", "4": "⁴", "5": "⁵", "6": "⁶", "7": "⁷", "8": "⁸", "9": "⁹",
    ]

    private static let subscripts: [Character: Character] = [
        "0": "₀", "1": "₁", "2": "₂", "3": "₃", "4": "₄", "5": "₅", "6": "₆", "7": "₇", "8": "₈", "9": "₉",
        "+": "₊", "-": "₋", "a": "ₐ", "e": "ₑ", "h": "ₕ", "i": "ᵢ", "j": "ⱼ", "k": "ₖ", "l": "ₗ", "m": "ₘ",
        "n": "ₙ", "o": "ₒ", "p": "ₚ", "r": "ᵣ", "s": "ₛ", "t": "ₜ", "u": "ᵤ", "v": "ᵥ", "x": "ₓ",
    ]

    /// 単位の 1 文字の記号（Unicode の互換用。CJK Compatibility の U+33xx）
    private static let units: [String: String] = [
        "mm": "㎜", "cm": "㎝", "km": "㎞", "mm2": "㎟", "cm2": "㎠", "m2": "㎡", "km2": "㎢",
        "mm3": "㎣", "cm3": "㎤", "m3": "㎥", "km3": "㎦",
        "mg": "㎎", "kg": "㎏", "ml": "㎖", "dl": "㎗", "kl": "㎘",
        "hz": "㎐", "khz": "㎑", "mhz": "㎒", "ghz": "㎓", "thz": "㎔",
        "pa": "㎩", "kpa": "㎪", "mpa": "㎫", "gpa": "㎬",
        "ps": "㎰", "ns": "㎱", "ms": "㎳", "pf": "㎊", "nf": "㎋",
        "kb": "㎅", "mb": "㎆", "gb": "㎇", "cal": "㎈", "kcal": "㎉", "db": "㏈", "ha": "㏊",
    ]
}
