// 文節ごとの下線と markedClauseSegment（ADR 0006 §8.3）。
import AppKit
import IkariProto
import Testing

@testable import IkariIME

/// shell-core が送る形（proto3 の既定は下線なしなので、下線つきを既定にする）
private func marked(
    text: String, highlightedLength: UInt32 = 0, segments: [UInt32] = [], focused: UInt32? = nil, underlined: Bool = true
) -> Ikari_Shell_V1_MarkedText {
    var marked = Ikari_Shell_V1_MarkedText()
    marked.text = text
    marked.highlightedLength = highlightedLength
    marked.segments = segments
    if let focused { marked.focused = focused }
    marked.underlined = underlined
    return marked
}

@Suite struct MarkedTextAttributesTests {
    private func style(_ text: NSAttributedString, at index: Int) -> Int? {
        text.attribute(.underlineStyle, at: index, effectiveRange: nil) as? Int
    }

    private func clause(_ text: NSAttributedString, at index: Int) -> Int? {
        text.attribute(.markedClauseSegment, at: index, effectiveRange: nil) as? Int
    }

    @Test func theFocusedSegmentIsThickAndTheOthersAreThinWithClauseNumbers() {
        let text = MarkedTextAttributes.attributedString(
            for: marked(text: "初めて知った", segments: [3, 3], focused: 1))
        #expect(style(text, at: 0) == NSUnderlineStyle.single.rawValue)
        #expect(style(text, at: 2) == NSUnderlineStyle.single.rawValue)
        #expect(style(text, at: 3) == NSUnderlineStyle.thick.rawValue)
        #expect(style(text, at: 5) == NSUnderlineStyle.thick.rawValue)
        #expect([0, 2, 3, 5].map { clause(text, at: $0) } == [0, 0, 1, 1])
    }

    @Test func deleteTargetStrikesOnlyTheFocusedSegment() {
        var input = marked(text: "初めて知った", segments: [3, 3], focused: 1)
        input.deleteTarget = true
        let text = MarkedTextAttributes.attributedString(for: input)
        func strike(_ index: Int) -> Int? { text.attribute(.strikethroughStyle, at: index, effectiveRange: nil) as? Int }
        func background(_ index: Int) -> NSColor? { text.attribute(.backgroundColor, at: index, effectiveRange: nil) as? NSColor }
        #expect([0, 2].map { strike($0) } == [nil, nil])
        #expect([3, 5].map { strike($0) } == [NSUnderlineStyle.single.rawValue, NSUnderlineStyle.single.rawValue])
        #expect(background(0) == nil)
        #expect(background(4) == MarkedTextAttributes.deleteTargetBackground)
        // 下線は今のまま（フォーカスは太い）
        #expect(style(text, at: 4) == NSUnderlineStyle.thick.rawValue)
    }

    @Test func withoutDeleteTargetThereIsNoStrikethrough() {
        let text = MarkedTextAttributes.attributedString(for: marked(text: "初めて知った", segments: [3, 3], focused: 1))
        #expect(text.attribute(.strikethroughStyle, at: 4, effectiveRange: nil) == nil)
        #expect(text.attribute(.backgroundColor, at: 4, effectiveRange: nil) == nil)
    }

    @Test func deleteTargetWithoutSegmentsMarksTheWholeText() {
        var input = marked(text: "ひんし")
        input.deleteTarget = true
        let text = MarkedTextAttributes.attributedString(for: input)
        #expect(text.attribute(.strikethroughStyle, at: 2, effectiveRange: nil) != nil)
    }

    @Test func segmentLengthsAreUtf16UnitsSoAnEmojiIsTwoUnits() {
        let text = MarkedTextAttributes.attributedString(for: marked(text: "😀知った", segments: [2, 3], focused: 1))
        #expect(style(text, at: 1) == NSUnderlineStyle.single.rawValue)
        #expect(style(text, at: 2) == NSUnderlineStyle.thick.rawValue)
        #expect(clause(text, at: 4) == 1)
    }

    @Test func aTextWithoutSegmentsKeepsTheOldLook() {
        let composing = MarkedTextAttributes.attributedString(for: marked(text: "ひんし", highlightedLength: 2))
        #expect(style(composing, at: 0) == NSUnderlineStyle.thick.rawValue)
        #expect(style(composing, at: 2) == NSUnderlineStyle.single.rawValue)
        #expect(clause(composing, at: 0) == nil)
        let converting = MarkedTextAttributes.attributedString(for: marked(text: "品詞", highlightedLength: 2))
        #expect(style(converting, at: 1) == NSUnderlineStyle.thick.rawValue)
    }

    @Test func shortSegmentLengthsNeverLeaveACharacterWithoutAnUnderline() {
        let text = MarkedTextAttributes.attributedString(for: marked(text: "あいう", segments: [1], focused: 0))
        #expect(style(text, at: 2) == NSUnderlineStyle.single.rawValue)
    }

    @Test func emptyTextHasNoAttributes() {
        #expect(MarkedTextAttributes.attributedString(for: marked(text: "")).length == 0)
    }

    @Test func notUnderlinedHasNoUnderlineStyleButStaysMarked() {
        let text = MarkedTextAttributes.attributedString(
            for: marked(text: ":ey", highlightedLength: 3, underlined: false))
        #expect((0..<3).allSatisfy { style(text, at: $0) == nil })
        #expect(clause(text, at: 0) == 0)
        let underlined = MarkedTextAttributes.attributedString(for: marked(text: ":ey", highlightedLength: 3))
        #expect(style(underlined, at: 0) == NSUnderlineStyle.thick.rawValue)
    }
}
