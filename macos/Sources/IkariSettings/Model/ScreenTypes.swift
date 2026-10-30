// 画面が持つ小さな型（M5 で IkariShellCore から移した。IME の殻は使わない）。
// 名前（rawValue）は settings.tsv のキーと値、ブリッジの `OpenTab.tab` の写し。検証・既定値・表記・衝突の判定は
// shell-core（`SettingsCore`。ADR 0041 §2.3）が持つ。ここには判断を書かない。
import Foundation

/// `input.method`
enum InputMethodKind: String, Sendable {
    case romaji
    case kana
}

/// `keymap.preset`
enum KeymapPreset: String, Sendable, CaseIterable {
    case macos
    case msIme = "ms-ime"
    case atok
}

/// 割り当てられる操作。名前（rawValue）は settings.tsv の `keymap.<操作>` のキー。画面の行の並びと既定の割り当ては
/// shell-core が返す（`Described.rows`）
enum KeymapAction: String, Sendable, CaseIterable {
    case toHiragana = "convert.hiragana"
    case toFullwidthKatakana = "convert.fullwidth_katakana"
    case toHalfwidthKatakana = "convert.halfwidth_katakana"
    case toFullwidthAlnum = "convert.fullwidth_alnum"
    case toHalfwidthAlnum = "convert.halfwidth_alnum"
    case segmentFocusLeft = "segment.focus_left"
    case segmentFocusRight = "segment.focus_right"
    case segmentShrink = "segment.shrink"
    case segmentExpand = "segment.expand"
    case candidateNext = "candidate.next"
    case candidatePrevious = "candidate.previous"
    case reconvert = "reconvert"
    case modeToggle = "mode.toggle"
    case modeEnglish = "mode.english"
    case modeJapanese = "mode.japanese"
}

struct KeyModifiers: OptionSet, Sendable, Hashable {
    let rawValue: UInt8
    init(rawValue: UInt8) { self.rawValue = rawValue }

    static let shift = KeyModifiers(rawValue: 1 << 0)
    static let control = KeyModifiers(rawValue: 1 << 1)
    static let option = KeyModifiers(rawValue: 1 << 2)
    static let command = KeyModifiers(rawValue: 1 << 3)
}

/// キーの組み合わせ（`ctrl+j`、`f6`、`shift+left`）。表記の読み書きと検証は shell-core（`KeyChordText`）
struct KeyChord: Sendable, Hashable {
    /// 小文字の名前。1文字のキー（`j`、`;`）、`left`、`right`、`up`、`down`、`f1`〜`f20` など
    var key: String
    var modifiers: KeyModifiers

    init(key: String, modifiers: KeyModifiers = []) {
        self.key = key
        self.modifiers = modifiers
    }
}

/// 分野（属性プロファイル）の重み。エンジンの `ProfileWeightKind` に対応する
enum ProfileWeight: Sendable, Equatable {
    case off, sometimes, often
}

/// 設定アプリのタブ。ブリッジの `OpenTab.tab` と、起動の引数 `--tab` には `rawValue` を入れる
enum SettingsTab: String, Sendable, CaseIterable {
    case input
    case conversion
    case userDictionary = "dictionary"
    case learning
    case about
}
