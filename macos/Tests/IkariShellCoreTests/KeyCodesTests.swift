// 仮想キーコードから状態機械の入力への対応。
import Testing

@testable import IkariShellCore

@Suite struct KeyCodesTests {
    @Test func functionKeysMapToTheirNumbers() {
        // F1〜F20 がすべて別の番号に対応する
        let codes: [UInt16] = [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111, 105, 107, 113, 106, 64, 79, 80, 90]
        #expect(codes.map { KeyCodes.functionNumber(forKeyCode: $0) } == (1...20).map { Optional($0) })
        // F6〜F10（字種変換）
        #expect(KeyCodes.functionNumber(forKeyCode: 97) == 6)
        #expect(KeyCodes.functionNumber(forKeyCode: 109) == 10)
        #expect(KeyCodes.functionNumber(forKeyCode: 0) == nil)
    }

    @Test func jisKeysCoverEveryIkariJisKeyExactlyOnce() {
        // ikari_shell.h の IkariJisKey は 0...47 の 48 個
        var seen = Set<UInt8>()
        for code in UInt16(0)...127 {
            if let key = KeyCodes.jisKey(forKeyCode: code) { seen.insert(key) }
        }
        #expect(seen == Set(0...47))
    }

    @Test func jisKeyPositionsFollowTheJisLayout() {
        #expect(KeyCodes.jisKey(forKeyCode: 0) == 22)  // A
        #expect(KeyCodes.jisKey(forKeyCode: 6) == 47)  // Z
        #expect(KeyCodes.jisKey(forKeyCode: 18) == 0)  // 1
        #expect(KeyCodes.jisKey(forKeyCode: 29) == 9)  // 0
        #expect(KeyCodes.jisKey(forKeyCode: 24) == 11)  // ^
        #expect(KeyCodes.jisKey(forKeyCode: 93) == 12)  // ¥
        #expect(KeyCodes.jisKey(forKeyCode: 94) == 21)  // ろ
        #expect(KeyCodes.jisKey(forKeyCode: 36) == nil)  // Return
    }

    @Test func arrowKeysBecomeLeftAndRight() {
        func input(_ code: UInt16, _ modifiers: KeyModifiers = []) -> KeyInput {
            KeyCodes.keyInput(keyCode: code, characters: "\u{F702}", modifiers: modifiers, inputMethod: .romaji)
        }
        #expect(input(123) == KeyInput(key: .left))
        #expect(input(124, .shift) == KeyInput(key: .right, modifiers: .shift))
        // ↑↓ は割り当てない（ADR 0006 §16-11）
        #expect(input(125).key != .left && input(126).key != .right)
    }

    /// 実機の矢印キーのフラグは fn と numpad（0xa00000）。修飾キーにはならない
    @Test func fnAndNumpadFlagsOfArrowKeysAreNotModifiers() {
        #expect(KeyCodes.modifiers(fromFlagBits: 0xa0_0000) == [])
        #expect(KeyCodes.modifiers(fromFlagBits: 0xa2_0000) == .shift)
        #expect(KeyCodes.modifiers(fromFlagBits: 0x10_0108) == .command)
        #expect(KeyCodes.modifiers(fromFlagBits: 0x1_0000) == [])  // CapsLock
    }

    @Test func specialKeysBecomeNamedInputs() {
        func input(_ code: UInt16, _ modifiers: KeyModifiers = []) -> KeyInput.Key {
            KeyCodes.keyInput(keyCode: code, characters: "x", modifiers: modifiers, inputMethod: .romaji).key
        }
        #expect(input(KeyCodes.space) == .space)
        #expect(input(KeyCodes.returnKey) == .enter)
        #expect(input(KeyCodes.keypadEnter) == .enter)
        #expect(input(KeyCodes.escape) == .escape)
        #expect(input(KeyCodes.delete) == .backspace)
        #expect(input(97) == .function(6))
    }

    @Test func romajiInputKeepsTheCharacters() {
        let input = KeyCodes.keyInput(keyCode: 4, characters: "h", modifiers: [], inputMethod: .romaji)
        #expect(input == KeyInput(key: .character("h")))
        let shifted = KeyCodes.keyInput(keyCode: 0, characters: "A", modifiers: .shift, inputMethod: .romaji)
        #expect(shifted == KeyInput(key: .character("A"), modifiers: .shift))
    }

    @Test func kanaInputUsesThePhysicalKeyExceptWithControlOptionOrCommand() {
        let plain = KeyCodes.keyInput(keyCode: 0, characters: "a", modifiers: [], inputMethod: .kana)
        #expect(plain.key == .jis(22))
        let shifted = KeyCodes.keyInput(keyCode: 0, characters: "A", modifiers: .shift, inputMethod: .kana)
        #expect(shifted == KeyInput(key: .jis(22), modifiers: .shift))
        // Ctrl+J などのキー割り当ては、かな入力でも文字として引く
        let chord = KeyCodes.keyInput(keyCode: 38, characters: "j", modifiers: .control, inputMethod: .kana)
        #expect(chord == KeyInput(key: .character("j"), modifiers: .control))
    }

    @Test func keysWithoutCharactersAreOther() {
        // 123/124 は ←/→、125/126 は ↓/↑ になった（ADR 0014）。ほかの文字を生まないキーは .other
        #expect(KeyCodes.keyInput(keyCode: 125, characters: "", modifiers: [], inputMethod: .romaji).key == .down)
        #expect(KeyCodes.keyInput(keyCode: 126, characters: "", modifiers: [], inputMethod: .romaji).key == .up)
    }
}
