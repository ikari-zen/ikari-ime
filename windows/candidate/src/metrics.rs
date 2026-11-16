//! 行の高さ（DIP。96 dpi を 1 とした単位）。
//!
//! 行の高さは、候補の行数、注釈の行の有無、開発者の行の有無だけで決まる純粋な関数にする。
//! 文字を測らない（ADR 0062 §6.2）。TIP がこの高さ × 倍率で `panel::layout_panel_y_down` を呼んで上下を決め、
//! 候補窓（XAML）も同じ高さを行の高さとして使う。macOS の `CandidateView.height(rows:)` と同じ考え方。
//!
//! **値は仮置き**（計測なし。見た目の値なので、決めるときはユーザーに見せる）。今は macOS の値
//! （`CandidatePanel.swift` の `rowHeight` と `noteHeight`）を写した。注釈の行は macOS に固定の高さがないので、開発者の行と同じにした。

/// 候補 1 行の高さ（DIP。仮置き。macOS の `CandidateView.rowHeight`）
pub const CANDIDATE_ROW_DIP: f64 = 26.0;
/// 注釈の行の高さ（DIP。仮置き）
pub const ANNOTATION_ROW_DIP: f64 = 20.0;
/// 開発者の行の高さ（DIP。仮置き。macOS の `CandidateView.noteHeight`）
pub const DEVELOPER_ROW_DIP: f64 = 20.0;
/// 候補が 0 件でも、候補の行はこの数だけ取る（macOS の `max(rows, 1)` と同じ）
const MIN_CANDIDATE_ROWS: u32 = 1;

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

impl PanelRows {
    /// モデルから行の種類を読む。`candidate_rows` は実際の候補の数。
    pub fn of(model: &ikari_shell_core::CandidatePanelModel) -> Self {
        Self {
            candidate_rows: u32::try_from(model.candidates.len()).unwrap_or(u32::MAX),
            has_annotation: !model.annotation.is_empty(),
            has_developer_note: !model.developer_note.is_empty(),
        }
    }

    /// 候補の行の数だけを `rows` にした写し（予約の行で高さを見込むとき）。
    pub fn with_candidate_rows(self, rows: u32) -> Self {
        Self {
            candidate_rows: rows,
            ..self
        }
    }
}

/// パネル全体の高さ（DIP）。
pub fn panel_height_dip(rows: PanelRows) -> f64 {
    let candidates = f64::from(rows.candidate_rows.max(MIN_CANDIDATE_ROWS)) * CANDIDATE_ROW_DIP;
    let annotation = if rows.has_annotation {
        ANNOTATION_ROW_DIP
    } else {
        0.0
    };
    let developer = if rows.has_developer_note {
        DEVELOPER_ROW_DIP
    } else {
        0.0
    };
    candidates + annotation + developer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(candidate_rows: u32, has_annotation: bool, has_developer_note: bool) -> PanelRows {
        PanelRows {
            candidate_rows,
            has_annotation,
            has_developer_note,
        }
    }

    #[test]
    fn height_grows_with_rows_and_extra_lines() {
        assert_eq!(panel_height_dip(rows(1, false, false)), CANDIDATE_ROW_DIP);
        assert_eq!(
            panel_height_dip(rows(9, false, false)),
            9.0 * CANDIDATE_ROW_DIP
        );
        assert_eq!(
            panel_height_dip(rows(3, true, true)),
            3.0 * CANDIDATE_ROW_DIP + ANNOTATION_ROW_DIP + DEVELOPER_ROW_DIP
        );
    }

    #[test]
    fn empty_list_still_takes_one_row() {
        assert_eq!(panel_height_dip(rows(0, false, false)), CANDIDATE_ROW_DIP);
    }

    #[test]
    fn huge_row_counts_do_not_overflow() {
        assert!(panel_height_dip(rows(u32::MAX, true, true)).is_finite());
    }
}
