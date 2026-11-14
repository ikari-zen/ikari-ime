//! LM の 1 回の推論の所要の集計（スパイク。ADR 0059 §2.1 の NF-1・NF-3）。
//!
//! ワーカーが測った `LmScorer::nll` 1 回の時間（待ちを含まない）から、回数、平均、p50、p95、p99、最大を出す。
//! 暖機（`warm_up`）の採点は除き、別に出す。読み込みの時間も別に出す。

use std::time::Duration;

use ikari_stack::LmTimings;

/// 集計の結果（ミリ秒）
#[derive(Clone, Debug, PartialEq)]
pub struct InferenceSummary {
    pub count: usize,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

/// 百分位（最近順位法: 小さい順に並べた `ceil(p/100 × n)` 番目）。`sorted` は空でないこと
fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let n = sorted.len();
    let rank = ((p / 100.0) * n as f64).ceil() as usize;
    sorted[rank.clamp(1, n) - 1]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// 推論の時間を集計する。空なら None
pub fn summarize_inference(times: &[Duration]) -> Option<InferenceSummary> {
    let max = *times.iter().max()?;
    let mut sorted = times.to_vec();
    sorted.sort_unstable();
    let total: Duration = sorted.iter().sum();
    Some(InferenceSummary {
        count: sorted.len(),
        mean_ms: ms(total) / sorted.len() as f64,
        p50_ms: ms(percentile(&sorted, 50.0)),
        p95_ms: ms(percentile(&sorted, 95.0)),
        p99_ms: ms(percentile(&sorted, 99.0)),
        max_ms: ms(max),
    })
}

/// 標準エラーに出す 1 行。`skip` は先頭から除く採点の数（暖機の分）。`devices` は LM のデバイスの要約
pub fn render_lm_timings(t: &LmTimings, skip: usize, devices: Option<&str>) -> String {
    let warm = t.inference.get(skip..).unwrap_or(&[]);
    let list = |ds: &[Duration]| {
        ds.iter()
            .map(|d| format!("{:.1}", ms(*d)))
            .collect::<Vec<_>>()
            .join(",")
    };
    let warm_up = &t.inference[..skip.min(t.inference.len())];
    let head = match summarize_inference(warm) {
        Some(s) => format!(
            "lm inference: n={} mean={:.2}ms p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms",
            s.count, s.mean_ms, s.p50_ms, s.p95_ms, s.p99_ms, s.max_ms
        ),
        None => "lm inference: n=0".to_owned(),
    };
    format!(
        "{head} warm_up_ms=[{}] load_ms=[{}] devices={}",
        list(warm_up),
        list(&t.loads),
        devices.unwrap_or("unknown")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        let times: Vec<Duration> = (1..=100).rev().map(d).collect();
        let s = summarize_inference(&times).unwrap();
        assert_eq!(s.count, 100);
        assert!(close(s.mean_ms, 50.5));
        assert!(close(s.p50_ms, 50.0));
        assert!(close(s.p95_ms, 95.0));
        assert!(close(s.p99_ms, 99.0));
        assert!(close(s.max_ms, 100.0));
    }

    #[test]
    fn single_sample_is_every_percentile() {
        let s = summarize_inference(&[d(7)]).unwrap();
        assert!(close(s.p50_ms, 7.0));
        assert!(close(s.p99_ms, 7.0));
    }

    #[test]
    fn empty_is_none() {
        assert_eq!(summarize_inference(&[]), None);
    }

    #[test]
    fn render_skips_warm_up() {
        let t = LmTimings {
            inference: vec![d(500), d(10), d(20)],
            loads: vec![d(1200)],
        };
        let line = render_lm_timings(&t, 1, Some("cpu,vulkan:IntegratedGpu"));
        assert!(line.starts_with("lm inference: n=2 "), "{line}");
        assert!(line.contains("warm_up_ms=[500.0]"), "{line}");
        assert!(line.contains("load_ms=[1200.0]"), "{line}");
        assert!(line.ends_with("devices=cpu,vulkan:IntegratedGpu"), "{line}");
    }

    #[test]
    fn render_when_skip_exceeds_samples() {
        let t = LmTimings {
            inference: vec![d(5)],
            loads: vec![],
        };
        let line = render_lm_timings(&t, 3, None);
        assert!(line.starts_with("lm inference: n=0 "), "{line}");
        assert!(line.contains("warm_up_ms=[5.0]"), "{line}");
    }
}
