// 候補ウィンドウの配置の規則（spec の「候補ウィンドウの配置」、ADR 0004 §6.4）。座標は AppKit（左下が原点）。
import CoreGraphics
import Testing

@testable import IkariShellCore

@Suite struct PanelPlacementTests {
    /// メニューバー（上 25）と Dock（下 70）を除いた表示領域
    let visible = CGRect(x: 0, y: 70, width: 1440, height: 805)
    let panel = CGSize(width: 400, height: 56)

    @Test func placesThePanelDirectlyBelowTheAnchorAlignedToItsLeftEdge() {
        let anchor = CGRect(x: 300, y: 500, width: 0, height: 18)
        let frame = placePanel(anchor: anchor, panelSize: panel, visibleFrame: visible)
        #expect(frame == CGRect(x: 300, y: 500 - 56, width: 400, height: 56))
    }

    @Test func flipsAboveOnlyWhenItDoesNotFitBelow() {
        // 下に置くと y = 100 - 56 = 44 < 70（Dock の上端）
        let anchor = CGRect(x: 300, y: 100, width: 0, height: 18)
        let frame = placePanel(anchor: anchor, panelSize: panel, visibleFrame: visible)
        #expect(frame.minY == anchor.maxY)
        #expect(frame.minX == 300)

        // ちょうど入るときは下のまま
        let exact = CGRect(x: 300, y: 126, width: 0, height: 18)
        #expect(placePanel(anchor: exact, panelSize: panel, visibleFrame: visible).maxY == 126)
    }

    @Test func dockIsExcludedFromTheBottomLimit() {
        // 画面（0 から）には入るが、Dock にかかる位置は上に出す
        let anchor = CGRect(x: 100, y: 90, width: 0, height: 18)
        let frame = placePanel(anchor: anchor, panelSize: panel, visibleFrame: visible)
        #expect(frame.minY >= visible.minY)
        #expect(frame.minY == anchor.maxY)
    }

    @Test func shiftsLeftWhenItOverflowsTheRightEdge() {
        let anchor = CGRect(x: 1300, y: 500, width: 0, height: 18)
        let frame = placePanel(anchor: anchor, panelSize: panel, visibleFrame: visible)
        #expect(frame.maxX == visible.maxX)
        #expect(frame.minX == 1040)
        #expect(frame.minY == 444)
    }

    @Test func fitsExactlyAtTheRightEdgeWithoutShifting() {
        let anchor = CGRect(x: 1040, y: 500, width: 0, height: 18)
        #expect(placePanel(anchor: anchor, panelSize: panel, visibleFrame: visible).minX == 1040)
    }

    @Test func anchorDoesNotMoveHorizontallyWhenThePanelWidthChangesOnPaging() {
        let anchor = CGRect(x: 300, y: 500, width: 0, height: 18)
        let widths: [CGFloat] = [120, 400, 640, 900]
        let xs = widths.map { placePanel(anchor: anchor, panelSize: CGSize(width: $0, height: 56), visibleFrame: visible).minX }
        #expect(xs == [300, 300, 300, 300])
    }

    @Test func anchorIsTheStartOfTheMarkedTextSoTypingDoesNotMoveThePanel() {
        // 変換文字列の先頭の矩形は、文字を足しても同じ。同じ入力なら同じ位置になる（純関数）
        let anchor = CGRect(x: 300, y: 500, width: 16, height: 18)
        let first = placePanel(anchor: anchor, panelSize: panel, visibleFrame: visible)
        let second = placePanel(anchor: anchor, panelSize: panel, visibleFrame: visible)
        #expect(first == second)
        #expect(first.minX == anchor.minX)
    }

    @Test func secondaryDisplayWithNegativeOriginIsHandledByItsOwnVisibleFrame() {
        let secondary = CGRect(x: -1920, y: 100, width: 1920, height: 1000)
        let nearRight = CGRect(x: -80, y: 600, width: 0, height: 18)
        let frame = placePanel(anchor: nearRight, panelSize: panel, visibleFrame: secondary)
        #expect(frame.maxX == secondary.maxX)  // 0
        #expect(frame.minX == -400)

        let nearLeft = CGRect(x: -1930, y: 600, width: 0, height: 18)
        #expect(placePanel(anchor: nearLeft, panelSize: panel, visibleFrame: secondary).minX == secondary.minX)

        let nearBottom = CGRect(x: -1000, y: 110, width: 0, height: 18)
        #expect(placePanel(anchor: nearBottom, panelSize: panel, visibleFrame: secondary).minY == 128)
    }

    @Test func panelTallerThanTheRoomAbovePinsToTheTop() {
        let tiny = CGRect(x: 0, y: 0, width: 800, height: 100)
        let anchor = CGRect(x: 10, y: 20, width: 0, height: 18)
        let frame = placePanel(anchor: anchor, panelSize: CGSize(width: 200, height: 90), visibleFrame: tiny)
        #expect(frame.minY >= tiny.minY)
        #expect(frame.maxY <= tiny.maxY)
    }

    @Test func panelWiderThanTheScreenPinsToTheLeftEdge() {
        let anchor = CGRect(x: 300, y: 500, width: 0, height: 18)
        let frame = placePanel(anchor: anchor, panelSize: CGSize(width: 2000, height: 56), visibleFrame: visible)
        #expect(frame.minX == visible.minX)
    }
}

@Suite struct AnchorResolutionTests {
    let screen = CGRect(x: 0, y: 70, width: 1440, height: 800)
    let good = CGRect(x: 300, y: 500, width: 0, height: 18)
    let other = CGRect(x: 10, y: 20, width: 0, height: 18)
    let last = CGRect(x: 50, y: 60, width: 0, height: 18)

    @Test func usableAnchorRejectsZeroNaNAndInfinity() {
        #expect(isUsableAnchor(good))
        #expect(isUsableAnchor(CGRect(x: 5, y: 5, width: 10, height: 0)))
        #expect(!isUsableAnchor(.zero))
        #expect(!isUsableAnchor(CGRect(x: 0, y: 0, width: 0, height: 17)))  // 行の高さだけの「取れなかった」
        #expect(!isUsableAnchor(CGRect(x: CGFloat.nan, y: 0, width: 0, height: 18)))
        #expect(!isUsableAnchor(CGRect(x: 0, y: CGFloat.infinity, width: 0, height: 18)))
        #expect(!isUsableAnchor(CGRect(x: 0, y: 0, width: 0, height: CGFloat.nan)))
        #expect(!isUsableAnchor(CGRect(x: 100, y: 100, width: 0, height: 0)))
        #expect(!isUsableAnchor(.null))
        #expect(!isUsableAnchor(.infinite))
    }

    @Test func prefersTheClientThenAccessibilityThenLastKnownThenTheMouseScreen() {
        #expect(resolveAnchor(client: good, accessibility: other, lastKnown: last, mouseScreenVisibleFrame: screen)
            == PanelAnchor(rect: good, source: .client))
        #expect(resolveAnchor(client: .zero, accessibility: other, lastKnown: last, mouseScreenVisibleFrame: screen)
            == PanelAnchor(rect: other, source: .accessibility))
        #expect(resolveAnchor(client: nil, accessibility: nil, lastKnown: last, mouseScreenVisibleFrame: screen)
            == PanelAnchor(rect: last, source: .lastKnown))
        let nanRect = CGRect(x: CGFloat.nan, y: 0, width: 0, height: 18)
        #expect(resolveAnchor(client: nanRect, accessibility: .zero, lastKnown: .zero, mouseScreenVisibleFrame: screen)
            .source == .mouseScreen)
    }

    @Test func lastResortIsTheLowerCenterOfTheMouseScreenAndTheResultStaysInsideIt() {
        let anchor = resolveAnchor(client: nil, accessibility: nil, lastKnown: nil, mouseScreenVisibleFrame: screen)
        #expect(anchor.source == .mouseScreen)
        #expect(anchor.rect.midX == screen.midX)
        let frame = placePanel(anchor: anchor.rect, panelSize: CGSize(width: 400, height: 56), visibleFrame: screen)
        #expect(screen.contains(frame))
    }
}

/// 候補を縦に積む配置（ADR 0014 §2.2）。上に出したときは 1 位を下にする
@Suite struct VerticalPanelPlacementTests {
    let visible = CGRect(x: 0, y: 70, width: 1440, height: 805)
    let rowHeight: CGFloat = 26

    @Test func sideIsBelowWhenItFits() {
        let anchor = CGRect(x: 300, y: 500, width: 0, height: 18)
        let placement = layoutPanel(anchor: anchor, panelSize: CGSize(width: 200, height: 78), visibleFrame: visible)
        #expect(placement.side == .below)
        #expect(placement.frame.maxY == anchor.minY)
    }

    @Test func sideIsAboveWhenItDoesNotFitBelow() {
        let anchor = CGRect(x: 300, y: 100, width: 0, height: 18)
        let placement = layoutPanel(anchor: anchor, panelSize: CGSize(width: 200, height: 78), visibleFrame: visible)
        #expect(placement.side == .above)
        #expect(placement.frame.minY == anchor.maxY)
    }

    @Test func expandingFromThreeToNineDoesNotSwapAboveAndBelow() {
        // 3 行（78）なら下に入るが、9 行（234）は入らない位置。広がる見込みで決めるので、最初から上
        let anchor = CGRect(x: 300, y: 200, width: 0, height: 18)
        #expect(anchor.minY - 78 >= visible.minY && anchor.minY - 234 < visible.minY)
        let reserved = 9 * rowHeight
        let collapsed = layoutPanel(
            anchor: anchor, panelSize: CGSize(width: 200, height: 3 * rowHeight), reservedHeight: reserved, visibleFrame: visible)
        let expanded = layoutPanel(
            anchor: anchor, panelSize: CGSize(width: 200, height: 9 * rowHeight), reservedHeight: reserved, visibleFrame: visible)
        #expect(collapsed.side == .above && expanded.side == .above)
        // 上のとき、下端（1 位の側）はアンカーに付いたまま、上へ伸びる
        #expect(collapsed.frame.minY == anchor.maxY && expanded.frame.minY == anchor.maxY)
    }

    @Test func belowKeepsTheTopEdgeWhenExpanding() {
        let anchor = CGRect(x: 300, y: 700, width: 0, height: 18)
        let reserved = 9 * rowHeight
        let collapsed = layoutPanel(
            anchor: anchor, panelSize: CGSize(width: 200, height: 3 * rowHeight), reservedHeight: reserved, visibleFrame: visible)
        let expanded = layoutPanel(
            anchor: anchor, panelSize: CGSize(width: 200, height: 9 * rowHeight), reservedHeight: reserved, visibleFrame: visible)
        #expect(collapsed.side == .below && expanded.side == .below)
        #expect(collapsed.frame.maxY == anchor.minY && expanded.frame.maxY == anchor.minY)
    }

    @Test func withoutAReservedHeightItMatchesPlacePanel() {
        let anchor = CGRect(x: 300, y: 100, width: 0, height: 18)
        let size = CGSize(width: 200, height: 78)
        #expect(layoutPanel(anchor: anchor, panelSize: size, visibleFrame: visible).frame
            == placePanel(anchor: anchor, panelSize: size, visibleFrame: visible))
    }
}
