// キー割り当て（ADR 0004 §6.3、docs/phase2-contract.md §4）。契約。
import Foundation

/// 割り当てられる操作。名前（rawValue）は keymap の TSV と settings.tsv の `keymap.<操作>` のキーに使う。
public enum KeymapAction: String, Sendable, CaseIterable {
    case toHiragana = "convert.hiragana"
    case toFullwidthKatakana = "convert.fullwidth_katakana"
    case toHalfwidthKatakana = "convert.halfwidth_katakana"
    case toFullwidthAlnum = "convert.fullwidth_alnum"
    case toHalfwidthAlnum = "convert.halfwidth_alnum"
    // 文節の操作と再変換（ADR 0006 §8.1、§16.1-7）。どのキーにも割り当てられる。設定アプリの画面は `allCases` から作る
    /// フォーカスを左の文節へ（変換中）。既定は `left`
    case segmentFocusLeft = "segment.focus_left"
    /// フォーカスを右の文節へ（変換中）。既定は `right`
    case segmentFocusRight = "segment.focus_right"
    /// フォーカス中の文節を 1 文字縮める（変換中）。既定は `shift+left`
    case segmentShrink = "segment.shrink"
    /// フォーカス中の文節を 1 文字伸ばす（変換中）。既定は `shift+right`
    case segmentExpand = "segment.expand"
    /// 次の候補（変換中。Space と同じ。ADR 0014 §2.4）。既定は `down`
    case candidateNext = "candidate.next"
    /// 前の候補（変換中。Shift+Space と同じ）。既定は `up`
    case candidatePrevious = "candidate.previous"
    /// 選択した確定済みの文字列の再変換（idle）。既定は両プリセットとも `ctrl+shift+r`
    case reconvert = "reconvert"
    // 入力モードの切り替え（ADR 0026 §5.3、D4）。**既定の割り当ては空**（どのプリセットにも入れない。ユーザー決定 Q2）。
    // 日本語・英語のどちらのモードでも効く。英数・かなキーは macOS が切り替えるので、これは 1 つのキーで行き来したいときの割り当て
    /// 日本語 ⇄ 英語を行き来する
    case modeToggle = "mode.toggle"
    /// 英語のモードにする（英語のモードでは何もしない）
    case modeEnglish = "mode.english"
    /// 日本語のモードにする（日本語のモードでは何もしない）
    case modeJapanese = "mode.japanese"

    /// F6〜F10 とプリセットの字種変換キー（`ScriptConverting` に渡せる操作）
    public var isScriptConversion: Bool {
        switch self {
        case .toHiragana, .toFullwidthKatakana, .toHalfwidthKatakana, .toFullwidthAlnum, .toHalfwidthAlnum:
            return true
        case .segmentFocusLeft, .segmentFocusRight, .segmentShrink, .segmentExpand, .reconvert, .candidateNext,
            .candidatePrevious, .modeToggle, .modeEnglish, .modeJapanese:
            return false
        }
    }

    /// 入力モードの切り替え（`mode.*`）
    public var isModeSwitch: Bool {
        switch self {
        case .modeToggle, .modeEnglish, .modeJapanese: return true
        default: return false
        }
    }
}

public struct KeyModifiers: OptionSet, Sendable, Hashable {
    public let rawValue: UInt8
    public init(rawValue: UInt8) { self.rawValue = rawValue }

    public static let shift = KeyModifiers(rawValue: 1 << 0)
    public static let control = KeyModifiers(rawValue: 1 << 1)
    public static let option = KeyModifiers(rawValue: 1 << 2)
    public static let command = KeyModifiers(rawValue: 1 << 3)
}

/// キーの組み合わせ（`ctrl+j`、`f6`、`shift+space` の表記）。
public struct KeyChord: Sendable, Hashable {
    /// 小文字の名前。1文字のキー（`j`、`;`）、`space`、`enter`、`escape`、`backspace`、`left`、`right`、`up`、`down`、`f1`〜`f20`
    public var key: String
    public var modifiers: KeyModifiers

    public init(key: String, modifiers: KeyModifiers = []) {
        self.key = key
        self.modifiers = modifiers
    }

    static let namedKeys: Set<String> = ["space", "enter", "escape", "backspace", "left", "right", "up", "down"]
    static let functionKeyRange = 1...20
    private static let separator: Character = "+"
    private static let modifierNames: [String: KeyModifiers] = [
        "ctrl": .control, "shift": .shift, "option": .option, "command": .command,
    ]

    /// `ctrl+j` のような表記から作る。読めなければ nil。
    /// キーは小文字に直す。未知の修飾キーと、修飾キーの重複は読めない扱いにする。
    public init?(parsing text: String) {
        let trimmed = text.trimmingCharacters(in: .whitespaces).lowercased()
        guard !trimmed.isEmpty else { return nil }
        let plus = String(Self.separator)

        // キー自体が `+` の場合（`+`、`ctrl++`）を先に切り分ける
        let keyPart: String
        let modifierPart: Substring
        if trimmed == plus {
            keyPart = plus
            modifierPart = ""
        } else if trimmed.hasSuffix(plus + plus) {
            keyPart = plus
            modifierPart = trimmed.dropLast(2)
            guard !modifierPart.isEmpty else { return nil }
        } else if let idx = trimmed.lastIndex(of: Self.separator) {
            keyPart = String(trimmed[trimmed.index(after: idx)...])
            modifierPart = trimmed[..<idx]
            guard !modifierPart.isEmpty else { return nil }
        } else {
            keyPart = trimmed
            modifierPart = ""
        }

        var modifiers: KeyModifiers = []
        if !modifierPart.isEmpty {
            for name in modifierPart.split(separator: Self.separator, omittingEmptySubsequences: false) {
                guard let modifier = Self.modifierNames[String(name)], !modifiers.contains(modifier) else {
                    return nil
                }
                modifiers.insert(modifier)
            }
        }
        guard Self.isValidKeyName(keyPart) else { return nil }
        self.init(key: keyPart, modifiers: modifiers)
    }

    /// `f1`〜`f20`、名前つきのキー、空白でない1文字のどれか
    static func isValidKeyName(_ name: String) -> Bool {
        if namedKeys.contains(name) { return true }
        if name.count == 1 { return !(name.first?.isWhitespace ?? true) }
        if name.hasPrefix("f"), let n = Int(name.dropFirst()) {
            return functionKeyRange.contains(n) && String(n) == name.dropFirst()
        }
        return false
    }

    var isFunctionKey: Bool {
        key.count > 1 && key.hasPrefix("f") && Int(key.dropFirst()) != nil
    }
}

public enum KeymapPreset: String, Sendable, CaseIterable {
    case macos
    case msIme = "ms-ime"
    case atok
}

public enum KeymapError: Error, Equatable, Sendable {
    /// プリセットの TSV の行が読めない（行番号は 1 始まり。内容は含めない）
    case invalidLine(number: Int)
}

public struct Keymap: Sendable {
    public var bindings: [KeyChord: KeymapAction]

    public init(bindings: [KeyChord: KeymapAction] = [:]) {
        self.bindings = bindings
    }

    /// プリセットの TSV（`macos/Resources/keymap/*.tsv`）と、`settings.tsv` の `keymap.<操作>` による上書きから作る。
    /// 上書きされた操作は、プリセットの文字キー（`ctrl+j` など）の割り当てを置き換える。
    /// F キーの割り当ては残す（F6〜F10 は常に使える。ADR 0004 §15.1）。
    /// プリセットの行が読めなければ `KeymapError` を投げる。
    public init(presetTSV: String, overrides: [KeymapAction: KeyChord]) throws {
        var table = try Self.parsePreset(presetTSV)
        for (action, chord) in overrides {
            Self.removeBindings(for: action, from: &table, keepFunctionKeys: true)
            table[chord] = action
        }
        self.bindings = table
    }

    /// `Settings` の上書き（キーの表記、または `none`）から作る。`none` はその操作の割り当てを F キーも含めて外す。
    /// 読めない上書きは無視する（入力を止めない。`Settings.parse` が先に警告を出している）。
    public init(presetTSV: String, settings: Settings) throws {
        var table = try Self.parsePreset(presetTSV)
        for (action, text) in settings.keymapOverrides {
            if text == Settings.noneValue {
                Self.removeBindings(for: action, from: &table, keepFunctionKeys: false)
            } else if let chord = KeyChord(parsing: text) {
                Self.removeBindings(for: action, from: &table, keepFunctionKeys: true)
                table[chord] = action
            }
        }
        self.bindings = table
    }

    public func action(for chord: KeyChord) -> KeymapAction? {
        bindings[chord]
    }

    private static func parsePreset(_ tsv: String) throws -> [KeyChord: KeymapAction] {
        var table: [KeyChord: KeymapAction] = [:]
        for (offset, rawLine) in tsv.split(omittingEmptySubsequences: false, whereSeparator: \.isNewline).enumerated() {
            let line = rawLine.trimmingCharacters(in: .whitespacesAndNewlines)
            if line.isEmpty || line.hasPrefix("#") { continue }
            let columns = line.split(separator: "\t", omittingEmptySubsequences: false)
            guard columns.count == 2,
                let chord = KeyChord(parsing: String(columns[0])),
                let action = KeymapAction(rawValue: columns[1].trimmingCharacters(in: .whitespaces))
            else {
                throw KeymapError.invalidLine(number: offset + 1)
            }
            table[chord] = action
        }
        return table
    }

    private static func removeBindings(
        for action: KeymapAction, from table: inout [KeyChord: KeymapAction], keepFunctionKeys: Bool
    ) {
        for (chord, bound) in table where bound == action {
            if keepFunctionKeys && chord.isFunctionKey { continue }
            table[chord] = nil
        }
    }
}
