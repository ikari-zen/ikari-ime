// テスト用の替え玉。Rust の合成器（shell-ffi）と エンジンの代わり。
import Foundation

@testable import IkariShellCore

/// ローマ字の合成器の替え玉。状態機械が正しく呼んでいるかを見るためのもので、ローマ字表の正しさは Rust 側のテストが持つ。
final class FakeComposer: Composing {
    private(set) var kana = ""
    private(set) var pending = ""

    private static let table: [String: String] = [
        "a": "あ", "i": "い", "u": "う", "e": "え", "o": "お",
        "ka": "か", "ki": "き", "ku": "く", "ke": "け", "ko": "こ",
        "sa": "さ", "si": "し", "su": "す", "se": "せ", "so": "そ",
        "ta": "た", "ti": "ち", "tu": "つ", "te": "て", "to": "と",
        "na": "な", "ni": "に", "nu": "ぬ", "ne": "ね", "no": "の", "nn": "ん",
        "ha": "は", "hi": "ひ", "hu": "ふ", "he": "へ", "ho": "ほ",
        "ji": "じ", "ma": "ま", "mi": "み", "mu": "む", "me": "め", "mo": "も",
        "ya": "や", "yu": "ゆ", "yo": "よ",
        "ra": "ら", "ri": "り", "ru": "る", "re": "れ", "ro": "ろ",
        "wa": "わ", "wo": "を",
        "zh": "←", "zj": "↓", "zk": "↑", "zl": "→",
        "-": "ー", ",": "、", ".": "。", ":": "：",
    ]
    private static let prefixes: Set<String> = {
        var result = Set<String>()
        for key in table.keys { for length in 1..<key.count { result.insert(String(key.prefix(length))) } }
        return result
    }()
    /// JIS かな: 物理キーの番号（IkariJisKey）→ かな
    private static let jisTable: [UInt8: String] = [
        22: "ち", 32: "の", 33: "り", 30: "に",
        // ショートコードのテスト用: E、Y、S のキーと けのキー（ADR 0027 §3.6）
        26: "い", 46: "ん", 40: "と", 16: "け",
    ]

    private(set) var pushedJis: [(key: UInt8, shift: Bool)] = []
    private static let colonKey: UInt8 = 16

    func pushCharacter(_ scalar: Unicode.Scalar) {
        pending.unicodeScalars.append(scalar)
        resolve()
    }

    func pushJis(key: UInt8, shift: Bool) {
        pushedJis.append((key, shift))
        // 本物の合成器と同じく、Shift+けのキーは何も出さない（ADR 0027 §1.1）
        if key == Self.colonKey && shift { return }
        if let value = Self.jisTable[key] { kana += value }
    }

    @discardableResult func backspace() -> Bool {
        if !pending.isEmpty {
            pending.removeLast()
            return true
        }
        if !kana.isEmpty {
            kana.removeLast()
            return true
        }
        return false
    }

    func flush() {
        if pending == "n" { kana += "ん" }
        pending = ""
    }

    func clear() {
        kana = ""
        pending = ""
    }

    private func resolve() {
        while !pending.isEmpty {
            if let value = Self.table[pending] {
                kana += value
                pending = ""
                return
            }
            // 数字は半角のまま読みに入る（本物の合成器と同じ。`10:30` が `10：30` になる）
            if pending.count == 1, let first = pending.first, first.isASCII, first.isNumber {
                kana += pending
                pending = ""
                return
            }
            if Self.prefixes.contains(pending) { return }
            let chars = Array(pending)
            if !chars[0].isASCII {
                // 本物の合成器と同じく、ローマ字でない文字（かな）はそのまま読みに入る
                kana.append(chars[0])
            } else if chars[0] == "n" {
                kana += "ん"
            } else if chars.count >= 2, chars[0] == chars[1], chars[0].isLetter {
                kana += "っ"
            }
            pending = String(chars.dropFirst())
        }
    }
}

/// 字種変換の替え玉（Foundation の変換で近似する）。
struct FakeScripts: ScriptConverting {
    func convert(_ text: String, to script: KeymapAction) -> String {
        switch script {
        case .toHiragana:
            return text
        case .toFullwidthKatakana:
            return text.applyingTransform(.hiraganaToKatakana, reverse: false) ?? text
        case .toHalfwidthKatakana:
            let katakana = text.applyingTransform(.hiraganaToKatakana, reverse: false) ?? text
            return katakana.applyingTransform(.fullwidthToHalfwidth, reverse: false) ?? katakana
        case .toFullwidthAlnum:
            return text.applyingTransform(.fullwidthToHalfwidth, reverse: true) ?? text
        case .toHalfwidthAlnum:
            return text.applyingTransform(.fullwidthToHalfwidth, reverse: false) ?? text
        case .segmentFocusLeft, .segmentFocusRight, .segmentShrink, .segmentExpand, .reconvert, .candidateNext,
            .candidatePrevious, .modeToggle, .modeEnglish, .modeJapanese:
            return text
        }
    }
}

/// エンジンの替え玉。呼ばれた内容を記録し、返事は差し替えられる。
final class MockEngine: EngineClient, @unchecked Sendable {
    enum Call: Equatable {
        case hello
        case beginSession(SessionInfo)
        case endSession(UInt64)
        case prefetch(ConversionRequest)
        case convert(ConversionRequest, Duration)
        case page(conversionID: UInt64, index: Int, Duration)
        case commit(conversionID: UInt64, candidateIndex: Int)
        case commitText(sessionID: UInt64, reading: String, surface: String)
        case cancel(UInt64)
        case segmentPage(conversionID: UInt64, layoutRev: UInt32, segment: Int, index: Int, left: [SegmentChoiceValue], Duration)
        case resize(conversionID: UInt64, layoutRev: UInt32, segment: Int, delta: Int, left: [SegmentChoiceValue], Duration)
        case commitSegments(conversionID: UInt64, layoutRev: UInt32, choices: [SegmentChoiceValue])
        case reconvert(ReconvertRequest, Duration)
        case shutdown(EngineShutdownReason)
        case relay(Data, Duration)
    }

    private let lock = NSLock()
    private var recorded: [Call] = []

    /// `convert` の返事。投げれば失敗（時間切れ、落ちた）の扱いになる
    var onConvert: @Sendable (ConversionRequest) throws -> ConversionPage = { _ in MockEngine.defaultPages[0] }
    var onPage: @Sendable (UInt64, Int) throws -> ConversionPage = { _, index in
        guard defaultPages.indices.contains(index) else { throw EngineClientError.engine(.badRequest) }
        return defaultPages[index]
    }

    /// 文節と再変換の返事。既定は失敗（lane S のテストが差し替える）
    var onSegmentPage: @Sendable (UInt64, Int, Int) throws -> SegmentPage = { _, _, _ in
        throw EngineClientError.engine(.badRequest)
    }
    var onResize: @Sendable (UInt64, Int, Int) throws -> SegmentLayoutInfo = { _, _, _ in
        throw EngineClientError.engine(.badRequest)
    }
    var onReconvert: @Sendable (ReconvertRequest) throws -> SegmentLayoutInfo = { _ in
        throw EngineClientError.engine(.badRequest)
    }

    /// `relay` の返事。既定は空のバイト列
    var onRelay: @Sendable (Data) throws -> Data = { _ in Data() }

    /// 3ページ分（3 + 3 + 2 件）。候補の位置が通し番号 0...7 になる
    static let defaultPages: [ConversionPage] = [
        ConversionPage(
            conversionID: 7, pageIndex: 0,
            candidates: [
                CandidateItem(surface: "品詞", annotation: "ひんし(品詞)"),
                CandidateItem(surface: "瀕死", annotation: "ひんし(瀕死)"),
                CandidateItem(surface: "ひんし", kind: .hiragana),
            ],
            hasNext: true),
        ConversionPage(
            conversionID: 7, pageIndex: 1,
            candidates: [CandidateItem(surface: "ヒンシ", kind: .katakana), CandidateItem(surface: "貧士"), CandidateItem(surface: "品指")],
            hasNext: true),
        ConversionPage(
            conversionID: 7, pageIndex: 2,
            candidates: [CandidateItem(surface: "牝糸"), CandidateItem(surface: "ﾋﾝｼ", kind: .katakana)],
            hasNext: false),
    ]

    var calls: [Call] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    private func record(_ call: Call) {
        lock.lock()
        recorded.append(call)
        lock.unlock()
    }

    func hello(shellVersion: String, timeout: Duration) throws -> EngineHello {
        record(.hello)
        return EngineHello(engineVersion: "test", dictLabel: "test", lm: .ready)
    }
    func beginSession(_ session: SessionInfo) { record(.beginSession(session)) }
    func endSession(sessionID: UInt64) { record(.endSession(sessionID)) }
    func prefetch(_ request: ConversionRequest) { record(.prefetch(request)) }
    func convert(_ request: ConversionRequest, timeout: Duration) throws -> ConversionPage {
        record(.convert(request, timeout))
        return try onConvert(request)
    }
    func page(conversionID: UInt64, index: Int, timeout: Duration) throws -> ConversionPage {
        record(.page(conversionID: conversionID, index: index, timeout))
        return try onPage(conversionID, index)
    }
    func commit(conversionID: UInt64, candidateIndex: Int) {
        record(.commit(conversionID: conversionID, candidateIndex: candidateIndex))
    }
    func commitText(sessionID: UInt64, reading: String, surface: String) {
        record(.commitText(sessionID: sessionID, reading: reading, surface: surface))
    }
    func cancel(conversionID: UInt64) { record(.cancel(conversionID)) }
    func segmentPage(
        conversionID: UInt64, layoutRev: UInt32, segment: Int, index: Int, left: [SegmentChoiceValue], timeout: Duration
    ) throws -> SegmentPage {
        record(.segmentPage(conversionID: conversionID, layoutRev: layoutRev, segment: segment, index: index, left: left, timeout))
        return try onSegmentPage(conversionID, segment, index)
    }
    func resize(
        conversionID: UInt64, layoutRev: UInt32, segment: Int, delta: Int, left: [SegmentChoiceValue], timeout: Duration
    ) throws -> SegmentLayoutInfo {
        record(.resize(conversionID: conversionID, layoutRev: layoutRev, segment: segment, delta: delta, left: left, timeout))
        return try onResize(conversionID, segment, delta)
    }
    func commitSegments(conversionID: UInt64, layoutRev: UInt32, choices: [SegmentChoiceValue]) {
        record(.commitSegments(conversionID: conversionID, layoutRev: layoutRev, choices: choices))
    }
    func reconvert(_ request: ReconvertRequest, timeout: Duration) throws -> SegmentLayoutInfo {
        record(.reconvert(request, timeout))
        return try onReconvert(request)
    }
    func shutdown(reason: EngineShutdownReason) { record(.shutdown(reason)) }
    /// 診断ログの事象。`calls` には入れない（既存の呼び出しの比較を乱さない）
    func diag(_ event: DiagEventValue) {
        lock.lock()
        diagRecorded.append(event)
        lock.unlock()
    }
    private var diagRecorded: [DiagEventValue] = []
    var diagEvents: [DiagEventValue] {
        lock.lock()
        defer { lock.unlock() }
        return diagRecorded
    }
    func relay(_ request: Data, timeout: Duration) throws -> Data {
        record(.relay(request, timeout))
        return try onRelay(request)
    }

    // 取り出しの補助
    var prefetchedReadings: [String] {
        calls.compactMap { if case .prefetch(let request) = $0 { request.reading } else { nil } }
    }
    var convertRequests: [ConversionRequest] {
        calls.compactMap { if case .convert(let request, _) = $0 { request } else { nil } }
    }
    func count(where predicate: (Call) -> Bool) -> Int { calls.filter(predicate).count }
}

/// 状態機械・合成器・エンジンをまとめたテスト用の入れ物。
final class Harness {
    let engine: MockEngine
    let composer = FakeComposer()
    let machine: InputStateMachine
    /// 周辺テキストの取得を数える
    private(set) var surroundingFetches = 0
    var surroundingText = SurroundingText(left: "ユーザー辞書に単語を登録するときは", available: true)
    /// 再変換のキーで評価される選択範囲。評価された回数を数える
    var selection: SelectedText?
    private(set) var selectionFetches = 0

    static let sessionID: UInt64 = 42

    init(
        preset: KeymapPreset = .macos,
        settings: Settings = .default,
        secure: Bool = false,
        engine: MockEngine = MockEngine(),
        bundleID: String = "com.apple.TextEdit",
        shortcodeTable: ShortcodeTable = .empty
    ) throws {
        self.engine = engine
        let keymap = try Keymap(presetTSV: Harness.presetTSV(preset), settings: settings)
        machine = InputStateMachine(
            engine: engine,
            composer: composer,
            scripts: FakeScripts(),
            keymap: keymap,
            settings: settings,
            session: SessionInfo(sessionID: Harness.sessionID, clientBundleID: bundleID, secure: secure)
        )
        machine.setShortcodeTable(shortcodeTable)
    }

    /// `macos/Resources/keymap/<preset>.tsv`
    static func presetTSV(_ preset: KeymapPreset) throws -> String {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let url = root.appendingPathComponent("Resources/keymap/\(preset.rawValue).tsv")
        return try String(contentsOf: url, encoding: .utf8)
    }

    @discardableResult
    func press(_ key: KeyInput.Key, _ modifiers: KeyModifiers = [], repeating: Bool = false) -> [KeyAction] {
        machine.handle(
            KeyInput(key: key, modifiers: modifiers),
            surrounding: {
                self.surroundingFetches += 1
                return self.surroundingText
            }(),
            selection: {
                self.selectionFetches += 1
                return self.selection
            }(),
            isRepeat: repeating)
    }

    /// 文字ごとに打つ（修飾キーなし）
    @discardableResult
    func type(_ text: String) -> [KeyAction] {
        var last: [KeyAction] = []
        for character in text { last = press(.character(String(character))) }
        return last
    }

    var markedText: String? {
        switch machine.state {
        case .idle: nil
        case .composing(let kana, let pending): kana + pending
        case .asciiRun(let text): text
        case .shortcode(let typing): typing.marked
        case .shortcodeChoice(let choosing): choosing.marked
        case .converting(let state): state.candidates[state.selected].surface
        }
    }

    var converting: ConvertingState? {
        if case .converting(let state) = machine.state { state } else { nil }
    }
}
