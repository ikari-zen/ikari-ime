// キーの表記（docs/phase2-contract.md §4.1）と、キーを押して記録するための変換。AppKit に依存しない純関数。
import Foundation
import IkariShellCore

enum KeyChordText {
    /// 矢印のキー名（`left` / `right`。ADR 0006 §8.1）の画面での表記
    private static let arrowSymbols = ["left": "←", "right": "→"]

    /// 設定ファイルの表記（`ctrl+shift+j`、`f6`）。`KeyChord(parsing:)` で読み戻せる
    static func string(for chord: KeyChord) -> String {
        var parts: [String] = []
        if chord.modifiers.contains(.control) { parts.append("ctrl") }
        if chord.modifiers.contains(.shift) { parts.append("shift") }
        if chord.modifiers.contains(.option) { parts.append("option") }
        if chord.modifiers.contains(.command) { parts.append("command") }
        parts.append(chord.key)
        return parts.joined(separator: "+")
    }

    /// 画面に出す表記（`⌃J`、`F6`、`⇧←`）。修飾キーは Apple の順（⌃⌥⇧⌘）。矢印は記号で出す
    static func display(for chord: KeyChord) -> String {
        var text = ""
        if chord.modifiers.contains(.control) { text += "⌃" }
        if chord.modifiers.contains(.option) { text += "⌥" }
        if chord.modifiers.contains(.shift) { text += "⇧" }
        if chord.modifiers.contains(.command) { text += "⌘" }
        return text + (arrowSymbols[chord.key] ?? chord.key.uppercased())
    }

    /// 設定ファイルの表記から画面の表記へ。読めなければ元の文字列
    static func display(forText text: String) -> String {
        KeyChord(parsing: text).map(display(for:)) ?? text
    }

    static func isFunctionKey(_ chord: KeyChord) -> Bool {
        chord.key.count > 1 && chord.key.hasPrefix("f") && Int(chord.key.dropFirst()) != nil
    }

    enum Recording: Equatable {
        case chord(KeyChord)
        /// 修飾キーなしの Esc。記録をやめる
        case cancel
        case rejected(Reason)
    }

    enum Reason: Equatable {
        /// 修飾キーのない文字キー（打鍵そのものを奪ってしまう）
        case needsModifier
        /// 状態機械が割り当てとして引かないキー（Space、Enter、Esc、Backspace など）
        case unsupportedKey
    }

    /// 押されたキーを記録する。状態機械が引くのは「F キー」「←/→/↑/↓（修飾キーの有無を問わない）」
    /// 「Ctrl / Option / Command を伴う 1 文字」だけ（`InputStateMachine.boundAction`）なので、それ以外は断る。
    /// `charactersIgnoringModifiers` は `NSEvent` のもの（Ctrl を伴っても押したキーの文字になる）
    static func record(keyCode: UInt16, charactersIgnoringModifiers: String, modifiers: KeyModifiers) -> Recording {
        if keyCode == KeyCodes.escape, modifiers.isEmpty { return .cancel }
        let input = KeyCodes.keyInput(
            keyCode: keyCode, characters: charactersIgnoringModifiers, modifiers: modifiers, inputMethod: .romaji)
        switch input.key {
        case .function(let number):
            return validated(KeyChord(key: "f\(number)", modifiers: modifiers))
        case .left:
            return validated(KeyChord(key: "left", modifiers: modifiers))
        case .right:
            return validated(KeyChord(key: "right", modifiers: modifiers))
        case .up:
            return validated(KeyChord(key: "up", modifiers: modifiers))
        case .down:
            return validated(KeyChord(key: "down", modifiers: modifiers))
        case .character(let text):
            guard text.count == 1 else { return .rejected(.unsupportedKey) }
            let commandModifiers: KeyModifiers = [.control, .option, .command]
            guard !modifiers.intersection(commandModifiers).isEmpty else { return .rejected(.needsModifier) }
            return validated(KeyChord(key: text.lowercased(), modifiers: modifiers))
        default:
            return .rejected(.unsupportedKey)
        }
    }

    /// 表記に直して読み戻せるものだけを通す
    private static func validated(_ chord: KeyChord) -> Recording {
        guard KeyChord(parsing: string(for: chord)) == chord else { return .rejected(.unsupportedKey) }
        return .chord(chord)
    }
}
