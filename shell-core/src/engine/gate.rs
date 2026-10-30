//! エンジン（Rust）の寿命の門（#51。今の Swift の `IkariEngineService/EngineGate.swift`）。
//!
//! 使う側は `enter` で処理中の数を増やしてから資源を使い、終わったら `leave` で減らす。`close` は closed を
//! 立てて新しい `enter` を断り、処理中の数が 0 になるのを待ってから資源を返す（呼び手が解放する）。
//! 待ちが上限を超えたら資源を返さずに残す。解放後の使用（UAF）より、リークのほうが安全なため。

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// `EngineGate::close` の結果
#[derive(Debug, PartialEq, Eq)]
pub enum GateClose<R> {
    /// 処理中がなくなった。呼び手が資源を解放する
    Release(R),
    /// 処理中が上限の時間までに終わらなかった。解放せずに残す（リーク）
    Leaked,
    /// 解放する資源がない（開いていない、すでに閉じた）
    Nothing,
}

struct GateState<R> {
    resource: Option<R>,
    in_flight: usize,
    closed: bool,
}

/// 資源 `R` は、使う側に複製して渡す（`Arc` やポインタの包みを想定）。
pub struct EngineGate<R> {
    state: Mutex<GateState<R>>,
    idle: Condvar,
}

impl<R> Default for EngineGate<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R> EngineGate<R> {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(GateState {
                resource: None,
                in_flight: 0,
                closed: false,
            }),
            idle: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, GateState<R>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `enter` が資源を返したときにだけ、1 回呼ぶ（多く呼んでも数は 0 より下がらない）
    pub fn leave(&self) {
        let mut state = self.lock();
        debug_assert!(state.in_flight > 0, "EngineGate::leave without enter");
        state.in_flight = state.in_flight.saturating_sub(1);
        if state.in_flight == 0 {
            self.idle.notify_all();
        }
    }

    /// closed を立て、処理中が 0 になるのを `timeout` まで待つ。すでに解放を返したあとは `Nothing`。
    /// `Leaked` のあとに呼び直せば、そのときに処理中がなければ解放を返す
    pub fn close(&self, timeout: Duration) -> GateClose<R> {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock();
        state.closed = true;
        while state.in_flight > 0 {
            // 偽の起床もあるので、時刻で判断する
            let wait = deadline.saturating_duration_since(Instant::now());
            if wait.is_zero() {
                return GateClose::Leaked;
            }
            state = self
                .idle
                .wait_timeout(state, wait)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        match state.resource.take() {
            Some(resource) => GateClose::Release(resource),
            None => GateClose::Nothing,
        }
    }

    pub fn is_closed(&self) -> bool {
        self.lock().closed
    }
}

impl<R: Clone> EngineGate<R> {
    /// 処理中の数を増やして資源を返す。資源がなければ `open` で開く（ロックの中で呼ぶ）。
    /// closed のあと、または開けなかったときは `None` を返し、数は増やさない
    pub fn enter(&self, open: impl FnOnce() -> Option<R>) -> Option<R> {
        let mut state = self.lock();
        if state.closed {
            return None;
        }
        if state.resource.is_none() {
            state.resource = open();
        }
        let resource = state.resource.clone()?;
        state.in_flight += 1;
        Some(resource)
    }

    /// `enter` から `leave` までを `body` の前後で行う。断られたときは `None`
    pub fn with_resource<T>(
        &self,
        open: impl FnOnce() -> Option<R>,
        body: impl FnOnce(R) -> T,
    ) -> Option<T> {
        struct Leave<'a, R>(&'a EngineGate<R>);
        impl<R> Drop for Leave<'_, R> {
            fn drop(&mut self) {
                self.0.leave();
            }
        }
        let resource = self.enter(open)?;
        let _leave = Leave(self);
        Some(body(resource))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread;

    /// 解放の記録つきの資源（エンジンの代わり）
    #[derive(Default)]
    struct FakeEngine {
        released: AtomicBool,
        used_after_release: AtomicBool,
    }

    impl FakeEngine {
        fn use_it(&self) {
            if self.released.load(Ordering::SeqCst) {
                self.used_after_release.store(true, Ordering::SeqCst);
            }
        }
        fn release(&self) {
            self.released.store(true, Ordering::SeqCst);
        }
    }

    const HOLD: Duration = Duration::from_millis(300);
    const LONG: Duration = Duration::from_secs(10);
    const SHORT: Duration = Duration::from_millis(50);

    fn opened_gate(engine: &Arc<FakeEngine>) -> Arc<EngineGate<Arc<FakeEngine>>> {
        let gate = Arc::new(EngineGate::new());
        if gate.enter(|| Some(engine.clone())).is_some() {
            gate.leave();
        }
        gate
    }

    #[test]
    fn close_waits_for_in_flight_request() {
        let engine = Arc::new(FakeEngine::default());
        let gate = opened_gate(&engine);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (finish_tx, finish_rx) = mpsc::channel::<()>();
        let worker_gate = gate.clone();
        let worker = thread::spawn(move || {
            worker_gate.with_resource(
                || None,
                |resource| {
                    entered_tx.send(()).unwrap();
                    finish_rx.recv().unwrap();
                    resource.use_it();
                },
            )
        });
        entered_rx.recv().unwrap();
        let started = Instant::now();
        let watcher_engine = engine.clone();
        let watcher = thread::spawn(move || {
            thread::sleep(HOLD);
            // 処理の終わりまで解放されていないこと
            assert!(!watcher_engine.released.load(Ordering::SeqCst));
            finish_tx.send(()).unwrap();
        });
        let GateClose::Release(closing) = gate.close(LONG) else {
            panic!("expected release");
        };
        closing.release();
        assert!(started.elapsed() >= HOLD.mul_f64(0.9));
        assert!(!engine.used_after_release.load(Ordering::SeqCst));
        watcher.join().unwrap();
        assert!(worker.join().unwrap().is_some());
    }

    #[test]
    fn request_after_close_is_rejected() {
        let engine = Arc::new(FakeEngine::default());
        let gate = opened_gate(&engine);
        assert!(matches!(gate.close(LONG), GateClose::Release(_)));
        let mut opened = false;
        let result = gate.with_resource(
            || {
                opened = true;
                Some(Arc::new(FakeEngine::default()))
            },
            |_| 1,
        );
        assert_eq!(result, None);
        assert!(!opened); // closed のあとは開き直さない
        assert!(gate.is_closed());
    }

    #[test]
    fn close_times_out_and_leaves_resource() {
        let engine = Arc::new(FakeEngine::default());
        let gate = opened_gate(&engine);
        assert!(gate.enter(|| None).is_some());
        // 処理中のまま上限を超えたら、解放せずに残す
        assert!(matches!(gate.close(SHORT), GateClose::Leaked));
        gate.leave();
        // 処理中がなくなれば、呼び直しで解放できる
        assert!(matches!(gate.close(SHORT), GateClose::Release(_)));
        assert!(matches!(gate.close(SHORT), GateClose::Nothing));
    }

    #[test]
    fn failed_open_does_not_count_as_in_flight() {
        let gate = EngineGate::<Arc<FakeEngine>>::new();
        assert!(gate.enter(|| None).is_none());
        assert!(matches!(gate.close(SHORT), GateClose::Nothing));
    }
}
