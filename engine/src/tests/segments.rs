//! 文節の編集と再変換のテスト（ADR 0006）。役割は 2 つに分ける。
//!
//! - **配線のテスト**（このファイルと `commit` / `resize` / `segment_page` / `reconvert` / `lookahead` /
//!   `robust` / `log`）: `MockSeam`（コアの `Segmented` の代わり）で通す。エンジンが番号、版、締め切り、
//!   上限、ログ、学習の事象を正しく受け渡すかだけを見る。候補の文字列（`mock_surface`）に変換の意味はない。
//! - **本物のコアのテスト**（`real_core.rs`）: RealSeam と fixture の辞書で通す。文字列ではなく、
//!   候補の番号と区切りの構造で確かめる。
//! - `perf.rs` は実辞書の速さ（`#[ignore]`）。
//!
//! モックを入れていないエンジン（`Engine::open`、`engine_with`）は本物のコアを使う。

use super::*;
use crate::mock_seam::{MOCK_CHUNK, MOCK_LIST_LEN, MockSeam, mock_surface};

mod commit;
mod log;
mod lookahead;
mod perf;
mod real_core;
mod reconvert;
mod resize;
mod robust;
mod segment_page;

fn seg_engine_with(
    delay_ms: u64,
    learning_path: Option<&Path>,
) -> (Engine, Arc<MockSeam>, Arc<MockLm>) {
    let mock = MockLm::new(Duration::from_millis(delay_ms));
    let stack = mock_stack(&mock, learning_path);
    let seam = MockSeam::new(Arc::clone(&stack.converter));
    let engine = Engine::from_parts_seam(
        EngineParts {
            stack,
            rebuild: None,
            data_dir: None,
            lm_requested: true,
            lm_budget: LmBudget::default(),
            convert_wait: DEFAULT_CONVERT_WAIT,
            lock: None,
        },
        Some(seam.clone()),
    );
    (engine, seam, mock)
}

fn seg_engine() -> (Engine, Arc<MockSeam>, Arc<MockLm>) {
    seg_engine_with(0, None)
}

fn layout_of(r: &pb::ConvertReply) -> &pb::SegmentLayout {
    r.layout
        .as_ref()
        .expect("new engines always send the layout")
}

fn readings(l: &pb::SegmentLayout) -> Vec<&str> {
    l.segments.iter().map(|s| s.reading.as_str()).collect()
}

#[test]
fn convert_reply_carries_a_one_segment_layout_by_default() {
    let (e, _seam, _mock) = seg_engine();
    begin(&e, 1, false);
    let r = convert(&e, 1, READING, 0);
    let layout = layout_of(&r);
    assert_eq!(layout.conversion_id, r.conversion_id);
    assert_eq!(layout.layout_rev, 0);
    assert_eq!(layout.focused, 0);
    assert_eq!(readings(layout), [READING]);
    assert_eq!(layout.segments[0].surface, surfaces(&r)[0]);
    // 1 文節なら focused_page は読み全体の 1 ページ目と同じ
    let page0 = layout.focused_page.as_ref().expect("focused page");
    assert_eq!(page0.segment, 0);
    assert_eq!(page0.page_index, 0);
    assert_eq!(page0.candidates, r.candidates);
    assert_eq!(page0.has_next, r.has_next);
}

#[test]
fn convert_reply_layout_lists_every_segment_and_the_first_page_of_the_first() {
    let (e, seam, _mock) = seg_engine();
    seam.split(READING, &[2]);
    begin(&e, 1, false);
    let r = convert(&e, 1, READING, 0);
    let layout = layout_of(&r);
    assert_eq!(readings(layout), ["ひん", "し"]);
    assert_eq!(layout.segments[0].surface, mock_surface("ひん", 0));
    let page0 = layout.focused_page.as_ref().unwrap();
    assert_eq!(page0.candidates.len(), 9);
    assert_eq!(page0.candidates[0].surface, mock_surface("ひん", 0));
    assert!(page0.has_next);
    // candidates は今までどおり読み全体の候補
    assert_eq!(surfaces(&r)[0], BASE_TOP);
}

#[test]
fn page_reply_carries_the_layout_without_a_page() {
    let (e, seam, _mock) = seg_engine();
    seam.split(READING, &[1]);
    begin(&e, 1, false);
    let r = convert(&e, 1, READING, 0);
    let reply = match page(&e, r.conversion_id, 1) {
        envelope::Body::PageReply(p) => p,
        other => panic!("{other:?}"),
    };
    let layout = layout_of(&reply);
    assert_eq!(layout.conversion_id, r.conversion_id);
    assert_eq!(readings(layout), ["ひ", "んし"]);
    assert!(layout.focused_page.is_none());
}

// モックの列は 2 ページ（9 件 + 残り）。ページ送りのテストの前提
const _: () = assert!(MOCK_LIST_LEN > MOCK_CHUNK && MOCK_LIST_LEN <= 2 * MOCK_CHUNK);
