// FR-07: 末尾の未確定が「n」1文字のときだけ、変換を始める直前に「ん」を確定する（2026-10-13 ユーザー決定）。
import Foundation
import Testing

@testable import IkariShellCore

@Suite struct TrailingNTests {
    @Test func aTrailingNBecomesNWhenSpaceStartsTheConversion() throws {
        let h = try Harness()
        h.type("hin")
        #expect(h.machine.state == .composing(kana: "ひ", pending: "n"))
        h.press(.space)
        #expect(h.engine.convertRequests.first?.reading == "ひん")
        #expect(h.converting?.reading == "ひん")
        #expect(h.composer.pending.isEmpty)
    }

    @Test func otherPendingConsonantsAreStillExcluded() throws {
        let h = try Harness()
        h.type("hik")
        h.press(.space)
        #expect(h.engine.convertRequests.first?.reading == "ひ")
    }

    @Test func nnIsNotChanged() throws {
        let h = try Harness()
        h.type("hinn")
        #expect(h.machine.state == .composing(kana: "ひん", pending: ""))
        h.press(.space)
        #expect(h.engine.convertRequests.first?.reading == "ひん")
    }

    @Test func aLoneNDoesNotStartAnEmptyReading() throws {
        let h = try Harness()
        h.type("n")
        h.press(.space)
        // 「n」だけでも、ん として変換を始める（読みは空ではない）
        #expect(h.engine.convertRequests.first?.reading == "ん")
    }

    @Test func goingBackToTheReadingKeepsTheConfirmedN() throws {
        let h = try Harness()
        h.type("hin")
        h.press(.space)
        h.press(.escape)
        #expect(h.machine.state == .composing(kana: "ひん", pending: ""))
    }

    @Test func afterBackspaceFromAReconversionTheTrailingNIsConfirmedToo() throws {
        let h = try ReconvertFixture.converting()
        h.press(.backspace)
        h.type("n")
        #expect(h.machine.state == .composing(kana: "ひんし", pending: "n"))
        h.engine.onConvert = { _ in MockEngine.defaultPages[0] }
        h.press(.space)
        #expect(h.engine.convertRequests.first?.reading == "ひんしん")
    }

    @Test func aScriptConversionAlsoConfirmsTheTrailingN() throws {
        let h = try Harness()
        h.type("hin")
        h.press(.function(7))
        #expect(h.converting?.reading == "ひん")
    }
}
