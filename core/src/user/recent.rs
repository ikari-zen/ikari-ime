//! 同じ読みで直前に選んだ表記（ADR 0045）。メモリだけに持ち、保存しない。
//!
//! 読みごとに最後の 1 表記と選んだ時刻だけを覚える。加点の形と窓は ranker の設定
//! （`learning.recent_bonus`、`learning.recent_window_secs`）が決める。ここは事実の記録だけ。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Instant;

/// 覚える読みの数の上限。窓は ranker の設定でここからは分からないので、時間ではなく数で切る。
/// 窓は分単位で、その間に打つ読みの種類は数百を超えない。余裕を見て大きくとり、順位には影響しない
/// （超えたときは古い方の半分を捨てる。捨てるのは窓が過ぎている可能性の高いもの）。
const MAX_REMEMBERED_READINGS: usize = 4096;

/// 経過時間の時計（単調なミリ秒）
#[derive(Clone, Debug)]
pub enum RecentClock {
    /// プロセスの中で最初に時計を使ってからの経過（`Instant`）
    System,
    /// テストとシナリオ用。呼び出し側が進める
    Manual(Arc<AtomicU64>),
}

impl RecentClock {
    fn now_ms(&self) -> u64 {
        match self {
            Self::System => {
                static START: OnceLock<Instant> = OnceLock::new();
                let start = START.get_or_init(Instant::now);
                u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
            }
            Self::Manual(ms) => ms.load(Ordering::Relaxed),
        }
    }
}

/// 読み -> (表記のキー, 選んだ時刻 ms)
pub struct RecentChoices {
    clock: RecentClock,
    map: Mutex<HashMap<String, (String, u64)>>,
}

impl RecentChoices {
    pub fn new(clock: RecentClock) -> Self {
        Self {
            clock,
            map: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, (String, u64)>> {
        self.map.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 読みの最後の選択を `surface` に置き換える。
    pub fn note(&self, reading: &str, surface: &str) {
        let now = self.clock.now_ms();
        let mut map = self.lock();
        if map.len() >= MAX_REMEMBERED_READINGS && !map.contains_key(reading) {
            let mut times: Vec<u64> = map.values().map(|(_, t)| *t).collect();
            times.sort_unstable();
            let cutoff = times[times.len() / 2];
            map.retain(|_, (_, t)| *t > cutoff);
        }
        map.insert(reading.to_string(), (surface.to_string(), now));
    }

    /// 読みの最後の選択と、選んでからの経過ミリ秒
    pub fn latest(&self, reading: &str) -> Option<(String, u64)> {
        let now = self.clock.now_ms();
        self.lock()
            .get(reading)
            .map(|(s, t)| (s.clone(), now.saturating_sub(*t)))
    }

    pub fn clear(&self) {
        self.lock().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manual() -> (RecentChoices, Arc<AtomicU64>) {
        let ms = Arc::new(AtomicU64::new(0));
        (RecentChoices::new(RecentClock::Manual(ms.clone())), ms)
    }

    #[test]
    fn keeps_only_last_surface_per_reading() {
        let (r, ms) = manual();
        r.note("ひんしを", "品詞を");
        ms.store(1500, Ordering::Relaxed);
        r.note("ひんしを", "瀕死を");
        ms.store(2000, Ordering::Relaxed);
        assert_eq!(r.latest("ひんしを"), Some(("瀕死を".to_string(), 500)));
    }

    #[test]
    fn other_reading_is_none() {
        let (r, _) = manual();
        r.note("ひんしを", "品詞を");
        assert_eq!(r.latest("ひんし"), None);
    }

    #[test]
    fn clear_forgets() {
        let (r, _) = manual();
        r.note("ひんしを", "品詞を");
        r.clear();
        assert_eq!(r.latest("ひんしを"), None);
    }

    #[test]
    fn bounded_memory() {
        let (r, ms) = manual();
        for i in 0..(MAX_REMEMBERED_READINGS * 2) {
            ms.store(i as u64, Ordering::Relaxed);
            r.note(&format!("r{i}"), "x");
        }
        assert!(r.lock().len() <= MAX_REMEMBERED_READINGS);
        let last = MAX_REMEMBERED_READINGS * 2 - 1;
        assert!(r.latest(&format!("r{last}")).is_some());
    }
}
