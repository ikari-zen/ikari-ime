//! 候補窓の横の位置。**スタブ**（L2 が実装する）。
//!
//! 規則は `shell-core::panel::layout_panel` の横と同じ: アンカーの左端に揃え、作業領域の右端からはみ出すなら左へ、
//! 作業領域の左端より左へは出さない。上下は TIP が決めた `side` に従うので、ここでは扱わない（ADR 0062 §6.2）。

use ikari_shell_core::panel::Rect;

/// 窓の左端の x（物理のピクセル）。スタブ（アンカーの左端をそのまま返す。L2 が実装する）。
pub fn horizontal_origin(anchor: &Rect, _panel_width: f64, _work_area: &Rect) -> f64 {
    anchor.min_x()
}
