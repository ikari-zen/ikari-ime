//! 候補ウィンドウの配置（今の `PanelPlacement.swift`。ADR 0004 §6.4。レーン L-A1）。純関数。
//!
//! 座標は AppKit と同じ（左下が原点、y は上向き）。`CGRect` の代わりに `f64` の `Rect` を使う
//! （`Rect` は正規化しない。幅・高さは負にならない前提）。

/// 画面の座標として現実的な上限（`CGRect.infinite` のような巨大な矩形を弾く）。
const MAX_ANCHOR_COORDINATE: f64 = 1_000_000.0;
/// 水平方向の最後の拠り所を置く、表示領域の高さに対する割合（下から 1/4）。
const LAST_RESORT_HEIGHT_RATIO: f64 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            origin: Point { x, y },
            size: Size { width, height },
        }
    }

    pub fn min_x(&self) -> f64 {
        self.origin.x
    }
    pub fn max_x(&self) -> f64 {
        self.origin.x + self.size.width
    }
    pub fn mid_x(&self) -> f64 {
        self.origin.x + self.size.width / 2.0
    }
    pub fn min_y(&self) -> f64 {
        self.origin.y
    }
    pub fn max_y(&self) -> f64 {
        self.origin.y + self.size.height
    }

    /// `other` が全部この矩形に入っている。
    pub fn contains(&self, other: &Rect) -> bool {
        other.min_x() >= self.min_x()
            && other.max_x() <= self.max_x()
            && other.min_y() >= self.min_y()
            && other.max_y() <= self.max_y()
    }
}

/// アンカーの矩形がどこから取れたか。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorSource {
    /// `client.firstRect(forCharacterRange:)`
    Client,
    /// アクセシビリティ API（権限があるときだけ。自動で権限を求めない。NFR-20）
    Accessibility,
    /// 同じアプリで最後に取れた正常な値
    LastKnown,
    /// マウスのあるスクリーンの中央下
    MouseScreen,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelAnchor {
    pub rect: Rect,
    pub source: AnchorSource,
}

/// 矩形がゼロ、または NaN / 無限大なら取れていない扱いにする。
pub fn is_usable_anchor(rect: &Rect) -> bool {
    let values = [
        rect.origin.x,
        rect.origin.y,
        rect.size.width,
        rect.size.height,
    ];
    if !values
        .iter()
        .all(|v| v.is_finite() && v.abs() <= MAX_ANCHOR_COORDINATE)
    {
        return false;
    }
    // 幅だけがゼロの矩形（キャレット）は正常。原点がゼロのものは、大きさに関わらずクライアントが返す「取れなかった」の値
    // （行の高さだけを入れた (0, 0, 0, 17) を返すアプリがある。採ると主ディスプレイの左下にパネルが出る）
    if rect.origin.x == 0.0 && rect.origin.y == 0.0 {
        return false;
    }
    rect.size.height > 0.0 || rect.size.width > 0.0
}

/// パネルをアンカーのどちら側に置いたか。`Above` のときは、候補の並びを上下逆にして 1 位を下（アンカーに近い側）に描く
/// （ADR 0014 §2.2。user-feedback の決定: カーソルに近い側が 1 位）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelSide {
    Below,
    Above,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelPlacement {
    pub frame: Rect,
    pub side: PanelSide,
}

/// フォーカスしている文節の先頭（1 文節なら変換文字列の先頭）の矩形 `anchor` の下に `panel_size` のパネルを置く。
/// 下端に入らないときだけ上に置き、右端からはみ出すなら左にずらす。基準は `visible_frame`（Dock とメニューバーを除く）。
/// 水平はアンカーの左端に揃える（アンカーはフォーカスしている文節の先頭なので、打鍵やページ送りでは動かず、
/// フォーカスを移したときだけ動く）。上にも入らないときは、`visible_frame` の上端に合わせる（このとき `side` は `Below`）。
///
/// `reserved_height` は、上下どちらに置くかを決めるときに見込む高さ（`panel_size.height` より大きいとき）。
/// 候補が 3 件から 9 件に広がっても上下が入れ替わらないよう、広がったあとの高さで決める。
/// 置く位置は実際の高さで決める（下なら上端がアンカーの下、上なら下端がアンカーの上）。
pub fn layout_panel(
    anchor: &Rect,
    panel_size: Size,
    reserved_height: Option<f64>,
    visible_frame: &Rect,
) -> PanelPlacement {
    let deciding_height = panel_size.height.max(reserved_height.unwrap_or(0.0));
    let mut y = anchor.min_y() - panel_size.height;
    let mut side = PanelSide::Below;
    if anchor.min_y() - deciding_height < visible_frame.min_y() {
        let above = anchor.max_y();
        if above + deciding_height <= visible_frame.max_y() {
            y = above;
            side = PanelSide::Above;
        } else {
            y = visible_frame.max_y() - panel_size.height;
        }
    }
    y = y.max(visible_frame.min_y());

    let mut x = anchor.min_x();
    if x + panel_size.width > visible_frame.max_x() {
        x = visible_frame.max_x() - panel_size.width;
    }
    x = x.max(visible_frame.min_x());
    PanelPlacement {
        frame: Rect {
            origin: Point { x, y },
            size: panel_size,
        },
        side,
    }
}

/// `layout_panel` の y 軸が下向きの版（左上が原点。Windows の画面座標。ADR 0062 §3、§6.3）。
/// 引数と結果の矩形はすべて y 下向き。新しい規則は持たない: y を反転して `layout_panel` を呼び、結果を戻すだけ
/// （`PanelSide::Below` は「画面の下側（y が大きい側）に置いた」意味のまま変わらない）。
/// `anchor` は上辺の y が `origin.y`、`work_area` も同じ。結果の `frame.origin.y` はパネルの上辺。
pub fn layout_panel_y_down(
    anchor: &Rect,
    panel_size: Size,
    reserved_height: Option<f64>,
    work_area: &Rect,
) -> PanelPlacement {
    let placement = layout_panel(
        &flip_y(anchor),
        panel_size,
        reserved_height,
        &flip_y(work_area),
    );
    PanelPlacement {
        frame: flip_y(&placement.frame),
        side: placement.side,
    }
}

/// y の向きを反転する（上辺が `y` の矩形 ↔ 下辺が `-y` の矩形。自分自身の逆）。
fn flip_y(rect: &Rect) -> Rect {
    Rect::new(
        rect.origin.x,
        -(rect.origin.y + rect.size.height),
        rect.size.width,
        rect.size.height,
    )
}

/// `layout_panel` の枠だけ（予約の高さなし）。
pub fn place_panel(anchor: &Rect, panel_size: Size, visible_frame: &Rect) -> Rect {
    layout_panel(anchor, panel_size, None, visible_frame).frame
}

/// 使うアンカーを選ぶ。順は、クライアント、アクセシビリティ、同じアプリで最後に取れた正常値、マウスのあるスクリーンの中央下。
/// `mouse_screen_visible_frame` はマウスのあるスクリーンの `visibleFrame`。
pub fn resolve_anchor(
    client: Option<&Rect>,
    accessibility: Option<&Rect>,
    last_known: Option<&Rect>,
    mouse_screen_visible_frame: &Rect,
) -> PanelAnchor {
    let candidates = [
        (client, AnchorSource::Client),
        (accessibility, AnchorSource::Accessibility),
        (last_known, AnchorSource::LastKnown),
    ];
    for (rect, source) in candidates {
        if let Some(rect) = rect.filter(|r| is_usable_anchor(r)) {
            return PanelAnchor {
                rect: *rect,
                source,
            };
        }
    }
    // 幅のない矩形を、表示領域の水平方向の中央、下から 1/4 の高さに置く（そのすぐ下にパネルが出る）
    let frame = mouse_screen_visible_frame;
    let point = Point {
        x: frame.mid_x(),
        y: frame.min_y() + frame.size.height * LAST_RESORT_HEIGHT_RATIO,
    };
    PanelAnchor {
        rect: Rect {
            origin: point,
            size: Size::default(),
        },
        source: AnchorSource::MouseScreen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// メニューバー（上 25）と Dock（下 70）を除いた表示領域
    const VISIBLE: Rect = Rect::new(0.0, 70.0, 1440.0, 805.0);
    const PANEL: Size = Size {
        width: 400.0,
        height: 56.0,
    };

    fn size(width: f64, height: f64) -> Size {
        Size { width, height }
    }

    #[test]
    fn places_the_panel_directly_below_the_anchor_aligned_to_its_left_edge() {
        let anchor = Rect::new(300.0, 500.0, 0.0, 18.0);
        let frame = place_panel(&anchor, PANEL, &VISIBLE);
        assert_eq!(frame, Rect::new(300.0, 500.0 - 56.0, 400.0, 56.0));
    }

    #[test]
    fn flips_above_only_when_it_does_not_fit_below() {
        // 下に置くと y = 100 - 56 = 44 < 70（Dock の上端）
        let anchor = Rect::new(300.0, 100.0, 0.0, 18.0);
        let frame = place_panel(&anchor, PANEL, &VISIBLE);
        assert_eq!(frame.min_y(), anchor.max_y());
        assert_eq!(frame.min_x(), 300.0);

        // ちょうど入るときは下のまま
        let exact = Rect::new(300.0, 126.0, 0.0, 18.0);
        assert_eq!(place_panel(&exact, PANEL, &VISIBLE).max_y(), 126.0);
    }

    #[test]
    fn dock_is_excluded_from_the_bottom_limit() {
        // 画面（0 から）には入るが、Dock にかかる位置は上に出す
        let anchor = Rect::new(100.0, 90.0, 0.0, 18.0);
        let frame = place_panel(&anchor, PANEL, &VISIBLE);
        assert!(frame.min_y() >= VISIBLE.min_y());
        assert_eq!(frame.min_y(), anchor.max_y());
    }

    #[test]
    fn shifts_left_when_it_overflows_the_right_edge() {
        let anchor = Rect::new(1300.0, 500.0, 0.0, 18.0);
        let frame = place_panel(&anchor, PANEL, &VISIBLE);
        assert_eq!(frame.max_x(), VISIBLE.max_x());
        assert_eq!(frame.min_x(), 1040.0);
        assert_eq!(frame.min_y(), 444.0);
    }

    #[test]
    fn fits_exactly_at_the_right_edge_without_shifting() {
        let anchor = Rect::new(1040.0, 500.0, 0.0, 18.0);
        assert_eq!(place_panel(&anchor, PANEL, &VISIBLE).min_x(), 1040.0);
    }

    #[test]
    fn anchor_does_not_move_horizontally_when_the_panel_width_changes_on_paging() {
        let anchor = Rect::new(300.0, 500.0, 0.0, 18.0);
        let xs: Vec<f64> = [120.0, 400.0, 640.0, 900.0]
            .iter()
            .map(|w| place_panel(&anchor, size(*w, 56.0), &VISIBLE).min_x())
            .collect();
        assert_eq!(xs, [300.0, 300.0, 300.0, 300.0]);
    }

    #[test]
    fn anchor_is_the_start_of_the_marked_text_so_typing_does_not_move_the_panel() {
        let anchor = Rect::new(300.0, 500.0, 16.0, 18.0);
        let first = place_panel(&anchor, PANEL, &VISIBLE);
        let second = place_panel(&anchor, PANEL, &VISIBLE);
        assert_eq!(first, second);
        assert_eq!(first.min_x(), anchor.min_x());
    }

    #[test]
    fn secondary_display_with_negative_origin_is_handled_by_its_own_visible_frame() {
        let secondary = Rect::new(-1920.0, 100.0, 1920.0, 1000.0);
        let near_right = Rect::new(-80.0, 600.0, 0.0, 18.0);
        let frame = place_panel(&near_right, PANEL, &secondary);
        assert_eq!(frame.max_x(), secondary.max_x()); // 0
        assert_eq!(frame.min_x(), -400.0);

        let near_left = Rect::new(-1930.0, 600.0, 0.0, 18.0);
        assert_eq!(
            place_panel(&near_left, PANEL, &secondary).min_x(),
            secondary.min_x()
        );

        let near_bottom = Rect::new(-1000.0, 110.0, 0.0, 18.0);
        assert_eq!(place_panel(&near_bottom, PANEL, &secondary).min_y(), 128.0);
    }

    #[test]
    fn panel_taller_than_the_room_above_pins_to_the_top() {
        let tiny = Rect::new(0.0, 0.0, 800.0, 100.0);
        let anchor = Rect::new(10.0, 20.0, 0.0, 18.0);
        let frame = place_panel(&anchor, size(200.0, 90.0), &tiny);
        assert!(frame.min_y() >= tiny.min_y());
        assert!(frame.max_y() <= tiny.max_y());
    }

    #[test]
    fn panel_wider_than_the_screen_pins_to_the_left_edge() {
        let anchor = Rect::new(300.0, 500.0, 0.0, 18.0);
        let frame = place_panel(&anchor, size(2000.0, 56.0), &VISIBLE);
        assert_eq!(frame.min_x(), VISIBLE.min_x());
    }

    const SCREEN: Rect = Rect::new(0.0, 70.0, 1440.0, 800.0);
    const GOOD: Rect = Rect::new(300.0, 500.0, 0.0, 18.0);
    const OTHER: Rect = Rect::new(10.0, 20.0, 0.0, 18.0);
    const LAST: Rect = Rect::new(50.0, 60.0, 0.0, 18.0);

    #[test]
    fn usable_anchor_rejects_zero_nan_and_infinity() {
        assert!(is_usable_anchor(&GOOD));
        assert!(is_usable_anchor(&Rect::new(5.0, 5.0, 10.0, 0.0)));
        assert!(!is_usable_anchor(&Rect::ZERO));
        // 行の高さだけの「取れなかった」
        assert!(!is_usable_anchor(&Rect::new(0.0, 0.0, 0.0, 17.0)));
        assert!(!is_usable_anchor(&Rect::new(f64::NAN, 0.0, 0.0, 18.0)));
        assert!(!is_usable_anchor(&Rect::new(0.0, f64::INFINITY, 0.0, 18.0)));
        assert!(!is_usable_anchor(&Rect::new(0.0, 0.0, 0.0, f64::NAN)));
        assert!(!is_usable_anchor(&Rect::new(100.0, 100.0, 0.0, 0.0)));
        // CGRect.null（原点が無限大）と CGRect.infinite（巨大な矩形）
        assert!(!is_usable_anchor(&Rect::new(
            f64::INFINITY,
            f64::INFINITY,
            0.0,
            0.0
        )));
        assert!(!is_usable_anchor(&Rect::new(
            -8.98e307, -8.98e307, 1.79e308, 1.79e308
        )));
    }

    #[test]
    fn prefers_the_client_then_accessibility_then_last_known_then_the_mouse_screen() {
        let resolved = resolve_anchor(Some(&GOOD), Some(&OTHER), Some(&LAST), &SCREEN);
        assert_eq!(
            (resolved.rect, resolved.source),
            (GOOD, AnchorSource::Client)
        );
        let resolved = resolve_anchor(Some(&Rect::ZERO), Some(&OTHER), Some(&LAST), &SCREEN);
        assert_eq!(
            (resolved.rect, resolved.source),
            (OTHER, AnchorSource::Accessibility)
        );
        let resolved = resolve_anchor(None, None, Some(&LAST), &SCREEN);
        assert_eq!(
            (resolved.rect, resolved.source),
            (LAST, AnchorSource::LastKnown)
        );
        let nan_rect = Rect::new(f64::NAN, 0.0, 0.0, 18.0);
        let resolved = resolve_anchor(
            Some(&nan_rect),
            Some(&Rect::ZERO),
            Some(&Rect::ZERO),
            &SCREEN,
        );
        assert_eq!(resolved.source, AnchorSource::MouseScreen);
    }

    #[test]
    fn last_resort_is_the_lower_center_of_the_mouse_screen_and_the_result_stays_inside_it() {
        let anchor = resolve_anchor(None, None, None, &SCREEN);
        assert_eq!(anchor.source, AnchorSource::MouseScreen);
        assert_eq!(anchor.rect.mid_x(), SCREEN.mid_x());
        let frame = place_panel(&anchor.rect, size(400.0, 56.0), &SCREEN);
        assert!(SCREEN.contains(&frame));
    }

    // 候補を縦に積む配置（ADR 0014 §2.2）。上に出したときは 1 位を下にする
    const ROW_HEIGHT: f64 = 26.0;

    #[test]
    fn side_is_below_when_it_fits() {
        let anchor = Rect::new(300.0, 500.0, 0.0, 18.0);
        let placement = layout_panel(&anchor, size(200.0, 78.0), None, &VISIBLE);
        assert_eq!(placement.side, PanelSide::Below);
        assert_eq!(placement.frame.max_y(), anchor.min_y());
    }

    #[test]
    fn side_is_above_when_it_does_not_fit_below() {
        let anchor = Rect::new(300.0, 100.0, 0.0, 18.0);
        let placement = layout_panel(&anchor, size(200.0, 78.0), None, &VISIBLE);
        assert_eq!(placement.side, PanelSide::Above);
        assert_eq!(placement.frame.min_y(), anchor.max_y());
    }

    #[test]
    fn expanding_from_three_to_nine_does_not_swap_above_and_below() {
        // 3 行（78）なら下に入るが、9 行（234）は入らない位置。広がる見込みで決めるので、最初から上
        let anchor = Rect::new(300.0, 200.0, 0.0, 18.0);
        assert!(
            anchor.min_y() - 78.0 >= VISIBLE.min_y() && anchor.min_y() - 234.0 < VISIBLE.min_y()
        );
        let reserved = Some(9.0 * ROW_HEIGHT);
        let collapsed = layout_panel(&anchor, size(200.0, 3.0 * ROW_HEIGHT), reserved, &VISIBLE);
        let expanded = layout_panel(&anchor, size(200.0, 9.0 * ROW_HEIGHT), reserved, &VISIBLE);
        assert_eq!(collapsed.side, PanelSide::Above);
        assert_eq!(expanded.side, PanelSide::Above);
        // 上のとき、下端（1 位の側）はアンカーに付いたまま、上へ伸びる
        assert_eq!(collapsed.frame.min_y(), anchor.max_y());
        assert_eq!(expanded.frame.min_y(), anchor.max_y());
    }

    #[test]
    fn below_keeps_the_top_edge_when_expanding() {
        let anchor = Rect::new(300.0, 700.0, 0.0, 18.0);
        let reserved = Some(9.0 * ROW_HEIGHT);
        let collapsed = layout_panel(&anchor, size(200.0, 3.0 * ROW_HEIGHT), reserved, &VISIBLE);
        let expanded = layout_panel(&anchor, size(200.0, 9.0 * ROW_HEIGHT), reserved, &VISIBLE);
        assert_eq!(collapsed.side, PanelSide::Below);
        assert_eq!(expanded.side, PanelSide::Below);
        assert_eq!(collapsed.frame.max_y(), anchor.min_y());
        assert_eq!(expanded.frame.max_y(), anchor.min_y());
    }

    #[test]
    fn without_a_reserved_height_it_matches_place_panel() {
        let anchor = Rect::new(300.0, 100.0, 0.0, 18.0);
        let s = size(200.0, 78.0);
        assert_eq!(
            layout_panel(&anchor, s, None, &VISIBLE).frame,
            place_panel(&anchor, s, &VISIBLE)
        );
    }

    // y 下向き（Windows）。作業領域は 1920x1040（上 0、下はタスクバーを除く）
    const WORK_DOWN: Rect = Rect::new(0.0, 0.0, 1920.0, 1040.0);

    #[test]
    fn y_down_places_below_the_anchor_when_it_fits() {
        // アンカーの下辺は y=318。パネルの上辺はそこに揃う
        let anchor = Rect::new(300.0, 300.0, 0.0, 18.0);
        let placement = layout_panel_y_down(&anchor, PANEL, None, &WORK_DOWN);
        assert_eq!(placement.side, PanelSide::Below);
        assert_eq!(placement.frame, Rect::new(300.0, 318.0, 400.0, 56.0));
    }

    #[test]
    fn y_down_places_above_the_anchor_when_the_bottom_is_full() {
        let anchor = Rect::new(300.0, 1000.0, 0.0, 18.0);
        let placement = layout_panel_y_down(&anchor, PANEL, None, &WORK_DOWN);
        assert_eq!(placement.side, PanelSide::Above);
        // パネルの下辺がアンカーの上辺（y=1000）に揃う
        assert_eq!(
            placement.frame,
            Rect::new(300.0, 1000.0 - 56.0, 400.0, 56.0)
        );
    }

    #[test]
    fn y_down_decides_the_side_with_the_reserved_height() {
        // 実際の高さ 56 なら下に入るが、予約の高さ 300 では入らない
        let anchor = Rect::new(300.0, 900.0, 0.0, 18.0);
        let placement = layout_panel_y_down(&anchor, PANEL, Some(300.0), &WORK_DOWN);
        assert_eq!(placement.side, PanelSide::Above);
    }

    #[test]
    fn y_down_shifts_left_at_the_right_edge_and_respects_a_work_area_that_does_not_start_at_zero() {
        // 2 枚目の画面（x が 1920 から。y も 0 でない）
        let work = Rect::new(1920.0, 100.0, 1920.0, 1040.0);
        let anchor = Rect::new(3800.0, 200.0, 0.0, 18.0);
        let placement = layout_panel_y_down(&anchor, PANEL, None, &work);
        assert_eq!(
            placement.frame,
            Rect::new(3840.0 - 400.0, 218.0, 400.0, 56.0)
        );
    }

    #[test]
    fn y_down_agrees_with_the_y_up_layout_on_the_side() {
        // 同じ配置を y 上向きで書いて、side が一致する（画面の高さ 1040 で上下を反転）
        for anchor_top in [0.0, 300.0, 700.0, 980.0, 1030.0] {
            let down_anchor = Rect::new(500.0, anchor_top, 0.0, 18.0);
            let up_anchor = Rect::new(500.0, 1040.0 - anchor_top - 18.0, 0.0, 18.0);
            let up_work = Rect::new(0.0, 0.0, 1920.0, 1040.0);
            let down = layout_panel_y_down(&down_anchor, PANEL, Some(200.0), &WORK_DOWN);
            let up = layout_panel(&up_anchor, PANEL, Some(200.0), &up_work);
            assert_eq!(down.side, up.side, "anchor_top={anchor_top}");
            assert_eq!(down.frame.origin.x, up.frame.origin.x);
            assert_eq!(
                down.frame.origin.y,
                1040.0 - up.frame.origin.y - PANEL.height,
                "anchor_top={anchor_top}"
            );
        }
    }
}
