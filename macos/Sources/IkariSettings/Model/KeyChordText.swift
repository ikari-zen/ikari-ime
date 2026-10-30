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

    /// 押されたキーを記録する。割り当てとして受け入れるか、断る理由（修飾キーが要る／使えないキー）は
    /// shell-core が決める（`bound_action` と同じ規則。`KeyChord::record`）。ここは keyCode から `KeyInput` を作って頼み、
    /// 返事を写すだけ。`charactersIgnoringModifiers` は `NSEvent` のもの（Ctrl を伴っても押したキーの文字になる）
    static func record(keyCode: UInt16, charactersIgnoringModifiers: String, modifiers: KeyModifiers) -> Recording {
        let input = KeyEvents.keyInput(
            keyCode: keyCode, characters: charactersIgnoringModifiers, modifiers: SettingsCore.modifiers(modifiers),
            isRepeat: false, secure: false)
        guard let info = SettingsCore.chord(.record(input)) else { return .rejected(.unsupportedKey) }
        switch info.outcome {
        case .accepted:
            return .chord(KeyChord(key: info.accepted.key, modifiers: SettingsCore.modifiers(info.accepted.modifiers)))
        case .cancel: return .cancel
        case .needsModifier: return .rejected(.needsModifier)
        case .unsupportedKey, .unspecified, .UNRECOGNIZED: return .rejected(.unsupportedKey)
        }
    }
}
