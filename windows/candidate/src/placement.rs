//! 候補窓の横の位置。
//!
//! 規則は `shell-core::panel::layout_panel` の横と同じ: アンカーの左端に揃え、作業領域の右端からはみ出すなら左へ、
//! 作業領域の左端より左へは出さない。上下は TIP が決めた `side` に従うので、ここでは扱わない（ADR 0062 §6.2）。
//! 新しい規則を作らないため、`layout_panel_y_down` を呼んで x だけを使う。
//!
//! 縦の位置（`vertical_origin`）は、TIP が決めた `side` に従って置くだけ（下ならアンカーの下端、上ならアンカーの上端に
//! 窓の下端を合わせる）。そのうえで、窓を作業領域の外に出さない（ADR 0068 §5）。上下を決め直すことはしない。

use ikari_shell_core::panel::{PanelSide, Rect, Size, layout_panel_y_down};

/// 窓の左端の x（物理のピクセル）。値が有限でなければ、アンカーの左端を返す（呼ぶ側は `limits` で確かめた値を渡す）。
pub fn horizontal_origin(anchor: &Rect, panel_width: f64, work_area: &Rect) -> f64 {
    if !(panel_width.is_finite() && anchor.min_x().is_finite() && work_area.max_x().is_finite()) {
        return anchor.min_x();
    }
    // 高さは横の位置に効かない。0 で呼ぶ
    let size = Size {
        width: panel_width.max(0.0),
        height: 0.0,
    };
    layout_panel_y_down(anchor, size, None, work_area)
        .frame
        .min_x()
}

/// 窓の上端の y（物理のピクセル。y は下向き）。`side` は TIP が決めたもの。値が有限でなければアンカーの下端を返す。
pub fn vertical_origin(anchor: &Rect, panel_height: f64, side: PanelSide, work_area: &Rect) -> f64 {
    if !(panel_height.is_finite() && anchor.max_y().is_finite() && work_area.max_y().is_finite()) {
        return anchor.max_y();
    }
    let height = panel_height.max(0.0);
    let y = match side {
        PanelSide::Below => anchor.max_y(),
        PanelSide::Above => anchor.min_y() - height,
    };
    // 下へはみ出さない。そのうえで、上へもはみ出さない（窓が作業領域より高いときは上端に揃える）
    y.min(work_area.max_y() - height).max(work_area.min_y())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = Rect::new(0.0, 0.0, 1920.0, 1040.0);

    #[test]
    fn aligns_to_the_anchor_when_it_fits() {
        let anchor = Rect::new(300.0, 500.0, 10.0, 20.0);
        assert_eq!(horizontal_origin(&anchor, 200.0, &WORK), 300.0);
    }

    #[test]
    fn shifts_left_at_the_right_edge() {
        let anchor = Rect::new(1850.0, 500.0, 10.0, 20.0);
        assert_eq!(horizontal_origin(&anchor, 200.0, &WORK), 1720.0);
    }

    #[test]
    fn never_goes_left_of_the_work_area() {
        // 2 枚目の画面（左端が 1920）で、窓が作業領域より広い
        let work = Rect::new(1920.0, 0.0, 300.0, 1040.0);
        let anchor = Rect::new(2000.0, 500.0, 10.0, 20.0);
        assert_eq!(horizontal_origin(&anchor, 500.0, &work), 1920.0);
    }

    #[test]
    fn negative_work_area_origin_is_respected() {
        // 主画面の左にある画面は x が負
        let work = Rect::new(-1280.0, 0.0, 1280.0, 1000.0);
        let anchor = Rect::new(-100.0, 500.0, 10.0, 20.0);
        assert_eq!(horizontal_origin(&anchor, 200.0, &work), -200.0);
    }

    #[test]
    fn non_finite_width_falls_back_to_the_anchor() {
        let anchor = Rect::new(300.0, 500.0, 10.0, 20.0);
        assert_eq!(horizontal_origin(&anchor, f64::NAN, &WORK), 300.0);
    }

    #[test]
    fn vertical_follows_the_side() {
        let anchor = Rect::new(300.0, 500.0, 10.0, 20.0);
        assert_eq!(
            vertical_origin(&anchor, 100.0, PanelSide::Below, &WORK),
            520.0
        );
        assert_eq!(
            vertical_origin(&anchor, 100.0, PanelSide::Above, &WORK),
            400.0
        );
    }

    #[test]
    fn vertical_stays_inside_the_work_area() {
        // 下に置けと言われたが、下端からはみ出す
        let low = Rect::new(300.0, 1000.0, 10.0, 20.0);
        assert_eq!(vertical_origin(&low, 100.0, PanelSide::Below, &WORK), 940.0);
        // 上に置けと言われたが、上端からはみ出す
        let high = Rect::new(300.0, 30.0, 10.0, 20.0);
        assert_eq!(vertical_origin(&high, 100.0, PanelSide::Above, &WORK), 0.0);
        // 作業領域より高い窓は上端に揃える
        assert_eq!(vertical_origin(&high, 5000.0, PanelSide::Below, &WORK), 0.0);
    }

    #[test]
    fn vertical_agrees_with_the_shared_layout_when_it_fits() {
        for y in [100.0, 500.0, 980.0] {
            let anchor = Rect::new(300.0, y, 10.0, 20.0);
            let size = Size {
                width: 200.0,
                height: 120.0,
            };
            let shared = layout_panel_y_down(&anchor, size, None, &WORK);
            assert_eq!(
                vertical_origin(&anchor, size.height, shared.side, &WORK),
                shared.frame.min_y()
            );
        }
    }
}
