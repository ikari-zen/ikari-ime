// 候補ウィンドウ（ADR 0004 §6.4、ADR 0014、spec の「候補リストの表示」）。
// 枠のない、アクティブにならない NSPanel。縦一列（1 行に 1 候補）、番号ラベル、選択中は背景色。
// 注釈の行は、注釈のデータが入るまで出さない（ADR 0014 §9 Q7）。
// 候補の `label`（ショートコードの名前 `:eyes:`。ADR 0027 §3.4）は、行の右に薄く出す。
// アンカーの下に置き、入らないときだけ上に置く。上に置いたときは 1 位を下（アンカーに近い側）にする。
import AppKit
import IkariShellCore

@MainActor
final class CandidatePanel {
    private let panel: NSPanel
    private let view = CandidateView()
    private var anchor: CGRect = .zero
    private var visibleFrame: CGRect = .zero

    init() {
        panel = NSPanel(
            contentRect: NSRect(origin: .zero, size: CandidateView.minimumSize),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: true
        )
        panel.contentView = view
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.isFloatingPanel = true
        panel.hidesOnDeactivate = false
        panel.ignoresMouseEvents = true
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
    }

    var isVisible: Bool { panel.isVisible }

    /// 新しく出す。アンカーは変換文字列の先頭で、`hide()` までは動かさない
    @discardableResult
    func show(model: CandidatePanelModel, anchor: CGRect, visibleFrame: CGRect, level: NSWindow.Level) -> PanelSide {
        self.anchor = anchor
        self.visibleFrame = visibleFrame
        panel.level = level
        view.resetWidth()
        let side = apply(model)
        panel.orderFrontRegardless()
        return side
    }

    /// 選択やページが変わったとき。アンカーは同じ。上下は、広がったあとの高さで決めてあるので入れ替わらない
    @discardableResult
    func update(model: CandidatePanelModel) -> PanelSide? {
        guard panel.isVisible else { return nil }
        return apply(model)
    }

    func hide() {
        panel.orderOut(nil)
    }

    private func apply(_ model: CandidatePanelModel) -> PanelSide {
        view.model = model
        let size = view.preferredSize
        let placement = layoutPanel(
            anchor: anchor, panelSize: size, reservedHeight: CandidateView.height(rows: model.reservedRows),
            visibleFrame: visibleFrame)
        view.side = placement.side
        panel.setFrame(placement.frame, display: true)
        return placement.side
    }
}

/// 縦一列の候補（番号、候補）を描く。
@MainActor
final class CandidateView: NSView {
    static let rowHeight: CGFloat = 26
    static let cellPadding: CGFloat = 8
    static let labelGap: CGFloat = 4
    /// 候補と、右に出す名前（`label`）の間の最小の隙間
    static let trailingLabelGap: CGFloat = 16
    /// 選んでいない行の名前の濃さ（選んでいる行は反転色のまま、少し薄くする）
    static let trailingLabelAlpha: CGFloat = 0.6
    static let separatorInset: CGFloat = 8
    static let cornerRadius: CGFloat = 6
    static let selectionInset: CGFloat = 2
    static let minimumWidth: CGFloat = 160
    static let minimumSize = NSSize(width: minimumWidth, height: rowHeight)

    private let candidateFont = NSFont.systemFont(ofSize: 15)
    private let labelFont = NSFont.monospacedDigitSystemFont(ofSize: 11, weight: .regular)
    private let trailingFont = NSFont.systemFont(ofSize: 11)

    /// この変換でこれまでに見せた最大の幅。ページを送るたびに右端が揺れないよう、縮めない
    private var widestSoFar: CGFloat = 0

    var model = CandidatePanelModel(candidates: [], selected: 0, annotation: "") {
        didSet { needsDisplay = true }
    }

    /// パネルがアンカーのどちらにあるか。上のときは並びを上下逆にして、1 位を下にする
    var side: PanelSide = .below {
        didSet { needsDisplay = true }
    }

    override var isFlipped: Bool { true }

    static func height(rows: Int) -> CGFloat {
        CGFloat(max(rows, 1)) * rowHeight
    }

    /// 新しく出すとき（変換の始まりやフォーカスの移動）に幅の記憶を捨てる
    func resetWidth() {
        widestSoFar = 0
    }

    private func width(of text: String, font: NSFont) -> CGFloat {
        ceil((text as NSString).size(withAttributes: [.font: font]).width)
    }

    private func contentWidth() -> CGFloat {
        let widest = model.candidates.indices.map { index -> CGFloat in
            let label = width(of: String(index + 1), font: labelFont)
            let surface = width(of: model.candidates[index].surface, font: candidateFont)
            let trailing = model.candidates[index].label
            let trailingWidth = trailing.isEmpty ? 0 : Self.trailingLabelGap + width(of: trailing, font: trailingFont)
            return Self.cellPadding * 2 + label + Self.labelGap + surface + trailingWidth
        }.max() ?? 0
        return max(Self.minimumWidth, widest)
    }

    /// 幅は見せた候補のいちばん長いものに合わせ、この変換の間は縮めない。高さは見せている件数ぶん
    var preferredSize: NSSize {
        widestSoFar = max(widestSoFar, contentWidth())
        return NSSize(width: widestSoFar, height: Self.height(rows: model.candidates.count))
    }

    /// `index` 番目の候補の行の矩形。上に置いたときは、1 位が下になる
    private func rowRect(_ index: Int) -> NSRect {
        let slot = side == .above ? model.candidates.count - 1 - index : index
        return NSRect(x: 0, y: CGFloat(slot) * Self.rowHeight, width: bounds.width, height: Self.rowHeight)
    }

    override func draw(_ dirtyRect: NSRect) {
        let background = NSBezierPath(roundedRect: bounds, xRadius: Self.cornerRadius, yRadius: Self.cornerRadius)
        NSColor.windowBackgroundColor.setFill()
        background.fill()
        NSColor.separatorColor.setStroke()
        background.lineWidth = 0.5
        background.stroke()

        for (index, candidate) in model.candidates.enumerated() {
            let row = rowRect(index)
            let isSelected = index == model.selected
            if isSelected {
                let selection = row.insetBy(dx: Self.selectionInset, dy: Self.selectionInset)
                NSColor.controlAccentColor.setFill()
                NSBezierPath(roundedRect: selection, xRadius: Self.cornerRadius - 2, yRadius: Self.cornerRadius - 2).fill()
            }
            let primary = isSelected ? NSColor.alternateSelectedControlTextColor : NSColor.labelColor
            let secondary = isSelected ? NSColor.alternateSelectedControlTextColor : NSColor.secondaryLabelColor
            let label = String(index + 1)
            let labelWidth = width(of: label, font: labelFont)
            draw(label, font: labelFont, color: secondary, at: row.minX + Self.cellPadding, in: row)
            draw(
                candidate.surface, font: candidateFont, color: primary,
                at: row.minX + Self.cellPadding + labelWidth + Self.labelGap, in: row)
            if !candidate.label.isEmpty {
                let trailingColor = secondary.withAlphaComponent(Self.trailingLabelAlpha)
                let trailingWidth = width(of: candidate.label, font: trailingFont)
                draw(
                    candidate.label, font: trailingFont, color: trailingColor,
                    at: row.maxX - Self.cellPadding - trailingWidth, in: row)
            }

            // 候補同士は線で区切る（画面の上から数えて最後の行の下には引かない）
            if row.maxY < bounds.height - 0.5 {
                NSColor.separatorColor.setFill()
                NSRect(x: Self.separatorInset, y: row.maxY - 0.5, width: bounds.width - Self.separatorInset * 2, height: 0.5)
                    .fill()
            }
        }
    }

    private func draw(_ text: String, font: NSFont, color: NSColor, at x: CGFloat, in row: NSRect) {
        let attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color]
        let size = (text as NSString).size(withAttributes: attributes)
        (text as NSString).draw(at: NSPoint(x: x, y: row.minY + (row.height - size.height) / 2), withAttributes: attributes)
    }
}
