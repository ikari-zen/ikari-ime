// 文節ごとの下線と markedClauseSegment（ADR 0006 §8.3）。
import AppKit
import IkariShellCore
import Testing

@testable import IkariIME

@Suite struct MarkedTextAttributesTests {
    private func style(_ text: NSAttributedString, at index: Int) -> Int? {
        text.attribute(.underlineStyle, at: index, effectiveRange: nil) as? Int
    }

    private func clause(_ text: NSAttributedString, at index: Int) -> Int? {
        text.attribute(.markedClauseSegment, at: index, effectiveRange: nil) as? Int
    }

    @Test func theFocusedSegmentIsThickAndTheOthersAreThinWithClauseNumbers() {
        let text = MarkedTextAttributes.attributedString(
            for: MarkedText(text: "初めて知った", segments: [3, 3], focused: 1))
        #expect(style(text, at: 0) == NSUnderlineStyle.single.rawValue)
        #expect(style(text, at: 2) == NSUnderlineStyle.single.rawValue)
        #expect(style(text, at: 3) == NSUnderlineStyle.thick.rawValue)
        #expect(style(text, at: 5) == NSUnderlineStyle.thick.rawValue)
        #expect([0, 2, 3, 5].map { clause(text, at: $0) } == [0, 0, 1, 1])
    }

    @Test func segmentLengthsAreUtf16UnitsSoAnEmojiIsTwoUnits() {
        let text = MarkedTextAttributes.attributedString(for: MarkedText(text: "😀知った", segments: [2, 3], focused: 1))
        #expect(style(text, at: 1) == NSUnderlineStyle.single.rawValue)
        #expect(style(text, at: 2) == NSUnderlineStyle.thick.rawValue)
        #expect(clause(text, at: 4) == 1)
    }

    @Test func aTextWithoutSegmentsKeepsTheOldLook() {
        let composing = MarkedTextAttributes.attributedString(for: MarkedText(text: "ひんし", highlightedLength: 2))
        #expect(style(composing, at: 0) == NSUnderlineStyle.thick.rawValue)
        #expect(style(composing, at: 2) == NSUnderlineStyle.single.rawValue)
        #expect(clause(composing, at: 0) == nil)
        let converting = MarkedTextAttributes.attributedString(for: MarkedText(text: "品詞", highlightedLength: 2))
        #expect(style(converting, at: 1) == NSUnderlineStyle.thick.rawValue)
    }

    @Test func shortSegmentLengthsNeverLeaveACharacterWithoutAnUnderline() {
        let text = MarkedTextAttributes.attributedString(for: MarkedText(text: "あいう", segments: [1], focused: 0))
        #expect(style(text, at: 2) == NSUnderlineStyle.single.rawValue)
    }

    @Test func emptyTextHasNoAttributes() {
        #expect(MarkedTextAttributes.attributedString(for: MarkedText(text: "")).length == 0)
    }

    @Test func notUnderlinedHasNoUnderlineStyleButStaysMarked() {
        let text = MarkedTextAttributes.attributedString(
            for: MarkedText(text: ":ey", highlightedLength: 3, underlined: false))
        #expect((0..<3).allSatisfy { style(text, at: $0) == nil })
        #expect(clause(text, at: 0) == 0)
        let underlined = MarkedTextAttributes.attributedString(for: MarkedText(text: ":ey", highlightedLength: 3))
        #expect(style(underlined, at: 0) == NSUnderlineStyle.thick.rawValue)
    }
}
