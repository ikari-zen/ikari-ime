// 診断ログ D4b: `sh_episode` の panel / ctx、ショートコードの pick_kind、英語のモードの合成、`sh_mode`（ADR 0034 §4.2）。
import Foundation
import Testing

@testable import IkariShellCore

private let table = ShortcodeTable(entries: [
    ShortcodeEntry(name: "eye", emoji: "👁"),
    ShortcodeEntry(name: "eyes", emoji: "👀"),
])

private var shortcodeSettings: Settings {
    var settings = Settings.default
    settings.emojiShortcode = true
    settings.emojiSmallLetters = true
    return settings
}

/// engine/src/diag_schema.rs の語（panel / ctx / pick_kind / via）
private let panelWords: Set<String> = ["client", "accessibility", "last_known", "mouse_screen", "none"]
private let ctxWords: Set<String> = ["ok", "none"]
private let pickKindWords: Set<String> = [
    "dict", "hiragana", "katakana", "rewriter", "literal", "fallback", "emoji", "small_letter",
]

@Suite struct DiagPanelContextTests {
    @Test func panelIsNoneWhenTheWindowWasNeverShown() throws {
        let h = try Harness()
        h.type("hi")
        h.press(.enter)
        let event = try #require(h.engine.diagEvents.first)
        #expect(event.tags["panel"] == "none")
        #expect(event.tags["ctx"] == nil)  // 変換しなかったので周辺テキストは見ていない
    }

    @Test func panelKeepsTheFirstSourceAndCtxIsOkWhenSurroundingTextIsAvailable() throws {
        let h = try Harness()
        h.type("hinsi")
        h.press(.space)
        h.machine.notePanelShown(source: .accessibility)
        h.press(.space)
        h.machine.notePanelShown(source: .mouseScreen)  // 2 回目以降は見ない
        h.press(.enter)
        let event = try #require(h.engine.diagEvents.first)
        #expect(event.tags["panel"] == "accessibility")
        #expect(event.tags["ctx"] == "ok")
    }

    @Test func ctxIsNoneWhenSurroundingTextIsUnavailable() throws {
        let h = try Harness()
        h.surroundingText = .unavailable
        h.type("hinsi")
        h.press(.space)
        h.press(.enter)
        #expect(h.engine.diagEvents.first?.tags["ctx"] == "none")
    }

    @Test(arguments: [AnchorSource.client, .accessibility, .lastKnown, .mouseScreen])
    func everyAnchorSourceMapsToAnAllowedWord(source: AnchorSource) {
        #expect(panelWords.contains(DiagPanel(source).rawValue))
    }

    @Test func panelNotifiedAfterTheEpisodeEndedIsIgnored() throws {
        let h = try Harness()
        h.type("hi")
        h.press(.enter)
        h.machine.notePanelShown(source: .client)
        h.type("hi")
        h.press(.enter)
        #expect(h.engine.diagEvents.map { $0.tags["panel"] } == ["none", "none"])
    }
}

@Suite struct DiagShortcodePickKindTests {
    @Test func japaneseEmojiPickIsEmoji() throws {
        let h = try Harness(settings: shortcodeSettings, shortcodeTable: table)
        h.type(":eyes:")
        h.press(.space)  // 1 つ目の絵文字へ
        h.press(.enter)
        let event = try #require(h.engine.diagEvents.first)
        #expect(event.tags["mode"] == "shortcode_ja")
        #expect(event.tags["end"] == "shortcode_pick")
        #expect(event.tags["pick_kind"] == "emoji")
        #expect(event.nums["segs"] == nil)
    }

    @Test func japaneseTheTypedRowIsLiteral() throws {
        let h = try Harness(settings: shortcodeSettings, shortcodeTable: table)
        h.type(":eyes:")
        h.press(.enter)  // 先頭の「そのままの入力」を確定
        let event = try #require(h.engine.diagEvents.first)
        #expect(event.tags["end"] == "shortcode_pick")
        #expect(event.tags["pick_kind"] == "literal")
    }

    @Test func backspaceOnTheBareColonIsBsEmpty() throws {
        let h = try Harness(settings: shortcodeSettings, shortcodeTable: table)
        h.type(":")
        h.press(.backspace)
        let event = try #require(h.engine.diagEvents.first)
        #expect(event.tags["end"] == "bs_empty")
        #expect(event.nums["bs"] == 1)
    }
}

/// 英語のモード（`EnglishMode`）の合成 1 回。コントローラの代わりに結果を取り出して、状態機械のダブルへ渡す
private struct EnglishApp {
    var mode = EnglishMode()
    var clock: UInt64 = 0
    let engine: MockEngine
    let machine: InputStateMachine
    let settings = shortcodeSettings

    init(secure: Bool = false) throws {
        let engine = MockEngine()
        let h = try Harness(secure: secure, engine: engine)
        self.engine = engine
        machine = h.machine
        keep = h
        mode.setShortcodeTable(table)
    }

    private let keep: Harness
    var events: [DiagEventValue] { engine.diagEvents }

    mutating func press(_ key: KeyInput.Key, modifiers: KeyModifiers = []) {
        clock += 5_000_000
        let environment = EnglishShortcodeEnvironment(
            settings: settings, clientBundleID: "com.tinyspeck.slackmacgap", secure: false, isRepeat: false)
        _ = mode.handleText(KeyInput(key: key, modifiers: modifiers), environment: environment, now: clock)
        machine.recordEnglishEpisode(mode.takeFinishedDiag())
    }

    mutating func type(_ string: String) {
        for character in string { press(character == " " ? .space : .character(String(character))) }
    }

    mutating func flush(_ end: DiagEnd) {
        clock += 5_000_000
        _ = mode.flushShortcode(end: end, now: clock)
        machine.recordEnglishEpisode(mode.takeFinishedDiag())
    }
}

@Suite struct DiagEnglishShortcodeTests {
    @Test func pickingAnEmojiIsOneEpisode() throws {
        var app = try EnglishApp()
        app.type(":eyes:")
        #expect(app.events.isEmpty)  // 打鍵ごとには送らない
        app.mode.notePanelShown(source: .client)
        app.press(.down)  // 絵文字へ
        app.press(.enter)
        let event = try #require(app.events.first)
        try #require(app.events.count == 1)
        #expect(event.kind == "sh_episode")
        #expect(event.sessionID == Harness.sessionID)
        #expect(event.tags["mode"] == "shortcode_en")
        #expect(event.tags["end"] == "shortcode_pick")
        #expect(event.tags["pick_kind"] == "emoji")
        #expect(event.tags["panel"] == "client")
        #expect(event.tags["prev"] == nil)
        #expect(event.nums["keys"] == 8)
        #expect(event.nums["keys_to_conv"] == 6)
        #expect(event.nums["cand_moves"] == 1)
        #expect(event.nums["dur_ms"] ?? -1 > 0)
        #expect(event.nums["pick_ms"] ?? -1 > 0)
        #expect(event.nums["wait_us"] == nil)
    }

    @Test func englishSmallLetterPick() throws {
        var app = try EnglishApp()
        app.type(":m2:")
        app.press(.down)
        app.press(.enter)
        #expect(app.events.first?.tags["pick_kind"] == "small_letter")
    }

    @Test func aFlushCarriesTheReasonAndThePreviousState() throws {
        var app = try EnglishApp()
        app.type(":ey")
        app.flush(.modeSwitch)
        #expect(app.events.count == 1)
        #expect(app.events[0].tags["end"] == "mode_switch")
        #expect(app.events[0].tags["prev"] == "shortcode")
        #expect(app.events[0].tags["panel"] == "none")

        _ = app.mode.flush()  // 区切り（直前が語の頭になる）
        app.type(":eyes:")
        app.flush(.deactivate)
        try #require(app.events.count == 2)
        #expect(app.events[1].tags["end"] == "deactivate")
        #expect(app.events[1].tags["prev"] == "shortcode_choice")
    }

    @Test func retypeIsNotCounted() throws {
        var app = try EnglishApp()
        app.type(":ey")
        app.press(.character("!"))  // 名前の文字ではない: 打ち直し
        #expect(app.events.isEmpty)
        #expect(app.mode.diagEpisode == nil)
    }

    @Test func backspaceOnTheBareColonIsBsEmpty() throws {
        var app = try EnglishApp()
        app.type(":")
        app.press(.backspace)
        #expect(app.events.first?.tags["end"] == "bs_empty")
        #expect(app.events.first?.nums["bs"] == 1)
    }

    @Test func ordinaryTypingMakesNoEpisode() throws {
        var app = try EnglishApp()
        app.type("hello world")
        app.press(.enter)
        #expect(app.events.isEmpty)
    }

    @Test func secureNeverSends() throws {
        var app = try EnglishApp(secure: true)
        app.type(":eyes:")
        app.press(.down)
        app.press(.enter)
        #expect(app.events.isEmpty)
    }

    @Test func allWordsAreInTheAllowList() throws {
        var app = try EnglishApp()
        app.type(":eyes:")
        app.press(.down)
        app.press(.enter)
        let event = try #require(app.events.first)
        #expect(pickKindWords.contains(event.tags["pick_kind"] ?? ""))
        #expect(panelWords.contains(event.tags["panel"] ?? ""))
    }
}

@Suite struct DiagModeSwitchTests {
    @Test func recordsDirectionAndRoute() throws {
        let h = try Harness()
        h.machine.recordModeSwitch(from: .japanese, to: .english, viaKey: true)
        h.machine.recordModeSwitch(from: .english, to: .japanese, viaKey: false)
        let events = h.engine.diagEvents
        try #require(events.count == 2)
        #expect(events[0].kind == "sh_mode")
        #expect(events[0].sessionID == Harness.sessionID)
        #expect(events[0].tags == ["from": "ja", "to": "en", "via": "key"])
        #expect(events[0].nums.isEmpty)
        #expect(events[1].tags == ["from": "en", "to": "ja", "via": "os"])
    }

    @Test func nothingForTheSameModeOrWhenSecure() throws {
        let h = try Harness()
        h.machine.recordModeSwitch(from: .japanese, to: .japanese, viaKey: true)
        #expect(h.engine.diagEvents.isEmpty)

        let secure = try Harness(secure: true)
        secure.machine.recordModeSwitch(from: .japanese, to: .english, viaKey: true)
        #expect(secure.engine.diagEvents.isEmpty)
    }

    @Test func switchingMidCompositionSendsTheEpisodeFirst() throws {
        let h = try Harness()
        h.type("hi")
        _ = h.machine.commitComposition(reason: .modeSwitch)
        h.machine.recordModeSwitch(from: .japanese, to: .english, viaKey: true)
        #expect(h.engine.diagEvents.map(\.kind) == ["sh_episode", "sh_mode"])
        #expect(h.engine.diagEvents.first?.tags["end"] == "mode_switch")
    }
}
