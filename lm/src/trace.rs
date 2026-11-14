//! スレッドごとの LM 計測の記録（ADR 0061 §4.1）。
//!
//! `ZenzRanker::rank` を呼んだスレッドの thread-local に足す。`begin` を呼ばないスレッドでも足すだけで、
//! だれも読まない（費用は `Instant::now()` 数回と `Cell` の書き込み）。計測は結果にも順位にも影響しない。

use std::cell::Cell;
use std::time::Duration;

use crate::scorer::LmDevice;

/// 呼び出したスレッドの、`rank` の記録（ADR 0061 §4.1）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LmTrace {
    /// 採点の対象があり締め切りのある `rank` の回数（`LmStats.calls` と同じ数え方）
    pub calls: u32,
    /// そのうち間に合わなかった回数
    pub timeouts: u32,
    /// 遅れて届いた結果を使った回数
    pub late_hits: u32,
    /// 返事を待った時間の合計（us）
    pub wait_micros: u64,
    /// 受け取った返事の、ワーカーでの推論の時間の合計（us）
    pub infer_micros: u64,
    /// 取り出したときの装置。モデルが読まれていなければ `Unknown`
    pub device: LmDevice,
}

thread_local! {
    static TRACE: Cell<LmTrace> = const {
        Cell::new(LmTrace {
            calls: 0,
            timeouts: 0,
            late_hits: 0,
            wait_micros: 0,
            infer_micros: 0,
            device: LmDevice::Unknown,
        })
    };
}

/// このスレッドの記録を 0 にする。
pub fn begin() {
    TRACE.with(|t| t.set(LmTrace::default()));
}

/// このスレッドの記録を取り出して 0 にする。
pub fn take() -> LmTrace {
    TRACE.with(|t| t.replace(LmTrace::default()))
}

fn update(f: impl FnOnce(&mut LmTrace)) {
    TRACE.with(|t| {
        let mut v = t.get();
        f(&mut v);
        t.set(v);
    });
}

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// 採点の対象があり締め切りのある `rank` の呼び出し（`LmStats.calls` と同じ）。
pub(crate) fn record_call(device: LmDevice) {
    update(|t| {
        t.calls = t.calls.saturating_add(1);
        t.device = device;
    });
}

pub(crate) fn record_timeout(waited: Duration) {
    update(|t| {
        t.timeouts = t.timeouts.saturating_add(1);
        t.wait_micros = t.wait_micros.saturating_add(micros(waited));
    });
}

pub(crate) fn record_reply(waited: Duration, infer: Duration) {
    update(|t| {
        t.wait_micros = t.wait_micros.saturating_add(micros(waited));
        t.infer_micros = t.infer_micros.saturating_add(micros(infer));
    });
}

pub(crate) fn record_late_hit(infer: Duration) {
    update(|t| {
        t.late_hits = t.late_hits.saturating_add(1);
        t.infer_micros = t.infer_micros.saturating_add(micros(infer));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_and_take_reset() {
        begin();
        record_call(LmDevice::Cpu);
        record_reply(Duration::from_micros(5), Duration::from_micros(3));
        let t = take();
        assert_eq!((t.calls, t.wait_micros, t.infer_micros), (1, 5, 3));
        assert_eq!(t.device, LmDevice::Cpu);
        assert_eq!(take(), LmTrace::default());
    }

    #[test]
    fn threads_are_separate() {
        begin();
        record_call(LmDevice::Cpu);
        std::thread::spawn(|| assert_eq!(take(), LmTrace::default()))
            .join()
            .unwrap();
        assert_eq!(take().calls, 1);
    }
}
