//! 時間に依存するテストの時間の倍率。
//!
//! 遅い CI のランナーでは、「8ms の計算が 30ms の待ちに間に合う」のようなテストが揺れる。
//! 環境変数 `IKARI_TEST_TIME_SCALE`（正の有限の数。既定 1）で、計算の長さ、待ち時間、
//! 上限と下限の値を同じ比率で伸ばす。不正な値（0、負、数でない、無限）は 1 として扱う。
//! 倍率が 1 のときは、渡した値をそのまま返す。

use std::sync::OnceLock;
use std::time::Duration;

/// 倍率を指定する環境変数
pub const TIME_SCALE_ENV: &str = "IKARI_TEST_TIME_SCALE";

const DEFAULT_SCALE: f64 = 1.0;

fn parse_scale(raw: Option<&str>) -> f64 {
    raw.and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(DEFAULT_SCALE)
}

/// 倍率（プロセスの中で 1 回だけ読む）
pub fn scale() -> f64 {
    static SCALE: OnceLock<f64> = OnceLock::new();
    *SCALE.get_or_init(|| parse_scale(std::env::var(TIME_SCALE_ENV).ok().as_deref()))
}

fn scale_by(d: Duration, factor: f64) -> Duration {
    if factor == DEFAULT_SCALE {
        return d;
    }
    Duration::try_from_secs_f64(d.as_secs_f64() * factor).unwrap_or(Duration::MAX)
}

/// `d` に倍率を掛ける
pub fn scaled(d: Duration) -> Duration {
    scale_by(d, scale())
}

/// `n` ミリ秒に倍率を掛ける
pub fn ms(n: u64) -> Duration {
    scaled(Duration::from_millis(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_values_fall_back_to_one() {
        for raw in [
            None,
            Some(""),
            Some("abc"),
            Some("0"),
            Some("-2"),
            Some("inf"),
            Some("NaN"),
        ] {
            assert_eq!(parse_scale(raw), 1.0, "{raw:?}");
        }
        assert_eq!(parse_scale(Some(" 2.5 ")), 2.5);
    }

    #[test]
    fn scaling_keeps_the_ratio_and_one_is_identity() {
        let d = Duration::from_millis(30);
        assert_eq!(scale_by(d, 1.0), d);
        assert_eq!(scale_by(d, 5.0), Duration::from_millis(150));
        assert_eq!(scale_by(Duration::ZERO, 5.0), Duration::ZERO);
    }
}
