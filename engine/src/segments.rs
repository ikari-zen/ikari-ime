//! 文節の編集と再変換のエンジン側（ADR 0006 §5.3）。
//!
//! コア（`Segmented`）との境目は `SegmentSeam` / `SegConv` の薄い trait。本番は `RealSeam`（コアの
//! `Converter::segment` などを呼ぶ）、テストはモックで通す（`mock_seam.rs`）。
//! ログには種類、所要時間、文節の数、区切り直しの向きだけを書く。読みと表記は書かない（NFR-19）。

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use ikari_core::{
    Candidate, Commit, Context, Conversion, Converter, SegmentCommit, SegmentError, SegmentLayout,
    Segmented, SegmentedOrigin,
};
use ikari_proto::v1 as pb;
use ikari_proto::v1::{ErrorCode, envelope};

use crate::cache::{SharedConversion, lock as lock_mutex};
use crate::{Fail, candidate_kind};

/// 文節の候補の 1 塊（コアの `Page` を所有型にしたもの）
pub(crate) struct Chunk {
    pub candidates: Vec<Candidate>,
    pub has_next: bool,
}

/// 区切りと文節ごとの候補の列 1 つ（コアの `Segmented`、またはテストのモック）
pub(crate) trait SegConv: Send {
    fn layout(&self) -> &SegmentLayout;
    /// 読み全体の変換（`ConvertReply.candidates`、旧式の `PageRequest` と `Commit`）
    fn whole(&self) -> &Conversion;
    fn whole_mut(&mut self) -> &mut Conversion;
    /// 文節 `seg` の `index` 塊目。範囲外の塊は `Ok(None)`
    fn chunk(
        &mut self,
        seg: usize,
        index: usize,
        left: &[ikari_core::SegmentChoice],
        deadline: Option<Instant>,
    ) -> Result<Option<Chunk>, SegmentError>;
    fn origin(&self) -> SegmentedOrigin;
    /// 確定（ADR 0006 §6.1）。学習の事象は変えた文節ごとと読み全体。secure なら空
    fn commit(&self, choices: &[ikari_core::SegmentChoice]) -> Result<SegmentCommit, SegmentError>;
    /// 区切り直し（ADR 0006 §4）。動かせないときは何も変えずに `Ok`（`layout().rev` が変わらない）
    fn resize(
        &mut self,
        index: usize,
        delta: i32,
        left: &[ikari_core::SegmentChoice],
        deadline: Option<Instant>,
    ) -> Result<(), SegmentError>;
}

/// `convert_until` の結果を区切るところ
pub(crate) trait SegmentSeam: Send + Sync {
    fn segment(&self, whole: Conversion) -> Box<dyn SegConv>;
    /// 再変換（ADR 0006 §9）。`hint` は確定履歴（新しい順）。読める部分がなければ `Empty`
    fn reconvert(
        &self,
        surface: &str,
        ctx: &Context,
        hint: &[Commit],
        deadline: Option<Instant>,
    ) -> Result<Box<dyn SegConv>, SegmentError>;
}

/// 本番の seam。コアの `Converter` を呼ぶ
pub(crate) struct RealSeam(pub Arc<Converter>);

struct RealSeg {
    converter: Arc<Converter>,
    seg: Segmented,
}

impl SegmentSeam for RealSeam {
    fn segment(&self, whole: Conversion) -> Box<dyn SegConv> {
        Box::new(RealSeg {
            converter: Arc::clone(&self.0),
            seg: self.0.segment(whole),
        })
    }

    fn reconvert(
        &self,
        surface: &str,
        ctx: &Context,
        hint: &[Commit],
        deadline: Option<Instant>,
    ) -> Result<Box<dyn SegConv>, SegmentError> {
        Ok(Box::new(RealSeg {
            converter: Arc::clone(&self.0),
            seg: self.0.reconvert(surface, ctx, hint, deadline)?,
        }))
    }
}

impl SegConv for RealSeg {
    fn layout(&self) -> &SegmentLayout {
        self.seg.layout()
    }

    fn whole(&self) -> &Conversion {
        self.seg.whole()
    }

    fn whole_mut(&mut self) -> &mut Conversion {
        self.seg.whole_mut()
    }

    fn chunk(
        &mut self,
        seg: usize,
        index: usize,
        left: &[ikari_core::SegmentChoice],
        deadline: Option<Instant>,
    ) -> Result<Option<Chunk>, SegmentError> {
        Ok(self
            .seg
            .page_until(seg, index, left, deadline)?
            .map(|page| Chunk {
                candidates: page.candidates.to_vec(),
                has_next: page.has_next,
            }))
    }

    fn origin(&self) -> SegmentedOrigin {
        self.seg.origin()
    }

    fn commit(&self, choices: &[ikari_core::SegmentChoice]) -> Result<SegmentCommit, SegmentError> {
        self.seg.commit(choices)
    }

    fn resize(
        &mut self,
        index: usize,
        delta: i32,
        left: &[ikari_core::SegmentChoice],
        deadline: Option<Instant>,
    ) -> Result<(), SegmentError> {
        self.converter
            .resize(&mut self.seg, index, delta, left, deadline)
    }
}

/// 文節 1 つぶんの、エンジン側で持つ生成済みの列。コアが返した塊を順に足したもの（コアの列と同じ並び）
#[derive(Default)]
struct SegList {
    candidates: Vec<Candidate>,
    next_chunk: usize,
    exhausted: bool,
}

/// 殻へ返した変換 1 つ（`conversion_id` の中身）。ロックの中で使う。
pub(crate) struct SegEntry {
    seg: Box<dyn SegConv>,
    lists: Vec<SegList>,
    /// 最初の区切りの版（再変換で、区切りを変えていないかを見る）
    initial_rev: u32,
}

impl SegEntry {
    pub fn new(seg: Box<dyn SegConv>) -> SegEntry {
        let lists = (0..seg.layout().spans.len())
            .map(|_| SegList::default())
            .collect();
        let initial_rev = seg.layout().rev;
        SegEntry {
            seg,
            lists,
            initial_rev,
        }
    }

    /// 再変換で、区切りも選択も元のまま（何も変えていない）か。変えていなければ学習しない（§9.7）
    fn is_untouched_reconversion(&self, choices: &[ikari_core::SegmentChoice]) -> bool {
        self.seg.origin() == SegmentedOrigin::Reconvert
            && self.seg.layout().rev == self.initial_rev
            && choices
                .iter()
                .all(|c| matches!(c, ikari_core::SegmentChoice::Candidate(0)))
    }

    pub fn layout(&self) -> &SegmentLayout {
        self.seg.layout()
    }

    pub fn whole(&self) -> &Conversion {
        self.seg.whole()
    }

    fn origin(&self) -> SegmentedOrigin {
        self.seg.origin()
    }

    fn commit(&self, choices: &[ikari_core::SegmentChoice]) -> Result<SegmentCommit, SegmentError> {
        self.seg.commit(choices)
    }

    pub fn whole_mut(&mut self) -> &mut Conversion {
        self.seg.whole_mut()
    }

    /// 2 文節以上なら、文節 `seg` の 1 ページ目まで作っておく（先行計算。ADR 0006 §5.3）。失敗は無視する
    pub fn prefill(&mut self, seg: usize, page_size: usize, deadline: Instant) {
        if self.layout().spans.len() >= 2 {
            let _ = self.segment_page(seg, 0, page_size, &[], Some(deadline));
        }
    }

    /// 区切り直し。変わったら（版が上がったら）`index` 以降の列は作り直しになるので捨てる
    pub fn resize(
        &mut self,
        index: usize,
        delta: i32,
        left: &[ikari_core::SegmentChoice],
        deadline: Option<Instant>,
    ) -> Result<(), Fail> {
        let before = self.seg.layout().rev;
        self.seg
            .resize(index, delta, left, deadline)
            .map_err(fail_of)?;
        if self.seg.layout().rev != before {
            let count = self.seg.layout().spans.len();
            self.lists.truncate(index);
            self.lists.resize_with(count, SegList::default);
        }
        Ok(())
    }

    /// 文節 `seg` の生成済みの列を、少なくとも `need` 件まで（作れる限り）伸ばす
    fn ensure_len(
        &mut self,
        seg: usize,
        need: usize,
        left: &[ikari_core::SegmentChoice],
        deadline: Option<Instant>,
    ) -> Result<(), Fail> {
        let list = self
            .lists
            .get_mut(seg)
            .ok_or(Fail::bad("segment out of range"))?;
        while list.candidates.len() < need && !list.exhausted {
            match self
                .seg
                .chunk(seg, list.next_chunk, left, deadline)
                .map_err(fail_of)?
            {
                Some(chunk) => {
                    list.candidates.extend(chunk.candidates);
                    list.next_chunk += 1;
                    list.exhausted = !chunk.has_next;
                }
                None => list.exhausted = true,
            }
        }
        Ok(())
    }

    /// 文節 `seg` の `index` ページ目（1 ページ `page_size` 件）と、次のページがあるか。
    /// 一度返したページは変わらない（コアの性質）。`left` は初めて作るときだけ使われる
    pub fn segment_page(
        &mut self,
        seg: usize,
        index: usize,
        page_size: usize,
        left: &[ikari_core::SegmentChoice],
        deadline: Option<Instant>,
    ) -> Result<(Vec<pb::Candidate>, bool), Fail> {
        let start = index
            .checked_mul(page_size)
            .ok_or(Fail::bad("page index out of range"))?;
        let end = start + page_size;
        // 次のページがあるかを正確に返すため、end の次の 1 件まで作る
        self.ensure_len(seg, end + 1, left, deadline)?;
        let list = &self.lists[seg].candidates;
        if start >= list.len() {
            return Err(Fail::bad("page out of range"));
        }
        let candidates = list[start..end.min(list.len())]
            .iter()
            .map(candidate_info)
            .collect();
        Ok((candidates, list.len() > end))
    }
}

pub(crate) fn candidate_info(c: &Candidate) -> pb::Candidate {
    pb::Candidate {
        surface: c.surface.clone(),
        annotation: String::new(),
        kind: candidate_kind(c.origin) as i32,
    }
}

/// コアの失敗を `ErrorReply` にする（固定の文だけ。ユーザーの文字列は入れない）
pub(crate) fn fail_of(e: SegmentError) -> Fail {
    match e {
        SegmentError::StaleLayout => Fail::new(ErrorCode::StaleLayout, "stale layout"),
        SegmentError::OutOfRange => Fail::bad("segment out of range"),
        SegmentError::Empty => Fail::bad("nothing to convert"),
    }
}

/// `SegmentLayout` の返事（殻へ返す形）
pub(crate) fn layout_reply(
    conversion_id: u64,
    layout: &SegmentLayout,
    focused: usize,
    focused_page: Option<pb::SegmentPageReply>,
    engine_micros: u32,
) -> pb::SegmentLayout {
    pb::SegmentLayout {
        conversion_id,
        layout_rev: layout.rev,
        segments: layout
            .spans
            .iter()
            .map(|s| pb::SegmentInfo {
                reading: s.reading.clone(),
                surface: s.surface.clone(),
            })
            .collect(),
        focused: u32::try_from(focused).unwrap_or(u32::MAX),
        focused_page,
        engine_micros,
    }
}

/// `SegmentChoice` の数は読みの文字数まで（`MAX_READING_CHARS`）、`literal` は 256 文字まで
pub(crate) const MAX_LITERAL_CHARS: usize = 256;
/// `Reconvert.surface` の上限（文字数。ADR 0006 §9.2）
pub(crate) const MAX_RECONVERT_CHARS: usize = 64;

/// 殻の選択をコアの選択にする。上限を超えたら `BAD_REQUEST`
pub(crate) fn core_choices(
    choices: &[pb::SegmentChoice],
) -> Result<Vec<ikari_core::SegmentChoice>, Fail> {
    if choices.len() > crate::MAX_READING_CHARS {
        return Err(Fail::bad("too many choices"));
    }
    choices
        .iter()
        .map(|c| {
            if crate::char_count_exceeds(&c.literal, MAX_LITERAL_CHARS) {
                return Err(Fail::bad("literal too long"));
            }
            Ok(if c.literal.is_empty() {
                ikari_core::SegmentChoice::Candidate(
                    usize::try_from(c.candidate_index).unwrap_or(usize::MAX),
                )
            } else {
                ikari_core::SegmentChoice::Literal(c.literal.clone())
            })
        })
        .collect()
}

/// 区切り直しの向き（ログ用の固定の語）。`ResizeSegment` 以外は None
pub(crate) fn resize_direction(body: &envelope::Body) -> Option<&'static str> {
    match body {
        envelope::Body::ResizeSegment(r) => Some(match r.delta.cmp(&0) {
            std::cmp::Ordering::Less => "shrink",
            std::cmp::Ordering::Greater => "expand",
            std::cmp::Ordering::Equal => "none",
        }),
        _ => None,
    }
}

/// ログの注記。返事に区切りがあれば文節の数（`segments=3`）、区切り直しなら向き（`dir=shrink`）。
/// 読みと表記は入れない（NFR-19）
pub(crate) fn log_note(reply: &envelope::Body, resize: Option<&'static str>) -> Option<String> {
    let count = match reply {
        envelope::Body::ConvertReply(r) => r.layout.as_ref().map(|l| l.segments.len()),
        envelope::Body::SegmentLayout(l) => Some(l.segments.len()),
        _ => None,
    };
    match (count, resize) {
        (Some(n), Some(dir)) => Some(format!("segments={n} dir={dir}")),
        (Some(n), None) => Some(format!("segments={n}")),
        (None, Some(dir)) => Some(format!("dir={dir}")),
        (None, None) => None,
    }
}

/// 古い版の要求は `STALE_LAYOUT`（壊れた殻と再送の守り）
fn check_rev(entry: &SegEntry, rev: u32) -> Result<(), Fail> {
    if entry.layout().rev == rev {
        Ok(())
    } else {
        Err(fail_of(SegmentError::StaleLayout))
    }
}

/// 文節の先読みの依頼 1 件。`Convert` の返事のあと、文節 1 以降の 1 ページ目を LM つきで作る（ADR 0006 §3.4）
pub(crate) struct LookJob {
    pub session: u64,
    pub conv: SharedConversion,
    pub page_size: usize,
    /// 依頼したときの区切りの版。区切り直しがあったら続けない（`Resize` が作り直す）
    pub rev: u32,
    /// 新しい依頼や `Prefetch`、セッションの終わりで立てる。計算の途中でも止める
    pub cancel: Arc<AtomicBool>,
    seq: u64,
}

impl LookJob {
    pub fn new(session: u64, conv: SharedConversion, page_size: usize, rev: u32) -> LookJob {
        LookJob {
            session,
            conv,
            page_size,
            rev,
            cancel: Arc::new(AtomicBool::new(false)),
            seq: 0,
        }
    }
}

#[derive(Default)]
struct LookState {
    /// セッションごとに最新の 1 件だけ
    jobs: HashMap<u64, LookJob>,
    seq: u64,
    closed: bool,
}

/// 先読みの待ち行列（`PrefetchQueue` と同じ形。セッションごとに最新の 1 件、新しい依頼から処理する）
#[derive(Default)]
pub(crate) struct LookaheadQueue {
    state: Mutex<LookState>,
    wake: Condvar,
    /// 処理中の依頼の `cancel`（セッションごと）
    running: Mutex<HashMap<u64, Arc<AtomicBool>>>,
}

impl LookaheadQueue {
    /// 依頼を置く。同じセッションの古い依頼は、待っていても処理中でも止める
    pub fn submit(&self, mut job: LookJob) {
        let mut state = lock_mutex(&self.state);
        if state.closed {
            return;
        }
        state.seq += 1;
        job.seq = state.seq;
        if let Some(running) = lock_mutex(&self.running).remove(&job.session) {
            running.store(true, Ordering::Release);
        }
        if let Some(old) = state.jobs.insert(job.session, job) {
            old.cancel.store(true, Ordering::Release);
        }
        self.wake.notify_one();
    }

    /// 処理を終えた。止める対象から外す
    pub fn finish(&self, job: &LookJob) {
        let mut running = lock_mutex(&self.running);
        if running
            .get(&job.session)
            .is_some_and(|c| Arc::ptr_eq(c, &job.cancel))
        {
            running.remove(&job.session);
        }
    }

    /// セッションの待ち依頼を捨てる。処理中のものも止める（`cancel`）
    pub fn forget_session(&self, session: u64) {
        if let Some(old) = lock_mutex(&self.state).jobs.remove(&session) {
            old.cancel.store(true, Ordering::Release);
        }
        if let Some(running) = lock_mutex(&self.running).remove(&session) {
            running.store(true, Ordering::Release);
        }
    }

    /// 次の依頼を待つ。閉じられたら None
    pub fn next(&self) -> Option<LookJob> {
        let mut state = lock_mutex(&self.state);
        loop {
            if state.closed {
                return None;
            }
            let newest = state
                .jobs
                .iter()
                .max_by_key(|(_, j)| j.seq)
                .map(|(session, _)| *session);
            if let Some(session) = newest {
                let job = state.jobs.remove(&session)?;
                lock_mutex(&self.running).insert(session, Arc::clone(&job.cancel));
                return Some(job);
            }
            state = self.wake.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    pub fn close(&self) {
        let mut state = lock_mutex(&self.state);
        state.closed = true;
        for (_, job) in state.jobs.drain() {
            job.cancel.store(true, Ordering::Release);
        }
        self.wake.notify_all();
    }
}

/// 再変換の表記に改行は入れない（ADR 0006 §9.2。複数行の再変換はしない）
fn has_line_break(text: &str) -> bool {
    text.chars()
        .any(|c| matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'))
}

impl crate::Inner {
    /// 先読みのスレッド。`Convert` が返した 2 文節以上の変換について、文節 1 以降の 1 ページ目を
    /// LM つきで作っておく。人が → を押すまでの間（100ms 以上）にたいてい間に合う
    pub(crate) fn lookahead_loop(&self) {
        while let Some(job) = self.lookahead.next() {
            let ran = catch_unwind(AssertUnwindSafe(|| self.run_lookahead(&job)));
            self.lookahead.finish(&job);
            if ran.is_err() {
                self.log
                    .record("lookahead", 0, self.lm_state_name(), Some("internal"));
            }
        }
    }

    fn run_lookahead(&self, job: &LookJob) {
        let started = Instant::now();
        let count = lock_mutex(&job.conv).layout().spans.len();
        for seg in 1..count {
            if job.cancel.load(Ordering::Acquire) {
                return;
            }
            let mut entry = lock_mutex(&job.conv);
            if entry.layout().rev != job.rev {
                return;
            }
            // 左の選択は、返した時点の 0 番（表示している表記）。殻が選び直す前に作るので
            let left = vec![ikari_core::SegmentChoice::Candidate(0); seg];
            let deadline = Instant::now() + self.lm_budget.max;
            if entry
                .segment_page(seg, 0, job.page_size, &left, Some(deadline))
                .is_err()
            {
                return;
            }
        }
        let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.log
            .record("lookahead_done", micros, self.lm_state_name(), None);
    }

    /// 再変換（ADR 0006 §9）。選択した表記から文節の列を作る。確定履歴は読みの推定にだけ使い（新しい順）、
    /// 左文脈は殻が送った周辺テキストだけにする。LM は待たない。secure では始めない（NFR-18）
    pub(crate) fn reconvert(&self, r: pb::Reconvert) -> Result<envelope::Body, Fail> {
        let started = Instant::now();
        if r.surface.is_empty() {
            return Err(Fail::bad("empty surface"));
        }
        if crate::char_count_exceeds(&r.surface, MAX_RECONVERT_CHARS) {
            return Err(Fail::bad("surface too long"));
        }
        if has_line_break(&r.surface) {
            return Err(Fail::bad("surface has a line break"));
        }
        if r.page_size > crate::MAX_PAGE_SIZE {
            return Err(Fail::bad("page_size out of range"));
        }
        let page_size = if r.page_size == 0 {
            crate::MAX_PAGE_SIZE
        } else {
            r.page_size
        } as usize;
        let (secure, mut history) = self.session_info(r.session_id)?;
        if secure {
            return Err(Fail::bad("reconvert is not available in a secure session"));
        }
        history.reverse();
        let stack = self.stack();
        let ctx = Self::build_context(&stack, false, Vec::new(), r.context.as_ref());
        let before = self.lm_stats();
        let seg = self
            .seam(&stack)
            .reconvert(&r.surface, &ctx, &history, Some(Instant::now()))
            .map_err(fail_of)?;
        let mut entry = SegEntry::new(seg);
        let (candidates, has_next) =
            entry.segment_page(0, 0, page_size, &[], Some(Instant::now()))?;
        let conversion_id = self.next_conversion.fetch_add(1, Ordering::Relaxed);
        let page = pb::SegmentPageReply {
            segment: 0,
            page_index: 0,
            candidates,
            has_next,
            lm_applied: self.lm_applied_since(before),
            engine_micros: crate::micros_u32(started),
        };
        let reply = layout_reply(
            conversion_id,
            entry.layout(),
            0,
            Some(page),
            crate::micros_u32(started),
        );
        crate::lock_mutex(&self.conversions).insert(
            conversion_id,
            r.session_id,
            Arc::new(Mutex::new(entry)),
            page_size,
        );
        Ok(envelope::Body::SegmentLayout(reply))
    }

    /// 全体の確定（ADR 0006 §6）。コアが返した学習の事象を順に記録し、確定履歴に積む。
    /// secure のセッションは何も記録しない（NFR-18）。再変換は履歴に積まず、何も変えていなければ学習もしない
    /// 返事と、ログの注記（Rewriter の候補を確定したら `rw=<名前>`。secure と変更なしの再変換では None）
    pub(crate) fn commit_segments(
        &self,
        c: pb::CommitSegments,
    ) -> Result<(envelope::Body, Option<String>), Fail> {
        let choices = core_choices(&c.choices)?;
        let (conv, _, session) = crate::lock_mutex(&self.conversions)
            .get(c.conversion_id)
            .ok_or(crate::UNKNOWN_CONVERSION)?;
        let (commit, origin) = {
            let entry = crate::lock_mutex(&conv);
            check_rev(&entry, c.layout_rev)?;
            let origin = entry.origin();
            if entry.is_untouched_reconversion(&choices) {
                (None, origin)
            } else {
                (Some(entry.commit(&choices).map_err(fail_of)?), origin)
            }
        };
        crate::lock_mutex(&self.conversions).remove(c.conversion_id);
        let secure = crate::lock_mutex(&self.sessions)
            .get(&session)
            .is_none_or(|s| s.secure);
        let Some(commit) = commit.filter(|_| !secure) else {
            return Ok((envelope::Body::Ack(pb::Ack {}), None));
        };
        let note = crate::diag::rewriter_note(commit.events.iter().map(|e| &e.candidate.origin));
        if !commit.events.is_empty() {
            let stack = self.stack();
            if let Some(store) = &stack.learning {
                for event in &commit.events {
                    store.record(
                        event,
                        stack.vectors.as_deref(),
                        ikari_core::user::today_unix_day(),
                    );
                }
                crate::lock_mutex(&self.save).dirty = true;
            }
        }
        let surface: String = commit
            .events
            .iter()
            .map(|e| e.candidate.surface.as_str())
            .collect();
        crate::lock_mutex(&self.ctx_trace).note_commit(session, &surface);
        if origin == SegmentedOrigin::Convert {
            for item in commit.history {
                self.push_history(session, item.reading, item.surface);
            }
        }
        Ok((envelope::Body::Ack(pb::Ack {}), note))
    }

    /// 区切り直し（ADR 0006 §4）。LM を待たずに計算する（§16.1-6）。返事は新しい区切りと、
    /// フォーカスしている文節の 1 ページ目
    pub(crate) fn resize_segment(&self, r: pb::ResizeSegment) -> Result<envelope::Body, Fail> {
        let started = Instant::now();
        if usize::try_from(r.delta.unsigned_abs()).unwrap_or(usize::MAX) > crate::MAX_READING_CHARS
        {
            return Err(Fail::bad("delta out of range"));
        }
        let left = core_choices(&r.left)?;
        let segment = usize::try_from(r.segment).unwrap_or(usize::MAX);
        let (conv, page_size, _) = crate::lock_mutex(&self.conversions)
            .get(r.conversion_id)
            .ok_or(crate::UNKNOWN_CONVERSION)?;
        let before = self.lm_stats();
        let mut entry = crate::lock_mutex(&conv);
        check_rev(&entry, r.layout_rev)?;
        if segment >= entry.layout().spans.len() {
            return Err(Fail::bad("segment out of range"));
        }
        entry.resize(segment, r.delta, &left, Some(Instant::now()))?;
        let (candidates, has_next) =
            entry.segment_page(segment, 0, page_size, &left, Some(Instant::now()))?;
        let page = pb::SegmentPageReply {
            segment: r.segment,
            page_index: 0,
            candidates,
            has_next,
            lm_applied: self.lm_applied_since(before),
            engine_micros: crate::micros_u32(started),
        };
        Ok(envelope::Body::SegmentLayout(layout_reply(
            r.conversion_id,
            entry.layout(),
            segment,
            Some(page),
            crate::micros_u32(started),
        )))
    }

    /// 文節のページ（ADR 0006 §3.4）。作ってあれば（先読み、または前に返した）それを返し、
    /// ないときは、1 ページ目は LM を待たずに作る。2 ページ目以降は届いている LM の結果を使ってよい
    pub(crate) fn segment_page(
        &self,
        r: pb::SegmentPageRequest,
    ) -> Result<pb::envelope::Body, Fail> {
        let started = Instant::now();
        let index = usize::try_from(r.index).unwrap_or(usize::MAX);
        if index > crate::MAX_PAGE_INDEX {
            return Err(Fail::bad("page index out of range"));
        }
        let left = core_choices(&r.left)?;
        let segment = usize::try_from(r.segment).unwrap_or(usize::MAX);
        let (conv, page_size, _) = crate::lock_mutex(&self.conversions)
            .get(r.conversion_id)
            .ok_or(crate::UNKNOWN_CONVERSION)?;
        let before = self.lm_stats();
        let mut entry = crate::lock_mutex(&conv);
        check_rev(&entry, r.layout_rev)?;
        let span = entry
            .layout()
            .spans
            .get(segment)
            .ok_or(Fail::bad("segment out of range"))?;
        let deadline = if index == 0 {
            Instant::now()
        } else {
            Instant::now() + self.lm_budget.for_reading(&span.reading)
        };
        let (candidates, has_next) =
            entry.segment_page(segment, index, page_size, &left, Some(deadline))?;
        Ok(envelope::Body::SegmentPageReply(pb::SegmentPageReply {
            segment: r.segment,
            page_index: r.index,
            candidates,
            has_next,
            lm_applied: self.lm_applied_since(before),
            engine_micros: crate::micros_u32(started),
        }))
    }
}
