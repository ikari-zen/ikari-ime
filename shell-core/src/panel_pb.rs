//! 候補窓の型（`CandidatePanelModel`、`PanelSide`、`Rect`）と proto（`ikari.shell.v1`）の写し。
//!
//! macOS の `shell-ffi` と、Windows の TIP・候補窓（プロセス間の契約。ADR 0062 §7.2）が同じものを使う。
//! 写しをもう 1 つ作らない（#394）。proto → 型 は、型で表せない値（列挙が範囲外・UNSPECIFIED）を
//! `PanelConvertError` にする（黙って既定値に寄せない）。ユーザーの文字列はエラーに含めない。

use ikari_proto::shell::v1 as pb;

use crate::panel::{PanelSide, Rect};
use crate::{CandidateItem, CandidateKind, CandidatePanelModel};

/// proto を型にできなかった理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelConvertError {
    /// 列挙が型の範囲の外（UNSPECIFIED を含む）。欄の名前
    OutOfRange(&'static str),
}

impl std::fmt::Display for PanelConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfRange(field) => write!(f, "value out of range: {field}"),
        }
    }
}

impl std::error::Error for PanelConvertError {}

type Result<T> = std::result::Result<T, PanelConvertError>;

fn enum_value<T: TryFrom<i32>>(value: i32, field: &'static str) -> Result<T> {
    T::try_from(value).map_err(|_| PanelConvertError::OutOfRange(field))
}

fn candidate_kind_from_pb(kind: i32) -> Result<CandidateKind> {
    Ok(
        match enum_value::<pb::CandidateKind>(kind, "CandidateItem.kind")? {
            pb::CandidateKind::Unspecified => CandidateKind::Unspecified,
            pb::CandidateKind::Dictionary => CandidateKind::Dictionary,
            pb::CandidateKind::Hiragana => CandidateKind::Hiragana,
            pb::CandidateKind::Katakana => CandidateKind::Katakana,
            pb::CandidateKind::Rewriter => CandidateKind::Rewriter,
            pb::CandidateKind::Literal => CandidateKind::Literal,
        },
    )
}

fn candidate_kind_to_pb(kind: CandidateKind) -> pb::CandidateKind {
    match kind {
        CandidateKind::Unspecified => pb::CandidateKind::Unspecified,
        CandidateKind::Dictionary => pb::CandidateKind::Dictionary,
        CandidateKind::Hiragana => pb::CandidateKind::Hiragana,
        CandidateKind::Katakana => pb::CandidateKind::Katakana,
        CandidateKind::Rewriter => pb::CandidateKind::Rewriter,
        CandidateKind::Literal => pb::CandidateKind::Literal,
    }
}

pub fn panel_from_pb(panel: pb::CandidatePanelModel) -> Result<CandidatePanelModel> {
    let candidates = panel
        .candidates
        .into_iter()
        .map(|item| {
            Ok(CandidateItem {
                surface: item.surface,
                annotation: item.annotation,
                kind: candidate_kind_from_pb(item.kind)?,
                label: item.label,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(CandidatePanelModel {
        developer_note: panel.developer_note,
        candidates,
        selected: panel.selected,
        annotation: panel.annotation,
        reserved_rows: panel.reserved_rows,
    })
}

pub fn panel_to_pb(panel: &CandidatePanelModel) -> pb::CandidatePanelModel {
    pb::CandidatePanelModel {
        developer_note: panel.developer_note.clone(),
        candidates: panel
            .candidates
            .iter()
            .map(|item| pb::CandidateItem {
                surface: item.surface.clone(),
                annotation: item.annotation.clone(),
                kind: candidate_kind_to_pb(item.kind).into(),
                label: item.label.clone(),
            })
            .collect(),
        selected: panel.selected,
        annotation: panel.annotation.clone(),
        reserved_rows: panel.reserved_rows,
    }
}

pub fn side_from_pb(value: i32) -> Result<PanelSide> {
    match enum_value::<pb::PanelSide>(value, "PanelSide")? {
        pb::PanelSide::Unspecified => Err(PanelConvertError::OutOfRange("PanelSide")),
        pb::PanelSide::Below => Ok(PanelSide::Below),
        pb::PanelSide::Above => Ok(PanelSide::Above),
    }
}

pub fn side_to_pb(side: PanelSide) -> pb::PanelSide {
    match side {
        PanelSide::Below => pb::PanelSide::Below,
        PanelSide::Above => pb::PanelSide::Above,
    }
}

pub fn rect_from_pb(rect: Option<pb::Rect>) -> Option<Rect> {
    rect.map(|r| Rect::new(r.x, r.y, r.width, r.height))
}

pub fn rect_to_pb(rect: &Rect) -> pb::Rect {
    pb::Rect {
        x: rect.origin.x,
        y: rect.origin.y,
        width: rect.size.width,
        height: rect.size.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_round_trips() {
        let panel = CandidatePanelModel {
            developer_note: "note".into(),
            candidates: vec![CandidateItem {
                surface: "漢字".into(),
                annotation: "a".into(),
                kind: CandidateKind::Rewriter,
                label: "1".into(),
            }],
            selected: 0,
            annotation: "b".into(),
            reserved_rows: 3,
        };
        assert_eq!(panel_from_pb(panel_to_pb(&panel)), Ok(panel));
    }

    #[test]
    fn out_of_range_kind_and_side_are_errors() {
        let mut pb_panel = pb::CandidatePanelModel {
            candidates: vec![pb::CandidateItem::default()],
            ..Default::default()
        };
        pb_panel.candidates[0].kind = 99;
        assert!(panel_from_pb(pb_panel).is_err());
        assert!(side_from_pb(0).is_err());
        assert_eq!(
            side_from_pb(pb::PanelSide::Above as i32),
            Ok(PanelSide::Above)
        );
    }

    #[test]
    fn rect_round_trips() {
        let rect = Rect::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(rect_from_pb(Some(rect_to_pb(&rect))), Some(rect));
        assert_eq!(rect_from_pb(None), None);
    }
}
