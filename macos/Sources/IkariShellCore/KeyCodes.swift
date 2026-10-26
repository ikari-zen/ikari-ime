// macOS の仮想キーコード（kVK_*）から、状態機械の入力への対応。AppKit に依存しない純関数にしてテストする。
// かな入力は物理キーの位置で決まる（配列に依存しない）ので、キーコードから JisKey の番号（ikari_shell.h の IkariJisKey）へ直す。
import Foundation

public enum KeyCodes {
    public static let space: UInt16 = 49
    public static let returnKey: UInt16 = 36
    public static let keypadEnter: UInt16 = 76
    public static let escape: UInt16 = 53
    /// 後退（Backspace）
    public static let delete: UInt16 = 51
    /// kVK_LeftArrow / kVK_RightArrow / kVK_DownArrow / kVK_UpArrow（↑↓は ADR 0014 で割り当てた。ADR 0006 §16-11 を置き換え）
    public static let leftArrow: UInt16 = 123
    public static let rightArrow: UInt16 = 124
    public static let downArrow: UInt16 = 125
    public static let upArrow: UInt16 = 126

    /// `NSEvent.ModifierFlags.rawValue` の修飾キーのビット（AppKit に依存しないよう値で持つ。`NSEvent.ModifierFlags` と同じ値）
    private static let shiftBit: UInt = 1 << 17
    private static let controlBit: UInt = 1 << 18
    private static let optionBit: UInt = 1 << 19
    private static let commandBit: UInt = 1 << 20

    /// `NSEvent.modifierFlags.rawValue` から、Shift / Control / Option / Command だけを取り出す。
    /// 矢印キーに付く fn（1 << 23）と numpad（1 << 21）、CapsLock、デバイス依存のビットは修飾キーとして扱わない
    public static func modifiers(fromFlagBits bits: UInt) -> KeyModifiers {
        var result: KeyModifiers = []
        if bits & shiftBit != 0 { result.insert(.shift) }
        if bits & controlBit != 0 { result.insert(.control) }
        if bits & optionBit != 0 { result.insert(.option) }
        if bits & commandBit != 0 { result.insert(.command) }
        return result
    }

    /// F1〜F20 の仮想キーコード
    private static let functionKeys: [UInt16: Int] = [
        122: 1, 120: 2, 99: 3, 118: 4, 96: 5, 97: 6, 98: 7, 100: 8, 101: 9, 109: 10,
        103: 11, 111: 12, 105: 13, 107: 14, 113: 15, 106: 16, 64: 17, 79: 18, 80: 19, 90: 20,
    ]

    /// 物理キーの位置 → IkariJisKey の番号。JIS 配列の位置で並べている
    /// （`^` は kVK_ANSI_Equal、`@` は kVK_ANSI_LeftBracket、`[` は kVK_ANSI_RightBracket、
    /// `:` は kVK_ANSI_Quote、`]` は kVK_ANSI_Backslash、`¥` は kVK_JIS_Yen、`ろ` は kVK_JIS_Underscore）
    private static let jisKeys: [UInt16: UInt8] = [
        // 数字の段 1234567890
        18: 0, 19: 1, 20: 2, 21: 3, 23: 4, 22: 5, 26: 6, 28: 7, 25: 8, 29: 9,
        27: 10,  // -
        24: 11,  // ^
        93: 12,  // ¥
        33: 13,  // @
        30: 14,  // [
        41: 15,  // ;
        39: 16,  // :
        42: 17,  // ]
        43: 18,  // ,
        47: 19,  // .
        44: 20,  // /
        94: 21,  // ろ
        // A〜Z（IkariJisKey は A = 22 から アルファベット順）
        0: 22, 11: 23, 8: 24, 2: 25, 14: 26, 3: 27, 5: 28, 4: 29, 34: 30, 38: 31, 40: 32, 37: 33, 46: 34,
        45: 35, 31: 36, 35: 37, 12: 38, 15: 39, 1: 40, 17: 41, 32: 42, 9: 43, 13: 44, 7: 45, 16: 46, 6: 47,
    ]

    /// F キーなら 1〜20 の番号
    public static func functionNumber(forKeyCode keyCode: UInt16) -> Int? {
        functionKeys[keyCode]
    }

    /// かな入力に使えるキーなら IkariJisKey の番号
    public static func jisKey(forKeyCode keyCode: UInt16) -> UInt8? {
        jisKeys[keyCode]
    }

    /// キーコードと文字から、状態機械の入力を作る。
    /// `characters` は Shift だけを適用した文字（Ctrl / Option / Command の修飾キーは外した文字）を渡す。
    /// かな入力のときは、修飾キー（Shift 以外）がなく、JIS の位置にあるキーを `.jis` にする。
    public static func keyInput(
        keyCode: UInt16,
        characters: String,
        modifiers: KeyModifiers,
        inputMethod: InputMethodKind
    ) -> KeyInput {
        switch keyCode {
        case space: return KeyInput(key: .space, modifiers: modifiers)
        case returnKey, keypadEnter: return KeyInput(key: .enter, modifiers: modifiers)
        case escape: return KeyInput(key: .escape, modifiers: modifiers)
        case delete: return KeyInput(key: .backspace, modifiers: modifiers)
        case leftArrow: return KeyInput(key: .left, modifiers: modifiers)
        case rightArrow: return KeyInput(key: .right, modifiers: modifiers)
        case downArrow: return KeyInput(key: .down, modifiers: modifiers)
        case upArrow: return KeyInput(key: .up, modifiers: modifiers)
        default: break
        }
        if let number = functionNumber(forKeyCode: keyCode) {
            return KeyInput(key: .function(number), modifiers: modifiers)
        }
        let commandModifiers: KeyModifiers = [.control, .option, .command]
        if inputMethod == .kana, modifiers.intersection(commandModifiers).isEmpty,
            let jis = jisKey(forKeyCode: keyCode)
        {
            return KeyInput(key: .jis(jis), modifiers: modifiers)
        }
        guard !characters.isEmpty else { return KeyInput(key: .other, modifiers: modifiers) }
        return KeyInput(key: .character(characters), modifiers: modifiers)
    }
}
