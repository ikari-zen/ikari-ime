// スパイク（捨てるコード。issue #27、#28。ADR 0032 §9）。英語のモードで、語頭の `te` + 数字 7 桁、`u` + 16 進 4〜6 桁を
// ためて、最後の桁を打った瞬間に候補ウィンドウを自動で出せるかを実機で確かめる（2026-10-28 のユーザー方針: 下線なし、
// ↑/↓ だけで選ぶ、Space・Tab では選ばない、数字だけの 7 桁は対象外）。検証用のフラグ（既定オフ）の裏でだけ働く。
// 住所の引き当ては固定のダミー表（公開の地名だけ）。本番の `PostalMachine` ではない。マージしない。
// ログに出すのは状態の名前、キーの種類、長さ、時間だけ。文字は出さない（NFR-19）。
import Foundation

/// 検証用の設定（殻のプロセスの UserDefaults。`defaults write <殻の bundle id> <キー> ...`）
public enum SpikePostalFlag {
    /// 有効にする（Bool。既定 false）
    public static let enabledKey = "ikariSpikePostalEnglish"
    /// ダミーの非同期の問い合わせの遅れ（ミリ秒。既定 20）。0 なら次の runloop で返す
    public static let delayMillisKey = "ikariSpikePostalDelayMs"
    public static let defaultDelayMillis = 20

    public static func isEnabled(_ defaults: UserDefaults = .standard) -> Bool {
        defaults.bool(forKey: enabledKey)
    }

    public static func delayMillis(_ defaults: UserDefaults = .standard) -> Int {
        defaults.object(forKey: delayMillisKey) == nil ? defaultDelayMillis : max(0, defaults.integer(forKey: delayMillisKey))
    }
}

/// ダミーの郵便番号表（公開の地名の数件）
public enum SpikePostalTable {
    public static let rows: [String: [String]] = [
        "1000001": ["東京都千代田区千代田"],
        "1000004": ["東京都千代田区大手町"],
        "1500002": ["東京都渋谷区渋谷"],
        "5300001": ["大阪府大阪市北区梅田"],
        "6008216": ["京都府京都市下京区東塩小路町"],
    ]

    public static func lookup(_ code: String) -> [String] { rows[code] ?? [] }
}

/// 1 打鍵の記録（文字なし）。コントローラがログに 1 行で書く
public struct SpikeTrace: Sendable, Equatable {
    public var event: String
    public var from: String
    public var to: String
    /// typing / choices / move / commit / retype / cancel / lookup_requested / lookup_arrived / lookup_stale
    public var result: String
    public var kind: String
    public var bufferLength: Int
    /// 入ってから受けたキーの数（取りこぼしの突き合わせ用）
    public var keysIn: Int
    /// 前の打鍵からのミリ秒（-1 は初回）
    public var gapMillis: Int
    /// `setMarkedText` で渡した変換中の文字列の UTF-16 の長さ（nil は渡していない）
    public var expectedMarkedLength: Int?
}

/// 1 打鍵の結果
public struct SpikeStep: Sendable, Equatable {
    public var actions: [KeyAction] = []
    public var passKey = false
    /// 確定した文字列（文脈のたまりに足す）
    public var committed: String?
    /// 非同期に問い合わせる郵便番号
    public var lookup: String?
    public var trace: SpikeTrace
}

public struct SpikePostalMachine: Sendable, Equatable {
    enum Kind: String, Sendable { case te, unicode }

    struct Choosing: Sendable, Equatable {
        var kind: Kind
        var buffer: String
        var rows: [String]
        var selected: Int
        var lookupCode: String?
    }

    enum Phase: Sendable, Equatable {
        case inactive
        case typing(Kind, String)
        case choosing(Choosing)

        var name: String {
            switch self {
            case .inactive: "inactive"
            case .typing: "typing"
            case .choosing: "choosing"
            }
        }
    }

    static let postalLength = 7
    static let hexMin = 4
    static let hexMax = 6
    static let pageSize = 9
    private static let tab = "\t"
    private static let backTab = "\u{19}"
    private static let commandModifiers: KeyModifiers = [.control, .option, .command]

    var phase: Phase = .inactive
    var keysIn = 0
    var lastKeyAt: UInt64?

    public init() {}

    public var isActive: Bool { phase != .inactive }

    // MARK: キー

    mutating func handle(_ input: KeyInput, canEnter: Bool, now: UInt64) -> SpikeStep? {
        let gap = lastKeyAt.map { Int(($0 > now ? 0 : now - $0) / 1_000_000) } ?? -1
        let from = phase.name
        let event = Self.eventName(input)
        var step: SpikeStep?
        switch phase {
        case .inactive:
            step = enter(input, canEnter: canEnter)
        case .typing(let kind, let buffer):
            step = typing(kind, buffer, input)
        case .choosing(let choosing):
            step = choosing_(choosing, input)
        }
        guard var step else { return nil }
        lastKeyAt = now
        step.trace.event = event
        step.trace.from = from
        step.trace.gapMillis = gap
        return step
    }

    private mutating func enter(_ input: KeyInput, canEnter: Bool) -> SpikeStep? {
        guard canEnter, input.modifiers.intersection(Self.commandModifiers).isEmpty,
            case .character(let text) = input.key, text.count == 1, let c = text.first
        else { return nil }
        let kind: Kind
        if text == "t" {
            kind = .te
        } else if text == "u" {
            kind = .unicode
        } else {
            return nil
        }
        keysIn = 1
        phase = .typing(kind, text)
        return typingStep(kind, text, result: "typing", from: "inactive")
    }

    private mutating func typing(_ kind: Kind, _ buffer: String, _ input: KeyInput) -> SpikeStep {
        let plain = input.modifiers.intersection(Self.commandModifiers).isEmpty
        if plain {
            switch input.key {
            case .character(let text) where text.count == 1 && text != Self.tab && text != Self.backTab:
                if let next = Self.append(text, to: buffer, kind: kind) {
                    keysIn += 1
                    return settle(kind, next, from: "typing")
                }
            case .backspace:
                keysIn += 1
                var shorter = buffer
                shorter.removeLast()
                if shorter.isEmpty {
                    phase = .inactive
                    return SpikeStep(
                        actions: [Self.mark("", underlined: false)], passKey: false,
                        trace: trace(to: "inactive", result: "cancel", kind: kind, length: 0, marked: 0))
                }
                phase = .typing(kind, shorter)
                return typingStep(kind, shorter, result: "typing", from: "typing")
            default:
                break
            }
        }
        return retype(kind: kind, buffer: buffer, hide: false)
    }

    private mutating func choosing_(_ choosing: Choosing, _ input: KeyInput) -> SpikeStep {
        let plain = input.modifiers.intersection(Self.commandModifiers).isEmpty
        guard plain else { return retype(kind: choosing.kind, buffer: choosing.buffer, hide: true) }
        keysIn += 1
        switch input.key {
        case .down:
            return move(choosing, by: 1)
        case .up:
            return move(choosing, by: -1)
        case .enter:
            return commit(choosing, index: choosing.selected)
        case .escape:
            var step = retype(kind: choosing.kind, buffer: choosing.buffer, hide: true)
            step.passKey = false
            step.trace.result = "dismiss"
            return step
        case .backspace:
            var shorter = choosing.buffer
            shorter.removeLast()
            var step = settle(choosing.kind, shorter, from: "choosing")
            if case .typing = phase { step.actions.append(.hideCandidates) }
            return step
        case .character(let text) where text.count == 1 && text != Self.tab && text != Self.backTab:
            if let next = Self.append(text, to: choosing.buffer, kind: choosing.kind),
                Self.baseRows(choosing.kind, next) != nil
            {
                return settle(choosing.kind, next, from: "choosing")
            }
            return retype(kind: choosing.kind, buffer: choosing.buffer, hide: true)
        default:
            return retype(kind: choosing.kind, buffer: choosing.buffer, hide: true)
        }
    }

    // MARK: 遷移の部品

    /// 形がそろったら候補を出し、そろっていなければ「ためている状態」にする
    private mutating func settle(_ kind: Kind, _ buffer: String, from: String) -> SpikeStep {
        if let opened = open(kind, buffer) { return opened }
        phase = .typing(kind, buffer)
        return typingStep(kind, buffer, result: "typing", from: from)
    }

    private mutating func open(_ kind: Kind, _ buffer: String) -> SpikeStep? {
        guard let rows = Self.baseRows(kind, buffer) else { return nil }
        let code = Self.postalCode(kind, buffer)
        let choosing = Choosing(kind: kind, buffer: buffer, rows: rows, selected: 0, lookupCode: code)
        phase = .choosing(choosing)
        var step = SpikeStep(
            actions: [Self.mark(rows[0], underlined: false), .showCandidates(Self.model(choosing))], passKey: false,
            trace: trace(to: "choosing", result: "choices", kind: kind, length: buffer.count, marked: rows[0].utf16.count))
        step.lookup = code
        return step
    }

    private mutating func move(_ choosing: Choosing, by delta: Int) -> SpikeStep {
        var next = choosing
        let count = max(choosing.rows.count, 1)
        next.selected = (choosing.selected + delta + count) % count
        phase = .choosing(next)
        let samePage = next.selected / Self.pageSize == choosing.selected / Self.pageSize
        let model = Self.model(next)
        return SpikeStep(
            actions: [Self.mark(next.rows[next.selected], underlined: false), samePage ? .moveSelection(model) : .showCandidates(model)],
            passKey: false,
            trace: trace(
                to: "choosing", result: "move", kind: next.kind, length: next.buffer.count,
                marked: next.rows[next.selected].utf16.count))
    }

    private mutating func commit(_ choosing: Choosing, index: Int) -> SpikeStep {
        let text = choosing.rows[index]
        phase = .inactive
        var step = SpikeStep(
            actions: [.commit(text), .hideCandidates], passKey: false,
            trace: trace(to: "inactive", result: "commit", kind: choosing.kind, length: text.count, marked: nil))
        step.committed = text
        return step
    }

    /// 形から外れた。見えていた文字列を確定し、今のキーだけアプリに渡す（ADR 0032 §9）
    private mutating func retype(kind: Kind, buffer: String, hide: Bool) -> SpikeStep {
        phase = .inactive
        var actions: [KeyAction] = [.commit(buffer)]
        if hide { actions.append(.hideCandidates) }
        var step = SpikeStep(
            actions: actions, passKey: true,
            trace: trace(to: "inactive", result: "retype", kind: kind, length: buffer.count, marked: nil))
        step.committed = buffer
        return step
    }

    private func typingStep(_ kind: Kind, _ buffer: String, result: String, from: String) -> SpikeStep {
        SpikeStep(
            actions: [Self.mark(buffer, underlined: false)], passKey: false,
            trace: trace(to: "typing", result: result, kind: kind, length: buffer.count, marked: buffer.utf16.count))
    }

    private func trace(to: String, result: String, kind: Kind, length: Int, marked: Int?) -> SpikeTrace {
        SpikeTrace(
            event: "", from: "", to: to, result: result, kind: kind.rawValue, bufferLength: length, keysIn: keysIn,
            gapMillis: -1, expectedMarkedLength: marked)
    }

    // MARK: 非同期の返事

    /// ダミーの問い合わせの返事。同じ状態（候補を出している、同じ郵便番号）のときだけ候補に足す
    mutating func lookupArrived(code: String, addresses: [String]) -> SpikeStep {
        guard case .choosing(var choosing) = phase, choosing.lookupCode == code else {
            return SpikeStep(
                trace: SpikeTrace(
                    event: "lookup", from: phase.name, to: phase.name, result: "lookup_stale", kind: "-",
                    bufferLength: 0, keysIn: keysIn, gapMillis: -1, expectedMarkedLength: nil))
        }
        choosing.rows += addresses
        choosing.lookupCode = nil
        phase = .choosing(choosing)
        return SpikeStep(
            actions: [.showCandidates(Self.model(choosing))],
            trace: SpikeTrace(
                event: "lookup", from: "choosing", to: "choosing", result: "lookup_arrived", kind: choosing.kind.rawValue,
                bufferLength: choosing.buffer.count, keysIn: keysIn, gapMillis: -1, expectedMarkedLength: nil))
    }

    /// 状態を捨てる（モードの切り替え、secure、`deactivateServer` など）。見えている文字列を確定する
    mutating func flush() -> SpikeStep? {
        switch phase {
        case .inactive:
            return nil
        case .typing(let kind, let buffer):
            return retypeFlush(kind: kind, buffer: buffer, hide: false)
        case .choosing(let choosing):
            return retypeFlush(kind: choosing.kind, buffer: choosing.buffer, hide: true)
        }
    }

    private mutating func retypeFlush(kind: Kind, buffer: String, hide: Bool) -> SpikeStep {
        let from = phase.name
        var step = retype(kind: kind, buffer: buffer, hide: hide)
        step.passKey = false
        step.trace.event = "flush"
        step.trace.from = from
        return step
    }

    // MARK: 純関数

    static func append(_ text: String, to buffer: String, kind: Kind) -> String? {
        guard let c = text.first else { return nil }
        switch kind {
        case .te:
            if buffer == "t" { return text == "e" ? "te" : nil }
            return c.isASCII && c.isNumber && buffer.count - 2 < postalLength ? buffer + text : nil
        case .unicode:
            if buffer == "u", text == "+" { return "u+" }
            let digits = buffer.dropFirst(buffer.hasPrefix("u+") ? 2 : 1)
            return c.isASCII && c.isHexDigit && digits.count < hexMax ? buffer + text : nil
        }
    }

    /// 最後の桁がそろった形か（te は 7 桁、u は 4 桁以上）。そろっているなら、問い合わせ前の候補（0 番は「そのままの入力」）
    static func baseRows(_ kind: Kind, _ buffer: String) -> [String]? {
        switch kind {
        case .te:
            let code = String(buffer.dropFirst(2))
            guard code.count == postalLength else { return nil }
            let hyphenated = "\(code.prefix(3))-\(code.suffix(4))"
            return [buffer, "〒" + hyphenated, "〒" + code]
        case .unicode:
            let digits = buffer.dropFirst(buffer.hasPrefix("u+") ? 2 : 1)
            guard digits.count >= hexMin, let value = UInt32(digits, radix: 16), let scalar = Unicode.Scalar(value) else {
                return nil
            }
            return [buffer, String(Character(scalar))]
        }
    }

    static func postalCode(_ kind: Kind, _ buffer: String) -> String? {
        switch kind {
        case .te: String(buffer.dropFirst(2))
        case .unicode: nil
        }
    }

    static func model(_ choosing: Choosing) -> CandidatePanelModel {
        let page = choosing.selected / pageSize
        let start = page * pageSize
        let rows = choosing.rows[start..<min(start + pageSize, choosing.rows.count)]
        return CandidatePanelModel(
            candidates: rows.enumerated().map { offset, row in
                CandidateItem(surface: row, kind: start + offset == 0 ? .literal : .unspecified)
            },
            selected: choosing.selected - start, annotation: "", reservedRows: min(choosing.rows.count, pageSize))
    }

    static func eventName(_ input: KeyInput) -> String {
        switch input.key {
        case .character(let text):
            if text == tab { return input.modifiers.contains(.shift) ? "backtab" : "tab" }
            if text == backTab { return "backtab" }
            guard text.count == 1, let c = text.first else { return "other_char" }
            if c.isASCII, c.isNumber { return "digit" }
            if text == "t" { return "t" }
            if text == "e" { return "e" }
            if text == "u" { return "u" }
            if text == "+" { return "plus" }
            return c.isASCII && c.isHexDigit ? "hex" : "other_char"
        case .space: return "space"
        case .enter: return "enter"
        case .escape: return "esc"
        case .backspace: return "bs"
        case .left, .right: return "horizontal"
        case .up: return "up"
        case .down: return "down"
        case .function, .jis, .other: return "other"
        }
    }

    private static func mark(_ text: String, underlined: Bool) -> KeyAction {
        .setMarkedText(MarkedText(text: text, highlightedLength: text.utf16.count, underlined: underlined))
    }
}

extension EnglishMode {
    /// 検証用: ためている状態か
    public var isSpikePostalActive: Bool { spikePostal.isActive }

    /// 検証用: 打鍵を処理する。関係なければ nil（ショートコードの口に流す）
    mutating func handleSpikePostal(_ input: KeyInput, environment: EnglishShortcodeEnvironment, now: UInt64) -> EnglishTextOutcome? {
        guard environment.spikePostal, !shortcode.isActive else { return nil }
        let canEnter = !environment.secure && !environment.isRepeat
            && !environment.settings.emojiShortcodeExclude.contains(environment.clientBundleID)
            && Self.isWordStart(after: lastCharacter)
        guard let step = spikePostal.handle(input, canEnter: canEnter, now: now) else { return nil }
        return spikeOutcome(step, key: input)
    }

    /// 検証用: ダミーの問い合わせの返事を受ける。記録は `takeSpikeStep` で取る
    public mutating func spikePostalLookupArrived(code: String, addresses: [String]) -> EnglishTextOutcome {
        let step = spikePostal.lookupArrived(code: code, addresses: addresses)
        lastSpikeStep = step
        return EnglishTextOutcome(actions: step.actions, passKey: false)
    }

    /// 検証用: モードの切り替えなどで状態を捨てる
    public mutating func flushSpikePostal() -> EnglishTextOutcome? {
        guard let step = spikePostal.flush() else { return nil }
        return spikeOutcome(step, key: nil)
    }

    /// 直近の処理が作った記録（コントローラが取り出してログに書く）
    public mutating func takeSpikeStep() -> SpikeStep? {
        defer { lastSpikeStep = nil }
        return lastSpikeStep
    }

    private mutating func spikeOutcome(_ step: SpikeStep, key: KeyInput?) -> EnglishTextOutcome {
        lastSpikeStep = step
        var flushed: [String] = []
        if let committed = step.committed, let overflow = appendCommitted(committed) { flushed.append(overflow) }
        if step.passKey, let key, let recorded = record(key) { flushed.append(recorded) }
        return EnglishTextOutcome(actions: step.actions, passKey: step.passKey, flushed: flushed)
    }
}
