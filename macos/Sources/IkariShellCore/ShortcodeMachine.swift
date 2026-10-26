// ショートコードの状態機械（ADR 0027 §3.3、§3.4、§9.1）。契約（B47 の契約コミット）。
// 型とシグネチャだけで、中身はレーン S が埋める。純粋（AppKit にもエンジンにも依存しない）。
//
// 責任の分け方（§9.1）: `ShortcodeMachine` は「今の状態と、キーを受けたときの結果」だけを持つ。
// 打ち直し（`retype`）と確定後のキーの扱い（`ShortcodeAfterCommit`）の実行は、呼ぶ側
// （日本語のモードは `InputStateMachine`、英語のモードは B48 の英語の経路）が持つ。
import Foundation

/// モードごとの差（§3.1 の表）。名前の文字の読み方と、無視したときのキーの扱いを決める
public enum ShortcodeMode: Sendable, Equatable {
    /// 英語のモード（ADR 0026）。届いた文字を読む。打ち直しと無視は素通し（B48）
    case english
    /// 日本語のモードのローマ字入力。届いた文字を読む。打ち直しは idle からの処理に流す
    case romaji
    /// 日本語のモードのかな入力（JIS）。`.jis` のキーを `JisKeyAscii` の刻印で読む。届いたのが文字なら文字
    case kanaJis
}

/// 候補の 1 行。`label` は絵文字の行の右に薄く出す名前（`:eyes:`）。「そのままの入力」の行は nil（§3.4）
public struct ShortcodeCandidate: Sendable, Equatable {
    public var surface: String
    public var label: String?

    public init(surface: String, label: String? = nil) {
        self.surface = surface
        self.label = label
    }
}

/// `shortcode(name)`: 打っている途中（変換中の文字列は `:` + name。§3.3）
public struct ShortcodeTyping: Sendable, Equatable {
    /// `:` を除いた、ここまでの名前
    public var name: String
    /// 入ってから受けたキー（入りの `:` を含む。打ち直しで流し直す。§3.3）
    public var keys: [KeyInput]

    public init(name: String = "", keys: [KeyInput] = []) {
        self.name = name
        self.keys = keys
    }

    /// 変換中の文字列（`:` + name）
    public var marked: String { ":" + name }
}

/// `shortcodeChoice(candidates, selected)`: 候補を出している（変換中の文字列は選んでいる候補。§3.4）
public struct ShortcodeChoosing: Sendable, Equatable {
    public var name: String
    /// 見えていた半角の入力（`:eyes:`、前方一致のときは `:ey`）。候補の 0 番の `surface`（Q16）
    public var typed: String
    /// 0 番は「そのままの入力」（`typed`）。§3.5 の並び
    public var candidates: [ShortcodeCandidate]
    public var selected: Int
    /// 入ってから受けたキー（BackSpace・Esc で `shortcode(name)` に戻るため）
    public var keys: [KeyInput]

    public init(name: String, typed: String, candidates: [ShortcodeCandidate], selected: Int = 0, keys: [KeyInput] = []) {
        self.name = name
        self.typed = typed
        self.candidates = candidates
        self.selected = selected
        self.keys = keys
    }

    /// 選んでいる候補（変換中の文字列）。範囲外なら `typed`
    public var marked: String {
        candidates.indices.contains(selected) ? candidates[selected].surface : typed
    }
}

/// 今のショートコードの状態
public enum ShortcodePhase: Sendable, Equatable {
    case inactive
    case typing(ShortcodeTyping)
    case choosing(ShortcodeChoosing)
}

/// 確定したあとの、今のキーの扱い（§3.4 の表の「キーをアプリへ」「新しい合成」）
public enum ShortcodeAfterCommit: Sendable, Equatable {
    /// キーを消費する（絵文字を選んだ Enter、Esc、日本語のモードの数字キーでの確定など）
    case consume
    /// キーをアプリに渡す（英語のモードの Space・数字・文字、第 1 候補のままの Enter。矢印・修飾キー付き）
    case passKey
    /// そのキーで新しい合成を始める（日本語のモードの印字できる文字。`continueAfterCommit`）
    case continueWith(KeyInput)
}

/// 確定の指示
public struct ShortcodeCommit: Sendable, Equatable {
    /// 確定する文字列（絵文字、またはそのままの入力）
    public var text: String
    /// `CommitText` の読み。`:` + 名前（§3.7）
    public var reading: String
    public var after: ShortcodeAfterCommit

    public init(text: String, reading: String, after: ShortcodeAfterCommit) {
        self.text = text
        self.reading = reading
        self.after = after
    }
}

/// キーを受けた結果。呼ぶ側はこれを `KeyAction` に直す（`InputStateMachine`）か、英語の経路の操作に直す（B48）
public enum ShortcodeOutcome: Sendable, Equatable {
    /// ショートコードに関係ないキー。呼ぶ側が今までどおり処理する（`phase` は `.inactive` のまま）
    case notHandled
    /// 入った、または名前が変わった。変換中の文字列を `marked` にする。`hidesCandidates` は候補を閉じて戻ったとき
    case typing(marked: String, hidesCandidates: Bool)
    /// `:` だけで BackSpace。変換中の文字列を消して元の状態に戻る（キーは消費）
    case cancelled
    /// 候補を出した（初めて）。変換中の文字列は `ShortcodeChoosing.marked`
    case showChoices(ShortcodeChoosing)
    /// 候補の選択が動いた
    case moveChoice(ShortcodeChoosing)
    /// 確定する
    case commit(ShortcodeCommit)
    /// 打ち直す（§3.3）。`keys` は入ってから受けたキー（入りの `:` から今のキーまで。最後が今のキー。
    /// 日本語のモードの Esc は「合成に戻る」ので、今のキーを含めない）。
    /// `literal` は今のキーを除く表示（`:ey`）。英語のモードは `literal` を確定し、今のキーだけをアプリに渡す。
    /// 日本語のモードは `keys` を、ショートコードを切った idle からの処理に順に流す
    case retype(keys: [KeyInput], literal: String)
}

/// キーを受けた結果と、そのあとの状態
public struct ShortcodeResult: Sendable, Equatable {
    public var outcome: ShortcodeOutcome
    public var phase: ShortcodePhase

    public init(outcome: ShortcodeOutcome, phase: ShortcodePhase) {
        self.outcome = outcome
        self.phase = phase
    }
}

/// ショートコードの状態機械。値型で、状態は `phase` が全部。表とモードは作るときに渡す。
public struct ShortcodeMachine: Sendable, Equatable {
    /// 候補ウィンドウの 1 ページの行数（数字キー 1〜9。ADR 0014）。選択が動くとページも動く
    public static let candidatePageSize = 9
    /// Tab と Shift+Tab（`NSBackTabCharacter`）が `.character` で届くときの文字
    static let tabCharacter = "\t"
    static let backTabCharacter = "\u{19}"
    /// 英語のモードで、前方一致が切れても保つ名前の長さの上限（表の名前の最大長。ADR 0027 §3.3、B52）
    static let maxNameLength = 64
    private static let colon: Character = ":"
    private static let commandModifiers: KeyModifiers = [.control, .option, .command]

    public var table: ShortcodeTable
    public var mode: ShortcodeMode
    /// 上付き・下付きの小さい文字の候補（`:m2:` → m²。`emoji.small_letters`。#264）。英語のモードの呼び出しが、キーごとに設定を写す
    public var smallLetters = false
    public private(set) var phase: ShortcodePhase = .inactive
    /// 直近に確定した候補の種類（診断ログの `pick_kind`。ADR 0034）。確定のたびに更新する
    private(set) var lastPickKind: DiagPickKind = .literal

    public init(table: ShortcodeTable = .empty, mode: ShortcodeMode) {
        self.table = table
        self.mode = mode
    }

    /// 状態に入っている（`.inactive` でない）
    public var isActive: Bool { phase != .inactive }

    /// キーを 1 つ受ける。
    /// - `canEnter`: 入ってよいか。呼ぶ側が §3.2 の共通の条件（secure でない、`emoji.shortcode` が on、
    ///   前面のアプリが除外にない、Ctrl・Option・Command なし、キーリピートでない）と、モードごとの条件
    ///   （英語は語の頭、日本語は idle）を判定して渡す。表が空のときは `canEnter` にかかわらず入らない（機械が見る）。
    ///   `isActive` の間は使わない
    /// - 返り値の `phase` は `self.phase` にも反映される
    public mutating func handle(_ input: KeyInput, canEnter: Bool) -> ShortcodeResult {
        let outcome: ShortcodeOutcome
        switch phase {
        case .inactive: outcome = enter(input, canEnter: canEnter)
        case .typing(let typing): outcome = handleTyping(input, typing)
        case .choosing(let choosing): outcome = handleChoosing(input, choosing)
        }
        return ShortcodeResult(outcome: outcome, phase: phase)
    }

    /// モードの切り替え、secure になる、`deactivateServer`、クリック（`commitComposition`）のとき。
    /// 今の表示のまま確定する指示を返し、`.inactive` に戻る。`.inactive` なら nil（§3.3、§3.4 の最後の行）
    public mutating func flush() -> ShortcodeCommit? {
        defer { phase = .inactive }
        switch phase {
        case .inactive:
            return nil
        case .typing(let typing):
            return ShortcodeCommit(text: typing.marked, reading: typing.marked, after: .consume)
        case .choosing(let choosing):
            return commitValue(choosing, index: choosing.selected, after: .consume)
        }
    }

    /// 何も確定せずに状態を捨てる（呼ぶ側が表示を消したあとなど）
    public mutating func reset() {
        phase = .inactive
    }

    // MARK: 入り

    private mutating func enter(_ input: KeyInput, canEnter: Bool) -> ShortcodeOutcome {
        guard canEnter, !table.isEmpty, !hasCommandModifiers(input), isEntryKey(input) else { return .notHandled }
        let typing = ShortcodeTyping(name: "", keys: [input])
        phase = .typing(typing)
        return .typing(marked: typing.marked, hidesCandidates: false)
    }

    /// 入りのキー。かな入力は Shift+けのキー（Shift なしは「け」のまま）。どのモードでも `:` の文字
    private func isEntryKey(_ input: KeyInput) -> Bool {
        switch input.key {
        case .character(let text):
            return text == String(Self.colon)
        case .jis(let key):
            return mode == .kanaJis && key == JisKeyAscii.colonKey && input.modifiers.contains(.shift)
        default:
            return false
        }
    }

    // MARK: shortcode(name)（§3.3）

    private mutating func handleTyping(_ input: KeyInput, _ typing: ShortcodeTyping) -> ShortcodeOutcome {
        if hasCommandModifiers(input) { return retype(typing, with: input, includingKey: true) }
        switch input.key {
        case .backspace:
            guard !typing.name.isEmpty else {
                phase = .inactive
                return .cancelled
            }
            var shortened = typing
            shortened.name.removeLast()
            shortened.keys.removeLast()
            phase = .typing(shortened)
            return .typing(marked: shortened.marked, hidesCandidates: false)
        case .escape:
            // 日本語のモードは「合成に戻る」ので Esc を流さない。英語のモードは Esc をアプリへ渡す
            return retype(typing, with: input, includingKey: mode == .english)
        case .space:
            return mode == .english ? retype(typing, with: input, includingKey: true) : showPrefix(typing, with: input)
        case .down:
            return showPrefix(typing, with: input)
        case .character(Self.tabCharacter):
            return showPrefix(typing, with: input)
        default:
            break
        }
        guard let character = asciiCharacter(of: input) else { return retype(typing, with: input, includingKey: true) }
        if character == Self.colon { return closeColon(typing, with: input) }
        guard Self.isNameCharacter(character) else { return retype(typing, with: input, includingKey: true) }
        var extended = typing
        extended.name.append(character)
        extended.keys.append(input)
        if mode == .english {
            // 英語のモード（B52）: 前方一致が切れても、名前の文字が続く間は状態を保つ（Backspace で直せる）。長さの上限で打ち直す
            guard extended.name.count <= Self.maxNameLength else { return retype(typing, with: input, includingKey: true) }
        } else {
            guard !table.prefix(extended.name).isEmpty else { return retype(typing, with: input, includingKey: true) }
        }
        phase = .typing(extended)
        return .typing(marked: extended.marked, hidesCandidates: false)
    }

    private mutating func retype(_ typing: ShortcodeTyping, with input: KeyInput, includingKey: Bool) -> ShortcodeOutcome {
        phase = .inactive
        return .retype(keys: includingKey ? typing.keys + [input] : typing.keys, literal: typing.marked)
    }

    /// 閉じの `:`。名前が完全に一致したときだけ候補を出す
    private mutating func closeColon(_ typing: ShortcodeTyping, with input: KeyInput) -> ShortcodeOutcome {
        let found = table.exact(typing.name).map { [$0] } ?? (smallLetters ? SmallLetters.candidates(for: typing.name) : [])
        guard !found.isEmpty else { return retype(typing, with: input, includingKey: true) }
        let typed = typing.marked + String(Self.colon)
        let choosing = ShortcodeChoosing(
            name: typing.name,
            typed: typed,
            candidates: [ShortcodeCandidate(surface: typed)] + found.map { ShortcodeCandidate(surface: $0, label: typed) },
            selected: 0,
            keys: typing.keys + [input])
        phase = .choosing(choosing)
        return .showChoices(choosing)
    }

    /// Tab、↓（日本語のモードは Space も）。前方一致の候補（§3.5）。名前が空なら打ち直す
    private mutating func showPrefix(_ typing: ShortcodeTyping, with input: KeyInput) -> ShortcodeOutcome {
        let matched = table.prefix(typing.name)
        guard !matched.isEmpty else { return retype(typing, with: input, includingKey: true) }
        let candidates =
            [ShortcodeCandidate(surface: typing.marked)]
            + matched.map {
                ShortcodeCandidate(surface: $0.emoji, label: Self.colonText + $0.name + Self.colonText)
            }
        let choosing = ShortcodeChoosing(
            name: typing.name, typed: typing.marked, candidates: candidates, selected: 0, keys: typing.keys)
        phase = .choosing(choosing)
        return .showChoices(choosing)
    }

    private static let colonText = String(colon)

    // MARK: shortcodeChoice（§3.4）

    private mutating func handleChoosing(_ input: KeyInput, _ choosing: ShortcodeChoosing) -> ShortcodeOutcome {
        if hasCommandModifiers(input) { return commit(choosing, index: choosing.selected, after: .passKey) }
        let english = mode == .english
        switch input.key {
        case .down:
            return move(choosing, by: 1)
        case .up:
            return move(choosing, by: -1)
        case .space:
            if english { return commit(choosing, index: choosing.selected, after: .passKey) }
            return move(choosing, by: input.modifiers.contains(.shift) ? -1 : 1)
        case .enter:
            if english {
                // 第 1 候補のままの Enter はアプリへ渡す（Slack ならそのまま送信）。絵文字を選んでいたら消費（Q14）
                return commit(choosing, index: choosing.selected, after: choosing.selected == 0 ? .passKey : .consume)
            }
            return commit(choosing, index: choosing.selected, after: .consume)
        case .backspace:
            return backToTyping(choosing)
        case .escape:
            return english ? commit(choosing, index: 0, after: .consume) : backToTyping(choosing)
        case .left, .right, .function:
            return commit(choosing, index: choosing.selected, after: .passKey)
        case .other:
            return english ? commit(choosing, index: choosing.selected, after: .passKey) : move(choosing, by: 0)
        case .character(Self.tabCharacter):
            return move(choosing, by: input.modifiers.contains(.shift) ? -1 : 1)
        case .character(Self.backTabCharacter):
            return move(choosing, by: -1)
        case .character, .jis:
            break
        }
        if let character = asciiCharacter(of: input), let number = character.wholeNumberValue, (1...9).contains(number) {
            if english { return commit(choosing, index: choosing.selected, after: .passKey) }
            let index = choosing.selected / Self.candidatePageSize * Self.candidatePageSize + number - 1
            return choosing.candidates.indices.contains(index)
                ? commit(choosing, index: index, after: .consume) : move(choosing, by: 0)
        }
        if case .character(let text) = input.key, !Self.isPrintable(text) {
            return english ? commit(choosing, index: choosing.selected, after: .passKey) : move(choosing, by: 0)
        }
        // そのほかの印字できる文字。英語のモードはキーをアプリへ、日本語のモードはそのキーで新しい合成
        return commit(choosing, index: choosing.selected, after: english ? .passKey : .continueWith(input))
    }

    /// 選択を動かす。端では折り返す。`delta` が 0 なら何も動かさない（受け止めるだけ）
    private mutating func move(_ choosing: ShortcodeChoosing, by delta: Int) -> ShortcodeOutcome {
        var moved = choosing
        let count = choosing.candidates.count
        moved.selected = ((choosing.selected + delta) % count + count) % count
        phase = .choosing(moved)
        return .moveChoice(moved)
    }

    /// 「そのままの入力」の行は `literal`。ほかは、表にある名前の絵文字なら `emoji`、閉じの `:` で表にない名前から作った
    /// 上付き・下付きの文字は `small_letter`（前方一致の候補は閉じの `:` がなく、必ず表の絵文字）
    private func pickKind(of choosing: ShortcodeChoosing, index: Int) -> DiagPickKind {
        guard choosing.candidates.indices.contains(index), choosing.candidates[index].label != nil else { return .literal }
        let closed = choosing.typed.hasSuffix(Self.colonText)
        return closed && table.exact(choosing.name) == nil ? .smallLetter : .emoji
    }

    private mutating func commitValue(_ choosing: ShortcodeChoosing, index: Int, after: ShortcodeAfterCommit) -> ShortcodeCommit {
        lastPickKind = pickKind(of: choosing, index: index)
        let text = choosing.candidates.indices.contains(index) ? choosing.candidates[index].surface : choosing.typed
        return ShortcodeCommit(text: text, reading: Self.colonText + choosing.name, after: after)
    }

    private mutating func commit(_ choosing: ShortcodeChoosing, index: Int, after: ShortcodeAfterCommit) -> ShortcodeOutcome {
        phase = .inactive
        return .commit(commitValue(choosing, index: index, after: after))
    }

    /// 候補を閉じて `shortcode(name)` に戻る。閉じの `:` で出した候補なら、その `:` を消す
    private mutating func backToTyping(_ choosing: ShortcodeChoosing) -> ShortcodeOutcome {
        var keys = choosing.keys
        if choosing.typed.hasSuffix(Self.colonText) { keys.removeLast() }
        let typing = ShortcodeTyping(name: choosing.name, keys: keys)
        phase = .typing(typing)
        return .typing(marked: typing.marked, hidesCandidates: true)
    }

    // MARK: キーの読み方

    private func hasCommandModifiers(_ input: KeyInput) -> Bool {
        !input.modifiers.intersection(Self.commandModifiers).isEmpty
    }

    /// 名前を読むための ASCII 1 文字。届いた文字（1 文字の ASCII）か、かな入力のキーの刻印
    private func asciiCharacter(of input: KeyInput) -> Character? {
        switch input.key {
        case .character(let text):
            guard text.count == 1, let character = text.first, character.isASCII else { return nil }
            return character
        case .jis(let key):
            guard mode == .kanaJis else { return nil }
            return JisKeyAscii.character(forJisKey: key, shifted: input.modifiers.contains(.shift))
        default:
            return nil
        }
    }

    /// 名前の文字は `a-z 0-9 _ + -`（§3.3）
    static func isNameCharacter(_ character: Character) -> Bool {
        guard character.isASCII else { return false }
        return character.isLowercase || character.isNumber || character == "_" || character == "+" || character == "-"
    }

    /// 制御文字と私用領域（矢印キーなどが返す U+F700 台）は入力として扱わない（`InputStateMachine` と同じ）
    private static func isPrintable(_ text: String) -> Bool {
        !text.isEmpty
            && text.unicodeScalars.allSatisfy {
                switch $0.properties.generalCategory {
                case .control, .privateUse, .surrogate, .unassigned: false
                default: true
                }
            }
    }
}
