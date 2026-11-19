//! 受けたものの上限（ADR 0068 §5。B-1）。
//!
//! 候補窓は送り手を確かめられない（どのアプリの TIP も正当な送り手）。その代わり、受けたものを「描く文字」としてだけ
//! 扱い、行の数、1 行の長さ、文字の種類、座標と倍率に上限を置く。外れたものは、切り詰めるか、命令ごと捨てる:
//! - 文字（候補、注釈、名前、開発者の行）: 制御文字と、表示の向きを変える文字を除き、長さを切り詰める
//! - 候補の行の数、予約の行の数: 切り詰める
//! - 選択の位置が候補の外: 命令を捨てる（どの行を強調するかが決まらない）
//! - 座標、大きさ、倍率が有限でない・範囲の外: 命令を捨てる（窓をどこに置くかが決まらない）
//!
//! **値はすべて仮置き**（計測なし。ADR 0068 §7 は「何に上限を置くか」だけを決めた）。ふつうの候補（1 ページ 9 件、
//! 短い語）が引っかからない大きさにしてある。

use ikari_proto::candidate::v1::Show;
use ikari_proto::shell::v1::{CandidatePanelModel, Rect};

/// 候補の行の上限（仮置き。ふつうは 1 ページ 9 件）
pub const MAX_CANDIDATE_ROWS: usize = 16;
/// 1 つの文字列の長さの上限（Unicode のスカラー値の数。仮置き）
pub const MAX_TEXT_CHARS: usize = 256;
/// 座標と大きさの絶対値の上限（物理のピクセル。仮置き。`shell-core::panel` の `MAX_ANCHOR_COORDINATE` と同じ大きさ）
pub const MAX_COORDINATE: f64 = 1_000_000.0;
/// 倍率（DPI / 96）の下限と上限（仮置き。Windows の表示の拡大は 100%〜500%）
pub const MIN_SCALE: f64 = 0.5;
pub const MAX_SCALE: f64 = 8.0;

/// 命令を捨てた理由（固定の語。ログにはこれだけを書く）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// 選択の位置が候補の外
    SelectionOutOfRange,
    /// アンカーか作業領域が無い・有限でない・範囲の外
    BadGeometry,
    /// 倍率が有限でない・範囲の外
    BadScale,
}

impl Rejection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SelectionOutOfRange => "selection-out-of-range",
            Self::BadGeometry => "bad-geometry",
            Self::BadScale => "bad-scale",
        }
    }
}

/// 表示の向きを変える文字（U+202A〜U+202E、U+2066〜U+2069、U+200E、U+200F、U+061C）と、見えない文字（行・段落の区切り
/// U+2028、U+2029、幅のないスペース U+200B、U+2060、U+FEFF、タグ文字 U+E0000〜U+E007F）。見た目で別の文字列に見せたり、
/// 見えない文字を運んだりできるので除く（ZWJ・ZWNJ は絵文字と一部の文字の綴りに要るので残す）
fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{061C}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{200B}'
            | '\u{2060}'
            | '\u{FEFF}'
            | '\u{E0000}'..='\u{E007F}'
    )
}

/// 描く文字列にする: 制御文字（改行、タブを含む）と、表示の向きを変える文字を除き、長さを切り詰める。
pub fn sanitize_text(text: &str) -> String {
    text.chars()
        .filter(|&c| !c.is_control() && !is_bidi_control(c))
        .take(MAX_TEXT_CHARS)
        .collect()
}

/// モデルを描けるものにする。選択の位置が候補の外なら `Err`。
pub fn sanitize_model(model: CandidatePanelModel) -> Result<CandidatePanelModel, Rejection> {
    let mut candidates = model.candidates;
    candidates.truncate(MAX_CANDIDATE_ROWS);
    for item in &mut candidates {
        item.surface = sanitize_text(&item.surface);
        item.annotation = sanitize_text(&item.annotation);
        item.label = sanitize_text(&item.label);
    }
    let selected_ok = candidates.is_empty()
        || usize::try_from(model.selected).is_ok_and(|selected| selected < candidates.len());
    if !selected_ok {
        return Err(Rejection::SelectionOutOfRange);
    }
    let max_rows = u32::try_from(MAX_CANDIDATE_ROWS).unwrap_or(u32::MAX);
    Ok(CandidatePanelModel {
        candidates,
        selected: model.selected,
        annotation: sanitize_text(&model.annotation),
        reserved_rows: model.reserved_rows.min(max_rows),
        developer_note: sanitize_text(&model.developer_note),
    })
}

fn coordinate_ok(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_COORDINATE
}

fn extent_ok(v: f64) -> bool {
    v.is_finite() && (0.0..=MAX_COORDINATE).contains(&v)
}

fn rect_ok(rect: &Rect) -> bool {
    coordinate_ok(rect.x)
        && coordinate_ok(rect.y)
        && extent_ok(rect.width)
        && extent_ok(rect.height)
}

/// 窓の置き場所（アンカー、作業領域、倍率）が使えるかを確かめる。作業領域は幅と高さが正のものだけ。
pub fn check_geometry(
    anchor: Option<&Rect>,
    work_area: Option<&Rect>,
    scale: f64,
) -> Result<(), Rejection> {
    let (Some(anchor), Some(work_area)) = (anchor, work_area) else {
        return Err(Rejection::BadGeometry);
    };
    if !rect_ok(anchor) || !rect_ok(work_area) || work_area.width <= 0.0 || work_area.height <= 0.0
    {
        return Err(Rejection::BadGeometry);
    }
    if !(scale.is_finite() && (MIN_SCALE..=MAX_SCALE).contains(&scale)) {
        return Err(Rejection::BadScale);
    }
    Ok(())
}

/// アンカーを作業領域の中に寄せる（窓を作業領域の外に出さない。ADR 0068 §5）。大きさは作業領域に収まるよう縮める。
pub fn clamp_anchor(anchor: &Rect, work_area: &Rect) -> Rect {
    let width = anchor.width.min(work_area.width);
    let height = anchor.height.min(work_area.height);
    let x = anchor
        .x
        .clamp(work_area.x, work_area.x + work_area.width - width);
    let y = anchor
        .y
        .clamp(work_area.y, work_area.y + work_area.height - height);
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// `Show` を描けるものにする（モデル、置き場所、倍率）。捨てるなら `Err`。
pub fn sanitize_show(show: Show) -> Result<Show, Rejection> {
    check_geometry(show.anchor.as_ref(), show.work_area.as_ref(), show.scale)?;
    let model = sanitize_model(show.model.unwrap_or_default())?;
    let work_area = show.work_area.unwrap_or_default();
    let anchor = clamp_anchor(&show.anchor.unwrap_or_default(), &work_area);
    Ok(Show {
        generation: show.generation,
        model: Some(model),
        anchor: Some(anchor),
        work_area: Some(work_area),
        side: show.side,
        scale: show.scale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikari_proto::shell::v1::CandidateItem;

    fn item(surface: &str) -> CandidateItem {
        CandidateItem {
            surface: surface.to_owned(),
            ..Default::default()
        }
    }

    fn model(n: usize, selected: u32) -> CandidatePanelModel {
        CandidatePanelModel {
            candidates: (0..n).map(|i| item(&format!("候補{i}"))).collect(),
            selected,
            ..Default::default()
        }
    }

    const WORK: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1040.0,
    };

    fn show(anchor: Rect, scale: f64) -> Show {
        Show {
            generation: 1,
            model: Some(model(3, 0)),
            anchor: Some(anchor),
            work_area: Some(WORK),
            side: 1,
            scale,
        }
    }

    #[test]
    fn text_drops_control_and_bidi_characters_and_is_truncated() {
        assert_eq!(sanitize_text("漢\n字\t\u{202E}かな\u{0007}"), "漢字かな");
        let long: String = "あ".repeat(MAX_TEXT_CHARS + 10);
        assert_eq!(sanitize_text(&long).chars().count(), MAX_TEXT_CHARS);
        // ふつうの文字（絵文字の結合、全角の空白）は残す
        assert_eq!(sanitize_text("👍🏽\u{3000}x"), "👍🏽\u{3000}x");
    }

    #[test]
    fn text_drops_invisible_characters() {
        assert_eq!(
            sanitize_text("a\u{2028}b\u{2029}c\u{200B}d\u{2060}e\u{FEFF}f\u{E0041}g"),
            "abcdefg"
        );
        // 絵文字の結合に使う ZWJ は残す
        assert_eq!(sanitize_text("👨\u{200D}👩"), "👨\u{200D}👩");
    }

    #[test]
    fn rows_are_truncated_and_reserved_rows_are_capped() {
        let mut m = model(MAX_CANDIDATE_ROWS + 5, 2);
        m.reserved_rows = 1000;
        let out = sanitize_model(m).unwrap();
        assert_eq!(out.candidates.len(), MAX_CANDIDATE_ROWS);
        assert_eq!(out.reserved_rows as usize, MAX_CANDIDATE_ROWS);
    }

    #[test]
    fn selection_outside_the_shown_rows_is_rejected() {
        assert_eq!(
            sanitize_model(model(3, 3)),
            Err(Rejection::SelectionOutOfRange)
        );
        // 切り詰めた結果、選択が外に出た
        assert_eq!(
            sanitize_model(model(MAX_CANDIDATE_ROWS + 1, MAX_CANDIDATE_ROWS as u32)),
            Err(Rejection::SelectionOutOfRange)
        );
        // 候補が無いときは選択を問わない（かなの候補もない、空の窓）
        assert!(sanitize_model(model(0, 0)).is_ok());
    }

    #[test]
    fn geometry_must_be_finite_and_bounded() {
        let ok = Rect {
            x: 100.0,
            y: 100.0,
            width: 1.0,
            height: 20.0,
        };
        assert!(sanitize_show(show(ok, 1.5)).is_ok());
        for bad in [
            Rect { x: f64::NAN, ..ok },
            Rect {
                x: f64::INFINITY,
                ..ok
            },
            Rect {
                y: MAX_COORDINATE * 2.0,
                ..ok
            },
            Rect { width: -1.0, ..ok },
        ] {
            assert_eq!(sanitize_show(show(bad, 1.0)), Err(Rejection::BadGeometry));
        }
        let mut no_anchor = show(ok, 1.0);
        no_anchor.anchor = None;
        assert_eq!(sanitize_show(no_anchor), Err(Rejection::BadGeometry));
        let mut empty_work = show(ok, 1.0);
        empty_work.work_area = Some(Rect { width: 0.0, ..WORK });
        assert_eq!(sanitize_show(empty_work), Err(Rejection::BadGeometry));
    }

    #[test]
    fn scale_must_be_in_range() {
        let ok = Rect {
            x: 100.0,
            y: 100.0,
            width: 1.0,
            height: 20.0,
        };
        for bad in [0.0, -1.0, f64::NAN, MAX_SCALE + 0.1, MIN_SCALE - 0.1] {
            assert_eq!(sanitize_show(show(ok, bad)), Err(Rejection::BadScale));
        }
    }

    #[test]
    fn anchor_is_pulled_inside_the_work_area() {
        let outside = Rect {
            x: 5000.0,
            y: -300.0,
            width: 10.0,
            height: 20.0,
        };
        let out = sanitize_show(show(outside, 1.0)).unwrap().anchor.unwrap();
        assert_eq!(
            out,
            Rect {
                x: 1910.0,
                y: 0.0,
                width: 10.0,
                height: 20.0
            }
        );
        // 作業領域より大きいアンカーは縮める
        let huge = Rect {
            x: 0.0,
            y: 0.0,
            width: 5000.0,
            height: 5000.0,
        };
        assert_eq!(clamp_anchor(&huge, &WORK), WORK);
    }
}
