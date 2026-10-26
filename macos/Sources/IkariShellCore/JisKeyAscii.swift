// かな入力（JIS）のキー → ASCII の表（ADR 0027 §3.1、§9.4）。契約（B47 の契約コミット）。中身はレーン S。
import Foundation

/// かな入力の `.jis(UInt8)`（`IkariJisKey`）を、ショートコードの名前を読むために刻印の英字・数字・記号に直す。
/// 名前の文字は `a-z 0-9 _ + -`（§3.3）。Shift で `_`、`+`。ここに無いキーは nil
public enum JisKeyAscii {
    /// けのキー（`:`）。入りと閉じのキー（`KeyCodes` の `jisKeys` の 16）
    public static let colonKey: UInt8 = 16

    /// 刻印の文字。`shifted` は Shift が付いているか。対応がなければ nil
    public static func character(forJisKey key: UInt8, shifted: Bool) -> Character? {
        switch key {
        case digitKeys:
            // 数字の段は Shift で別のかな（ぬふあ…）になる。名前の文字ではない
            return shifted ? nil : Character(String((key + 1) % 10))
        case hyphenKey:
            return shifted ? nil : "-"
        case plusKey:
            return shifted ? "+" : nil
        case underscoreKey:
            return shifted ? "_" : nil
        case colonKey:
            return ":"
        case letterKeys:
            // Shift 付きの英字は大文字（名前の文字ではない）なので対応しない
            guard !shifted else { return nil }
            let scalar = Unicode.Scalar(UInt8(ascii: "a") + (key - letterKeys.lowerBound))
            return Character(scalar)
        default:
            return nil
        }
    }

    /// 数字の段（`1`〜`9`、`0`）。`IkariJisKey` の 0〜9
    private static let digitKeys: ClosedRange<UInt8> = 0...9
    /// `-` のキー（11 番の `^` ではなく 10 番）
    private static let hyphenKey: UInt8 = 10
    /// `;` のキー。Shift で `+`
    private static let plusKey: UInt8 = 15
    /// `ろ` のキー。Shift で `_`
    private static let underscoreKey: UInt8 = 21
    /// A〜Z（`IkariJisKey` の 22 から アルファベット順）
    private static let letterKeys: ClosedRange<UInt8> = 22...47
}
