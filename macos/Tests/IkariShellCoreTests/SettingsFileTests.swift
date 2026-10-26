// SettingsFile.update（ADR 0005 §7.1、§10.1）。
import Testing

@testable import IkariShellCore

@Suite struct SettingsFileTests {
    private func update(_ text: String, set: [(String, String)] = [], reset: [String] = []) -> String {
        SettingsFile.update(text: text, set: set.map { (key: $0.0, value: $0.1) }, reset: reset)
    }

    @Test func replacesOnlyTheValueAndKeepsCommentsUnknownKeysAndOrder() {
        let text = "# my notes\ninput.method\tromaji\n\n# lm\nengine.lm\ton\nfuture.key\tkeep\npaging.size\t5\n"
        let result = update(text, set: [("input.method", "kana")])
        #expect(result == "# my notes\ninput.method\tkana\n\n# lm\nengine.lm\ton\nfuture.key\tkeep\npaging.size\t5\n")
    }

    @Test func replacesTheLastLineWhenTheKeyRepeats() {
        let text = "paging.size\t3\npaging.wrap\ttrue\npaging.size\t4\n"
        #expect(update(text, set: [("paging.size", "7")]) == "paging.size\t3\npaging.wrap\ttrue\npaging.size\t7\n")
    }

    @Test func appendsMissingKeysInTheGivenOrder() {
        let result = update("# c\ninput.method\tkana\n", set: [("paging.size", "2"), ("paging.wrap", "false")])
        #expect(result == "# c\ninput.method\tkana\npaging.size\t2\npaging.wrap\tfalse\n")
    }

    @Test func appendsToAFileWithoutATrailingNewline() {
        #expect(update("input.method\tkana", set: [("paging.size", "2")]) == "input.method\tkana\npaging.size\t2\n")
    }

    @Test func appendsToAnEmptyFile() {
        #expect(update("", set: [("input.method", "kana")]) == "input.method\tkana\n")
    }

    @Test func keepsTheTrailingNewlineStateWhenNothingIsAppended() {
        #expect(update("paging.size\t3", set: [("paging.size", "4")]) == "paging.size\t4")
        #expect(update("paging.size\t3\n", set: [("paging.size", "4")]) == "paging.size\t4\n")
    }

    @Test func resetRemovesEveryLineOfTheKeyOnly() {
        let text = "# keep\npaging.size\t3\ninput.method\tkana\npaging.size\t4\n"
        #expect(update(text, reset: ["paging.size"]) == "# keep\ninput.method\tkana\n")
    }

    @Test func resettingAMissingKeyChangesNothing() {
        let text = "input.method\tkana\n"
        #expect(update(text, reset: ["paging.size"]) == text)
    }

    @Test func aKeyInBothSetAndResetEndsUpWithTheSetValue() {
        let result = update("paging.size\t3\npaging.size\t4\n", set: [("paging.size", "9")], reset: ["paging.size"])
        #expect(result == "paging.size\t9\n")
    }

    @Test func aRepeatedKeyInSetKeepsTheLastValueOnce() {
        let result = update("", set: [("paging.size", "2"), ("paging.size", "3")])
        #expect(result == "paging.size\t3\n")
    }

    @Test func commentedOutKeysAreNotTreatedAsLines() {
        let result = update("#paging.size\t3\n", set: [("paging.size", "5")])
        #expect(result == "#paging.size\t3\npaging.size\t5\n")
    }

    @Test func preservesCRLFLineEndings() {
        let result = update("input.method\tromaji\r\npaging.size\t3\r\n", set: [("input.method", "kana")])
        #expect(result == "input.method\tkana\r\npaging.size\t3\r\n")
    }

    @Test func ignoresKeysAndValuesThatCouldInjectLines() {
        let text = "input.method\tkana\n"
        #expect(update(text, set: [("paging.size", "3\nengine.lm\toff")]) == text)
        #expect(update(text, set: [("paging.size\tx", "3")]) == text)
        #expect(update(text, set: [("", "3")]) == text)
        #expect(update(text, set: [("#x", "3")]) == text)
        #expect(update(text, reset: ["input.method\n"]) == text)
    }

    @Test func theResultParsesBackToTheRequestedValues() {
        let text = "# notes\nkeymap.preset\tmacos\nengine.lm\ton\n"
        let result = update(
            text, set: [("keymap.preset", "ms-ime"), ("keymap.convert.hiragana", "none"), ("engine.lm", "off")])
        let parsed = Settings.parse(tsv: result)
        #expect(parsed.warnings.isEmpty)
        #expect(parsed.settings.keymapPreset == .msIme)
        #expect(parsed.settings.keymapOverrides[.toHiragana] == Settings.noneValue)
        #expect(!parsed.settings.engineLM)
    }

    @Test func unchangedInputIsReturnedAsIs() {
        let text = "# only comments\n\n"
        #expect(update(text) == text)
    }
}
