// IMK のクライアント（入力欄を持つアプリ）への問い合わせ。周辺テキストとキャレットの矩形（ADR 0004 §6.4、§6.5）。
// 値を取って shell-core の形にするだけ。「取れた」と言ってよいか（左が空で文書の長さも 0）と切り詰めは shell-core が決める。
import AppKit
import ApplicationServices
import IkariProto
import InputMethodKit

/// 主スレッドでだけ使う（IMK のクライアントは主スレッドから呼ぶ）
struct ClientAccess: @unchecked Sendable {
    /// 左の文脈を読む上限（エンジンに渡す上限と同じ。OS から読み過ぎないため）
    static let leftContextLimit = 2000
    /// 変換文字列のあとの文脈として取る長さ。左を主、右は補助（spec の「文脈の取得元」）
    static let rightContextLimit = 200

    let client: any IMKTextInput

    private static let notFound = NSRange(location: NSNotFound, length: NSNotFound)

    // MARK: 周辺テキスト

    /// 選択範囲（変換文字列があればその）の手前と後ろと、文書の長さ。取れなければ nil（ターミナル、一部の Electron）
    func surroundingText() -> Ikari_Shell_V1_SurroundingText? {
        let selected = client.selectedRange()
        guard selected.location != NSNotFound else { return nil }

        // 変換中の下線つきの文字列は文脈に含めない
        let marked = client.markedRange()
        let hasMarked = marked.location != NSNotFound && marked.length > 0
        let leftEnd = hasMarked ? marked.location : selected.location
        let rightStart = hasMarked ? marked.location + marked.length : selected.location + selected.length

        var text = Ikari_Shell_V1_SurroundingText()
        if leftEnd > 0 {
            let start = max(0, leftEnd - Self.leftContextLimit)
            guard let left = string(NSRange(location: start, length: leftEnd - start)) else { return nil }
            text.left = left
        }
        let total = client.length()
        if total > rightStart {
            let length = min(Self.rightContextLimit, total - rightStart)
            text.right = string(NSRange(location: rightStart, length: length)) ?? ""
        }
        text.documentLength = UInt32(clamping: max(0, total))
        return text
    }

    private func string(_ range: NSRange) -> String? {
        var actual = NSRange(location: 0, length: 0)
        return client.string(from: range, actualRange: &actual)
    }

    // MARK: キャレットの矩形

    /// アンカーにする矩形（スクリーン座標）。変換文字列の先頭の文字の矩形で、文節の移動では動かさない。
    /// IMK の標準の `attributes(forCharacterIndex:lineHeightRectangle:)` に相対位置 0 を渡す
    /// （S0-3: VS Code では絶対位置ではなく相対位置で、その文字の矩形が取れる。`firstRect` は使えない）。
    /// 取れなければ `firstRect` を試す（長さ 0 の範囲に答えないクライアントがあるので、1 文字ぶんでも試す）。
    /// 使えるか（ゼロ、NaN など）は shell-core が決めるので、取れた値を順に返す
    func caretRects() -> [CGRect] {
        var rects: [CGRect] = []
        var lineRect = NSRect.zero
        _ = client.attributes(forCharacterIndex: 0, lineHeightRectangle: &lineRect)
        rects.append(lineRect)

        let marked = client.markedRange()
        guard marked.location != NSNotFound else { return rects }
        for length in [0, 1] {
            var actual = NSRange(location: 0, length: 0)
            rects.append(
                client.firstRect(forCharacterRange: NSRange(location: marked.location, length: length), actualRange: &actual))
        }
        return rects
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

    /// 変換中の文字列を置く（見た目は `MarkedTextAttributes`）。`replacementRange` があるとき（再変換の始まり）だけ、
    /// 選択していた範囲を置き換える
    func setMarkedText(_ marked: Ikari_Shell_V1_MarkedText) {
        let attributed = MarkedTextAttributes.attributedString(for: marked)
        let replacement =
            marked.hasReplacementRange
            ? NSRange(location: Int(marked.replacementRange.location), length: Int(marked.replacementRange.length))
            : Self.notFound
        client.setMarkedText(
            attributed, selectionRange: NSRange(location: attributed.length, length: 0), replacementRange: replacement)
    }

    /// 元の文字列を、変換中の範囲を置き換える 1 回の `insertText` で書き戻す。前後に `setMarkedText("")` も `unmarkText` も呼ばない
    /// （Electron の「元に戻す」の履歴に変換中の状態を残しにくくする。ADR 0006 §16.3）。
    /// 範囲は shell-core が渡した `replacing` を使う。`client.markedRange()` は、アプリによって相対位置や空を返すことがある。
    /// 食い違うときは、長さだけを記録する（文字は残さない）
    func restore(_ text: String, replacing: Ikari_Shell_V1_Utf16Range) {
        let range = NSRange(location: Int(replacing.location), length: Int(replacing.length))
        let marked = client.markedRange()
        if marked != range {
            AppContext.log.notice(
                "restore range differs: marked=\(marked.location),\(marked.length) expected=\(range.location),\(range.length)")
        }
        client.insertText(text, replacementRange: range)
    }

    // MARK: 選択範囲と取り残された変換中の文字列

    /// 再変換の対象。shell-core が要るときだけ呼ぶ。選択がない、取れない、アプリが別の範囲を返した（`actualRange` が違う）ときは nil
    /// （長さなどの断りは shell-core が判断する。ADR 0006 §9.2）
    func selectedText() -> Ikari_Shell_V1_SelectedText? {
        let selected = client.selectedRange()
        guard Self.isValid(selected) else { return nil }
        var actual = NSRange(location: 0, length: 0)
        guard let text = client.string(from: selected, actualRange: &actual), !text.isEmpty, actual == selected else {
            return nil
        }
        var result = Ikari_Shell_V1_SelectedText()
        result.range.location = UInt32(clamping: selected.location)
        result.range.length = UInt32(clamping: selected.length)
        result.text = text
        return result
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
}

/// `MarkedText` から下線つきの属性文字列を作る（AppKit だけで決まるのでテストできる）
enum MarkedTextAttributes {
    static func attributedString(for marked: Ikari_Shell_V1_MarkedText) -> NSAttributedString {
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
            let highlight = NSRange(location: 0, length: min(Int(marked.highlightedLength), full.length))
            if highlight.length > 0 {
                attributed.addAttribute(.underlineStyle, value: NSUnderlineStyle.thick.rawValue, range: highlight)
            }
            return attributed
        }

        let focused: Int? = marked.hasFocused ? Int(marked.focused) : nil
        var location = 0
        for (index, length) in marked.segments.enumerated() {
            let range = NSRange(location: location, length: min(Int(length), full.length - location))
            guard range.length > 0 else { break }
            let style: NSUnderlineStyle = index == focused ? .thick : .single
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
