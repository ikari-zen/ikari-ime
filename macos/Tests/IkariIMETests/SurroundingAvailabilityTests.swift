// 周辺テキストを「取れた」と言ってよいか（ADR 0015 D2）。左が空で、文書の長さも 0 なら取れていない。
import Foundation
import Testing

@testable import IkariIME
@testable import IkariShellCore

@Suite struct SurroundingAvailabilityTests {
    @Test func emptyLeftWithZeroLengthIsNotAvailable() {
        #expect(!ClientAccess.isAvailable(left: "", documentLength: 0))
    }

    @Test func emptyLeftWithNonZeroLengthIsAvailable() {
        // 本当に文頭（右に文がある）
        #expect(ClientAccess.isAvailable(left: "", documentLength: 12))
    }

    @Test func nonEmptyLeftIsAvailable() {
        #expect(ClientAccess.isAvailable(left: "あ", documentLength: 0))
    }

    @Test func surroundingTextReportsUnavailableForAnAppThatReturnsNothing() {
        let client = FakeTextClient(document: "")
        client.selected = NSRange(location: 0, length: 0)
        #expect(ClientAccess(client: client).surroundingText() == .unavailable)
    }

    @Test func surroundingTextIsAvailableAtTheStartOfADocument() {
        let client = FakeTextClient(document: "あいう")
        client.selected = NSRange(location: 0, length: 0)
        let text = ClientAccess(client: client).surroundingText()
        #expect(text.available)
        #expect(text.left.isEmpty)
        #expect(text.right == "あいう")
    }

    @Test func surroundingTextReturnsTheLeftText() {
        let client = FakeTextClient(document: "あいう")
        client.selected = NSRange(location: 2, length: 0)
        let text = ClientAccess(client: client).surroundingText()
        #expect(text == SurroundingText(left: "あい", right: "う", available: true))
    }
}
