// 英語のモード（ADR 0026 D1、D4、D5、§5.2、§5.5）。AppKit にもエンジンにも依存しない純関数と値型。
// 英語のモードの殻は、打鍵をアプリにそのまま渡す（`handle` が false）。殻がするのは、
// モードの切り替えキーの判定と、英語の文脈のたまり（`EnglishContextPool`）の更新だけ。
import Foundation

/// ikari の入力モード
public enum InputMode: Sendable, Equatable {
    case japanese
    case english

    /// モードの ID（`setValue(_:forTag:client:)` の `kTextServiceInputModePropertyTag` の値）から。知らない ID は nil
    public init?(modeID: String) {
        switch modeID {
        case InputModeID.japanese: self = .japanese
        case InputModeID.english, InputModeID.englishInputSource: self = .english
        default: return nil
        }
    }

    public var modeID: String {
        switch self {
        case .japanese: return InputModeID.japanese
        case .english: return InputModeID.english
        }
    }

    /// 日本語 ⇄ 英語
    public var toggled: InputMode {
        self == .japanese ? .english : .japanese
    }
}

/// モードの ID（Info.plist の `ComponentInputModeDict` の辞書のキー。ADR 0026 §5.1、§12）
public enum InputModeID {
    /// 日本語。独自の辞書のキー（`TISInputSourceID` も同じ値）
    public static let japanese = "\(Identity.productID).Japanese"
    /// 英語。macOS の標準のキー。英数キーの行き先がこのキーで決まる（S-A の T1）。
    /// 入力ソースの ID（`TISInputSourceID`）は `englishInputSource`
    public static let english = "com.apple.inputmethod.Roman"
    /// 英語のモードの `TISInputSourceID`（ユーザー決定: 名前は Roman ではなく English）
    public static let englishInputSource = "\(Identity.productID).English"
}

/// 英語のモードの打鍵の分類（`EnglishMode.classify`）
public enum EnglishKeyClass: Sendable, Equatable {
    /// secure（パスワード欄など。D5）。アプリに素通しにする。記録しない。文脈にも入れない
    case passThroughSecure
    /// キー割り当ての `mode.*`。ショートコードの表示があれば確定してから、`target` のモードにする。キーは受け止める
    case switchMode(InputMode)
    /// ショートコードの口（B48。`EnglishMode.handleText`）を通したあと、文脈のたまりに足して素通しにする
    case text
}

/// 英語の文脈のたまり（ADR 0026 §5.5）。印字できる文字を足し、区切りで 1 件にして取り出す。値型。
/// 取り出した文字列は、殻が `CommitText`（`reading` は空、`surface` はたまり）としてエンジンに送る
public struct EnglishContextPool: Sendable, Equatable {
    /// たまりの上限（Unicode のスカラーの数）。エンジンの `MAX_SURFACE_CHARS`（2000）に合わせる。数え方もエンジンと同じ（スカラー）
    public static let maxScalars = 2000

    public private(set) var text = ""
    private var scalarCount = 0

    public init() {}

    public var isEmpty: Bool { text.isEmpty }

    /// たまりの最後の文字（`ShortcodeMachine` が語の頭を判定するのに読む。ADR 0027 §3.2）。空なら nil（直前が分からない）
    public var lastCharacter: Character? { text.last }

    /// 文字列を足す。上限に達したら、たまりを取り出して返す（区切り。たまりは空になる）
    public mutating func append(_ addition: String) -> String? {
        guard !addition.isEmpty else { return nil }
        text += addition
        scalarCount += addition.unicodeScalars.count
        return scalarCount >= Self.maxScalars ? flush() : nil
    }

    /// 末尾の 1 文字（見た目の 1 文字）を消す。空なら何もしない
    public mutating func removeLast() {
        guard let last = text.last else { return }
        text.removeLast()
        scalarCount -= last.unicodeScalars.count
    }

    /// たまりを取り出して空にする。空なら nil（送らない）
    public mutating func flush() -> String? {
        guard !text.isEmpty else { return nil }
        defer { discard() }
        return text
    }

    /// 送らずに捨てる（secure になったとき）
    public mutating func discard() {
        text = ""
        scalarCount = 0
    }
}

/// 英語のモードの打鍵の扱い。状態は文脈のたまりだけ
public struct EnglishMode: Sendable, Equatable {
    /// JIS の英数キー（kVK_JIS_Eisu）とかなキー（kVK_JIS_Kana）。macOS がモードを切り替えたあと、同じキーが切り替え先にも届く。
    /// 素通しにするとアプリに余計な文字が入る（S-A の T1）ので、どのモードでも受け止めて何もしない
    public static let eisuKeyCode: UInt16 = 102
    public static let kanaKeyCode: UInt16 = 104

    public private(set) var pool = EnglishContextPool()
    /// 絵文字のショートコード（ADR 0027 §3。B48）。英語のモードの規則で動く。処理は `ShortcodeEnglish.swift`
    var shortcode = ShortcodeMachine(mode: .english)
    /// いまのショートコードの合成 1 回の数（診断ログ。ADR 0034 §4.2 の `mode=shortcode_en`）。状態に入っていない間は nil
    var diagEpisode: DiagEpisode?
    /// 終わった合成。コントローラが `takeFinishedDiag` で取り出して `InputStateMachine.recordEnglishEpisode` に渡す
    var finishedDiag: DiagFinishedEpisode?
    /// スパイク（#27、捨てるコード）。`SpikePostalEnglish.swift`
    var spikePostal = SpikePostalMachine()
    var lastSpikeStep: SpikeStep?

    public init() {}

    /// 英数・かなキー（受け止めるだけのキー）
    public static func absorbsKeyCode(_ keyCode: UInt16) -> Bool {
        keyCode == eisuKeyCode || keyCode == kanaKeyCode
    }

    /// キーを分類する（ADR 0026 §5.2 の 4 と 5-2）。順は、secure → `mode.*` → 文字。
    /// `current` は今のモード（`mode.toggle` の行き先を決める）
    public static func classify(input: KeyInput, keymap: Keymap, secure: Bool, current: InputMode) -> EnglishKeyClass {
        if secure { return .passThroughSecure }
        if let target = modeSwitchTarget(for: input, keymap: keymap, current: current) { return .switchMode(target) }
        return .text
    }

    /// キー割り当ての `mode.*` に当たるキーなら、行き先のモード。日本語・英語のどちらのモードでも使う
    public static func modeSwitchTarget(for input: KeyInput, keymap: Keymap, current: InputMode) -> InputMode? {
        guard let chord = chord(for: input), let action = keymap.action(for: chord) else { return nil }
        switch action {
        case .modeToggle: return current.toggled
        case .modeEnglish: return .english
        case .modeJapanese: return .japanese
        default: return nil
        }
    }

    /// 打鍵を、キー割り当ての表記（`KeyChord`）に直す。文字のキーは、Ctrl・Option・Command があるときだけ割り当てを引く
    /// （`InputStateMachine` の割り当ての引き方と同じ。素の文字は割り当てにならない）
    static func chord(for input: KeyInput) -> KeyChord? {
        switch input.key {
        case .function(let number): return KeyChord(key: "f\(number)", modifiers: input.modifiers)
        case .character(let text):
            guard !input.modifiers.intersection(Self.commandModifiers).isEmpty, text.count == 1 else { return nil }
            return KeyChord(key: text.lowercased(), modifiers: input.modifiers)
        case .space: return KeyChord(key: "space", modifiers: input.modifiers)
        case .enter: return KeyChord(key: "enter", modifiers: input.modifiers)
        case .escape: return KeyChord(key: "escape", modifiers: input.modifiers)
        case .backspace: return KeyChord(key: "backspace", modifiers: input.modifiers)
        case .left: return KeyChord(key: "left", modifiers: input.modifiers)
        case .right: return KeyChord(key: "right", modifiers: input.modifiers)
        case .up: return KeyChord(key: "up", modifiers: input.modifiers)
        case .down: return KeyChord(key: "down", modifiers: input.modifiers)
        case .jis, .other: return nil
        }
    }

    private static let commandModifiers: KeyModifiers = [.control, .option, .command]

    // MARK: 文脈のたまり（ADR 0026 §5.5）

    /// たまりの最後の文字（`ShortcodeMachine` に渡す）
    public var lastCharacter: Character? { pool.lastCharacter }

    /// 素通しにする打鍵を、たまりに反映する。区切りになったら（Return、Tab、矢印、Escape、F キー、
    /// Ctrl・Option・Command 付き、制御文字、たまりが上限）、取り出したたまりを返す。送る側は `reading` を空にして `CommitText` にする。
    /// - 印字できる文字（Space を含む）: たまりに足す
    /// - Backspace（修飾キーなし、または Shift だけ）: 末尾の 1 文字を消す
    public mutating func record(_ input: KeyInput) -> String? {
        guard input.modifiers.intersection(Self.commandModifiers).isEmpty else { return flush() }
        switch input.key {
        case .character(let text):
            return Self.isPrintable(text) ? pool.append(text) : flush()
        case .space:
            return pool.append(" ")
        case .backspace:
            pool.removeLast()
            return nil
        case .enter, .escape, .left, .right, .up, .down, .function, .jis, .other:
            return flush()
        }
    }

    /// 区切り（モードの切り替え、`deactivateServer`、ショートコードの確定のあとなど）。たまりを取り出して空にする
    public mutating func flush() -> String? {
        pool.flush()
    }

    /// 確定した文字列（ショートコードの確定など）をたまりに足す。上限に達したら取り出す
    public mutating func appendCommitted(_ text: String) -> String? {
        pool.append(text)
    }

    /// 送らずに捨てる（secure になったとき）
    public mutating func discard() {
        pool.discard()
    }

    /// 制御文字、私用領域（矢印キーなどの U+F700 台）、サロゲート、未割り当て以外。`InputStateMachine` の判定と同じ
    static func isPrintable(_ text: String) -> Bool {
        !text.isEmpty
            && text.unicodeScalars.allSatisfy {
                switch $0.properties.generalCategory {
                case .control, .privateUse, .surrogate, .unassigned: false
                default: true
                }
            }
    }
}
