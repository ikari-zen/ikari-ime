//! 候補窓に渡す内容の上限（#473）。macOS の `CandidatePanel.swift` は、文字列の幅を測って窓の大きさを決めるので、
//! 極端に長い文字列や多い件数が来ると計算が重くなり、窓が不自然な大きさになる。判断は Rust に置き、Swift は結果を使うだけにする。
//!
//! 値は Windows の候補窓（`windows/candidate/src/limits.rs`）の `MAX_CANDIDATE_ROWS`、`MAX_TEXT_CHARS` と同じ。
//! **仮置き**（計測なし）。ふつうの候補（1 ページ最大 9 件、短い語）は引っかからない。
//! ここは切り詰めだけを行う（macOS の内容は同じプロセスの Rust が作る。Windows のような送り手の検証は要らない）。

use crate::CandidatePanelModel;

/// 候補の行の上限（仮置き。ふつうは 1 ページ最大 9 件）
pub const MAX_CANDIDATE_ROWS: usize = 16;
/// 1 つの文字列の長さの上限（Unicode のスカラー値の数。仮置き）
pub const MAX_TEXT_CHARS: usize = 256;

fn clip(text: &mut String) {
    if let Some((end, _)) = text.char_indices().nth(MAX_TEXT_CHARS) {
        text.truncate(end);
    }
}

/// 行の数と文字列の長さを上限に収める。`selected` が残った行の外になるときは、最後の行にする。
pub fn limit_for_display(mut model: CandidatePanelModel) -> CandidatePanelModel {
    model.candidates.truncate(MAX_CANDIDATE_ROWS);
    for item in &mut model.candidates {
        clip(&mut item.surface);
        clip(&mut item.annotation);
        clip(&mut item.label);
    }
    clip(&mut model.annotation);
    clip(&mut model.developer_note);
    let rows = u32::try_from(model.candidates.len()).unwrap_or(u32::MAX);
    if rows > 0 && model.selected >= rows {
        model.selected = rows - 1;
    }
    let max_rows = u32::try_from(MAX_CANDIDATE_ROWS).unwrap_or(u32::MAX);
    model.reserved_rows = model.reserved_rows.min(max_rows);
    model
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CandidateItem, CandidateKind};

    fn item(surface: &str) -> CandidateItem {
        CandidateItem {
            surface: surface.to_string(),
            annotation: String::new(),
            kind: CandidateKind::Dictionary,
            label: String::new(),
        }
    }

    fn model(candidates: Vec<CandidateItem>, selected: u32) -> CandidatePanelModel {
        CandidatePanelModel {
            reserved_rows: u32::try_from(candidates.len()).unwrap(),
            candidates,
            selected,
            annotation: String::new(),
            developer_note: String::new(),
        }
    }

    #[test]
    fn normal_model_is_unchanged() {
        let m = model(vec![item("漢字"), item("感じ")], 1);
        assert_eq!(limit_for_display(m.clone()), m);
    }

    #[test]
    fn long_text_is_clipped_by_chars_not_bytes() {
        let long = "あ".repeat(MAX_TEXT_CHARS * 4);
        let mut m = model(vec![item(&long)], 0);
        m.candidates[0].annotation = long.clone();
        m.candidates[0].label = long.clone();
        m.annotation = long.clone();
        m.developer_note = long;
        let out = limit_for_display(m);
        assert_eq!(out.candidates[0].surface.chars().count(), MAX_TEXT_CHARS);
        assert_eq!(out.candidates[0].annotation.chars().count(), MAX_TEXT_CHARS);
        assert_eq!(out.candidates[0].label.chars().count(), MAX_TEXT_CHARS);
        assert_eq!(out.annotation.chars().count(), MAX_TEXT_CHARS);
        assert_eq!(out.developer_note.chars().count(), MAX_TEXT_CHARS);
    }

    #[test]
    fn exactly_max_chars_is_kept() {
        let s = "い".repeat(MAX_TEXT_CHARS);
        let out = limit_for_display(model(vec![item(&s)], 0));
        assert_eq!(out.candidates[0].surface, s);
    }

    #[test]
    fn many_rows_are_truncated_and_selection_stays_inside() {
        let rows: Vec<_> = (0..MAX_CANDIDATE_ROWS * 3).map(|_| item("a")).collect();
        let out = limit_for_display(model(rows, 40));
        assert_eq!(out.candidates.len(), MAX_CANDIDATE_ROWS);
        assert_eq!(out.selected as usize, MAX_CANDIDATE_ROWS - 1);
        assert_eq!(out.reserved_rows as usize, MAX_CANDIDATE_ROWS);
    }

    #[test]
    fn empty_model_keeps_selection() {
        let out = limit_for_display(model(vec![], 0));
        assert!(out.candidates.is_empty());
        assert_eq!(out.selected, 0);
    }
}
