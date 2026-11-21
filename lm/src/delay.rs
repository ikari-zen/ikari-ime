//! 検証用の遅延の包み（ADR 0065 §3.4）。**feature `dev-knobs` のときだけ**あり、製品のビルドには入らない。
//!
//! 採点の結果を返す前に待ち、遅い機械を真似る。待ちの形は 2 つ（足し合わせる）:
//! - 倍率 `scale`（`k`）: 実際にかかった時間の `k − 1` 倍を足す（長い入力ほど遅い、を真似る）
//! - 固定 `extra`（`d`）: 1 回の採点ごとに固定の時間を足す
//!
//! 指定は `--ranker-config` の `zenz.debug_infer_scale` と `zenz.debug_infer_extra_ms`（[`ZenzConfig::from_kv`]）。
//! 製品のエンジンは `zenz.*` を設定ファイルから読まず、この feature も付けないので、利用者の設定からは届かない。
//!
//! 包みは、別プロセスの子の中ではなく、親の側（[`LmLoader`] が返す scorer）に置く。往復を含めて遅くなり、
//! 子との取り決めは変えない。
//!
//! 足した待ちは [`DELAY_STEPS`] 個の区切りに分け、区切りの間で取り消しの印（ADR 0074）を見る。
//! GPU の推論は decode の最中には止められず、止めどころは decode の間だけ（ADR 0074 §8.1: プロンプト 1 回 +
//! 候補のバッチ 1 回）なので、それに合わせる。印が立たない評価（取り消しを切った比較）では、全部待つ。

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::cancel::CancelToken;
use crate::error::LmError;
use crate::scorer::{LmDevice, LmLoader, LmScorer};

/// 足した待ちを分ける区切りの数。decode の数（プロンプト 1 回 + 候補のバッチ 1 回。ADR 0074 §8.1）に合わせる
pub const DELAY_STEPS: u32 = 2;

/// 倍率の下限（1 は遅くしない）
pub const MIN_SCALE: f32 = 1.0;

/// 人工の遅延の指定
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DebugInfer {
    /// 倍率 `k`（`MIN_SCALE` 以上。実際にかかった時間の `k − 1` 倍を足す）
    pub scale: f32,
    /// 固定の追加（1 回の採点ごと）
    pub extra: Duration,
}

impl Default for DebugInfer {
    /// 足さない
    fn default() -> Self {
        DebugInfer {
            scale: MIN_SCALE,
            extra: Duration::ZERO,
        }
    }
}

impl DebugInfer {
    /// 実際にかかった時間 `actual` に足す時間
    pub fn added(&self, actual: Duration) -> Duration {
        actual.mul_f32(self.scale - MIN_SCALE) + self.extra
    }

    /// 何も足さないか
    pub fn is_noop(&self) -> bool {
        self.scale <= MIN_SCALE && self.extra.is_zero()
    }
}

/// 採点の前後で待つ scorer の包み
pub struct DelayedScorer {
    inner: Box<dyn LmScorer>,
    delay: DebugInfer,
}

impl DelayedScorer {
    pub fn new(inner: Box<dyn LmScorer>, delay: DebugInfer) -> Self {
        DelayedScorer { inner, delay }
    }

    /// `loader` が返す scorer を包む loader
    pub fn wrap_loader(loader: LmLoader, delay: DebugInfer) -> LmLoader {
        if delay.is_noop() {
            return loader;
        }
        Arc::new(move || {
            let inner = loader()?;
            Ok(Box::new(DelayedScorer::new(inner, delay)) as Box<dyn LmScorer>)
        })
    }
}

impl LmScorer for DelayedScorer {
    fn nll(
        &mut self,
        prompt: &str,
        candidates: &[String],
        cancel: &CancelToken,
    ) -> Result<Vec<f32>, LmError> {
        let started = Instant::now();
        let scores = self.inner.nll(prompt, candidates, cancel)?;
        let added = self.delay.added(started.elapsed());
        let step = added / DELAY_STEPS;
        for _ in 0..DELAY_STEPS {
            if cancel.is_cancelled() {
                return Err(LmError::Cancelled);
            }
            std::thread::sleep(step);
        }
        Ok(scores)
    }

    fn device(&self) -> LmDevice {
        self.inner.device()
    }

    fn alive(&self) -> bool {
        self.inner.alive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 決まった時間かかって、決まった値を返す偽の scorer
    struct Fixed(Duration);

    impl LmScorer for Fixed {
        fn nll(
            &mut self,
            _prompt: &str,
            candidates: &[String],
            _cancel: &CancelToken,
        ) -> Result<Vec<f32>, LmError> {
            std::thread::sleep(self.0);
            Ok(vec![1.0; candidates.len()])
        }
    }

    fn cands() -> Vec<String> {
        vec!["a".to_owned(), "b".to_owned()]
    }

    #[test]
    fn added_is_k_minus_one_times_actual_plus_extra() {
        let d = DebugInfer {
            scale: 3.0,
            extra: Duration::from_millis(5),
        };
        assert_eq!(
            d.added(Duration::from_millis(10)),
            Duration::from_millis(25)
        );
        assert_eq!(
            DebugInfer::default().added(Duration::from_secs(1)),
            Duration::ZERO
        );
        assert!(DebugInfer::default().is_noop());
    }

    #[test]
    fn the_wrapper_waits_at_least_k_times_the_actual_time() {
        let actual = Duration::from_millis(20);
        let mut s = DelayedScorer::new(
            Box::new(Fixed(actual)),
            DebugInfer {
                scale: 3.0,
                extra: Duration::ZERO,
            },
        );
        let started = Instant::now();
        let out = s.nll("p", &cands(), &CancelToken::new()).unwrap();
        // sleep は短くなることがない。足りない側にだけ倒した下限（実測の 3 倍）を確かめる
        assert!(started.elapsed() >= actual * 3);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn the_wrapper_adds_the_fixed_extra() {
        let mut s = DelayedScorer::new(
            Box::new(Fixed(Duration::ZERO)),
            DebugInfer {
                scale: MIN_SCALE,
                extra: Duration::from_millis(30),
            },
        );
        let started = Instant::now();
        s.nll("p", &cands(), &CancelToken::new()).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(30));
    }

    #[test]
    fn a_cancelled_token_stops_the_wait_at_a_step_boundary() {
        let mut s = DelayedScorer::new(
            Box::new(Fixed(Duration::ZERO)),
            DebugInfer {
                scale: MIN_SCALE,
                extra: Duration::from_secs(60),
            },
        );
        let token = CancelToken::new();
        token.cancel();
        let started = Instant::now();
        assert_eq!(s.nll("p", &cands(), &token), Err(LmError::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_noop_setting_returns_the_loader_unchanged() {
        let loader: LmLoader =
            Arc::new(|| Ok(Box::new(Fixed(Duration::ZERO)) as Box<dyn LmScorer>));
        let wrapped = DelayedScorer::wrap_loader(Arc::clone(&loader), DebugInfer::default());
        assert!(Arc::ptr_eq(&loader, &wrapped));
    }
}
