// macOS の仮想キーコード（kVK_*）から、shell-core のキー（`Ikari_Shell_V1_KeyInput`）への写し（ADR 0041 §2.2 の
// `KeyCodes.swift`。OS の表だけを持ち、入力方式による読み分けは shell-core が行う）。AppKit に依存しない純関数。
import Foundation
import IkariProto

enum KeyEvents {
    static let space: UInt16 = 49
    static let returnKey: UInt16 = 36
    static let keypadEnter: UInt16 = 76
    static let escape: UInt16 = 53
    /// 後退（Backspace）
    static let delete: UInt16 = 51
    static let leftArrow: UInt16 = 123
    static let rightArrow: UInt16 = 124
    static let downArrow: UInt16 = 125
    static let upArrow: UInt16 = 126
    /// kVK_JIS_Eisu / kVK_JIS_Kana
    static let eisu: UInt16 = 102
    static let kana: UInt16 = 104

    /// `NSEvent.ModifierFlags.rawValue` の修飾キーのビット（`NSEvent.ModifierFlags` と同じ値）
    private static let shiftBit: UInt = 1 << 17
    private static let controlBit: UInt = 1 << 18
    private static let optionBit: UInt = 1 << 19
    private static let commandBit: UInt = 1 << 20

    /// `NSEvent.modifierFlags.rawValue` から、Shift / Control / Option / Command だけを取り出す。
    /// 矢印キーに付く fn と numpad、CapsLock、デバイス依存のビットは修飾キーとして扱わない
    static func modifiers(fromFlagBits bits: UInt) -> Ikari_Shell_V1_Modifiers {
        var result = Ikari_Shell_V1_Modifiers()
        result.shift = bits & shiftBit != 0
        result.control = bits & controlBit != 0
        result.option = bits & optionBit != 0
        result.command = bits & commandBit != 0
        return result
    }

    /// F1〜F20 の仮想キーコード
    private static let functionKeys: [UInt16: UInt32] = [
        122: 1, 120: 2, 99: 3, 118: 4, 96: 5, 97: 6, 98: 7, 100: 8, 101: 9, 109: 10,
        103: 11, 111: 12, 105: 13, 107: 14, 113: 15, 106: 16, 64: 17, 79: 18, 80: 19, 90: 20,
    ]

    /// 物理キーの位置 → IkariJisKey の番号。JIS 配列の位置で並べている
    /// （`^` は kVK_ANSI_Equal、`@` は kVK_ANSI_LeftBracket、`[` は kVK_ANSI_RightBracket、
    /// `:` は kVK_ANSI_Quote、`]` は kVK_ANSI_Backslash、`¥` は kVK_JIS_Yen、`ろ` は kVK_JIS_Underscore）
    private static let jisKeys: [UInt16: UInt32] = [
        18: 0, 19: 1, 20: 2, 21: 3, 23: 4, 22: 5, 26: 6, 28: 7, 25: 8, 29: 9,
        27: 10, 24: 11, 93: 12, 33: 13, 30: 14, 41: 15, 39: 16, 42: 17, 43: 18, 47: 19, 44: 20, 94: 21,
        0: 22, 11: 23, 8: 24, 2: 25, 14: 26, 3: 27, 5: 28, 4: 29, 34: 30, 38: 31, 40: 32, 37: 33, 46: 34,
        45: 35, 31: 36, 35: 37, 12: 38, 15: 39, 1: 40, 17: 41, 32: 42, 9: 43, 13: 44, 7: 45, 16: 46, 6: 47,
    ]

    /// キーコードと文字から、shell-core のキーを作る。`characters` は Shift だけを適用した文字
    /// （Ctrl / Option / Command を押しているときは、それらを外した文字）。
    /// JIS の位置にあるキーは、番号と配列の文字（`layout_text`）の両方を渡す（どちらで読むかは shell-core が決める）
    static func keyInput(
        keyCode: UInt16, characters: String, modifiers: Ikari_Shell_V1_Modifiers, isRepeat: Bool, secure: Bool
    ) -> Ikari_Shell_V1_KeyInput {
        var input = Ikari_Shell_V1_KeyInput()
        input.modifiers = modifiers
        input.isRepeat = isRepeat
        input.secure = secure
        switch keyCode {
        case space: input.kind = .space
        case returnKey, keypadEnter: input.kind = .enter
        case escape: input.kind = .escape
        case delete: input.kind = .backspace
        case leftArrow: input.kind = .left
        case rightArrow: input.kind = .right
        case downArrow: input.kind = .down
        case upArrow: input.kind = .up
        case eisu: input.kind = .eisu
        case kana: input.kind = .kana
        default:
            if let number = functionKeys[keyCode] {
                input.kind = .function
                input.function = number
            } else if let jis = jisKeys[keyCode] {
                input.kind = .jis
                input.jis = jis
                input.layoutText = characters
            } else if characters.isEmpty {
                input.kind = .other
            } else {
                input.kind = .character
                input.text = characters
            }
        }
        return input
    }
}
