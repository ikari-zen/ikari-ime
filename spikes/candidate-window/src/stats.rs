//! 中央値と p99（標本が少ないときの p99 は最大値に近づく。参考値）。

pub fn summarize(label: &str, samples_ms: &mut [f64]) {
    if samples_ms.is_empty() {
        println!("{label}: n=0");
        return;
    }
    samples_ms.sort_by(|a, b| a.total_cmp(b));
    let n = samples_ms.len();
    let median = samples_ms[n / 2];
    let p99 = samples_ms[((n as f64 * 0.99).ceil() as usize).clamp(1, n) - 1];
    println!(
        "RESULT {label}: n={n} median={median:.2}ms p99={p99:.2}ms max={:.2}ms",
        samples_ms[n - 1]
    );
}
