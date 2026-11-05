// 周辺テキストの読み取り（ADR 0015 D2）。「取れた」と言ってよいか（左が空で文書の長さも 0 なら取れていない）は shell-core が
// 決める（`machine::is_available` とそのテスト）。ここは、クライアントから読んだ生の値をそのまま渡すことを確かめる。
import Foundation
import IkariProto
import Testing

@testable import IkariIME

@Suite struct SurroundingAvailabilityTests {
    @Test func noCaretMeansTheHostCannotReadTheText() {
        let client = FakeTextClient(document: "あいう")
        #expect(ClientAccess(client: client).surroundingText() == nil)
    }

    @Test func anAppThatReturnsNothingPassesEmptyLeftAndZeroLength() {
        let client = FakeTextClient(document: "")
        client.selected = NSRange(location: 0, length: 0)
        let text = ClientAccess(client: client).surroundingText()
        #expect(text?.left == "")
        #expect(text?.documentLength == 0)
    }

    @Test func theStartOfADocumentHasTheRightTextAndTheLength() {
        let client = FakeTextClient(document: "あいう")
        client.selected = NSRange(location: 0, length: 0)
        let text = ClientAccess(client: client).surroundingText()
        #expect(text?.left == "")
        #expect(text?.right == "あいう")
        #expect(text?.documentLength == 3)
    }

    @Test func caretIsTheSelectedRange() {
        let client = FakeTextClient(document: "あいう")
        client.selected = NSRange(location: 2, length: 1)
        let text = ClientAccess(client: client).surroundingText()
        #expect(text?.caret.location == 2)
        #expect(text?.caret.length == 1)
    }

    @Test func deleteTextReplacesTheRangeWithNothingInOneCall() {
        let client = FakeTextClient(document: "今日は晴れ")
        var range = Ikari_Shell_V1_Utf16Range()
        range.location = 3
        range.length = 2
        ClientAccess(client: client).deleteText(range)
        #expect(client.document == "今日は")
        #expect(client.insertCalls.count == 1)
        #expect(client.insertCalls.first?.text == "")
        #expect(client.insertCalls.first?.range == NSRange(location: 3, length: 2))
    }

    @Test func deleteTextFallsBackWhenTheAppIgnoresTheReplacementRange() {
        // 範囲を無視するアプリ（TextEdit）。文書の長さが減らないので、空の変換中の文字列で置き換えて確定する
        let client = FakeTextClient(document: "今日は晴れ")
        client.ignoresInsertReplacementRange = true
        var range = Ikari_Shell_V1_Utf16Range()
        range.location = 3
        range.length = 2
        ClientAccess(client: client).deleteText(range)
        #expect(client.document == "今日は")
    }

    @Test func surroundingTextReturnsTheLeftText() {
        let client = FakeTextClient(document: "あいう")
        client.selected = NSRange(location: 2, length: 0)
        let text = ClientAccess(client: client).surroundingText()
        #expect(text?.left == "あい")
        #expect(text?.right == "う")
    }
}
