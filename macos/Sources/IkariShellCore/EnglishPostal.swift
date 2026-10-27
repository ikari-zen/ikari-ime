// 英語のモードの郵便番号（ADR 0032 §9.1。issue #27）。語の頭の `te` + 数字 7 桁をためて、7 桁目を打った瞬間に候補ウィンドウを出す。
// 7 桁目より前は候補も下線も出さない。候補を出している間も下線は付けない。選ぶのは ↑/↓、確定は Enter、Esc は打った文字のまま確定して閉じる。
// 住所はエンジンに 1 回だけ非同期に問い合わせ（`PostalLookupRequest`）、返事が来たとき状態が同じなら候補に足す。
// 英語の経路でエンジンを同期で呼ばない（ADR 0026 §9）。この型は問い合わせをしない。呼ぶ側（コントローラ）が `fetchPostalAddresses` で行う。
import Foundation

/// 住所の問い合わせ。`serial` は候補を出すたびに増え、返事が古いかを見分ける（同じ番号を打ち直した場合も区別する）
public struct PostalLookupRequest: Sendable, Equatable {
    public var code: String
    public var serial: Int
}

/// 住所の問い合わせの待ちの上限。値は暫定（実測が要る。ADR 0032 §9.1）
public let postalLookupTimeout: Duration = .seconds(1)

/// 主スレッドの外で呼ぶ。失敗（タイムアウトを含む）は空として扱う（住所が足されないだけで、入力は止めない）
public func fetchPostalAddresses(engine: any EngineClient, code: String, timeout: Duration = postalLookupTimeout) -> [String] {
    (try? engine.postalLookup(code: code, timeout: timeout)) ?? []
}

/// 1 打鍵の結果
struct PostalStep: Sendable, Equatable {
    var actions: [KeyAction] = []
    var passKey = false
    /// 確定した文字列（文脈のたまりに足す）
    var committed: String?
    /// 非同期に問い合わせる番号
    var lookup: PostalLookupRequest?
}

struct PostalMachine: Sendable, Equatable {
    /// ためる語の種類。今は `te` + 郵便番号だけ（`u+` は #28 で足す）
    enum Kind: Sendable, Equatable { case te }

    struct Choosing: Sendable, Equatable {
        var kind: Kind
        var buffer: String
        var rows: [String]
        var selected: Int
        /// 返事を待っている問い合わせ。返事が来たら nil
        var pending: PostalLookupRequest?
    }

    enum Phase: Sendable, Equatable {
        case inactive
        case typing(Kind, String)
        case choosing(Choosing)
    }

    static let postalDigits = 7
    static let pageSize = 9
    private static let tePrefix = "te"
    private static let commandModifiers: KeyModifiers = [.control, .option, .command]

    private(set) var phase: Phase = .inactive
    private var serial = 0

    var isActive: Bool { phase != .inactive }

    // MARK: キー

    /// 関係ないキー（ためていなくて、入りの条件も満たさない）は nil
    mutating func handle(_ input: KeyInput, canEnter: Bool) -> PostalStep? {
        switch phase {
        case .inactive:
            return enter(input, canEnter: canEnter)
        case .typing(let kind, let buffer):
            return typing(kind, buffer, input)
        case .choosing(let choosing):
            return choose(choosing, input)
        }
    }

    private mutating func enter(_ input: KeyInput, canEnter: Bool) -> PostalStep? {
        guard canEnter, Self.isPlain(input), case .character(let text) = input.key, text == "t" else { return nil }
        phase = .typing(.te, text)
        return Self.typingStep(text)
    }

    private mutating func typing(_ kind: Kind, _ buffer: String, _ input: KeyInput) -> PostalStep {
        guard Self.isPlain(input) else { return retype(buffer: buffer, hidesCandidates: false) }
        switch input.key {
        case .character(let text):
            if let next = Self.append(text, to: buffer, kind: kind) { return settle(kind, next) }
        case .backspace:
            var shorter = buffer
            shorter.removeLast()
            if shorter.isEmpty {
                phase = .inactive
                return PostalStep(actions: [Self.mark("")])
            }
            phase = .typing(kind, shorter)
            return Self.typingStep(shorter)
        default:
            break
        }
        return retype(buffer: buffer, hidesCandidates: false)
    }

    private mutating func choose(_ choosing: Choosing, _ input: KeyInput) -> PostalStep {
        guard Self.isPlain(input) else { return retype(buffer: choosing.buffer, hidesCandidates: true) }
        switch input.key {
        case .down:
            return move(choosing, by: 1)
        case .up:
            return move(choosing, by: -1)
        case .enter:
            return commit(choosing)
        case .escape:
            // 打った文字のまま確定して閉じる。Esc は消費する
            var step = retype(buffer: choosing.buffer, hidesCandidates: true)
            step.passKey = false
            return step
        case .backspace:
            var shorter = choosing.buffer
            shorter.removeLast()
            phase = .typing(choosing.kind, shorter)
            var step = Self.typingStep(shorter)
            step.actions.append(.hideCandidates)
            return step
        default:
            // Space、Tab、数字、ほかの文字、桁あふれ、←/→ など
            return retype(buffer: choosing.buffer, hidesCandidates: true)
        }
    }

    // MARK: 遷移の部品

    /// 最後の桁がそろったら候補を出し、そろっていなければ「ためている状態」にする
    private mutating func settle(_ kind: Kind, _ buffer: String) -> PostalStep {
        guard let rows = Self.baseRows(kind, buffer), let code = Self.postalCode(kind, buffer) else {
            phase = .typing(kind, buffer)
            return Self.typingStep(buffer)
        }
        serial += 1
        let request = PostalLookupRequest(code: code, serial: serial)
        let choosing = Choosing(kind: kind, buffer: buffer, rows: rows, selected: 0, pending: request)
        phase = .choosing(choosing)
        return PostalStep(actions: [Self.mark(rows[0]), .showCandidates(Self.model(choosing))], lookup: request)
    }

    private mutating func move(_ choosing: Choosing, by delta: Int) -> PostalStep {
        var next = choosing
        let count = max(choosing.rows.count, 1)
        next.selected = (choosing.selected + delta + count) % count
        phase = .choosing(next)
        let samePage = next.selected / Self.pageSize == choosing.selected / Self.pageSize
        let model = Self.model(next)
        return PostalStep(actions: [Self.mark(next.rows[next.selected]), samePage ? .moveSelection(model) : .showCandidates(model)])
    }

    private mutating func commit(_ choosing: Choosing) -> PostalStep {
        let text = choosing.rows[choosing.selected]
        phase = .inactive
        return PostalStep(actions: [.commit(text), .hideCandidates], committed: text)
    }

    /// 形から外れた。ためた文字をそのまま確定し、今のキーだけアプリに渡す（過去のキーは渡し直せない。ADR 0027 §3.3 と同じ）
    private mutating func retype(buffer: String, hidesCandidates: Bool) -> PostalStep {
        phase = .inactive
        var actions: [KeyAction] = [.commit(buffer)]
        if hidesCandidates { actions.append(.hideCandidates) }
        return PostalStep(actions: actions, passKey: true, committed: buffer)
    }

    /// 状態を捨てる（モードの切り替え、secure、`deactivateServer`、設定のオフなど）。ためた文字を確定し、キーは渡さない
    mutating func flush() -> PostalStep? {
        var step: PostalStep
        switch phase {
        case .inactive:
            return nil
        case .typing(_, let buffer):
            step = retype(buffer: buffer, hidesCandidates: false)
        case .choosing(let choosing):
            step = retype(buffer: choosing.buffer, hidesCandidates: true)
        }
        step.passKey = false
        return step
    }

    // MARK: 非同期の返事

    /// 返事を受ける。候補を出している間で、同じ問い合わせ（番号と通し番号）のときだけ候補に足す。違えば捨てる（nil）。住所が空なら足さない
    mutating func lookupArrived(_ request: PostalLookupRequest, addresses: [String]) -> PostalStep? {
        guard case .choosing(var choosing) = phase, choosing.pending == request else { return nil }
        choosing.pending = nil
        let added = addresses.filter { !$0.isEmpty && !choosing.rows.contains($0) }
        if !added.isEmpty { choosing.rows += added }
        phase = .choosing(choosing)
        return added.isEmpty ? nil : PostalStep(actions: [.showCandidates(Self.model(choosing))])
    }

    // MARK: 純関数

    private static func isPlain(_ input: KeyInput) -> Bool {
        input.modifiers.intersection(commandModifiers).isEmpty
    }

    /// 次の 1 文字を足した文字列。形に合わなければ nil
    static func append(_ text: String, to buffer: String, kind: Kind) -> String? {
        guard text.count == 1, let c = text.first else { return nil }
        switch kind {
        case .te:
            if buffer == "t" { return text == "e" ? tePrefix : nil }
            return c.isASCII && c.isNumber && buffer.count - tePrefix.count < postalDigits ? buffer + text : nil
        }
    }

    /// 最後の桁がそろった形なら、問い合わせ前の候補（0 番は「そのままの入力」）。そろっていなければ nil
    static func baseRows(_ kind: Kind, _ buffer: String) -> [String]? {
        switch kind {
        case .te:
            guard let code = postalCode(kind, buffer) else { return nil }
            return [buffer, "〒\(code.prefix(3))-\(code.suffix(4))", "〒" + code]
        }
    }

    /// 問い合わせる番号（最後の桁がそろったときだけ）
    static func postalCode(_ kind: Kind, _ buffer: String) -> String? {
        switch kind {
        case .te:
            let code = String(buffer.dropFirst(tePrefix.count))
            return buffer.hasPrefix(tePrefix) && code.count == postalDigits ? code : nil
        }
    }

    static func model(_ choosing: Choosing) -> CandidatePanelModel {
        let start = choosing.selected / pageSize * pageSize
        let rows = choosing.rows[start..<min(start + pageSize, choosing.rows.count)]
        return CandidatePanelModel(
            candidates: rows.enumerated().map { offset, row in
                CandidateItem(surface: row, kind: start + offset == 0 ? .literal : .unspecified)
            },
            selected: choosing.selected - start, annotation: "", reservedRows: min(choosing.rows.count, pageSize))
    }

    private static func typingStep(_ buffer: String) -> PostalStep { PostalStep(actions: [mark(buffer)]) }

    /// 下線なし（ADR 0032 §9.1）
    private static func mark(_ text: String) -> KeyAction {
        .setMarkedText(MarkedText(text: text, highlightedLength: text.utf16.count, underlined: false))
    }
}

extension EnglishMode {
    /// 郵便番号の語をためている（変換中の文字列か候補を出している）
    public var isPostalActive: Bool { postal.isActive }

    /// 打鍵を処理する。関係なければ nil（ショートコードの口に流す）
    mutating func handlePostal(_ input: KeyInput, environment: EnglishShortcodeEnvironment) -> EnglishTextOutcome? {
        guard !shortcode.isActive else { return nil }
        guard environment.settings.postalEnglish else {
            // ためている途中でスイッチが切れた。ためた文字を確定して、このキーはアプリに渡す
            guard var step = postal.flush() else { return nil }
            step.passKey = true
            return postalOutcome(step, key: input)
        }
        let canEnter = !environment.secure && !environment.isRepeat
            && !environment.settings.emojiShortcodeExclude.contains(environment.clientBundleID)
            && Self.isWordStart(after: lastCharacter)
        guard let step = postal.handle(input, canEnter: canEnter) else { return nil }
        return postalOutcome(step, key: input)
    }

    /// 住所の返事を受ける。状態が違えば捨てる（nil）
    public mutating func postalLookupArrived(_ request: PostalLookupRequest, addresses: [String]) -> EnglishTextOutcome? {
        guard let step = postal.lookupArrived(request, addresses: addresses) else { return nil }
        return EnglishTextOutcome(actions: step.actions, passKey: false)
    }

    /// モードの切り替え、secure、`deactivateServer`、クリックのとき、ためた文字をそのまま確定する。ためていなければ nil
    public mutating func flushPostal() -> EnglishTextOutcome? {
        guard let step = postal.flush() else { return nil }
        return postalOutcome(step, key: nil)
    }

    private mutating func postalOutcome(_ step: PostalStep, key: KeyInput?) -> EnglishTextOutcome {
        var flushed: [String] = []
        if let committed = step.committed, let overflow = appendCommitted(committed) { flushed.append(overflow) }
        if step.passKey, let key, let recorded = record(key) { flushed.append(recorded) }
        return EnglishTextOutcome(actions: step.actions, passKey: step.passKey, flushed: flushed, lookup: step.lookup)
    }
}
