//! 壊れた入力と、コアの panic でエンジンが落ちないこと（ADR 0006 §10.1）。

use std::sync::atomic::AtomicBool;

use ikari_core::{Commit, Context, Conversion, SegmentCommit, SegmentLayout, SegmentedOrigin};

use super::commit::commit_body;
use super::resize::resize_body;
use super::segment_page::{choice, literal, seg_page_body, two_segments};
use super::*;
use crate::segments::{Chunk, SegConv, SegmentSeam};

fn reconvert_request(session: u64) -> envelope::Body {
    envelope::Body::Reconvert(pb::Reconvert {
        session_id: session,
        surface: READING.into(),
        context: Some(pb::SurroundingText {
            available: true,
            left: "前の文".into(),
            right: String::new(),
        }),
        page_size: 9,
    })
}

#[test]
fn broken_segment_requests_never_panic_and_always_return_an_envelope() {
    let (e, seam, _mock) = seg_engine();
    let id = two_segments(&e, &seam).conversion_id;
    let valid: Vec<Vec<u8>> = [
        seg_page_body(id, 0, 1, 0, vec![choice(0), literal("シ")]),
        resize_body(id, 0, 0, -1, vec![choice(2)]),
        commit_body(id, 0, vec![choice(1), literal("シ")]),
        reconvert_request(1),
    ]
    .into_iter()
    .map(|body| {
        Envelope {
            schema_version: SCHEMA_VERSION,
            request_id: 1,
            body: Some(body),
        }
        .encode_to_vec()
    })
    .collect();
    let mut inputs: Vec<Vec<u8>> = Vec::new();
    for bytes in &valid {
        for cut in 0..bytes.len() {
            inputs.push(bytes[..cut].to_vec());
        }
        for i in 0..bytes.len() {
            for flip in [0xffu8, 0x01, 0x80] {
                let mut v = bytes.clone();
                v[i] ^= flip;
                inputs.push(v);
            }
        }
    }
    for input in inputs {
        let reply = e.handle(&input);
        Envelope::decode(reply.as_slice()).expect("reply must decode");
    }
    // 壊れた要求のあとも普通に動く（確定でも消えていなければ、変換が使える）
    let r = convert(&e, 1, READING, 7);
    assert!(!r.candidates.is_empty());
}

#[test]
fn out_of_range_and_extreme_values_are_errors_not_panics() {
    let (e, seam, _mock) = seg_engine();
    let id = two_segments(&e, &seam).conversion_id;
    let extremes = [
        seg_page_body(u64::MAX, u32::MAX, u32::MAX, u32::MAX, vec![]),
        seg_page_body(id, u32::MAX, 0, 0, vec![]),
        seg_page_body(id, 0, u32::MAX, 0, vec![choice(u32::MAX)]),
        seg_page_body(id, 0, 0, u32::MAX, vec![]),
        resize_body(u64::MAX, u32::MAX, u32::MAX, i32::MIN, vec![]),
        resize_body(id, 0, 1, i32::MAX, vec![choice(u32::MAX)]),
        resize_body(id, 0, 0, i32::MIN, vec![]),
        commit_body(u64::MAX, u32::MAX, vec![choice(u32::MAX)]),
        commit_body(id, 0, vec![choice(u32::MAX), choice(u32::MAX)]),
        commit_body(id, 0, vec![]),
        reconvert_request(u64::MAX),
    ];
    for body in extremes {
        let reply = send(&e, body);
        assert!(matches!(reply, envelope::Body::Error(_)), "{reply:?}");
    }
    // 失敗のあとも、変換は使える
    let ok = send(&e, commit_body(id, 0, vec![choice(0), choice(0)]));
    assert!(matches!(ok, envelope::Body::Ack(_)), "{ok:?}");
}

/// 区切りの計算が panic するコア
struct PanickingSeam;

impl SegmentSeam for PanickingSeam {
    fn segment(&self, _whole: Conversion) -> Box<dyn SegConv> {
        panic!("core is not ready");
    }

    fn reconvert(
        &self,
        _surface: &str,
        _ctx: &Context,
        _hint: &[Commit],
        _deadline: Option<Instant>,
    ) -> Result<Box<dyn SegConv>, ikari_core::SegmentError> {
        panic!("core is not ready");
    }
}

#[test]
fn a_panicking_core_becomes_internal_errors_and_the_engine_keeps_serving() {
    let mock = MockLm::new(Duration::ZERO);
    let e = Engine::from_parts_seam(
        EngineParts {
            stack: mock_stack(&mock, None),
            rebuild: None,
            data_dir: None,
            lm_requested: true,
            lm_budget: LmBudget::default(),
            lock: None,
        },
        Some(Arc::new(PanickingSeam)),
    );
    begin(&e, 1, false);
    // 先行計算の中の panic は、依頼を捨てるだけ
    prefetch(&e, 1, READING, 0);
    wait_until(|| !lock_mutex(&e.inner.cache).any_computing());
    assert_eq!(
        error_code(&send(&e, convert_req(1, READING, 0, 0))),
        ErrorCode::Internal
    );
    assert_eq!(
        error_code(&send(&e, reconvert_request(1))),
        ErrorCode::Internal
    );
    // 管理、確定履歴など、文節に関わらない要求は動く
    assert!(matches!(
        send(
            &e,
            envelope::Body::CommitText(pb::CommitText {
                session_id: 1,
                reading: "あ".into(),
                surface: "亜".into(),
            })
        ),
        envelope::Body::Ack(_)
    ));
}

/// 文節 1 以降の塊を作るときに 1 回だけ panic するコア
struct PanicOnceSeam {
    inner: Arc<MockSeam>,
    armed: Arc<AtomicBool>,
}

struct PanicOnceSeg {
    inner: Box<dyn SegConv>,
    armed: Arc<AtomicBool>,
}

impl SegmentSeam for PanicOnceSeam {
    fn segment(&self, whole: Conversion) -> Box<dyn SegConv> {
        Box::new(PanicOnceSeg {
            inner: self.inner.segment(whole),
            armed: Arc::clone(&self.armed),
        })
    }

    fn reconvert(
        &self,
        surface: &str,
        ctx: &Context,
        hint: &[Commit],
        deadline: Option<Instant>,
    ) -> Result<Box<dyn SegConv>, ikari_core::SegmentError> {
        self.inner.reconvert(surface, ctx, hint, deadline)
    }
}

impl SegConv for PanicOnceSeg {
    fn layout(&self) -> &SegmentLayout {
        self.inner.layout()
    }

    fn whole(&self) -> &Conversion {
        self.inner.whole()
    }

    fn whole_mut(&mut self) -> &mut Conversion {
        self.inner.whole_mut()
    }

    fn chunk(
        &mut self,
        seg: usize,
        index: usize,
        left: &[ikari_core::SegmentChoice],
        deadline: Option<Instant>,
    ) -> Result<Option<Chunk>, ikari_core::SegmentError> {
        if seg >= 1 && self.armed.swap(false, Ordering::AcqRel) {
            panic!("chunk failed once");
        }
        self.inner.chunk(seg, index, left, deadline)
    }

    fn origin(&self) -> SegmentedOrigin {
        self.inner.origin()
    }

    fn commit(
        &self,
        choices: &[ikari_core::SegmentChoice],
    ) -> Result<SegmentCommit, ikari_core::SegmentError> {
        self.inner.commit(choices)
    }

    fn resize(
        &mut self,
        index: usize,
        delta: i32,
        left: &[ikari_core::SegmentChoice],
        deadline: Option<Instant>,
    ) -> Result<(), ikari_core::SegmentError> {
        self.inner.resize(index, delta, left, deadline)
    }
}

#[test]
fn a_panic_in_the_lookahead_thread_does_not_stop_later_lookaheads() {
    let mock = MockLm::new(Duration::ZERO);
    let stack = mock_stack(&mock, None);
    let inner = MockSeam::new(Arc::clone(&stack.converter));
    inner.split(READING, &[2]);
    let armed = Arc::new(AtomicBool::new(true));
    let e = Engine::from_parts_seam(
        EngineParts {
            stack,
            rebuild: None,
            data_dir: None,
            lm_requested: true,
            lm_budget: LmBudget::default(),
            lock: None,
        },
        Some(Arc::new(PanicOnceSeam {
            inner: Arc::clone(&inner),
            armed: Arc::clone(&armed),
        })),
    );
    begin(&e, 1, false);
    let _ = convert(&e, 1, READING, 0);
    wait_until(|| !armed.load(Ordering::Acquire));
    // 2 回目の変換の先読みは、同じスレッドで動く
    let _ = convert(&e, 1, READING, 1);
    wait_until(|| {
        inner
            .calls()
            .iter()
            .any(|c| c.what == "chunk" && c.seg == 1 && c.index == 0)
    });
}
