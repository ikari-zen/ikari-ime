// 再変換の Esc の書き戻し（ADR 0006 §9.5、§16.3）。クライアントの markedRange が当てにならなくても、元の文字列を正しい範囲に書く。
import AppKit
import InputMethodKit
import Testing

@testable import IkariIME
import IkariProto

/// 文書と変換中の範囲を持つ替え玉のクライアント。`reportsRelativeMarkedRange` が真なら、
/// markedRange を文書の絶対位置ではなく変換中の文字列の中の相対位置（先頭は 0）で答える（実機のアプリにある振る舞いの想定）
final class FakeTextClient: NSObject, IMKTextInput {
    var document: String
    var marked: NSRange = NSRange(location: NSNotFound, length: 0)
    var reportsRelativeMarkedRange = false
    /// 真なら、変換中の文字列がないときの `insertText` の `replacementRange` を無視する（TextEdit の振る舞い）
    var ignoresInsertReplacementRange = false
    private(set) var insertCalls: [(text: String, range: NSRange)] = []

    init(document: String) { self.document = document }

    private func utf16Replace(_ range: NSRange, with text: String) {
        document = (document as NSString).replacingCharacters(in: range, with: text)
    }

    /// 置き換える範囲。指定がなければ変換中の範囲、それもなければ文書の末尾（キャレット）
    private func target(_ replacementRange: NSRange) -> NSRange {
        if replacementRange.location != NSNotFound { return replacementRange }
        if marked.location != NSNotFound { return marked }
        return NSRange(location: (document as NSString).length, length: 0)
    }

    func insertText(_ string: Any!, replacementRange: NSRange) {
        let text = (string as? String) ?? (string as? NSAttributedString)?.string ?? ""
        insertCalls.append((text, replacementRange))
        if ignoresInsertReplacementRange, marked.location == NSNotFound, replacementRange.location != NSNotFound { return }
        utf16Replace(target(replacementRange), with: text)
        marked = NSRange(location: NSNotFound, length: 0)
    }

    func setMarkedText(_ string: Any!, selectionRange: NSRange, replacementRange: NSRange) {
        let text = (string as? String) ?? (string as? NSAttributedString)?.string ?? ""
        let target = target(replacementRange)
        utf16Replace(target, with: text)
        marked = NSRange(location: target.location, length: (text as NSString).length)
    }

    func markedRange() -> NSRange {
        guard marked.location != NSNotFound, reportsRelativeMarkedRange else { return marked }
        return NSRange(location: 0, length: marked.length)
    }

    /// 選択範囲（キャレット）。既定は「取れない」
    var selected = NSRange(location: NSNotFound, length: 0)
    func selectedRange() -> NSRange { selected }
    func length() -> Int { (document as NSString).length }
    func string(from range: NSRange, actualRange: NSRangePointer!) -> String! {
        actualRange?.pointee = range
        return (document as NSString).substring(with: range)
    }
    func bundleIdentifier() -> String! { "test.fake" }
    func windowLevel() -> CGWindowLevel { 0 }
    func validAttributesForMarkedText() -> [Any]! { [] }
    func overrideKeyboard(withKeyboardNamed keyboardUniqueName: String!) {}
    func selectMode(_ modeIdentifier: String!) {}
    func supportsUnicode() -> Bool { true }
    func uniqueClientIdentifierString() -> String! { "fake" }
    func attributedSubstring(from range: NSRange) -> NSAttributedString! { NSAttributedString(string: "") }
    func attributes(forCharacterIndex index: Int, lineHeightRectangle lineRect: UnsafeMutablePointer<NSRect>!) -> [AnyHashable: Any]! { [:] }
    func characterIndex(for point: NSPoint, tracking mappingMode: IMKLocationToOffsetMappingMode, inMarkedRange: UnsafeMutablePointer<ObjCBool>!) -> Int { 0 }
    func firstRect(forCharacterRange aRange: NSRange, actualRange: NSRangePointer!) -> NSRect { .zero }
    func supportsProperty(_ property: TSMDocumentPropertyTag) -> Bool { false }
}

private func range(_ location: UInt32, _ length: UInt32) -> Ikari_Shell_V1_Utf16Range {
    var range = Ikari_Shell_V1_Utf16Range()
    range.location = location
    range.length = length
    return range
}

@Suite struct ClientAccessRestoreTests {
    private func reconvertedClient(relative: Bool) -> FakeTextClient {
        // 「あ品詞い」の「品詞」(位置 1、長さ 2) を再変換し、別の候補「瀕死」を表示中
        let client = FakeTextClient(document: "あ品詞い")
        client.reportsRelativeMarkedRange = relative
        client.setMarkedText("品詞", selectionRange: NSRange(location: 2, length: 0), replacementRange: NSRange(location: 1, length: 2))
        client.setMarkedText("瀕死", selectionRange: NSRange(location: 2, length: 0), replacementRange: NSRange(location: NSNotFound, length: NSNotFound))
        return client
    }

    @Test func restoreWritesTheOriginalOverTheMarkedRange() {
        let client = reconvertedClient(relative: false)
        ClientAccess(client: client).restore("品詞", replacing: range(1, 2))
        #expect(client.document == "あ品詞い")
        #expect(client.insertCalls.count == 1)
    }

    @Test func restoreDoesNotTrustAMarkedRangeThatIsRelative() {
        let client = reconvertedClient(relative: true)
        ClientAccess(client: client).restore("品詞", replacing: range(1, 2))
        #expect(client.document == "あ品詞い")
        #expect(client.insertCalls.map(\.range) == [NSRange(location: 1, length: 2)])
    }
}
