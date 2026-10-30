// 小さい文字の候補（`emoji.small_letters`。`:m2:` → m²、`:m_2:` → m₂、`:cm:` → ㎝。#264）
import Testing

@testable import IkariShellCore

@Suite struct SmallLettersRuleTests {
    @Test(arguments: [
        ("m2", ["m²", "㎡"]), ("m3", ["m³", "㎥"]), ("x10", ["x¹⁰"]), ("2", ["²"]),
        ("m_2", ["m₂"]), ("x_n", ["xₙ"]), ("a_i", ["aᵢ"]), ("_2", ["₂"]), ("x_n+1", ["xₙ₊₁"]),
        ("cm", ["㎝"]), ("cm2", ["cm²", "㎠"]), ("kg", ["㎏"]),
    ] as [(String, [String])])
    func candidates(name: String, expected: [String]) {
        #expect(SmallLetters.candidates(for: name) == expected)
    }

    @Test(arguments: ["", "m", "eyes", "x_", "x_q", "m2x", "x_n_1", "2m"])
    func noCandidates(name: String) {
        #expect(SmallLetters.candidates(for: name).isEmpty)
    }
}

private let table = ShortcodeTable(entries: [ShortcodeEntry(name: "eyes", emoji: "👀")])

@Suite struct SmallLettersEnglishTests {
    private func run(_ typed: String, on: Bool) -> (text: String, panel: [String], marked: String) {
        var mode = EnglishMode()
        mode.setShortcodeTable(table)
        var settings = Settings.default
        settings.emojiShortcode = true
        settings.emojiSmallLetters = on
        var text = "", marked = "", panel: [String] = []
        for character in typed {
            let input = KeyInput(key: .character(String(character)), modifiers: [])
            let environment = EnglishShortcodeEnvironment(
                settings: settings, clientBundleID: "com.example", secure: false, isRepeat: false)
            let outcome = mode.handleText(input, environment: environment)
            for action in outcome.actions {
                switch action {
                case .setMarkedText(let mark): marked = mark.text
                case .commit(let committed): text += committed; marked = ""
                case .showCandidates(let model), .moveSelection(let model): panel = model.candidates.map(\.surface)
                default: break
                }
            }
            if outcome.passKey { text += String(character) }
        }
        return (text, panel, marked)
    }

    @Test func offByDefaultLeavesTheTextAlone() {
        let result = run(":m2:", on: false)
        #expect(result.text == ":m2:")
        #expect(result.panel.isEmpty)
        #expect(!Settings.default.emojiSmallLetters)
    }

    @Test func onShowsTheTypedTextThenTheSmallLetters() {
        let result = run(":m2:", on: true)
        #expect(result.panel == [":m2:", "m²", "㎡"])
        #expect(result.marked == ":m2:")
    }

    @Test func emojiTableWinsOverTheRule() {
        #expect(run(":eyes:", on: true).panel == [":eyes:", "👀"])
    }

    @Test func settingParsesAndRoundTrips() {
        let parsed = Settings.parse(tsv: "emoji.small_letters\ton\n")
        #expect(parsed.warnings.isEmpty)
        #expect(parsed.settings.emojiSmallLetters)
        #expect(Settings.parse(tsv: "emoji.small_letters\tyes\n").warnings.count == 1)
    }
}
