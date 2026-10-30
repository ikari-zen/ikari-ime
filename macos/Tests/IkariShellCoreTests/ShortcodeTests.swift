// 絵文字のショートコード（ADR 0027 §3.3、§3.4、§3.5、§3.6 の表）。
// 日本語のモード（ローマ字とかな）は `InputStateMachine` を通して、英語のモードの規則は `ShortcodeMachine` を直接。
import Testing

@testable import IkariShellCore

/// テスト用の表（名前の昇順。UTF-8 のバイト順）
private let table = ShortcodeTable(entries: [
    ShortcodeEntry(name: "+1", emoji: "👍"),
    ShortcodeEntry(name: "100", emoji: "💯"),
    ShortcodeEntry(name: "a", emoji: "🅰"),
    ShortcodeEntry(name: "ab", emoji: "🆎"),
    ShortcodeEntry(name: "abc", emoji: "🔤"),
    ShortcodeEntry(name: "eye", emoji: "👁"),
    ShortcodeEntry(name: "eyeglasses", emoji: "👓"),
    ShortcodeEntry(name: "eyes", emoji: "👀"),
    ShortcodeEntry(name: "smile", emoji: "😄"),
    ShortcodeEntry(name: "smiley", emoji: "😃"),
    ShortcodeEntry(name: "smirk", emoji: "😏"),
    ShortcodeEntry(name: "tada", emoji: "🎉"),
])

private let tab = KeyInput.Key.character("\t")
private let backTab = KeyInput.Key.character("\u{19}")
private let colon = KeyInput.Key.character(":")

/// かな入力の `IkariJisKey`: 英字は A = 22 から順
private func jis(_ letter: Character) -> KeyInput.Key {
    .jis(22 + UInt8(letter.asciiValue! - Character("a").asciiValue!))
}
private let jisColonKey: UInt8 = 16

/// ショートコードをオンにした設定（既定はオフ）
private var shortcodeOnSettings: Settings {
    var settings = Settings.default
    settings.emojiShortcode = true
    return settings
}

private func romajiHarness(
    settings: Settings = shortcodeOnSettings, secure: Bool = false, bundleID: String = "com.apple.TextEdit",
    table: ShortcodeTable = table
) throws -> Harness {
    try Harness(settings: settings, secure: secure, bundleID: bundleID, shortcodeTable: table)
}

private func kanaHarness() throws -> Harness {
    try Harness(settings: Settings(inputMethod: .kana, emojiShortcode: true), shortcodeTable: table)
}

private extension Harness {
    var shortcodeName: String? {
        if case .shortcode(let typing) = machine.state { typing.name } else { nil }
    }
    var choosing: ShortcodeChoosing? {
        if case .shortcodeChoice(let choosing) = machine.state { choosing } else { nil }
    }
    var composingKana: String? {
        if case .composing(let kana, let pending) = machine.state { kana + pending } else { nil }
    }
    var commitTexts: [(reading: String, surface: String)] {
        engine.calls.compactMap {
            if case .commitText(_, let reading, let surface) = $0 { (reading, surface) } else { nil }
        }
    }
}

private func marked(_ text: String) -> KeyAction {
    .setMarkedText(MarkedText(text: text, highlightedLength: text.utf16.count))
}

// MARK: - 表

@Suite struct ShortcodeTableTests {
    @Test func exactFindsTheNameOnly() {
        #expect(table.exact("eyes") == "👀")
        #expect(table.exact("+1") == "👍")
        #expect(table.exact("tada") == "🎉")
        #expect(table.exact("ey") == nil)
        #expect(table.exact("") == nil)
        #expect(table.exact("zzz") == nil)
        #expect(ShortcodeTable.empty.exact("eyes") == nil)
    }

    @Test func prefixIsOrderedByLengthThenName() {
        // 名前の短い順、同じ長さなら名前の順（§3.5）
        #expect(table.prefix("ey").map(\.name) == ["eye", "eyes", "eyeglasses"])
        #expect(table.prefix("sm").map(\.name) == ["smile", "smirk", "smiley"])
        #expect(table.prefix("a").map(\.name) == ["a", "ab", "abc"])
    }

    @Test func prefixIncludesTheExactNameAndIsEmptyForNoMatch() {
        #expect(table.prefix("eyes").map(\.name) == ["eyes"])
        #expect(table.prefix("q").isEmpty)
        #expect(table.prefix("eyesx").isEmpty)
    }

    @Test func emptyNameGivesNoCandidates() {
        #expect(table.prefix("").isEmpty)
        #expect(ShortcodeTable.empty.prefix("a").isEmpty)
        #expect(ShortcodeTable.empty.isEmpty)
    }
}

@Suite struct JisKeyAsciiTests {
    @Test func lettersAreTheKeyTops() {
        for (offset, letter) in "abcdefghijklmnopqrstuvwxyz".enumerated() {
            #expect(JisKeyAscii.character(forJisKey: 22 + UInt8(offset), shifted: false) == letter)
            #expect(JisKeyAscii.character(forJisKey: 22 + UInt8(offset), shifted: true) == nil)
        }
    }

    @Test func digitsAndSymbols() {
        for (index, digit) in "1234567890".enumerated() {
            #expect(JisKeyAscii.character(forJisKey: UInt8(index), shifted: false) == digit)
            #expect(JisKeyAscii.character(forJisKey: UInt8(index), shifted: true) == nil)
        }
        #expect(JisKeyAscii.character(forJisKey: 10, shifted: false) == "-")
        #expect(JisKeyAscii.character(forJisKey: 15, shifted: true) == "+")
        #expect(JisKeyAscii.character(forJisKey: 21, shifted: true) == "_")
        #expect(JisKeyAscii.character(forJisKey: 15, shifted: false) == nil)
        #expect(JisKeyAscii.character(forJisKey: 21, shifted: false) == nil)
        #expect(JisKeyAscii.character(forJisKey: JisKeyAscii.colonKey, shifted: false) == ":")
        #expect(JisKeyAscii.character(forJisKey: JisKeyAscii.colonKey, shifted: true) == ":")
        for key: UInt8 in [11, 12, 13, 14, 17, 18, 19, 20, 48, 255] {
            #expect(JisKeyAscii.character(forJisKey: key, shifted: false) == nil)
        }
    }
}

// MARK: - 日本語のモード: ローマ字入力

@Suite struct RomajiShortcodeTests {
    // §3.2 入り方

    @Test func colonInIdleEntersTheShortcodeState() throws {
        let h = try romajiHarness()
        let actions = h.press(colon, [.shift])
        #expect(actions == [marked(":")])
        // 日本語のモードは下線つきのまま（B51。英語のモードだけ下線なし）
        if case .setMarkedText(let mark) = actions.first { #expect(mark.underlined) } else { Issue.record("no marked text") }
        #expect(h.machine.state == .shortcode(ShortcodeTyping(name: "", keys: [KeyInput(key: colon, modifiers: [.shift])])))
    }

    @Test func namesGrowWhileTheyMatchAPrefix() throws {
        let h = try romajiHarness()
        h.type(":ey")
        #expect(h.shortcodeName == "ey")
        #expect(h.markedText == ":ey")
        #expect(h.engine.prefetchedReadings.isEmpty)
    }

    @Test func doesNotEnterWithoutAnAllowingCondition() throws {
        // 設定が off
        let disabled = try romajiHarness(settings: .default)  // 既定はオフ
        disabled.type(":")
        #expect(disabled.machine.state == .composing(kana: "：", pending: ""))
        // 除外アプリ
        let excluded = try romajiHarness(bundleID: "com.apple.Terminal")
        excluded.type(":")
        #expect(excluded.machine.state == .composing(kana: "：", pending: ""))
        // secure
        let secure = try romajiHarness(secure: true)
        // secure は日本語のモードでも素通し（ADR 0026 D5）。ショートコードにも合成にも入らない
        #expect(secure.press(.character(":")) == [.passThrough])
        #expect(secure.machine.state == .idle)
        // 表がない（まだ届いていない、データがない）
        let empty = try romajiHarness(table: .empty)
        empty.type(":")
        #expect(empty.machine.state == .composing(kana: "：", pending: ""))
        // キーリピート
        let repeating = try romajiHarness()
        repeating.press(colon, [.shift], repeating: true)
        #expect(repeating.machine.state == .composing(kana: "：", pending: ""))
        // Ctrl 付きの `:` はアプリへ
        let control = try romajiHarness()
        #expect(control.press(colon, [.control]) == [.passThrough])
        #expect(control.machine.state == .idle)
    }

    @Test func enteringAgainAfterAnAppExclusionListIsEmptied() throws {
        var settings = shortcodeOnSettings
        settings.emojiShortcodeExclude = []
        let h = try romajiHarness(settings: settings, bundleID: "com.apple.Terminal")
        h.type(":")
        #expect(h.shortcodeName == "")
    }

    // §3.6 他の入力を壊さない

    @Test func colonInTheMiddleOfAComposition() throws {
        let h = try romajiHarness()
        h.type("10:30")
        #expect(h.machine.state == .composing(kana: "10：30", pending: ""))
        h.press(.escape)
        h.type("https://")
        #expect(h.composingKana != nil)
        #expect(h.shortcodeName == nil)
        h.press(.escape)
        h.type("chuui:")
        #expect(h.composingKana?.hasSuffix("：") == true)
        #expect(h.shortcodeName == nil)
    }

    @Test func retypingGoesBackToTheRomajiComposition() throws {
        let h = try romajiHarness()
        h.type(":a")
        #expect(h.shortcodeName == "a")
        let actions = h.type("k")
        // `:ak` は前方一致が切れたので、`：あk` の合成になる
        #expect(actions.first == .setMarkedText(MarkedText(text: "")))
        #expect(actions.last == .setMarkedText(MarkedText(text: "：あk", highlightedLength: 2)))
        #expect(h.machine.state == .composing(kana: "：あ", pending: "k"))
        h.type("ita")
        #expect(h.machine.state == .composing(kana: "：あきた", pending: ""))
        // 打ち直しのあいだ、アプリへはキーを渡さず、確定履歴にも送らない
        #expect(!actions.contains(.passThrough))
        #expect(h.commitTexts.isEmpty)
    }

    @Test func colonThenEnterCommitsTheFullwidthColon() throws {
        let h = try romajiHarness()
        h.type(":")
        let actions = h.press(.enter)
        #expect(actions.contains(.commit("：")))
        #expect(!actions.contains(.passThrough))
        #expect(h.machine.state == .idle)
        #expect(h.commitTexts.map(\.surface) == ["："])
    }

    @Test func closingColonOfAnUnknownNameRetypes() throws {
        let h = try romajiHarness()
        h.type(":ey:")
        #expect(h.machine.state == .composing(kana: "：え：", pending: ""))
    }

    // §3.3 shortcode(name)

    @Test func tabDownAndSpaceShowThePrefixCandidates() throws {
        for key in [tab, KeyInput.Key.down, KeyInput.Key.space] {
            let h = try romajiHarness()
            h.type(":ey")
            let actions = h.press(key)
            let choosing = try #require(h.choosing)
            #expect(
                choosing.candidates == [
                    ShortcodeCandidate(surface: ":ey"),
                    ShortcodeCandidate(surface: "👁", label: ":eye:"),
                    ShortcodeCandidate(surface: "👀", label: ":eyes:"),
                    ShortcodeCandidate(surface: "👓", label: ":eyeglasses:"),
                ])
            #expect(choosing.selected == 0)
            #expect(actions.first == marked(":ey"))
            if case .showCandidates(let model)? = actions.last {
                #expect(model.candidates.map(\.surface) == [":ey", "👁", "👀", "👓"])
                #expect(model.candidates.map(\.label) == ["", ":eye:", ":eyes:", ":eyeglasses:"])
            } else {
                Issue.record("候補ウィンドウが出ていない")
            }
        }
    }

    @Test func prefixCandidatesPutTheExactNameSecond() throws {
        let h = try romajiHarness()
        h.type(":eye")
        h.press(.space)
        let choosing = try #require(h.choosing)
        #expect(choosing.candidates.map(\.surface) == [":eye", "👁", "👀", "👓"])
    }

    @Test func tabWithoutANameRetypes() throws {
        let h = try romajiHarness()
        h.type(":")
        h.press(tab)
        #expect(h.machine.state == .composing(kana: "：", pending: ""))
    }

    @Test func backspaceShortensTheNameThenLeaves() throws {
        let h = try romajiHarness()
        h.type(":ey")
        #expect(h.press(.backspace) == [marked(":e")])
        #expect(h.shortcodeName == "e")
        h.press(.backspace)
        #expect(h.shortcodeName == "")
        let actions = h.press(.backspace)
        #expect(actions == [.setMarkedText(MarkedText(text: ""))])
        #expect(h.machine.state == .idle)
        // 消費された（アプリには渡さない）
        #expect(!actions.contains(.passThrough))
    }

    @Test func escapeReturnsToTheComposition() throws {
        let h = try romajiHarness()
        h.type(":ey")
        let actions = h.press(.escape)
        #expect(h.machine.state == .composing(kana: "：え", pending: "y"))
        #expect(!actions.contains(.passThrough))
    }

    @Test func upperCaseLetterRetypesAndStartsAnAsciiRun() throws {
        let h = try romajiHarness()
        h.type(":")
        h.press(.character("E"), [.shift])
        #expect(h.machine.state == .asciiRun(text: "：E"))
    }

    @Test func commandKeyCommitsTheDisplayAndPassesTheKey() throws {
        let h = try romajiHarness()
        h.type(":e")
        let actions = h.press(.character("c"), [.command])
        #expect(actions.contains(.commit("：え")))
        #expect(actions.contains(.passThrough))
        #expect(h.machine.state == .idle)
    }

    @Test func arrowRetypesAndKeepsTheComposition() throws {
        let h = try romajiHarness()
        h.type(":e")
        let actions = h.press(.left)
        #expect(h.machine.state == .composing(kana: "：え", pending: ""))
        #expect(!actions.contains(.passThrough))
    }

    // §3.4 shortcodeChoice

    @Test func closingColonOffersTheLiteralFirstAndTheEmojiSecond() throws {
        let h = try romajiHarness()
        let actions = h.type(":eyes:")
        let choosing = try #require(h.choosing)
        #expect(
            choosing.candidates == [
                ShortcodeCandidate(surface: ":eyes:"), ShortcodeCandidate(surface: "👀", label: ":eyes:"),
            ])
        #expect(choosing.selected == 0)
        #expect(choosing.typed == ":eyes:")
        #expect(h.markedText == ":eyes:")
        #expect(
            actions == [
                marked(":eyes:"),
                .showCandidates(
                    CandidatePanelModel(
                        candidates: [
                            CandidateItem(surface: ":eyes:", kind: .literal),
                            CandidateItem(surface: "👀", kind: .unspecified, label: ":eyes:"),
                        ], selected: 0, annotation: "", reservedRows: 2)),
            ])
    }

    @Test func selectingMovesBetweenCandidates() throws {
        let h = try romajiHarness()
        h.type(":eyes:")
        let down = h.press(.down)
        #expect(h.choosing?.selected == 1)
        #expect(h.markedText == "👀")
        if case .moveSelection(let model)? = down.last { #expect(model.selected == 1) } else { Issue.record("moveSelection でない") }
        h.press(.down)
        #expect(h.choosing?.selected == 0)  // 折り返す
        h.press(tab)
        #expect(h.choosing?.selected == 1)
        h.press(.up)
        #expect(h.choosing?.selected == 0)
        h.press(backTab)
        #expect(h.choosing?.selected == 1)
        h.press(.space, [.shift])
        #expect(h.choosing?.selected == 0)
        h.press(.space)
        #expect(h.choosing?.selected == 1)
    }

    @Test func numberKeyCommitsTheNumberedCandidate() throws {
        let h = try romajiHarness()
        h.type(":eyes:")
        let actions = h.press(.character("2"))
        #expect(actions == [.commit("👀"), .hideCandidates])
        #expect(h.machine.state == .idle)
        #expect(h.commitTexts.map(\.surface) == ["👀"])
        #expect(h.commitTexts.map(\.reading) == [":eyes"])
    }

    @Test func numberOutsideTheCandidatesDoesNothing() throws {
        let h = try romajiHarness()
        h.type(":eyes:")
        let before = h.machine.state
        h.press(.character("9"))
        #expect(h.machine.state == before)
        #expect(h.commitTexts.isEmpty)
    }

    @Test func enterCommitsTheSelectedCandidateAndIsConsumed() throws {
        let h = try romajiHarness()
        h.type(":eyes:")
        var actions = h.press(.enter)
        #expect(actions == [.commit(":eyes:"), .hideCandidates])
        #expect(h.commitTexts.map(\.surface) == [":eyes:"])
        h.type(":eyes:")
        h.press(.down)
        actions = h.press(.enter)
        #expect(actions == [.commit("👀"), .hideCandidates])
    }

    @Test func backspaceClosesTheCandidatesAndDropsTheClosingColon() throws {
        let h = try romajiHarness()
        h.type(":eyes:")
        let actions = h.press(.backspace)
        #expect(actions == [marked(":eyes"), .hideCandidates])
        #expect(h.shortcodeName == "eyes")
        // 名前は続けて編集できる
        h.press(.backspace)
        #expect(h.shortcodeName == "eye")
        // もう一度閉じられる
        h.type(":")
        #expect(h.choosing?.typed == ":eye:")
    }

    @Test func escapeClosesTheCandidatesAndReturnsToTyping() throws {
        let h = try romajiHarness()
        h.type(":eyes:")
        h.press(.down)
        let actions = h.press(.escape)
        #expect(actions == [marked(":eyes"), .hideCandidates])
        #expect(h.shortcodeName == "eyes")
    }

    @Test func backspaceFromPrefixCandidatesKeepsTheName() throws {
        let h = try romajiHarness()
        h.type(":ey")
        h.press(tab)
        h.press(.backspace)
        #expect(h.shortcodeName == "ey")
        h.press(.backspace)
        #expect(h.shortcodeName == "e")
    }

    @Test func printableKeyCommitsAndStartsANewComposition() throws {
        let h = try romajiHarness()
        h.type(":eyes:")
        let actions = h.press(.character("k"))
        #expect(actions.contains(.commit(":eyes:")))
        #expect(!actions.contains(.passThrough))
        #expect(h.machine.state == .composing(kana: "", pending: "k"))
        h.type("a")
        #expect(h.machine.state == .composing(kana: "か", pending: ""))
        #expect(h.commitTexts.map(\.surface) == [":eyes:"])
    }

    @Test func colonAfterCommitStartsANewShortcode() throws {
        let h = try romajiHarness()
        h.type(":eyes:")
        h.press(.character(":"), [.shift])
        #expect(h.shortcodeName == "")
        #expect(h.commitTexts.map(\.surface) == [":eyes:"])
    }

    @Test func commandOrArrowKeyCommitsAndPassesTheKey() throws {
        for (key, modifiers) in [
            (KeyInput.Key.left, KeyModifiers()), (.right, []), (.character("c"), [.command]), (.function(5), []),
        ] {
            let h = try romajiHarness()
            h.type(":eyes:")
            h.press(.down)
            let actions = h.press(key, modifiers)
            #expect(actions == [.commit("👀"), .hideCandidates, .passThrough])
            #expect(h.machine.state == .idle)
        }
    }

    @Test func switchingClickAndDeactivationCommitTheDisplayAsIs() throws {
        let typing = try romajiHarness()
        typing.type(":ey")
        #expect(typing.machine.commitComposition() == [.commit(":ey")])
        #expect(typing.commitTexts.map(\.reading) == [":ey"])
        #expect(typing.machine.state == .idle)

        let choosing = try romajiHarness()
        choosing.type(":eyes:")
        choosing.press(.down)
        #expect(choosing.machine.commitComposition() == [.commit("👀"), .hideCandidates])
        #expect(choosing.machine.state == .idle)
    }

    @Test func becomingSecureCommitsTheDisplayWithoutRecordingAndHandlesTheKey() throws {
        let h = try romajiHarness()
        h.type(":ey")
        h.machine.setSecure(true)
        let actions = h.press(.character("k"))
        #expect(actions.first == .commit(":ey"))
        #expect(actions.last == .passThrough)  // secure の間はキーを素通し（ADR 0026 D5）
        #expect(h.commitTexts.isEmpty)  // secure は確定履歴に送らない
        #expect(h.shortcodeName == nil)
        #expect(h.machine.state == .idle)
    }

    @Test func colonRightAfterAConversionCommitEntersTheShortcode() throws {
        let h = try romajiHarness()
        h.type("a")
        h.press(.space)
        #expect(h.converting != nil)
        let actions = h.press(colon, [.shift])
        #expect(actions.contains(where: { if case .commit = $0 { true } else { false } }))
        #expect(h.shortcodeName == "")
    }

    @Test func colonAfterAnEnterCommitEntersTheShortcode() throws {
        let h = try romajiHarness()
        h.type("a")
        h.press(.enter)
        #expect(h.machine.state == .idle)
        h.type(":")
        #expect(h.shortcodeName == "")
    }

    @Test func tableArrivingLaterEnablesTheShortcode() throws {
        let h = try romajiHarness(table: .empty)
        h.type(":")
        h.press(.escape)
        h.machine.setShortcodeTable(table)
        h.type(":")
        #expect(h.shortcodeName == "")
    }

    @Test func pagesShowNineCandidatesAtATime() throws {
        let names = (1...12).map { String(format: "x%02d", $0) }
        let big = ShortcodeTable(entries: names.map { ShortcodeEntry(name: $0, emoji: "🅧\($0)") })
        let h = try romajiHarness(table: big)
        h.type(":x")
        let shown = h.press(tab)
        guard case .showCandidates(let first)? = shown.last else { Issue.record("候補が出ていない"); return }
        #expect(first.candidates.count == 9)
        #expect(first.reservedRows == 9)
        for _ in 0..<8 { h.press(.down) }
        #expect(h.choosing?.selected == 8)
        let moved = h.press(.down)
        // 10 番目は 2 ページ目の先頭
        guard case .showCandidates(let second)? = moved.last else { Issue.record("ページが替わっていない"); return }
        #expect(second.candidates.count == 4)
        #expect(second.selected == 0)
        // 数字キーは今のページの番号
        h.press(.character("2"))
        #expect(h.commitTexts.map(\.surface) == ["🅧x10"])
    }
}

// MARK: - 日本語のモード: かな入力（JIS）

@Suite struct KanaShortcodeTests {
    private func enter(_ h: Harness) {
        h.press(.jis(jisColonKey), [.shift])
    }

    @Test func plainKeKeyStaysKe() throws {
        let h = try kanaHarness()
        h.press(.jis(jisColonKey))
        #expect(h.machine.state == .composing(kana: "け", pending: ""))
    }

    @Test func shiftKeKeyEntersTheShortcode() throws {
        let h = try kanaHarness()
        let actions = h.press(.jis(jisColonKey), [.shift])
        #expect(actions == [marked(":")])
        #expect(h.shortcodeName == "")
    }

    @Test func colonCharacterAlsoEnters() throws {
        let h = try kanaHarness()
        h.press(.character(":"), [.shift])
        #expect(h.shortcodeName == "")
    }

    @Test func namesAreReadFromTheKeyTops() throws {
        let h = try kanaHarness()
        enter(h)
        h.press(jis("e"))
        h.press(jis("y"))
        #expect(h.shortcodeName == "ey")
        #expect(h.markedText == ":ey")
    }

    @Test func closingWithAnUnknownNameRetypesToKanaComposition() throws {
        let h = try kanaHarness()
        enter(h)
        h.press(jis("e"))
        h.press(jis("y"))
        h.press(.jis(jisColonKey))
        // Shift+け は今どおり何も出さず、E Y け が かなになる
        #expect(h.machine.state == .composing(kana: "いんけ", pending: ""))
    }

    @Test func closingWithAFullNameOffersTheEmoji() throws {
        let h = try kanaHarness()
        enter(h)
        for letter: Character in ["e", "y", "e", "s"] { h.press(jis(letter)) }
        h.press(.jis(jisColonKey))
        let choosing = try #require(h.choosing)
        #expect(choosing.candidates.map(\.surface) == [":eyes:", "👀"])
        #expect(h.markedText == ":eyes:")
        // けのキーは Shift の有無を問わず閉じる
        let shifted = try kanaHarness()
        enter(shifted)
        for letter: Character in ["e", "y", "e", "s"] { shifted.press(jis(letter)) }
        shifted.press(.jis(jisColonKey), [.shift])
        #expect(shifted.choosing?.typed == ":eyes:")
    }

    @Test func digitsAndPlusAreNameCharacters() throws {
        let h = try kanaHarness()
        enter(h)
        h.press(.jis(15), [.shift])  // Shift+; = +
        h.press(.jis(0))  // 1
        #expect(h.shortcodeName == "+1")
        h.press(.jis(jisColonKey))
        #expect(h.choosing?.candidates.map(\.surface) == [":+1:", "👍"])
    }

    @Test func spaceShowsPrefixCandidatesAndNumberCommits() throws {
        let h = try kanaHarness()
        enter(h)
        h.press(jis("s"))
        h.press(jis("m"))
        h.press(.space)
        let choosing = try #require(h.choosing)
        #expect(choosing.candidates.map(\.surface) == [":sm", "😄", "😏", "😃"])
        // かな入力の数字キー（Shift なし）は番号
        h.press(.jis(1))  // 2
        #expect(h.commitTexts.map(\.surface) == ["😄"])
        #expect(h.machine.state == .idle)
    }

    @Test func escapeLeavesQuietlyWhenNothingWasComposed() throws {
        let h = try kanaHarness()
        enter(h)
        let actions = h.press(.escape)
        #expect(actions == [.setMarkedText(MarkedText(text: ""))])
        #expect(h.machine.state == .idle)
    }

    @Test func enterAfterShiftKeKeyIsHandedBackToTheApp() throws {
        let h = try kanaHarness()
        enter(h)
        let actions = h.press(.enter)
        // 今の「Shift+け のあとの Enter」と同じ。見えていた `:` は先に消す
        #expect(actions == [.setMarkedText(MarkedText(text: "")), .resolveStrayMarkedText])
        #expect(h.machine.state == .idle)
    }

    @Test func otherKeysAfterTheCommitContinueWithKana() throws {
        let h = try kanaHarness()
        enter(h)
        for letter: Character in ["e", "y", "e", "s"] { h.press(jis(letter)) }
        h.press(.jis(jisColonKey))
        h.press(jis("a"))
        #expect(h.commitTexts.map(\.surface) == [":eyes:"])
        #expect(h.machine.state == .composing(kana: "ち", pending: ""))
    }
}

// MARK: - ShortcodeMachine（英語のモードの規則を含む）

@Suite struct ShortcodeMachineTests {
    private func machine(_ mode: ShortcodeMode) -> ShortcodeMachine {
        ShortcodeMachine(table: table, mode: mode)
    }

    private func key(_ key: KeyInput.Key, _ modifiers: KeyModifiers = []) -> KeyInput {
        KeyInput(key: key, modifiers: modifiers)
    }

    /// 打鍵を順に流して、最後の結果を返す
    @discardableResult
    private func feed(_ machine: inout ShortcodeMachine, _ text: String) -> ShortcodeResult {
        var last = ShortcodeResult(outcome: .notHandled, phase: machine.phase)
        for (index, character) in text.enumerated() {
            last = machine.handle(key(.character(String(character))), canEnter: index == 0)
        }
        return last
    }

    @Test func entersOnlyWhenAllowed() {
        var denied = machine(.english)
        #expect(denied.handle(key(colon), canEnter: false).outcome == .notHandled)
        #expect(!denied.isActive)
        var emptyTable = ShortcodeMachine(table: .empty, mode: .english)
        #expect(emptyTable.handle(key(colon), canEnter: true).outcome == .notHandled)
        var other = machine(.english)
        #expect(other.handle(key(.character("a")), canEnter: true).outcome == .notHandled)
        var allowed = machine(.english)
        let result = allowed.handle(key(colon, [.shift]), canEnter: true)
        #expect(result.outcome == .typing(marked: ":", hidesCandidates: false))
        #expect(allowed.isActive)
    }

    @Test func englishSpaceAndEscapeRetypeIncludingTheKey() {
        var space = machine(.english)
        feed(&space, ":ey")
        let result = space.handle(key(.space), canEnter: false)
        guard case .retype(let keys, let literal) = result.outcome else { Issue.record("retype でない"); return }
        #expect(literal == ":ey")
        #expect(keys.last == key(.space))
        #expect(keys.count == 4)
        #expect(!space.isActive)

        var escape = machine(.english)
        feed(&escape, ":ey")
        guard case .retype(let escKeys, _) = escape.handle(key(.escape), canEnter: false).outcome else {
            Issue.record("retype でない")
            return
        }
        #expect(escKeys.last == key(.escape))
    }

    @Test func japaneseEscapeRetypesWithoutTheKey() {
        var romaji = machine(.romaji)
        feed(&romaji, ":ey")
        guard case .retype(let keys, _) = romaji.handle(key(.escape), canEnter: false).outcome else {
            Issue.record("retype でない")
            return
        }
        #expect(keys.count == 3)
        #expect(!keys.contains(key(.escape)))
    }

    @Test func englishTabShowsPrefixCandidates() {
        var english = machine(.english)
        feed(&english, ":ey")
        let result = english.handle(key(tab), canEnter: false)
        guard case .showChoices(let choosing) = result.outcome else { Issue.record("候補が出ていない"); return }
        #expect(choosing.candidates.count == 4)
    }

    @Test func englishChoosingSpaceDigitAndLetterCommitAndPassTheKey() {
        for input in [key(.space), key(.character("1")), key(.character("x")), key(.left)] {
            var english = machine(.english)
            feed(&english, ":eyes:")
            english.handle(key(.down), canEnter: false)
            let result = english.handle(input, canEnter: false)
            #expect(result.outcome == .commit(ShortcodeCommit(text: "👀", reading: ":eyes", after: .passKey)))
            #expect(!english.isActive)
        }
    }

    @Test func englishEnterPassesOnlyWhenTheLiteralIsSelected() {
        var literal = machine(.english)
        feed(&literal, ":eyes:")
        #expect(
            literal.handle(key(.enter), canEnter: false).outcome
                == .commit(ShortcodeCommit(text: ":eyes:", reading: ":eyes", after: .passKey)))
        var emoji = machine(.english)
        feed(&emoji, ":eyes:")
        emoji.handle(key(.down), canEnter: false)
        #expect(
            emoji.handle(key(.enter), canEnter: false).outcome
                == .commit(ShortcodeCommit(text: "👀", reading: ":eyes", after: .consume)))
    }

    @Test func englishEscapeCommitsTheLiteralAndConsumes() {
        var english = machine(.english)
        feed(&english, ":eyes:")
        english.handle(key(.down), canEnter: false)
        #expect(
            english.handle(key(.escape), canEnter: false).outcome
                == .commit(ShortcodeCommit(text: ":eyes:", reading: ":eyes", after: .consume)))
    }

    @Test func japaneseKeysContinueOrConsume() {
        var letter = machine(.romaji)
        feed(&letter, ":eyes:")
        let k = key(.character("k"))
        #expect(
            letter.handle(k, canEnter: false).outcome
                == .commit(ShortcodeCommit(text: ":eyes:", reading: ":eyes", after: .continueWith(k))))
        var enter = machine(.romaji)
        feed(&enter, ":eyes:")
        #expect(
            enter.handle(key(.enter), canEnter: false).outcome
                == .commit(ShortcodeCommit(text: ":eyes:", reading: ":eyes", after: .consume)))
    }

    @Test func flushCommitsWhateverIsShown() {
        var idle = machine(.english)
        #expect(idle.flush() == nil)
        var typing = machine(.english)
        feed(&typing, ":ey")
        #expect(typing.flush() == ShortcodeCommit(text: ":ey", reading: ":ey", after: .consume))
        #expect(!typing.isActive)
        var choosing = machine(.english)
        feed(&choosing, ":eyes:")
        choosing.handle(key(.down), canEnter: false)
        #expect(choosing.flush() == ShortcodeCommit(text: "👀", reading: ":eyes", after: .consume))
    }

    @Test func commandModifiersRetypeWhileTyping() {
        var english = machine(.english)
        feed(&english, ":e")
        guard case .retype(let keys, let literal) = english.handle(key(.character("c"), [.command]), canEnter: false).outcome
        else { Issue.record("retype でない"); return }
        #expect(literal == ":e")
        #expect(keys.last == key(.character("c"), [.command]))
    }

    @Test func resetDropsTheState() {
        var english = machine(.english)
        feed(&english, ":e")
        english.reset()
        #expect(english.phase == .inactive)
    }
}

// MARK: - 設定

@Suite struct ShortcodeSettingsTests {
    @Test func defaultsAreOffWithTerminalsExcluded() {
        let settings = Settings.default
        #expect(!settings.emojiShortcode)
        #expect(settings.emojiShortcodeExclude.contains("com.apple.Terminal"))
        #expect(!settings.emojiShortcodeExclude.contains("com.apple.TextEdit"))
    }

    @Test func parsesTheKeys() {
        let tsv = "emoji.shortcode\ton\nemoji.shortcode_exclude\tcom.a.b, com.c.d\n"
        let parsed = Settings.parse(tsv: tsv)
        #expect(parsed.warnings.isEmpty)
        #expect(parsed.settings.emojiShortcode)
        #expect(parsed.settings.emojiShortcodeExclude == ["com.a.b", "com.c.d"])
    }

    @Test func emptyExcludeListMeansNoExclusion() {
        let parsed = Settings.parse(tsv: "emoji.shortcode_exclude\t\n")
        #expect(parsed.warnings.isEmpty)
        #expect(parsed.settings.emojiShortcodeExclude.isEmpty)
    }

    @Test func invalidValuesWarnAndKeepTheDefault() {
        let parsed = Settings.parse(tsv: "emoji.shortcode\tmaybe\nemoji.shortcode_exclude\tcom.a b\n")
        #expect(parsed.warnings.count == 2)
        #expect(!parsed.settings.emojiShortcode)
        #expect(parsed.settings.emojiShortcodeExclude == Settings.defaultEmojiShortcodeExclude)
        // 警告に値の文字列は入らない
        #expect(parsed.warnings.allSatisfy { !$0.contains("maybe") && !$0.contains("com.a b") })
    }

    @Test func reconfigureSwitchesTheModeWithTheInputMethod() throws {
        let h = try romajiHarness()
        let keymap = try Keymap(presetTSV: Harness.presetTSV(.macos), settings: Settings(inputMethod: .kana, emojiShortcode: true))
        #expect(h.machine.reconfigure(settings: Settings(inputMethod: .kana, emojiShortcode: true), keymap: keymap, composer: FakeComposer()))
        // かな入力になったので、Shift+けのキーで入る
        h.press(.jis(jisColonKey), [.shift])
        #expect(h.shortcodeName == "")
    }
}
