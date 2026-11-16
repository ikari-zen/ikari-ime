//! 時間の百分位と、順位の一致度（ADR 0075 §4.4）。

/// 百分位（最近順位法: 小さい順に並べた `ceil(p/100 × n)` 番目。`ikari-eval` の `lm_timing` と同じ）。空なら None
pub fn percentile(values: &[u64], p: f64) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    let rank = ((p / 100.0) * n as f64).ceil() as usize;
    Some(sorted[rank.clamp(1, n) - 1])
}

/// 中央値（偶数個なら真ん中の 2 つの平均）。空なら None
pub fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

/// NLL の最も小さい候補（1 位）。有限の値が無ければ None。同じ値なら先の候補
pub fn top1(nll: &[Option<f32>]) -> Option<usize> {
    nll.iter()
        .enumerate()
        .filter_map(|(i, v)| v.filter(|x| x.is_finite()).map(|x| (i, x)))
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
        .map(|(i, _)| i)
}

/// Kendall の τ-b。両方が有限の候補だけで数える。2 つ未満、または片方がすべて同じ値なら None
pub fn kendall_tau(a: &[Option<f32>], b: &[Option<f32>]) -> Option<f64> {
    let pairs: Vec<(f32, f32)> = a
        .iter()
        .zip(b)
        .filter_map(|(x, y)| match (x, y) {
            (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Some((*x, *y)),
            _ => None,
        })
        .collect();
    if pairs.len() < 2 {
        return None;
    }
    let (mut concordant, mut discordant, mut ties_a, mut ties_b) = (0i64, 0i64, 0i64, 0i64);
    for i in 0..pairs.len() {
        for j in (i + 1)..pairs.len() {
            let da = (pairs[i].0 - pairs[j].0).signum_or_zero();
            let db = (pairs[i].1 - pairs[j].1).signum_or_zero();
            match (da, db) {
                (0, 0) => {}
                (0, _) => ties_a += 1,
                (_, 0) => ties_b += 1,
                _ if da == db => concordant += 1,
                _ => discordant += 1,
            }
        }
    }
    let n0 = concordant + discordant;
    let denom = (((n0 + ties_a) * (n0 + ties_b)) as f64).sqrt();
    if denom == 0.0 {
        return None;
    }
    Some((concordant - discordant) as f64 / denom)
}

trait SignumOrZero {
    fn signum_or_zero(self) -> i8;
}

impl SignumOrZero for f32 {
    fn signum_or_zero(self) -> i8 {
        if self > 0.0 {
            1
        } else if self < 0.0 {
            -1
        } else {
            0
        }
    }
}

/// 基準と比べた一致度
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Agreement {
    /// 比べられた件数（両方に有限の値が 1 つ以上ある件）
    pub items: usize,
    /// 1 位が同じ件数
    pub top1_same: usize,
    /// τ の中央値（候補が 2 つ以上の件だけ）
    pub tau_median: Option<f64>,
    /// NLL の差（絶対値）の最大と中央値（両方が有限の候補）
    pub diff_max: Option<f64>,
    pub diff_median: Option<f64>,
}

impl Agreement {
    /// 1 位の一致の割合（0〜1）。比べられた件が無ければ None
    pub fn top1_rate(&self) -> Option<f64> {
        (self.items > 0).then(|| self.top1_same as f64 / self.items as f64)
    }
}

/// 件ごとの NLL（`base` と `other` は同じ件の並び）の一致度
pub fn agreement(base: &[Vec<Option<f32>>], other: &[Vec<Option<f32>>]) -> Agreement {
    let mut out = Agreement::default();
    let mut taus = Vec::new();
    let mut diffs = Vec::new();
    for (a, b) in base.iter().zip(other) {
        let (Some(ta), Some(tb)) = (top1(a), top1(b)) else {
            continue;
        };
        out.items += 1;
        if ta == tb {
            out.top1_same += 1;
        }
        if let Some(t) = kendall_tau(a, b) {
            taus.push(t);
        }
        for (x, y) in a.iter().zip(b) {
            if let (Some(x), Some(y)) = (x, y)
                && x.is_finite()
                && y.is_finite()
            {
                diffs.push(f64::from((x - y).abs()));
            }
        }
    }
    out.tau_median = median(&taus);
    out.diff_max = diffs.iter().copied().reduce(f64::max);
    out.diff_median = median(&diffs);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(xs: &[f32]) -> Vec<Option<f32>> {
        xs.iter().map(|&x| Some(x)).collect()
    }

    #[test]
    fn percentile_nearest_rank() {
        let xs: Vec<u64> = (1..=100).rev().collect();
        assert_eq!(percentile(&xs, 50.0), Some(50));
        assert_eq!(percentile(&xs, 99.0), Some(99));
        assert_eq!(percentile(&[7], 99.0), Some(7));
        assert_eq!(percentile(&[], 50.0), None);
    }

    #[test]
    fn median_even_and_odd() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), Some(2.5));
        assert_eq!(median(&[]), None);
    }

    #[test]
    fn top1_skips_non_finite() {
        assert_eq!(
            top1(&[Some(f32::INFINITY), Some(2.0), None, Some(1.0)]),
            Some(3)
        );
        assert_eq!(top1(&[None, Some(f32::INFINITY)]), None);
        assert_eq!(top1(&v(&[1.0, 1.0])), Some(0));
    }

    #[test]
    fn kendall_same_and_reversed() {
        let a = v(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(kendall_tau(&a, &v(&[10.0, 20.0, 30.0, 40.0])), Some(1.0));
        assert_eq!(kendall_tau(&a, &v(&[4.0, 3.0, 2.0, 1.0])), Some(-1.0));
        assert_eq!(kendall_tau(&a[..1], &v(&[1.0])), None);
        assert_eq!(kendall_tau(&a, &v(&[1.0, 1.0, 1.0, 1.0])), None);
    }

    #[test]
    fn agreement_counts_items_with_results() {
        let base = vec![v(&[1.0, 2.0]), v(&[2.0, 1.0]), v(&[1.0, 2.0])];
        let other = vec![v(&[1.5, 2.0]), v(&[1.0, 2.0]), vec![None, None]];
        let a = agreement(&base, &other);
        assert_eq!(a.items, 2);
        assert_eq!(a.top1_same, 1);
        assert_eq!(a.top1_rate(), Some(0.5));
        assert_eq!(a.tau_median, Some(0.0));
        assert_eq!(a.diff_max, Some(1.0));
        assert_eq!(a.diff_median, Some(0.75));
    }
}
