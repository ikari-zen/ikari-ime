//! 裏のスレッド（先行計算、保存の掃除）の本体と停止。

use ikari_stack::{LmDevice, lm_trace};

use super::*;
use crate::convert::device_word;

impl Inner {
    pub(crate) fn stop(&self) {
        *lock_mutex(&self.stop.0) = true;
        self.stop.1.notify_all();
        self.queue.close();
        self.lookahead.close();
    }

    pub(crate) fn prefetch_loop(&self) {
        while let Some(job) = self.queue.next() {
            let entry = Arc::clone(&job.entry);
            let ran = catch_unwind(AssertUnwindSafe(|| self.run_job(job)));
            self.queue.finish();
            if ran.is_err() {
                entry.abandon();
            }
        }
    }

    /// 先行計算 1 件。`convert_until` と最初のページを、変換の上限の締め切りまで LM を待って作る。
    pub(crate) fn run_job(&self, job: Job) {
        let Job { entry, context, .. } = job;
        if !entry.is_computing() {
            return;
        }
        let started = Instant::now();
        let stack = self.stack();
        let before = self.lm_stats();
        lm_trace::begin();
        let deadline = started + self.lm_budget.max;
        let mut conv = stack
            .converter
            .convert_until(&entry.key.reading, &context, Some(deadline));
        let _ = conv.page_until(0, Some(deadline));
        let mut seg = SegEntry::new(self.seam(&stack).segment(conv));
        // 2 文節以上なら、文節 0 の 1 ページ目まで（同じ締め切りで）作っておく（ADR 0006 §5.3）
        seg.prefill(0, MAX_PAGE_SIZE as usize, deadline);
        let trace = lm_trace::take();
        let applied = self.lm_applied_since(before);
        let calls = self.lm_calls_since(before);
        entry.set_ready(Arc::new(Mutex::new(seg)), applied, trace);
        let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        // 固定の語と数だけ（読み・候補は書かない。NFR-19）
        let device = match trace.device {
            LmDevice::Unknown => self.lm_device(),
            d => d,
        };
        let note = format!(
            "lm_applied={} lm_calls={calls} lm_us={} lm_infer_us={} lm_deadline_us={} lm_dev={} lm_to={}",
            u8::from(applied),
            trace.wait_micros,
            trace.infer_micros,
            self.lm_budget.max.as_micros(),
            device_word(device),
            trace.timeouts
        );
        self.log.record_note(
            "prefetch_done",
            micros,
            self.lm_state_name(),
            None,
            Some(&note),
        );
    }

    pub(crate) fn janitor_loop(&self) {
        loop {
            let stopped = lock_mutex(&self.stop.0);
            if *stopped {
                return;
            }
            let (stopped, _) = self
                .stop
                .1
                .wait_timeout(stopped, JANITOR_TICK)
                .unwrap_or_else(|e| e.into_inner());
            if *stopped {
                return;
            }
            drop(stopped);
            self.tick();
        }
    }

    pub(crate) fn tick(&self) {
        self.refresh_data();
        let now = Instant::now();
        self.tick_diag(now);
        lock_mutex(&self.cache).expire(now);
        lock_mutex(&self.conversions).expire(now);
        self.expire_context(now);
        let due = {
            let state = lock_mutex(&self.save);
            state.dirty && state.last_save.elapsed() >= SAVE_INTERVAL
        };
        if due {
            self.save_now();
        }
    }
}
