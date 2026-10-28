// IMK のクライアント（入力欄を持つアプリ）への問い合わせ。周辺テキストとキャレットの矩形（ADR 0004 §6.4、§6.5）。
import AppKit
import ApplicationServices
import IkariShellCore
import InputMethodKit

struct ClientAccess {
    /// 変換文字列のあとの文脈として取る長さ。左を主、右は補助（spec の「文脈の取得元」）
    static let rightContextLimit = 200

    let client: any IMKTextInput

    private static let notFound = NSRange(location: NSNotFound, length: NSNotFound)

    // MARK: 周辺テキスト

    /// 選択範囲（変換文字列があればその）の手前を最大2000文字。取れなければ `unavailable`（ターミナル、一部の Electron）
    func surroundingText() -> SurroundingText {
        let selected = client.selectedRange()
        guard selected.location != NSNotFound else { return .unavailable }

        // 変換中の下線つきの文字列は文脈に含めない
        let marked = client.markedRange()
        let hasMarked = marked.location != NSNotFound && marked.length > 0
        let leftEnd = hasMarked ? marked.location : selected.location
        let rightStart = hasMarked ? marked.location + marked.length : selected.location + selected.length

        var left = ""
        if leftEnd > 0 {
            let start = max(0, leftEnd - SurroundingText.maxCharacters)
            guard let text = string(NSRange(location: start, length: leftEnd - start)) else { return .unavailable }
            left = text
        }
        let total = client.length()
        var right = ""
        if total > rightStart {
            let length = min(Self.rightContextLimit, total - rightStart)
            right = string(NSRange(location: rightStart, length: length)) ?? ""
        }
        // 左が空で、文書の長さも 0 のアプリは、文脈を返せていない（長さを返さないアプリ）。
        // 本当に空の文書とは区別できないが、「取れた」と言うより「取れていない」と言うほうが安全（ADR 0015 D2）
        guard Self.isAvailable(left: left, documentLength: total) else { return .unavailable }
        return SurroundingText(left: left, right: right, available: true)
    }

    /// 周辺テキストを「取れた」と言ってよいか。左が空で `length()` も 0 なら取れていない
    static func isAvailable(left: String, documentLength: Int) -> Bool {
        !(left.isEmpty && documentLength <= 0)
    }

    private func string(_ range: NSRange) -> String? {
        var actual = NSRange(location: 0, length: 0)
        return client.string(from: range, actualRange: &actual)
    }

    // MARK: キャレットの矩形

    /// アンカーにする矩形（スクリーン座標）。変換文字列の先頭の文字の矩形で、文節の移動では動かさない。
    /// IMK の標準の `attributes(forCharacterIndex:lineHeightRectangle:)` に相対位置 0 を渡す
    /// （S0-3: VS Code では絶対位置ではなく相対位置で、その文字の矩形が取れる。`firstRect` は使えない）。
    /// 取れなければ `firstRect` を試す。長さ0の範囲に答えないクライアントがあるので、1文字ぶんでも試す
    func caretRect() -> CGRect? {
        var lineRect = NSRect.zero
        _ = client.attributes(forCharacterIndex: 0, lineHeightRectangle: &lineRect)
        if isUsableAnchor(lineRect) { return lineRect }

        let marked = client.markedRange()
        guard marked.location != NSNotFound else { return nil }
        for length in [0, 1] {
            var actual = NSRange(location: 0, length: 0)
            let rect = client.firstRect(
                forCharacterRange: NSRange(location: marked.location, length: length), actualRange: &actual)
            if isUsableAnchor(rect) { return rect }
        }
        return nil
    }

    var windowLevel: NSWindow.Level {
        NSWindow.Level(rawValue: Int(client.windowLevel()) + 1)
    }

    var bundleIdentifier: String {
        client.bundleIdentifier() ?? ""
    }

    func insert(_ text: String) {
        client.insertText(text, replacementRange: Self.notFound)
    }

    /// 変換中の文字列を置く。文節に分かれているとき（`marked.segments`）は、文節ごとに `markedClauseSegment` を付け、
    /// フォーカスしている文節を太い下線、ほかを細い下線にする（ADR 0006 §8.3）。そうでなければ今までの見た目
    /// （全体を細い下線、`highlightedLength` の先頭を太い下線）。`replacementRange` があるとき（再変換の始まり）だけ、
    /// 選択していた範囲を置き換える
    func setMarkedText(_ marked: MarkedText) {
        let attributed = MarkedTextAttributes.attributedString(for: marked)
        let replacement = marked.replacementRange.map { NSRange(location: $0.location, length: $0.length) } ?? Self.notFound
        client.setMarkedText(
            attributed, selectionRange: NSRange(location: attributed.length, length: 0), replacementRange: replacement)
    }

    /// 元の文字列を、変換中の範囲を置き換える 1 回の `insertText` で書き戻す。前後に `setMarkedText("")` も `unmarkText` も呼ばない
    /// （Electron の「元に戻す」の履歴に変換中の状態を残しにくくする。ADR 0006 §16.3）。
    /// 範囲は状態機械が渡した `replacing`（再変換を始めた位置と表示中の長さ）を使う。`client.markedRange()` は、
    /// アプリによって相対位置や空を返すことがあり、そのまま使うと別の場所を書き換えるか、変換中の文字列が残る。
    /// 食い違うときは、長さだけを記録する（文字は残さない）
    func restore(_ text: String, replacing: IkariShellCore.TextRange) {
        let range = NSRange(location: replacing.location, length: replacing.length)
        let marked = client.markedRange()
        if marked != range {
            AppContext.log.notice(
                "restore range differs: marked=\(marked.location),\(marked.length) expected=\(range.location),\(range.length)")
        }
        client.insertText(text, replacementRange: range)
    }

    // MARK: 選択範囲と取り残された変換中の文字列

    /// 再変換の対象。`reconvert` のキーのときだけ呼ぶ。選択がない、取れない、アプリが別の範囲を返した（`actualRange` が違う）、
    /// 再変換を始めないアプリのときは nil（長さなどの断りは状態機械が判断する。ADR 0006 §9.2）
    func selectedText() -> SelectedText? {
        guard !Self.reconversionDeniedBundleIDs.contains(bundleIdentifier) else { return nil }
        let selected = client.selectedRange()
        guard Self.isValid(selected) else { return nil }
        var actual = NSRange(location: 0, length: 0)
        guard let text = client.string(from: selected, actualRange: &actual), !text.isEmpty, actual == selected else {
            return nil
        }
        return SelectedText(range: TextRange(location: selected.location, length: selected.length), text: text)
    }

    /// 何も入力していないのにクライアントに残っている変換中の文字列（ADR 0006 §16.3）。なければ nil
    func strayMarkedText() -> String? {
        let marked = client.markedRange()
        guard Self.isValid(marked) else { return nil }
        guard let text = string(marked), !text.isEmpty else { return nil }
        return text
    }

    /// 取り残された変換中の文字列を、その範囲を置き換えて確定する
    func commitStray(_ text: String) {
        let marked = client.markedRange()
        client.insertText(text, replacementRange: Self.isValid(marked) ? marked : Self.notFound)
    }

    private static func isValid(_ range: NSRange) -> Bool {
        range.location != NSNotFound && range.length != NSNotFound && range.length > 0
    }

    /// `setMarkedText(replacementRange:)` で選択を置き換えられないことが分かったアプリ（Bundle ID）。再変換を始めない
    /// （ADR 0006 §9.5、S0-5）。S0 では TextEdit、VS Code、Chrome で効くことを確かめた。見つかったら足す
    static let reconversionDeniedBundleIDs: Set<String> = []
}

/// `MarkedText` から下線つきの属性文字列を作る（AppKit だけで決まるのでテストできる）
enum MarkedTextAttributes {
    static func attributedString(for marked: MarkedText) -> NSAttributedString {
        let attributed = NSMutableAttributedString(string: marked.text)
        let full = NSRange(location: 0, length: attributed.length)
        guard full.length > 0 else { return attributed }

        // 下線なし（英語のモードのショートコードを打っている途中）。下線の属性は付けず、文節の印だけ付ける
        guard marked.underlined else {
            attributed.addAttribute(.markedClauseSegment, value: 0, range: full)
            return attributed
        }

        if marked.segments.isEmpty {
            attributed.addAttribute(.underlineStyle, value: NSUnderlineStyle.single.rawValue, range: full)
            let highlight = NSRange(location: 0, length: min(max(0, marked.highlightedLength), full.length))
            if highlight.length > 0 {
                attributed.addAttribute(.underlineStyle, value: NSUnderlineStyle.thick.rawValue, range: highlight)
            }
            return attributed
        }

        var location = 0
        for (index, length) in marked.segments.enumerated() {
            let range = NSRange(location: location, length: min(max(0, length), full.length - location))
            guard range.length > 0 else { break }
            let style: NSUnderlineStyle = index == marked.focused ? .thick : .single
            attributed.addAttribute(.underlineStyle, value: style.rawValue, range: range)
            attributed.addAttribute(.markedClauseSegment, value: index, range: range)
            location += range.length
        }
        // 長さが足りない壊れた入力でも、下線のない文字を残さない
        if location < full.length {
            let rest = NSRange(location: location, length: full.length - location)
            attributed.addAttribute(.underlineStyle, value: NSUnderlineStyle.single.rawValue, range: rest)
        }
        return attributed
    }
}

/// アクセシビリティ API によるフォールバック（spec の配置の節 ①）。
/// 権限があるときだけ使う。こちらから権限を求めない（NFR-20。`AXIsProcessTrusted` はダイアログを出さない）。
enum AccessibilityAnchor {
    /// フォーカスのある入力欄の左下（AppKit の座標。高さは1の矩形にして、パネルがすぐ下に出るようにする）
    static func focusedElementBottomLeft() -> CGRect? {
        guard AXIsProcessTrusted() else { return nil }
        let system = AXUIElementCreateSystemWide()
        var focused: CFTypeRef?
        guard AXUIElementCopyAttributeValue(system, kAXFocusedUIElementAttribute as CFString, &focused) == .success,
            let focused, CFGetTypeID(focused) == AXUIElementGetTypeID()
        else { return nil }
        let element = focused as! AXUIElement  // 型は上で確かめた

        guard let position = point(of: element, attribute: kAXPositionAttribute),
            let size = size(of: element, attribute: kAXSizeAttribute),
            let primaryHeight = NSScreen.screens.first?.frame.height
        else { return nil }
        // AX は左上原点、AppKit は左下原点（主ディスプレイの高さで反転する）
        return CGRect(x: position.x, y: primaryHeight - (position.y + size.height), width: 1, height: 1)
    }

    private static func value(of element: AXUIElement, attribute: String) -> AXValue? {
        var raw: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute as CFString, &raw) == .success,
            let raw, CFGetTypeID(raw) == AXValueGetTypeID()
        else { return nil }
        return (raw as! AXValue)
    }

    private static func point(of element: AXUIElement, attribute: String) -> CGPoint? {
        guard let value = value(of: element, attribute: attribute) else { return nil }
        var point = CGPoint.zero
        return AXValueGetValue(value, .cgPoint, &point) ? point : nil
    }

    private static func size(of element: AXUIElement, attribute: String) -> CGSize? {
        guard let value = value(of: element, attribute: attribute) else { return nil }
        var size = CGSize.zero
        return AXValueGetValue(value, .cgSize, &size) ? size : nil
    }
}
