//! 中継の同時数を数える（今の Swift の `IkariIME/BridgeServer.swift` の `RelayGate`）。満員なら待たずに断る
//! （連打でキューとメモリを溜めない）。

use std::sync::{Arc, Mutex, PoisonError};

/// 同時に走る中継の数の上限。暫定値（Swift で掃引なしで決めた値をそのまま写した）
pub const MAX_CONCURRENT_RELAYS: usize = 4;
/// この大きさ以上の要求は「大きな要求」として、同時に `MAX_CONCURRENT_LARGE_RELAYS` 本までにする（暫定値）
pub const LARGE_RELAY_BYTES: usize = 1024 * 1024;
pub const MAX_CONCURRENT_LARGE_RELAYS: usize = 1;

#[derive(Debug, Default)]
struct Counts {
    running: usize,
    running_large: usize,
}

#[derive(Debug)]
pub struct RelayGate {
    max_concurrent: usize,
    max_large: usize,
    counts: Mutex<Counts>,
}

impl Default for RelayGate {
    fn default() -> Self {
        Self::new(MAX_CONCURRENT_RELAYS, MAX_CONCURRENT_LARGE_RELAYS)
    }
}

impl RelayGate {
    pub fn new(max_concurrent: usize, max_large: usize) -> Self {
        Self {
            max_concurrent,
            max_large,
            counts: Mutex::new(Counts::default()),
        }
    }

    /// 枠を取る。満員なら `None`（待たない）。返した `RelayPermit` を落とすと枠が空く
    pub fn try_enter(self: &Arc<Self>, large: bool) -> Option<RelayPermit> {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        if counts.running >= self.max_concurrent
            || (large && counts.running_large >= self.max_large)
        {
            return None;
        }
        counts.running += 1;
        if large {
            counts.running_large += 1;
        }
        Some(RelayPermit {
            gate: Arc::clone(self),
            large,
        })
    }

    fn leave(&self, large: bool) {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        counts.running = counts.running.saturating_sub(1);
        if large {
            counts.running_large = counts.running_large.saturating_sub(1);
        }
    }
}

/// 中継 1 本の枠。落とすと空く（中継を別のスレッドで走らせるときは、そのスレッドへ移す）
#[derive(Debug)]
pub struct RelayPermit {
    gate: Arc<RelayGate>,
    large: bool,
}

impl RelayPermit {
    pub fn is_large(&self) -> bool {
        self.large
    }
}

impl Drop for RelayPermit {
    fn drop(&mut self) {
        self.gate.leave(self.large);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_large_requests_to_one() {
        let gate = Arc::new(RelayGate::new(4, 1));
        let large = gate.try_enter(true).unwrap();
        assert!(large.is_large());
        assert!(gate.try_enter(true).is_none());
        let _small = gate.try_enter(false).unwrap();
        drop(large);
        assert!(gate.try_enter(true).is_some());
    }

    #[test]
    fn refuses_beyond_the_limit_and_accepts_again_after_release() {
        let gate = Arc::new(RelayGate::default());
        let held: Vec<_> = (0..MAX_CONCURRENT_RELAYS)
            .map(|_| gate.try_enter(false).unwrap())
            .collect();
        for _ in 0..20 {
            assert!(gate.try_enter(false).is_none());
        }
        drop(held);
        assert!(gate.try_enter(false).is_some());
    }
}
