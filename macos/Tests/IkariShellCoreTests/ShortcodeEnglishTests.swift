// 英語のモードの絵文字のショートコード（ADR 0027 §3.2、§3.4 の英語の列、§3.6 の英語の行。B48）。
// コントローラの代わりに、`EnglishMode.handleText` の結果をアプリのテキストに当てはめる小さな模擬を使う。
import Testing

@testable import IkariShellCore

private let table = ShortcodeTable(entries: [
    ShortcodeEntry(name: "eye", emoji: "👁"),
    ShortcodeEntry(name: "eyes", emoji: "👀"),
    ShortcodeEntry(name: "smile", emoji: "😄"),
    ShortcodeEntry(name: "tada", emoji: "🎉"),
])

/// アプリの代わり。確定したテキスト、変換中の文字列、候補ウィンドウ、アプリに届いたキー
private struct App {
    var mode = EnglishMode()
    var settings: Settings = {
        var settings = Settings.default
        settings.emojiShortcode = true
        return settings
    }()
    var bundleID = "com.tinyspeck.slackmacgap"
    var secure = false
    /// アプリのテキスト（素通しのキーと確定）
    var text = ""
    var marked = ""
    var markedUnderlined = true
    var panelRows: [String] = []
    var panelVisible = false
    /// 区切りで送られた確定履歴（`CommitText` の surface。読みは空）
    var sent: [String] = []
    /// Enter など、アプリに渡したが文字にならないキー
    var passedKeys: [KeyInput.Key] = []

    init(table: ShortcodeTable = table) {
        mode.setShortcodeTable(table)
    }

    mutating func type(_ string: String) {
        for character in string { press(character == " " ? .space : .character(String(character))) }
    }

    mutating func press(_ key: KeyInput.Key, modifiers: KeyModifiers = [], isRepeat: Bool = false) {
        let input = KeyInput(key: key, modifiers: modifiers)
        let environment = EnglishShortcodeEnvironment(
            settings: settings, clientBundleID: bundleID, secure: secure, isRepeat: isRepeat)
        let outcome = mode.handleText(input, environment: environment)
        apply(outcome)
        guard outcome.passKey else { return }
        switch key {
        case .character(let string) where modifiers.isEmpty || modifiers == [.shift]: text += string
        case .space: text += " "
        default: passedKeys.append(key)
        }
    }

    mutating func apply(_ outcome: EnglishTextOutcome) {
        for action in outcome.actions {
            switch action {
            case .setMarkedText(let mark):
                marked = mark.text
                markedUnderlined = mark.underlined
            case .commit(let committed):
                text += committed
                marked = ""
            case .showCandidates(let model), .moveSelection(let model):
                panelVisible = true
                panelRows = model.candidates.map(\.surface)
            case .hideCandidates:
                panelVisible = false
                panelRows = []
            default: break
            }
        }
        sent += outcome.flushed
    }

    /// モードの切り替え、クリックなど
    mutating func flush() {
        if let outcome = mode.flushShortcode() { apply(outcome) }
        if let pooled = mode.flush() { sent.append(pooled) }
    }
}

@Suite struct EnglishWordStartTests {
    @Test(arguments: [nil, " ", "\n", "\t", "(", "[", "{", "\u{3000}"] as [Character?])
    func entersAfter(last: Character?) {
        #expect(EnglishMode.isWordStart(after: last))
    }

    @Test(arguments: ["0", "a", "Z", "/", ":", "\"", "'", ")", "-", "."] as [Character])
    func doesNotEnterAfter(last: Character) {
        #expect(!EnglishMode.isWordStart(after: last))
    }
}

@Suite struct EnglishShortcodeTableTests {
    // §3.6 の英語の行
    @Test func timeIsUntouched() {
        var app = App()
        app.type("10:30")
        #expect(app.text == "10:30")
        #expect(app.marked.isEmpty && !app.panelVisible)
    }

    @Test func urlIsUntouched() {
        var app = App()
        app.type("https://x")
        #expect(app.text == "https://x")
    }

    @Test func labelColonIsUntouched() {
        var app = App()
        app.type("Note: hi")
        #expect(app.text == "Note: hi")
    }

    @Test func smileyRetypesAtCloseParen() {
        var app = App()
        app.type(" :")
        #expect(app.marked == ":")
        #expect(app.text == " ")
        app.type(")")
        #expect(app.text == " :)")
        #expect(app.marked.isEmpty)
    }

    @Test func typingIgnoringTheShortcodeGivesTheSameText() {
        var app = App()
        app.type(" :eyes:")
        #expect(app.marked == ":eyes:")
        #expect(app.panelRows == [":eyes:", "👀"])
        app.type(" ")
        #expect(app.text == " :eyes: ")
        #expect(app.marked.isEmpty && !app.panelVisible)
    }

    @Test func backspaceOnCloseColonThenColonAgainShowsCandidatesAgain() {
        var app = App()
        app.type(" :eyes:")
        #expect(app.panelVisible)
        app.press(.backspace)
        #expect(!app.panelVisible)
        #expect(app.marked == ":eyes")
        app.type(":")
        #expect(app.panelVisible)
        #expect(app.panelRows == [":eyes:", "👀"])
    }

    @Test func smallLettersBackspaceThenColonAgain() {
        var app = App()
        app.settings.emojiSmallLetters = true
        app.type(" :m2:")
        #expect(app.panelRows == [":m2:", "m²", "㎡"])
        app.press(.backspace)
        #expect(app.marked == ":m2")
        app.type(":")
        #expect(app.panelRows == [":m2:", "m²", "㎡"])
    }

    @Test func enterOnFirstCandidatePassesEnter() {
        var app = App()
        app.type(" :eyes:")
        app.press(.enter)
        #expect(app.text == " :eyes:")
        #expect(app.passedKeys == [.enter])
        #expect(!app.panelVisible)
    }

    @Test func enterOnEmojiCommitsAndConsumesEnter() {
        var app = App()
        app.type(" :eyes:")
        app.press(.down)
        #expect(app.marked == "👀")
        app.press(.enter)
        #expect(app.text == " 👀")
        #expect(app.passedKeys.isEmpty)
        #expect(!app.panelVisible)
    }

    @Test func rubySymbolRetypes() {
        var app = App()
        app.type(" :foo,")
        #expect(app.text == " :foo,")
        #expect(app.marked.isEmpty)
    }

    @Test func markdownTableRetypes() {
        var app = App()
        app.type(" :---")
        // B52: `-` は名前の文字なので状態を保つ。次の非名前文字（表の ` |`）かフラッシュで確定される
        #expect(app.marked == ":---")
        app.type(" |")
        #expect(app.text == " :--- |")
        #expect(app.marked.isEmpty)
    }

    @Test func customEmojiRetypesWhenPrefixStopsMatching() {
        var app = App()
        app.type(" :p")
        // B52: 前方一致が切れても名前の文字が続く間は状態を保つ（Backspace で直せる）
        #expect(app.text == " ")
        #expect(app.marked == ":p")
        #expect(!app.markedUnderlined)
        // 閉じの `:` が完全一致しないので打ち直し。以後は Slack 自前の補完。ikari は何もしない
        app.type("arty_parrot:")
        #expect(app.text == " :party_parrot:")
        #expect(app.marked.isEmpty)
        #expect(!app.panelVisible)
    }

    // B52: 途中のタイポを直せる
    @Test func typoThenBackspaceThenFinishShowsCandidates() {
        var app = App()
        app.type(" :eyx")
        #expect(app.marked == ":eyx")
        #expect(app.text == " ")
        app.press(.backspace)
        #expect(app.marked == ":ey")
        app.type("es:")
        #expect(app.panelRows == [":eyes:", "👀"])
        #expect(app.marked == ":eyes:")
    }

    @Test func typoClosedWithColonRetypesTheWholeString() {
        var app = App()
        app.type(" :eyxs:")
        #expect(app.text == " :eyxs:")
        #expect(app.marked.isEmpty && !app.panelVisible)
    }

    @Test func tabAndDownRetypeWithoutPrefixMatch() {
        var tab = App()
        tab.type(" :eyx")
        tab.press(.character("\t"))
        #expect(tab.text == " :eyx\t")
        #expect(tab.marked.isEmpty)
        #expect(!tab.panelVisible)
        var down = App()
        down.type(" :eyx")
        down.press(.down)
        #expect(down.text == " :eyx")
        #expect(down.passedKeys == [.down])
        #expect(!down.panelVisible)
        var matched = App()
        matched.type(" :ey")
        matched.press(.character("\t"))
        #expect(matched.panelRows == [":ey", "👁", "👀"])
    }

    @Test(arguments: [" :foo,", " :D", " :P", " :)", " :3 ", " :eyx!", " :eyxA", " :ey\u{3042}"])
    func nonNameCharactersStillRetypeImmediately(_ typed: String) {
        var app = App()
        app.type(typed)
        #expect(app.text == typed)
        #expect(app.marked.isEmpty && !app.panelVisible)
    }

    @Test func hyphenRunIsCommittedByFlush() {
        var app = App()
        app.type(" :---")
        app.flush()
        #expect(app.text == " :---")
        #expect(app.marked.isEmpty)
    }

    @Test func nameLengthLimitRetypes() {
        var app = App()
        let name = String(repeating: "x", count: ShortcodeMachine.maxNameLength)
        app.type(" :" + name)
        #expect(app.marked == ":" + name)
        app.type("x")
        #expect(app.marked.isEmpty)
        #expect(app.text == " :" + name + "x")
    }

    @Test func backspaceToColonThenBackspaceCancels() {
        var app = App()
        app.type(" :zz")
        app.press(.backspace)
        app.press(.backspace)
        #expect(app.marked == ":")
        app.press(.backspace)
        #expect(app.marked.isEmpty)
        #expect(app.text == " ")
    }
}

@Suite struct EnglishShortcodeKeysTests {
    @Test func prefixCandidatesWithDownThenSelect() {
        var app = App()
        app.type(" :ey")
        #expect(app.marked == ":ey")
        app.press(.down)
        #expect(app.panelRows == [":ey", "👁", "👀"])
        app.press(.down)
        app.press(.down)
        #expect(app.marked == "👀")
        app.press(.up)
        #expect(app.marked == "👁")
        app.type(" ")
        #expect(app.text == " 👁 ")
    }

    @Test func tabAndShiftTabMoveSelection() {
        var app = App()
        app.type(":ey")
        app.press(.character("\t"))
        app.press(.character("\t"))
        #expect(app.marked == "👁")
        app.press(.character("\t"), modifiers: [.shift])
        #expect(app.marked == ":ey")
        app.press(.character("\u{19}"))
        #expect(app.marked == "👀")
    }

    @Test func spaceInTypingRetypesWithSpace() {
        var app = App()
        app.type(":ey ")
        #expect(app.text == ":ey ")
        #expect(app.marked.isEmpty)
    }

    @Test func digitCommitsSelectedAndPassesDigit() {
        var app = App()
        app.type(":eyes:")
        app.press(.down)
        app.press(.character("1"))
        #expect(app.text == "👀1")
    }

    @Test func letterCommitsAndPassesLetter() {
        var app = App()
        app.type(":eyes:")
        app.type("x")
        #expect(app.text == ":eyes:x")
    }

    @Test func backspaceCancelsBareColon() {
        var app = App()
        app.type(":")
        app.press(.backspace)
        #expect(app.text.isEmpty && app.marked.isEmpty)
        #expect(app.passedKeys.isEmpty)
    }

    @Test func backspaceShortensName() {
        var app = App()
        app.type(":eye")
        app.press(.backspace)
        #expect(app.marked == ":ey")
    }

    @Test func backspaceFromChoicesReturnsToTyping() {
        var app = App()
        app.type(":ey")
        app.press(.down)
        app.press(.backspace)
        #expect(app.marked == ":ey")
        #expect(!app.panelVisible)
    }

    @Test func escapeInChoicesCommitsLiteralAndConsumes() {
        var app = App()
        app.type(":eyes:")
        app.press(.down)
        app.press(.escape)
        #expect(app.text == ":eyes:")
        #expect(app.passedKeys.isEmpty)
    }

    @Test func escapeInTypingCommitsAndPassesEscape() {
        var app = App()
        app.type(":ey")
        app.press(.escape)
        #expect(app.text == ":ey")
        #expect(app.passedKeys == [.escape])
    }

    @Test func arrowAndModifierKeysCommitAndPass() {
        var app = App()
        app.type(":eyes:")
        app.press(.left)
        #expect(app.text == ":eyes:")
        #expect(app.passedKeys == [.left])

        app.type(" :ey")
        app.press(.character("c"), modifiers: [.command])
        #expect(app.text == ":eyes: :ey")
        #expect(app.passedKeys == [.left, .character("c")])
    }

    @Test func committedTextGoesToThePool() {
        var app = App()
        app.type("a :eyes:")
        app.press(.down)
        app.press(.enter)
        // Enter は消費。たまりはまだ区切られていない
        #expect(app.sent.isEmpty)
        app.type(" b")
        app.press(.enter)
        #expect(app.sent == ["a 👀 b"])
    }

    @Test func passedEnterAfterLiteralFlushesPoolWithShortcodeText() {
        var app = App()
        app.type(":eyes:")
        app.press(.enter)
        #expect(app.sent == [":eyes:"])
    }

    @Test func repeatedColonDoesNotEnter() {
        var app = App()
        app.press(.character(":"), isRepeat: true)
        #expect(app.text == ":")
        #expect(app.marked.isEmpty)
    }
}

@Suite struct EnglishShortcodeEntryConditionTests {
    @Test func lastPoolCharacterDecidesEntry() {
        var app = App()
        app.type("(:")
        #expect(app.marked == ":")
        app.press(.escape)
        app.type("a:")
        #expect(app.marked.isEmpty)
    }

    @Test func emptyPoolEntersAfterSeparator() {
        var app = App()
        app.type("ab")
        app.press(.enter)
        app.type(":e")
        #expect(app.marked == ":e")
    }

    @Test func settingOffDoesNotEnter() {
        var app = App()
        app.settings.emojiShortcode = false
        app.type(" :eyes:")
        #expect(app.text == " :eyes:")
        #expect(app.marked.isEmpty)
    }

    @Test func excludedAppDoesNotEnter() {
        var app = App()
        app.bundleID = "com.apple.Terminal"
        app.type(" :eyes:")
        #expect(app.text == " :eyes:")
        #expect(app.marked.isEmpty)
    }

    @Test func secureDoesNotEnter() {
        var app = App()
        app.secure = true
        app.type(" :eyes:")
        #expect(app.text == " :eyes:")
        #expect(app.marked.isEmpty)
    }

    @Test func emptyTableDoesNotEnter() {
        var app = App(table: .empty)
        app.type(" :eyes:")
        #expect(app.text == " :eyes:")
        #expect(app.marked.isEmpty)
    }

    @Test func modifierColonDoesNotEnter() {
        var app = App()
        app.press(.character(":"), modifiers: [.command])
        #expect(app.marked.isEmpty)
        #expect(app.passedKeys == [.character(":")])
    }
}

@Suite struct EnglishShortcodeFlushTests {
    @Test func flushCommitsTypingAsIs() {
        var app = App()
        app.type("x :ey")
        app.flush()
        #expect(app.text == "x :ey")
        #expect(!app.mode.isShortcodeActive)
        #expect(app.sent == ["x :ey"])
    }

    @Test func flushCommitsSelectedCandidateAndClosesPanel() {
        var app = App()
        app.type(":ey")
        app.press(.down)
        app.press(.down)
        #expect(app.panelVisible)
        app.flush()
        #expect(app.text == "👁")
        #expect(!app.panelVisible)
        #expect(app.sent == ["👁"])
    }

    @Test func flushWithoutShortcodeDoesNothing() {
        var app = App()
        app.type("hi")
        #expect(app.mode.flushShortcode() == nil)
    }
}

@Suite struct EnglishShortcodeUnderlineTests {
    @Test func typingHasNoUnderline() {
        var app = App()
        app.type(" :ey")
        #expect(app.marked == ":ey")
        #expect(!app.markedUnderlined)
    }

    @Test func choosingHasAnUnderline() {
        var app = App()
        app.type(" :eyes:")
        #expect(app.markedUnderlined)
    }

    @Test func downChoosingHasAnUnderline() {
        var app = App()
        app.type(" :ey")
        app.press(.down)
        #expect(app.panelVisible)
        #expect(app.markedUnderlined)
    }
}
