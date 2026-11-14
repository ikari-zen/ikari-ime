//! 行の高さ（DIP。96 dpi を 1 とした単位）。**スタブ**（数値は L2 が入れる。見た目の値なので、決めるときはユーザーに見せる）。
//!
//! 行の高さは、候補の行数（`reserved_rows` と実際の数）、注釈の行の有無、開発者の行の有無だけで決まる純粋な関数にする。
//! 文字を測らない（ADR 0062 §6.2）。TIP がこの高さ × 倍率で `panel::layout_panel_y_down` を呼んで上下を決め、
//! 候補窓（XAML）も同じ高さを行の高さとして使う。macOS の `CandidateView.height(rows:)` と同じ考え方。

/// パネルの高さを決める入力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelRows {
    /// 候補の行の数（実際に出す数。予約の行は別に `reserved_rows` で渡す）
    pub candidate_rows: u32,
    /// 注釈の行を出すか（`CandidatePanelModel.annotation` が空でない）
    pub has_annotation: bool,
    /// 開発者の行を出すか（`CandidatePanelModel.developer_note` が空でない。ADR 0061）
    pub has_developer_note: bool,
}

/// パネル全体の高さ（DIP）。スタブ（0.0 を返す。L2 が実装する）。
pub fn panel_height_dip(_rows: PanelRows) -> f64 {
    0.0
}
