//! 推論の取り消しの印（ADR 0074 §3）。
//!
//! - [`CancelToken`]: 共有のフラグ。立てるのは 1 回だけ。立ったときに呼ぶものを 1 つ登録できる
//!   （別プロセスの子に `Cancel` を送り、待っている受け口を起こす。一定の間隔で見に行かないため）。
//! - [`scope`]: 呼んだスレッドに印を結びつける。`ZenzRanker::rank` が [`current`] で読み、要求に付ける。
//!   `lm_trace` と同じ形（スレッドの範囲）。範囲の外（Convert、評価、CLI）では印が無い = 止めない。
//!
//! 何を止めるかを決めるのはエンジン。`lm` は印を見て止めるだけ。

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

type Hook = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct Slot {
    /// 登録の番号（外すときに、自分の登録だけを外すため）
    generation: u64,
    hook: Option<Hook>,
}

#[derive(Default)]
struct Inner {
    cancelled: AtomicBool,
    slot: Mutex<Slot>,
}

/// 取り消しの印。複製は同じ印を指す。
#[derive(Clone, Default)]
pub struct CancelToken {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancelToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

fn lock(m: &Mutex<Slot>) -> MutexGuard<'_, Slot> {
    // 中身は単純な値なので、パニックのあとでもそのまま使う
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl CancelToken {
    /// 立っていない印
    pub fn new() -> CancelToken {
        CancelToken::default()
    }

    /// 印を立てる。初めて立てたときだけ、登録されたものを呼ぶ（ロックの外で）。
    pub fn cancel(&self) {
        let hook = {
            let mut slot = lock(&self.inner.slot);
            if self.inner.cancelled.swap(true, Ordering::AcqRel) {
                return;
            }
            slot.hook.take()
        };
        if let Some(hook) = hook {
            hook();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    /// 立ったときに呼ぶものを登録する。返したものを落とすと外れる。前の登録は置き換える。
    /// すでに立っていれば、その場で呼ぶ。
    pub fn on_cancel(&self, hook: impl FnOnce() + Send + 'static) -> CancelHook<'_> {
        let mut slot = lock(&self.inner.slot);
        slot.generation = slot.generation.wrapping_add(1);
        let generation = slot.generation;
        if self.is_cancelled() {
            drop(slot);
            hook();
        } else {
            slot.hook = Some(Box::new(hook));
        }
        CancelHook {
            token: self,
            generation,
        }
    }
}

/// [`CancelToken::on_cancel`] の登録。落とすと外れる。
pub struct CancelHook<'a> {
    token: &'a CancelToken,
    generation: u64,
}

impl Drop for CancelHook<'_> {
    fn drop(&mut self) {
        let mut slot = lock(&self.token.inner.slot);
        if slot.generation == self.generation {
            slot.hook = None;
        }
    }
}

thread_local! {
    static CURRENT: RefCell<Option<CancelToken>> = const { RefCell::new(None) };
}

/// このスレッドに印を結びつける。返したものを落とすと、前の状態に戻る。
pub fn scope(token: CancelToken) -> Scope {
    let previous = CURRENT.with(|c| c.replace(Some(token)));
    Scope { previous }
}

/// このスレッドに結びついた印（範囲の外なら None）
pub fn current() -> Option<CancelToken> {
    CURRENT.with(|c| c.borrow().clone())
}

/// [`scope`] の範囲。落とすと前の状態に戻る。
pub struct Scope {
    previous: Option<CancelToken>,
}

impl Drop for Scope {
    fn drop(&mut self) {
        let previous = self.previous.take();
        CURRENT.with(|c| *c.borrow_mut() = previous);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    #[test]
    fn cancel_is_sticky_and_calls_the_hook_once() {
        let t = CancelToken::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        let _hook = t.on_cancel(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        assert!(!t.is_cancelled());
        t.clone().cancel();
        t.cancel();
        assert!(t.is_cancelled());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_dropped_hook_is_not_called_and_a_late_hook_runs_at_once() {
        let t = CancelToken::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        drop(t.on_cancel(move || {
            c.fetch_add(1, Ordering::SeqCst);
        }));
        t.cancel();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let c = Arc::clone(&calls);
        let _hook = t.on_cancel(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn scope_is_per_thread_and_restores() {
        assert!(current().is_none());
        let t = CancelToken::new();
        {
            let _s = scope(t.clone());
            t.cancel();
            assert!(current().is_some_and(|c| c.is_cancelled()));
            std::thread::spawn(|| assert!(current().is_none()))
                .join()
                .unwrap();
        }
        assert!(current().is_none());
    }
}
