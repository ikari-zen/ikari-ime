// キーの表記（docs/phase2-contract.md §4.1）と、キーを押して記録するための変換。
// 表記の読み書きと表示は shell-core（`KeyChord`。ADR 0041 §2.3）。ここに残すのは macOS の keyCode からの記録だけ。
import Foundation
import IkariProto
import IkariShellABI

enum KeyChordText {
    private static func info(for chord: KeyChord) -> Ikari_Settings_V1_ChordInfo? {
        var parts = Ikari_Settings_V1_ChordParts()
        parts.key = chord.key
        parts.modifiers = SettingsCore.modifiers(chord.modifiers)
        return SettingsCore.chord(.parts(parts))
    }

    /// 設定ファイルの表記（`ctrl+shift+j`、`f6`）。`KeyChord(parsing:)` で読み戻せる
    static func string(for chord: KeyChord) -> String {
        info(for: chord)?.text ?? ""
    }

    /// 画面に出す表記（`⌃J`、`F6`、`⇧←`）。修飾キーは Apple の順（⌃⌥⇧⌘）。矢印は記号で出す
    static func display(for chord: KeyChord) -> String {
        info(for: chord)?.display ?? chord.key
    }

    /// 設定ファイルの表記から画面の表記へ。読めなければ元の文字列
    static func display(forText text: String) -> String {
        SettingsCore.chord(.text(text))?.display ?? text
    }

    static func isFunctionKey(_ chord: KeyChord) -> Bool {
        info(for: chord)?.functionKey ?? false
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
    /// 「Ctrl / Option / Command を伴う 1 文字」だけ（shell-core の `bound_action`）なので、それ以外は断る。
    /// `charactersIgnoringModifiers` は `NSEvent` のもの（Ctrl を伴っても押したキーの文字になる）
    static func record(keyCode: UInt16, charactersIgnoringModifiers: String, modifiers: KeyModifiers) -> Recording {
        if keyCode == KeyEvents.escape, modifiers.isEmpty { return .cancel }
        let input = KeyEvents.keyInput(
            keyCode: keyCode, characters: charactersIgnoringModifiers, modifiers: SettingsCore.modifiers(modifiers),
            isRepeat: false, secure: false)
        // JIS の位置にあるキー（文字のキー）は、配列が出す文字（`layout_text`）で記録する
        let text: String
        switch input.kind {
        case .function:
            return validated(KeyChord(key: "f\(input.function)", modifiers: modifiers))
        case .left:
            return validated(KeyChord(key: "left", modifiers: modifiers))
        case .right:
            return validated(KeyChord(key: "right", modifiers: modifiers))
        case .up:
            return validated(KeyChord(key: "up", modifiers: modifiers))
        case .down:
            return validated(KeyChord(key: "down", modifiers: modifiers))
        case .character:
            text = input.text
        case .jis:
            text = input.layoutText
        default:
            return .rejected(.unsupportedKey)
        }
        guard text.count == 1 else { return .rejected(.unsupportedKey) }
        let commandModifiers: KeyModifiers = [.control, .option, .command]
        guard !modifiers.intersection(commandModifiers).isEmpty else { return .rejected(.needsModifier) }
        return validated(KeyChord(key: text.lowercased(), modifiers: modifiers))
    }

    /// 表記に直して読み戻せるものだけを通す（shell-core が確かめる）
    private static func validated(_ chord: KeyChord) -> Recording {
        guard info(for: chord)?.valid == true else { return .rejected(.unsupportedKey) }
        return .chord(chord)
    }
}
