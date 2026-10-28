// 候補ウィンドウの配置（ADR 0004 §6.4）。純関数にしてテストする。
import CoreGraphics
import Foundation

/// アンカーの矩形がどこから取れたか。
public enum AnchorSource: Sendable, Equatable {
    /// `client.firstRect(forCharacterRange:)`
    case client
    /// アクセシビリティ API（権限があるときだけ。自動で権限を求めない。NFR-20）
    case accessibility
    /// 同じアプリで最後に取れた正常な値
    case lastKnown
    /// マウスのあるスクリーンの中央下
    case mouseScreen
}

public struct PanelAnchor: Sendable, Equatable {
    public var rect: CGRect
    public var source: AnchorSource

    public init(rect: CGRect, source: AnchorSource) {
        self.rect = rect
        self.source = source
    }
}

/// 画面の座標として現実的な上限（`CGRect.infinite` のような巨大な矩形を弾く）
private let maxAnchorCoordinate: CGFloat = 1_000_000

/// 矩形がゼロ、または NaN / 無限大なら取れていない扱いにする。
public func isUsableAnchor(_ rect: CGRect) -> Bool {
    let values = [rect.origin.x, rect.origin.y, rect.size.width, rect.size.height]
    guard values.allSatisfy({ $0.isFinite && abs($0) <= maxAnchorCoordinate }) else { return false }
    // 幅だけがゼロの矩形（キャレット）は正常。原点がゼロのものは、大きさに関わらずクライアントが返す「取れなかった」の値
    // （行の高さだけを入れた (0, 0, 0, 17) を返すアプリがある。採ると主ディスプレイの左下にパネルが出る）
    if rect.origin == .zero { return false }
    return rect.size.height > 0 || rect.size.width > 0
}

/// パネルをアンカーのどちら側に置いたか。`above` のときは、候補の並びを上下逆にして 1 位を下（アンカーに近い側）に描く
/// （ADR 0014 §2.2。user-feedback の決定: カーソルに近い側が 1 位）
public enum PanelSide: Sendable, Equatable {
    case below
    case above
}

public struct PanelPlacement: Sendable, Equatable {
    public var frame: CGRect
    public var side: PanelSide

    public init(frame: CGRect, side: PanelSide) {
        self.frame = frame
        self.side = side
    }
}

/// フォーカスしている文節の先頭（1 文節なら変換文字列の先頭）の矩形 `anchor` の下に `panelSize` のパネルを置く。
/// 下端に入らないときだけ上に置き、右端からはみ出すなら左にずらす。基準は `visibleFrame`（Dock とメニューバーを除く）。
/// 座標は AppKit（左下が原点）。
/// 水平はアンカーの左端に揃える（アンカーはフォーカスしている文節の先頭なので、打鍵やページ送りでは動かず、フォーカスを移したときだけ動く）。
/// 上にも入らないときは、`visibleFrame` の上端に合わせる（このとき `side` は `below`）。
///
/// `reservedHeight` は、上下どちらに置くかを決めるときに見込む高さ（`panelSize.height` より大きいとき）。
/// 候補が 3 件から 9 件に広がっても上下が入れ替わらないよう、広がったあとの高さで決める。
/// 置く位置は実際の高さで決める（下なら上端がアンカーの下、上なら下端がアンカーの上）。
public func layoutPanel(
    anchor: CGRect, panelSize: CGSize, reservedHeight: CGFloat? = nil, visibleFrame: CGRect
) -> PanelPlacement {
    let decidingHeight = max(panelSize.height, reservedHeight ?? 0)
    var y = anchor.minY - panelSize.height
    var side = PanelSide.below
    if anchor.minY - decidingHeight < visibleFrame.minY {
        let above = anchor.maxY
        if above + decidingHeight <= visibleFrame.maxY {
            y = above
            side = .above
        } else {
            y = visibleFrame.maxY - panelSize.height
        }
    }
    y = max(y, visibleFrame.minY)

    var x = anchor.minX
    if x + panelSize.width > visibleFrame.maxX {
        x = visibleFrame.maxX - panelSize.width
    }
    x = max(x, visibleFrame.minX)
    return PanelPlacement(frame: CGRect(origin: CGPoint(x: x, y: y), size: panelSize), side: side)
}

public func placePanel(anchor: CGRect, panelSize: CGSize, visibleFrame: CGRect) -> CGRect {
    layoutPanel(anchor: anchor, panelSize: panelSize, visibleFrame: visibleFrame).frame
}

/// 使うアンカーを選ぶ。順は、クライアント、アクセシビリティ、同じアプリで最後に取れた正常値、マウスのあるスクリーンの中央下。
/// `mouseScreenVisibleFrame` はマウスのあるスクリーンの `visibleFrame`。
public func resolveAnchor(
    client: CGRect?,
    accessibility: CGRect?,
    lastKnown: CGRect?,
    mouseScreenVisibleFrame: CGRect
) -> PanelAnchor {
    if let client, isUsableAnchor(client) { return PanelAnchor(rect: client, source: .client) }
    if let accessibility, isUsableAnchor(accessibility) {
        return PanelAnchor(rect: accessibility, source: .accessibility)
    }
    if let lastKnown, isUsableAnchor(lastKnown) { return PanelAnchor(rect: lastKnown, source: .lastKnown) }
    // 幅のない矩形を、表示領域の水平方向の中央、下から 1/4 の高さに置く（そのすぐ下にパネルが出る）
    let point = CGPoint(
        x: mouseScreenVisibleFrame.midX,
        y: mouseScreenVisibleFrame.minY + mouseScreenVisibleFrame.height / 4
    )
    return PanelAnchor(rect: CGRect(origin: point, size: .zero), source: .mouseScreen)
}
