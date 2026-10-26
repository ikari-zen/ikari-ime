//! ログには種類、所要時間、文節の数、区切り直しの向きだけを書く（ADR 0006 §5.3。NFR-19）。

use super::commit::commit_body;
use super::resize::resize_body;
use super::segment_page::{choice, seg_page, two_segments};
use super::*;

#[test]
fn segment_log_lines_have_counts_and_directions_but_never_text() {
    let dir = temp_dir("seg-log");
    let mock = MockLm::new(Duration::ZERO);
    let stack = mock_stack(&mock, None);
    let seam = MockSeam::new(Arc::clone(&stack.converter));
    let e = Engine::from_parts_seam(
        EngineParts {
            stack,
            rebuild: None,
            data_dir: Some(dir.clone()),
            lm_requested: true,
            lm_budget: LmBudget::default(),
            lock: None,
        },
        Some(seam.clone()),
    );
    let r = two_segments(&e, &seam);
    let id = r.conversion_id;
    let _ = seg_page(&e, id, 1, 0);
    let shrunk = send(&e, resize_body(id, 0, 0, -1, vec![]));
    assert!(matches!(shrunk, envelope::Body::SegmentLayout(_)));
    let grown = send(&e, resize_body(id, 1, 0, 1, vec![]));
    assert!(matches!(grown, envelope::Body::SegmentLayout(_)));
    send(&e, commit_body(id, 2, vec![choice(1), choice(0)]));
    let _ = send(
        &e,
        envelope::Body::Reconvert(pb::Reconvert {
            session_id: 1,
            surface: READING.into(),
            context: None,
            page_size: 0,
        }),
    );
    drop(e);
    let log = std::fs::read_to_string(dir.join("logs").join("engine.log")).unwrap();
    let line = |kind: &str| -> String {
        log.lines()
            .find(|l| l.contains(&format!(" {kind} ")))
            .unwrap_or_else(|| panic!("no {kind} line in {log}"))
            .to_owned()
    };
    assert!(line("convert").contains("segments=2 cache="), "{log}");
    assert!(line("segment_page").contains("err=-"), "{log}");
    let resizes: Vec<&str> = log
        .lines()
        .filter(|l| l.contains(" resize_segment "))
        .collect();
    assert!(resizes[0].ends_with("segments=2 dir=shrink"), "{log}");
    assert!(resizes[1].ends_with("segments=2 dir=expand"), "{log}");
    assert!(line("commit_segments").contains("err=-"), "{log}");
    // 再変換も同じ読みなので、モックは同じ位置で区切る
    assert!(line("reconvert").ends_with("segments=2"), "{log}");
    for secret in [READING, "ひん", BASE_TOP, BOOSTED, "#"] {
        assert!(!log.contains(secret), "{secret} leaked into the log: {log}");
    }
    let _ = std::fs::remove_dir_all(dir);
}
