// 共通のシナリオ（shell-core/scenarios/*.json。ADR 0041 §5）の Swift の読み手。移行の間だけ使い、M5 で消す。
// 形の正は Rust の serde の型（shell-core/src/scenario.rs、types.rs）。説明は shell-core/scenarios/README.md。
import Foundation

@testable import IkariShellCore

// MARK: ファイルと準備

struct ScenarioFile: Decodable {
    var description: String?
    var defaults: ScenarioSetup?
    var scenarios: [Scenario]
}

struct ScenarioSetup: Decodable {
    var settings: [String]?
    var client: ScenarioClient?
    var engine: ScenarioEngineScript?
    var surrounding: ScenarioSurrounding?

    /// `scenario` を欄ごとに重ね、残りを既定で埋める（Rust の `Setup::resolve` と同じ規則）
    func resolve(_ scenario: ScenarioSetup?) -> ResolvedScenarioSetup {
        ResolvedScenarioSetup(
            settings: scenario?.settings ?? settings ?? [],
            client: scenario?.client ?? client ?? ScenarioClient(),
            engine: scenario?.engine ?? engine ?? ScenarioEngineScript(),
            surrounding: scenario?.surrounding ?? surrounding ?? .unavailable)
    }
}

struct ResolvedScenarioSetup {
    var settings: [String]
    var client: ScenarioClient
    var engine: ScenarioEngineScript
    var surrounding: ScenarioSurrounding
}

struct ScenarioClient: Decodable {
    var bundleID = "com.apple.TextEdit"
    var secure = false

    init() {}

    enum CodingKeys: String, CodingKey {
        case bundleID = "bundle_id"
        case secure
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        bundleID = try c.decodeIfPresent(String.self, forKey: .bundleID) ?? bundleID
        secure = try c.decodeIfPresent(Bool.self, forKey: .secure) ?? false
    }
}

struct ScenarioSurrounding: Decodable {
    var left = ""
    var right = ""
    var documentLength: Int?
    var available = true

    static let unavailable: ScenarioSurrounding = {
        var value = ScenarioSurrounding()
        value.available = false
        return value
    }()

    init() {}

    enum CodingKeys: String, CodingKey {
        case left, right, available
        case documentLength = "document_length"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        left = try c.decodeIfPresent(String.self, forKey: .left) ?? ""
        right = try c.decodeIfPresent(String.self, forKey: .right) ?? ""
        documentLength = try c.decodeIfPresent(Int.self, forKey: .documentLength)
        available = try c.decodeIfPresent(Bool.self, forKey: .available) ?? true
    }

    /// 今の `ClientAccess` が状態機械に渡す値（取れない → unavailable。左が空で文書の長さも 0 → unavailable）
    var machineValue: SurroundingText {
        guard available else { return .unavailable }
        let length = documentLength ?? (left.utf16.count + right.utf16.count)
        guard !(left.isEmpty && length <= 0) else { return .unavailable }
        return SurroundingText(left: left, right: right, available: true)
    }
}

struct ScenarioEngineScript: Decodable, Sendable {
    var pages: [ScenarioPage] = []
    var failures: [ScenarioFailure] = []
    var segmentPages: [ScenarioSegmentPage] = []
    var resizes: [ScenarioResize] = []
    var reconvert: ScenarioLayout?
    var emojiTable: [ScenarioEmoji] = []
    var postal: [String: [String]] = [:]

    init() {}

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        pages = try c.decodeIfPresent([ScenarioPage].self, forKey: .pages) ?? []
        failures = try c.decodeIfPresent([ScenarioFailure].self, forKey: .failures) ?? []
        segmentPages = try c.decodeIfPresent([ScenarioSegmentPage].self, forKey: .segmentPages) ?? []
        resizes = try c.decodeIfPresent([ScenarioResize].self, forKey: .resizes) ?? []
        reconvert = try c.decodeIfPresent(ScenarioLayout.self, forKey: .reconvert)
        emojiTable = try c.decodeIfPresent([ScenarioEmoji].self, forKey: .emojiTable) ?? []
        postal = try c.decodeIfPresent([String: [String]].self, forKey: .postal) ?? [:]
    }

    enum CodingKeys: String, CodingKey {
        case pages, failures, reconvert, postal, resizes
        case segmentPages = "segment_pages"
        case emojiTable = "emoji_table"
    }

    /// `segment_page` の返事（Rust の `EngineScript::segment_page` と同じ規則）
    func segmentPage(segment: Int, index: Int) -> SegmentPage? {
        segmentPages.first { $0.segment == segment && $0.pageIndex == index }.map {
            SegmentPage(
                segment: $0.segment, pageIndex: $0.pageIndex, candidates: $0.candidates.map(\.item), hasNext: $0.hasNext)
        }
    }

    /// `resize_segment` の返事（Rust の `EngineScript::resize` と同じ規則）
    func resize(segment: Int, delta: Int, layoutRev: UInt32) -> SegmentLayoutInfo? {
        resizes.first { $0.segment == segment && $0.delta == delta && ($0.layoutRev.map { $0 == layoutRev } ?? true) }?
            .layout.value
    }

    func failure(for request: String, nth: Int) -> EngineClientError? {
        failures.first { $0.request == request && $0.nth == nth }?.error
    }
}

struct ScenarioPage: Decodable, Sendable {
    var conversionID: UInt64
    var pageIndex: Int
    var candidates: [ScenarioCandidate]
    var hasNext: Bool
    var lmApplied: Bool
    var confident: Bool
    var layout: ScenarioLayout?

    enum CodingKeys: String, CodingKey {
        case conversionID = "conversion_id"
        case pageIndex = "page_index"
        case candidates
        case hasNext = "has_next"
        case lmApplied = "lm_applied"
        case confident, layout
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        conversionID = try c.decode(UInt64.self, forKey: .conversionID)
        pageIndex = try c.decode(Int.self, forKey: .pageIndex)
        candidates = try c.decode([ScenarioCandidate].self, forKey: .candidates)
        hasNext = try c.decode(Bool.self, forKey: .hasNext)
        lmApplied = try c.decodeIfPresent(Bool.self, forKey: .lmApplied) ?? false
        confident = try c.decodeIfPresent(Bool.self, forKey: .confident) ?? false
        layout = try c.decodeIfPresent(ScenarioLayout.self, forKey: .layout)
    }

    var conversionPage: ConversionPage {
        ConversionPage(
            conversionID: conversionID, pageIndex: pageIndex, candidates: candidates.map(\.item), hasNext: hasNext,
            lmApplied: lmApplied, confident: confident, layout: layout?.value)
    }
}

/// 区切り（Rust の `ScriptLayout`）
struct ScenarioLayout: Decodable, Sendable {
    var value: SegmentLayoutInfo

    enum CodingKeys: String, CodingKey {
        case conversionID = "conversion_id"
        case layoutRev = "layout_rev"
        case segments, focused
        case focusedPage = "focused_page"
    }

    struct Segment: Decodable {
        var reading: String
        var surface: String
    }

    struct Candidates: Decodable {
        var pageIndex: Int
        var candidates: [ScenarioCandidate]
        var hasNext: Bool

        enum CodingKeys: String, CodingKey {
            case pageIndex = "page_index"
            case candidates
            case hasNext = "has_next"
        }

        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            pageIndex = try c.decodeIfPresent(Int.self, forKey: .pageIndex) ?? 0
            candidates = try c.decode([ScenarioCandidate].self, forKey: .candidates)
            hasNext = try c.decodeIfPresent(Bool.self, forKey: .hasNext) ?? false
        }
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let focused = try c.decodeIfPresent(Int.self, forKey: .focused) ?? 0
        let page = try c.decodeIfPresent(Candidates.self, forKey: .focusedPage)
        value = SegmentLayoutInfo(
            conversionID: try c.decode(UInt64.self, forKey: .conversionID),
            layoutRev: try c.decodeIfPresent(UInt32.self, forKey: .layoutRev) ?? 0,
            segments: try c.decode([Segment].self, forKey: .segments).map {
                SegmentSummary(reading: $0.reading, surface: $0.surface)
            },
            focused: focused,
            focusedPage: page.map {
                SegmentPage(
                    segment: focused, pageIndex: $0.pageIndex, candidates: $0.candidates.map(\.item), hasNext: $0.hasNext)
            })
    }
}

/// `segment_page` の返事（Rust の `ScriptSegmentPage`）
struct ScenarioSegmentPage: Decodable, Sendable {
    var segment: Int
    var pageIndex: Int
    var candidates: [ScenarioCandidate]
    var hasNext: Bool

    enum CodingKeys: String, CodingKey {
        case segment, candidates
        case pageIndex = "page_index"
        case hasNext = "has_next"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        segment = try c.decode(Int.self, forKey: .segment)
        pageIndex = try c.decodeIfPresent(Int.self, forKey: .pageIndex) ?? 0
        candidates = try c.decode([ScenarioCandidate].self, forKey: .candidates)
        hasNext = try c.decodeIfPresent(Bool.self, forKey: .hasNext) ?? false
    }
}

/// `resize_segment` の返事（Rust の `ScriptResize`）
struct ScenarioResize: Decodable, Sendable {
    var segment: Int
    var delta: Int
    var layoutRev: UInt32?
    var layout: ScenarioLayout

    enum CodingKeys: String, CodingKey {
        case segment, delta, layout
        case layoutRev = "layout_rev"
    }
}

struct ScenarioEmoji: Decodable, Sendable {
    var name: String
    var emoji: String
}

struct ScenarioFailure: Decodable, Sendable {
    var request: String
    var nth: Int
    var error: EngineClientError

    enum CodingKeys: String, CodingKey { case request, nth, error }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        request = try c.decode(String.self, forKey: .request)
        nth = try c.decode(Int.self, forKey: .nth)
        error = try c.decode(ScenarioEngineError.self, forKey: .error).value
    }
}

/// `"unavailable"`、`"timeout"`、`"invalid_reply"`、`{"engine": "bad_request"}`
struct ScenarioEngineError: Decodable {
    var value: EngineClientError

    init(from decoder: Decoder) throws {
        if let name = try? decoder.singleValueContainer().decode(String.self) {
            switch name {
            case "unavailable": value = .unavailable
            case "timeout": value = .timeout
            case "invalid_reply": value = .invalidReply
            default: throw ScenarioFormatError("unknown engine error \(name)")
            }
            return
        }
        let object = try decoder.singleValueContainer().decode([String: String].self)
        guard let code = object["engine"] else { throw ScenarioFormatError("bad engine error \(object)") }
        let codes: [String: EngineErrorCode] = [
            "unspecified": .unspecified, "schema_mismatch": .schemaMismatch, "bad_request": .badRequest,
            "unknown_session": .unknownSession, "unknown_conversion": .unknownConversion,
            "engine_not_ready": .engineNotReady, "internal": .internal, "stale_layout": .staleLayout,
        ]
        guard let mapped = codes[code] else { throw ScenarioFormatError("unknown engine error code \(code)") }
        value = .engine(mapped)
    }
}

struct ScenarioFormatError: Error, CustomStringConvertible {
    var description: String
    init(_ description: String) { self.description = description }
}

// MARK: シナリオと手順

struct Scenario: Decodable {
    var name: String
    var from: String?
    /// Rust の実行器だけで流す理由（Swift の実行器は飛ばす）
    var rustOnly: String?
    var setup: ScenarioSetup?
    var steps: [ScenarioStep]

    enum CodingKeys: String, CodingKey {
        case name, from, setup, steps
        case rustOnly = "rust_only"
    }
}

/// `ShellEvent` で表せない OS とのやりとり（Rust の `HostCall`）。Swift の状態機械で流せるものだけ
enum ScenarioHostCall: Decodable {
    case commitComposition
    case panelShown(PanelSide)
    case panelHidden
    case unsupported(String)

    enum CodingKeys: String, CodingKey { case type, side }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        switch try c.decode(String.self, forKey: .type) {
        case "commit_composition": self = .commitComposition
        case "panel_hidden": self = .panelHidden
        case "panel_shown":
            switch try c.decode(String.self, forKey: .side) {
            case "above": self = .panelShown(.above)
            case "below": self = .panelShown(.below)
            case let side: throw ScenarioFormatError("unknown panel side \(side)")
            }
        case let type: self = .unsupported(type)
        }
    }
}

struct ScenarioStep: Decodable {
    var note: String?
    var event: ScenarioEvent?
    var press: String?
    var typeText: String?
    var surrounding: ScenarioSurrounding?
    var selection: ScenarioSelection?
    var expect: [ScenarioAction]?
    var engine: [[String: JSONValue]]?
    var host: ScenarioHostCall?
    var modeRequest: String?

    enum CodingKeys: String, CodingKey {
        case note, event, press, surrounding, selection, expect, engine, host
        case typeText = "type_text"
        case modeRequest = "mode_request"
    }

    /// この手順で状態機械に渡すキー（Rust の `Step::events` と同じ規則）。`host` の手順は空
    func keys() throws -> [KeyInput] {
        let given = [event != nil, press != nil, typeText != nil, host != nil].filter { $0 }.count
        guard given == 1 else {
            throw ScenarioFormatError("a step needs exactly one of event, press, type_text, host")
        }
        if host != nil { return [] }
        if let event {
            guard case .key(let input) = event else {
                throw ScenarioFormatError("the Swift runner handles only key events")
            }
            return [input]
        }
        if let press { return [try ScenarioPress.parse(press)] }
        let text = typeText ?? ""
        guard !text.isEmpty else { throw ScenarioFormatError("type_text is empty") }
        return text.map { KeyInput(key: .character(String($0))) }
    }
}

/// `ShellEvent`。Swift の状態機械が受けるのはキーだけ（入力先は `client` で決める）
enum ScenarioEvent: Decodable {
    case key(KeyInput)
    case unsupported(String)

    enum CodingKeys: String, CodingKey { case type, key, modifiers }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let type = try c.decode(String.self, forKey: .type)
        guard type == "key" else {
            self = .unsupported(type)
            return
        }
        let key = try c.decode(ScenarioKeyKind.self, forKey: .key).value
        let modifiers = try c.decodeIfPresent(ScenarioModifiers.self, forKey: .modifiers)?.value ?? []
        self = .key(KeyInput(key: key, modifiers: modifiers))
    }
}

struct ScenarioKeyKind: Decodable {
    var value: KeyInput.Key

    enum CodingKeys: String, CodingKey { case kind, text, code, number }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        switch try c.decode(String.self, forKey: .kind) {
        case "character": value = .character(try c.decode(String.self, forKey: .text))
        case "jis": value = .jis(try c.decode(UInt8.self, forKey: .code))
        case "space": value = .space
        case "enter": value = .enter
        case "escape": value = .escape
        case "backspace": value = .backspace
        case "left": value = .left
        case "right": value = .right
        case "up": value = .up
        case "down": value = .down
        case "function": value = .function(try c.decode(Int.self, forKey: .number))
        case "other": value = .other
        case let kind: throw ScenarioFormatError("unknown key kind \(kind)")
        }
    }
}

struct ScenarioModifiers: Decodable {
    var value: KeyModifiers = []

    enum CodingKeys: String, CodingKey { case shift, control, option, command }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        if try c.decodeIfPresent(Bool.self, forKey: .shift) == true { value.insert(.shift) }
        if try c.decodeIfPresent(Bool.self, forKey: .control) == true { value.insert(.control) }
        if try c.decodeIfPresent(Bool.self, forKey: .option) == true { value.insert(.option) }
        if try c.decodeIfPresent(Bool.self, forKey: .command) == true { value.insert(.command) }
    }
}

/// `press` の書式（Rust の `scenario::parse_press` と同じ規則）
enum ScenarioPress {
    static func parse(_ spec: String) throws -> KeyInput {
        let prefix: String?
        let key: String
        if spec.hasSuffix("++") {
            prefix = String(spec.dropLast(2))
            key = "+"
        } else if spec == "+" {
            prefix = nil
            key = "+"
        } else if let plus = spec.lastIndex(of: "+") {
            prefix = String(spec[..<plus])
            key = String(spec[spec.index(after: plus)...])
        } else {
            prefix = nil
            key = spec
        }
        var modifiers: KeyModifiers = []
        for name in prefix.map({ $0.split(separator: "+", omittingEmptySubsequences: false) }) ?? [] {
            switch name {
            case "shift": modifiers.insert(.shift)
            case "ctrl", "control": modifiers.insert(.control)
            case "option", "alt": modifiers.insert(.option)
            case "cmd", "command": modifiers.insert(.command)
            default: throw ScenarioFormatError("unknown modifier \(name) in \(spec)")
            }
        }
        let named: [String: KeyInput.Key] = [
            "space": .space, "enter": .enter, "escape": .escape, "backspace": .backspace, "left": .left,
            "right": .right, "up": .up, "down": .down, "other": .other,
        ]
        if let value = named[key] { return KeyInput(key: value, modifiers: modifiers) }
        if key.hasPrefix("jis:"), let code = UInt8(key.dropFirst(4)) {
            return KeyInput(key: .jis(code), modifiers: modifiers)
        }
        if key.hasPrefix("f"), let number = Int(key.dropFirst()), (1...20).contains(number) {
            return KeyInput(key: .function(number), modifiers: modifiers)
        }
        guard key.count == 1 else { throw ScenarioFormatError("unknown key \(key) in \(spec)") }
        return KeyInput(key: .character(key), modifiers: modifiers)
    }
}

struct ScenarioSelection: Decodable {
    var range: ScenarioRange
    var text: String

    var machineValue: SelectedText {
        SelectedText(range: TextRange(location: range.location, length: range.length), text: text)
    }
}

struct ScenarioRange: Decodable, Equatable {
    var location: Int
    var length: Int
}

// MARK: 動作

struct ScenarioMarkedText: Decodable, Equatable {
    var text: String
    var highlightedLength = 0
    var segments: [Int] = []
    var focused: Int?
    var replacementRange: ScenarioRange?
    var underlined = true

    enum CodingKeys: String, CodingKey {
        case text, segments, focused, underlined
        case highlightedLength = "highlighted_length"
        case replacementRange = "replacement_range"
    }

    init(_ marked: MarkedText) {
        text = marked.text
        highlightedLength = marked.highlightedLength
        segments = marked.segments
        focused = marked.focused
        replacementRange = marked.replacementRange.map { ScenarioRange(location: $0.location, length: $0.length) }
        underlined = marked.underlined
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        text = try c.decode(String.self, forKey: .text)
        highlightedLength = try c.decodeIfPresent(Int.self, forKey: .highlightedLength) ?? 0
        segments = try c.decodeIfPresent([Int].self, forKey: .segments) ?? []
        focused = try c.decodeIfPresent(Int.self, forKey: .focused)
        replacementRange = try c.decodeIfPresent(ScenarioRange.self, forKey: .replacementRange)
        underlined = try c.decodeIfPresent(Bool.self, forKey: .underlined) ?? true
    }
}

struct ScenarioCandidate: Decodable, Equatable, Sendable {
    var surface: String
    var annotation = ""
    var kind = "dictionary"
    var label = ""

    enum CodingKeys: String, CodingKey { case surface, annotation, kind, label }

    private static let kinds: [(CandidateKind, String)] = [
        (.unspecified, "unspecified"), (.dictionary, "dictionary"), (.hiragana, "hiragana"),
        (.katakana, "katakana"), (.rewriter, "rewriter"), (.literal, "literal"),
    ]

    init(_ item: CandidateItem) {
        surface = item.surface
        annotation = item.annotation
        kind = Self.kinds.first { $0.0 == item.kind }?.1 ?? "unspecified"
        label = item.label
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        surface = try c.decode(String.self, forKey: .surface)
        annotation = try c.decodeIfPresent(String.self, forKey: .annotation) ?? ""
        kind = try c.decodeIfPresent(String.self, forKey: .kind) ?? "dictionary"
        label = try c.decodeIfPresent(String.self, forKey: .label) ?? ""
    }

    var item: CandidateItem {
        CandidateItem(
            surface: surface, annotation: annotation, kind: Self.kinds.first { $0.1 == kind }?.0 ?? .unspecified,
            label: label)
    }
}

struct ScenarioPanel: Decodable, Equatable {
    var candidates: [ScenarioCandidate]
    var selected: Int
    var annotation = ""
    var reservedRows: Int

    enum CodingKeys: String, CodingKey {
        case candidates, selected, annotation
        case reservedRows = "reserved_rows"
    }

    init(_ model: CandidatePanelModel) {
        candidates = model.candidates.map(ScenarioCandidate.init)
        selected = model.selected
        annotation = model.annotation
        reservedRows = model.reservedRows
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        candidates = try c.decode([ScenarioCandidate].self, forKey: .candidates)
        selected = try c.decode(Int.self, forKey: .selected)
        annotation = try c.decodeIfPresent(String.self, forKey: .annotation) ?? ""
        reservedRows = try c.decode(Int.self, forKey: .reservedRows)
    }
}

/// `ShellAction`。Swift の状態機械が返さない種類（メニュー、タイマー、Bridge、設定を開く）は `unsupported`
enum ScenarioAction: Decodable, Equatable {
    case passThrough
    case consume
    case setMarkedText(ScenarioMarkedText)
    case commit(String)
    case showCandidates(ScenarioPanel)
    case moveSelection(ScenarioPanel)
    case hideCandidates
    case restore(String, replacing: ScenarioRange)
    case resolveStrayMarkedText
    case prefetch(String)
    case unsupported(String)

    enum CodingKeys: String, CodingKey { case type, text, replacing, reading }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        switch try c.decode(String.self, forKey: .type) {
        case "pass_through": self = .passThrough
        case "consume": self = .consume
        case "set_marked_text": self = .setMarkedText(try ScenarioMarkedText(from: decoder))
        case "commit": self = .commit(try c.decode(String.self, forKey: .text))
        case "show_candidates": self = .showCandidates(try ScenarioPanel(from: decoder))
        case "move_selection": self = .moveSelection(try ScenarioPanel(from: decoder))
        case "hide_candidates": self = .hideCandidates
        case "restore":
            self = .restore(
                try c.decode(String.self, forKey: .text), replacing: try c.decode(ScenarioRange.self, forKey: .replacing))
        case "resolve_stray_marked_text": self = .resolveStrayMarkedText
        case "prefetch": self = .prefetch(try c.decode(String.self, forKey: .reading))
        case let type: self = .unsupported(type)
        }
    }

    init(_ action: KeyAction) {
        switch action {
        case .passThrough: self = .passThrough
        case .consume: self = .consume
        case .setMarkedText(let marked): self = .setMarkedText(ScenarioMarkedText(marked))
        case .commit(let text): self = .commit(text)
        case .showCandidates(let model): self = .showCandidates(ScenarioPanel(model))
        case .moveSelection(let model): self = .moveSelection(ScenarioPanel(model))
        case .hideCandidates: self = .hideCandidates
        case .restore(let text, let range):
            self = .restore(text, replacing: ScenarioRange(location: range.location, length: range.length))
        case .resolveStrayMarkedText: self = .resolveStrayMarkedText
        case .prefetch(let reading): self = .prefetch(reading)
        }
    }
}

// MARK: エンジンへの要求

/// JSON の値（エンジンへの要求の期待。`type` と、書いた欄だけを比べる）
enum JSONValue: Decodable, Equatable, CustomStringConvertible {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() {
            self = .null
        } else if let value = try? c.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? c.decode(Double.self) {
            self = .number(value)
        } else if let value = try? c.decode(String.self) {
            self = .string(value)
        } else if let value = try? c.decode([JSONValue].self) {
            self = .array(value)
        } else {
            self = .object(try c.decode([String: JSONValue].self))
        }
    }

    var description: String {
        switch self {
        case .null: "null"
        case .bool(let value): "\(value)"
        case .number(let value): value == value.rounded() ? "\(Int64(value))" : "\(value)"
        case .string(let value): "\"\(value)\""
        case .array(let values): "[" + values.map(\.description).joined(separator: ",") + "]"
        case .object(let fields):
            "{" + fields.sorted { $0.key < $1.key }.map { "\"\($0.key)\":\($0.value)" }.joined(separator: ",") + "}"
        }
    }

    /// `expected`（`self`）の欄がすべて `actual` と等しいか（Rust の `scenario::request_matches` と同じ規則）
    func matches(_ actual: JSONValue) -> Bool {
        switch (self, actual) {
        case (.object(let expected), .object(let actual)):
            return expected.allSatisfy { key, value in actual[key].map { value.matches($0) } ?? false }
        case (.array(let expected), .array(let actual)):
            return expected.count == actual.count && zip(expected, actual).allSatisfy { $0.matches($1) }
        default:
            return self == actual
        }
    }
}

extension MockEngine.Call {
    /// `engine.proto` の欄の名前で書いた要求（Rust の実行器の `describe` と同じ形）
    var scenarioValue: JSONValue {
        func ms(_ duration: Duration) -> JSONValue {
            let (seconds, attoseconds) = duration.components
            return .number(Double(seconds) * 1000 + Double(attoseconds / 1_000_000_000_000_000))
        }
        func context(_ text: SurroundingText) -> JSONValue {
            .object(["left": .string(text.left), "right": .string(text.right), "available": .bool(text.available)])
        }
        func choices(_ values: [SegmentChoiceValue]) -> JSONValue {
            .array(
                values.map {
                    switch $0 {
                    case .candidate(let index): .object(["candidate_index": .number(Double(index))])
                    case .literal(let text): .object(["literal": .string(text)])
                    }
                })
        }
        func n<T: BinaryInteger>(_ value: T) -> JSONValue { .number(Double(value)) }
        var fields: [String: JSONValue]
        switch self {
        case .hello:
            fields = ["type": .string("hello")]
        case .beginSession(let session):
            fields = [
                "type": .string("begin_session"), "session_id": n(session.sessionID),
                "client_bundle_id": .string(session.clientBundleID), "secure": .bool(session.secure),
                "share_context": .bool(session.shareContext),
            ]
        case .endSession(let id):
            fields = ["type": .string("end_session"), "session_id": n(id)]
        case .prefetch(let request):
            fields = [
                "type": .string("prefetch"), "session_id": n(request.sessionID), "reading": .string(request.reading),
                "context": context(request.context), "context_rev": n(request.contextRev),
            ]
        case .convert(let request, let timeout):
            fields = [
                "type": .string("convert"), "session_id": n(request.sessionID), "reading": .string(request.reading),
                "context": context(request.context), "context_rev": n(request.contextRev),
                "page_size": n(request.pageSize), "timeout_ms": ms(timeout),
            ]
        case .page(let conversionID, let index, let timeout):
            fields = [
                "type": .string("page"), "conversion_id": n(conversionID), "index": n(index), "timeout_ms": ms(timeout),
            ]
        case .commit(let conversionID, let candidateIndex):
            fields = ["type": .string("commit"), "conversion_id": n(conversionID), "candidate_index": n(candidateIndex)]
        case .commitText(let sessionID, let reading, let surface):
            fields = [
                "type": .string("commit_text"), "session_id": n(sessionID), "reading": .string(reading),
                "surface": .string(surface),
            ]
        case .cancel(let conversionID):
            fields = ["type": .string("cancel"), "conversion_id": n(conversionID)]
        case .segmentPage(let conversionID, let layoutRev, let segment, let index, let left, let timeout):
            fields = [
                "type": .string("segment_page"), "conversion_id": n(conversionID), "layout_rev": n(layoutRev),
                "segment": n(segment), "index": n(index), "left": choices(left), "timeout_ms": ms(timeout),
            ]
        case .resize(let conversionID, let layoutRev, let segment, let delta, let left, let timeout):
            fields = [
                "type": .string("resize_segment"), "conversion_id": n(conversionID), "layout_rev": n(layoutRev),
                "segment": n(segment), "delta": n(delta), "left": choices(left), "timeout_ms": ms(timeout),
            ]
        case .commitSegments(let conversionID, let layoutRev, let values):
            fields = [
                "type": .string("commit_segments"), "conversion_id": n(conversionID), "layout_rev": n(layoutRev),
                "choices": choices(values),
            ]
        case .reconvert(let request, let timeout):
            fields = [
                "type": .string("reconvert"), "session_id": n(request.sessionID), "surface": .string(request.surface),
                "context": context(request.context), "page_size": n(request.pageSize), "timeout_ms": ms(timeout),
            ]
        case .shutdown(let reason):
            let name =
                switch reason {
                case .versionMismatch: "version_mismatch"
                case .settingsChanged: "settings_changed"
                }
            fields = ["type": .string("shutdown"), "reason": .string(name)]
        case .relay(_, let timeout):
            fields = ["type": .string("relay"), "timeout_ms": ms(timeout)]
        case .postalLookup(let code, let timeout):
            fields = ["type": .string("postal_lookup"), "code": .string(code), "timeout_ms": ms(timeout)]
        }
        return .object(fields)
    }
}

// MARK: 読み込み

enum ScenarioFiles {
    /// `shell-core/scenarios`
    static var directory: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("shell-core/scenarios")
    }

    static func load() throws -> [(name: String, file: ScenarioFile)] {
        let urls = try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)
            .filter { $0.pathExtension == "json" }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
        return try urls.map { url in
            do {
                return (url.lastPathComponent, try JSONDecoder().decode(ScenarioFile.self, from: Data(contentsOf: url)))
            } catch {
                throw ScenarioFormatError("\(url.lastPathComponent): \(error)")
            }
        }
    }
}
