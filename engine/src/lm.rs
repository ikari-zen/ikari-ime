//! LM の状態の問い合わせと、モデルの暖機（別スレッド）。

use super::*;

impl Inner {
    pub(crate) fn lm_state(&self) -> LmState {
        let stack = self.stack();
        match &stack.lm {
            None if self.lm_requested => LmState::LmFailed,
            None => LmState::LmDisabled,
            Some(lm) if lm.is_loaded() => LmState::LmReady,
            Some(lm) if lm.stats().load_failures > 0 => LmState::LmFailed,
            Some(_) => LmState::LmLoading,
        }
    }

    pub(crate) fn lm_state_name(&self) -> &'static str {
        match self.lm_state() {
            LmState::LmReady => "ready",
            LmState::LmLoading => "loading",
            LmState::LmFailed => "failed",
            _ => "disabled",
        }
    }

    pub(crate) fn lm_stats(&self) -> Option<LmStats> {
        self.stack().lm.as_ref().map(|lm| lm.stats())
    }

    /// 前回の統計から、LM の採点が間に合って使われたか（calls が増え、timeouts は増えていない）。
    pub(crate) fn lm_applied_since(&self, before: Option<LmStats>) -> bool {
        match (before, self.lm_stats()) {
            (Some(b), Some(a)) => a.calls > b.calls && a.timeouts == b.timeouts,
            _ => false,
        }
    }

    /// 前回の統計から、LM の採点（`rank()`）が呼ばれた回数。LM がなければ 0。
    pub(crate) fn lm_calls_since(&self, before: Option<LmStats>) -> u64 {
        match (before, self.lm_stats()) {
            (Some(b), Some(a)) => a.calls.saturating_sub(b.calls),
            _ => 0,
        }
    }

    /// 再変換の読みの逆引き表を作り、辞書のページを読み込んでおく（別スレッド。ADR 0006 §9.3。
    /// どちらも約 100ms。後者がないと、最初の再変換がページフォールトで 1〜8ms かかる）
    pub(crate) fn warm_dictionary(&self) {
        let dict = Arc::clone(&self.stack().dict);
        let _ = std::thread::Builder::new()
            .name("ikari-dict-warm".to_owned())
            .spawn(move || {
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    dict.warm_reading_index();
                    dict.prefault();
                }));
            });
    }

    /// モデルが読み込まれていなければ、別スレッドで warm_up を始める（重ねて始めない）。
    pub(crate) fn ensure_warm(&self) {
        let stack = self.stack();
        let Some(lm) = stack.lm.clone() else {
            return;
        };
        if lm.is_loaded() || self.warming.swap(true, Ordering::AcqRel) {
            return;
        }
        let flag = Arc::clone(&self.warming);
        let log = Arc::clone(&self.log);
        let loaded_flag = self.diag.loaded_flag();
        let started = std::thread::Builder::new()
            .name("ikari-lm-warmup".to_owned())
            .spawn(move || {
                let began = Instant::now();
                let _ = catch_unwind(AssertUnwindSafe(|| lm.warm_up()));
                // 読み込み（またはその失敗）が終わった。所要を `lm_load` に書く（ADR 0034 §4.3）
                let loaded = lm.is_loaded();
                loaded_flag.store(loaded, Ordering::Release);
                Inner::lm_load_line(&log, began, loaded);
                flag.store(false, Ordering::Release);
            });
        if started.is_err() {
            self.warming.store(false, Ordering::Release);
        }
    }
}
