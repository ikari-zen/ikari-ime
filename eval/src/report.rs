//! 集計とレポート（Markdown と、ケース別の TSV）。

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Duration;

use crate::{CaseResult, Split};

/// 1位正解率と MRR（見つからなければ 0）
#[derive(Clone, Debug, Default)]
pub struct Group {
    pub n: usize,
    pub top1: usize,
    pub mrr: f64,
}

impl Group {
    /// 1位正解率（0..=1）。件数が 0 なら 0
    pub fn top1_rate(&self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            self.top1 as f64 / self.n as f64
        }
    }
}

/// 遅延の統計。p99 は nearest-rank 法（昇順で `ceil(0.99*n)-1` 番目）
#[derive(Clone, Debug, Default)]
pub struct LatencyStats {
    pub n: usize,
    pub p50: Duration,
    pub p99: Duration,
    pub max: Duration,
}

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub overall: Group,
    pub by_kind: BTreeMap<String, Group>,
    pub by_tag: BTreeMap<String, Group>,
    pub warm: LatencyStats,
    pub cold: LatencyStats,
}

/// LM の動作の集計（レポートの時間切れ率）
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LmReport {
    pub calls: u64,
    pub timeouts: u64,
    pub late_hits: u64,
}

impl LmReport {
    pub fn timeout_rate(&self) -> f64 {
        if self.calls == 0 {
            0.0
        } else {
            self.timeouts as f64 / self.calls as f64
        }
    }
}

/// レポートのヘッダ（§6.3、ADR 0002 §15.1）に出す情報
#[derive(Clone, Debug, Default)]
pub struct ReportMeta {
    pub set_path: String,
    pub dict_source: String,
    pub grammar_id: u64,
    pub ranker: String,
    /// "release" | "debug"
    pub build: String,
    pub repeat: usize,
    pub max_rank: usize,
    /// 構成の1行（ranker、analyzer、vectors、profile、lm）
    pub config: String,
    /// `--split` の指定（"dev" / "test" / "all"）。空なら出さない
    pub split: String,
    /// `unreachable` で除いた件数
    pub excluded_unreachable: usize,
    /// getrusage の最大 RSS（バイト）。測れなければ None
    pub max_rss_bytes: Option<u64>,
    pub lm: Option<LmReport>,
}

const NANOS_PER_MILLI: f64 = 1_000_000.0;
const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;
const PERCENT_MEDIAN: usize = 50;
const PERCENT_P99: usize = 99;

fn add_to_group(g: &mut Group, r: &CaseResult) {
    g.n += 1;
    if r.rank == Some(1) {
        g.top1 += 1;
    }
    // mrr には逆順位の合計を入れ、`finish_group` で件数で割る
    if let Some(rank) = r.rank {
        g.mrr += 1.0 / rank as f64;
    }
}

fn finish_group(g: &mut Group) {
    if g.n > 0 {
        g.mrr /= g.n as f64;
    }
}

/// nearest-rank 法。昇順の `sorted` から `ceil(percent * n / 100) - 1` 番目を返す。
fn nearest_rank(sorted: &[Duration], percent: usize) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = (percent * sorted.len()).div_ceil(100).max(1) - 1;
    sorted[idx.min(sorted.len() - 1)]
}

fn latency_stats(mut samples: Vec<Duration>) -> LatencyStats {
    samples.sort();
    LatencyStats {
        n: samples.len(),
        p50: nearest_rank(&samples, PERCENT_MEDIAN),
        p99: nearest_rank(&samples, PERCENT_P99),
        max: samples.last().copied().unwrap_or(Duration::ZERO),
    }
}

pub fn summarize(results: &[CaseResult]) -> Summary {
    let mut summary = Summary::default();
    for r in results {
        add_to_group(&mut summary.overall, r);
        add_to_group(summary.by_kind.entry(r.case.kind.clone()).or_default(), r);
        for tag in &r.case.tags {
            add_to_group(summary.by_tag.entry(tag.clone()).or_default(), r);
        }
    }
    finish_group(&mut summary.overall);
    summary.by_kind.values_mut().for_each(finish_group);
    summary.by_tag.values_mut().for_each(finish_group);
    summary.warm = latency_stats(
        results
            .iter()
            .flat_map(|r| r.warm.iter().copied())
            .collect(),
    );
    summary.cold = latency_stats(results.iter().map(|r| r.cold).collect());
    summary
}

fn ms(d: Duration) -> String {
    format!("{:.3}", d.as_nanos() as f64 / NANOS_PER_MILLI)
}

fn case_warm_p99(r: &CaseResult) -> Duration {
    let mut w = r.warm.clone();
    w.sort();
    nearest_rank(&w, PERCENT_P99)
}

/// Markdown の表のセルに入れられるよう、`|` と改行をエスケープする
pub(crate) fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

fn group_row(label: &str, g: &Group) -> String {
    format!(
        "| {} | {} | {} | {:.1}% | {:.3} |\n",
        cell(label),
        g.n,
        g.top1,
        g.top1_rate() * 100.0,
        g.mrr
    )
}

fn latency_row(label: &str, l: &LatencyStats) -> String {
    format!(
        "| {label} | {} | {} | {} | {} |\n",
        l.n,
        ms(l.p50),
        ms(l.p99),
        ms(l.max)
    )
}

/// §6.3 の形式の Markdown レポート
pub fn render_markdown(summary: &Summary, results: &[CaseResult], meta: &ReportMeta) -> String {
    // String への write! は失敗しないので、結果は無視する
    let mut out = String::new();
    let _ = writeln!(out, "# ikari-eval report");
    let _ = writeln!(
        out,
        "- set: {} ({} cases) / dict: {} grammar_id={:#018x} / ranker: {}",
        meta.set_path,
        results.len(),
        meta.dict_source,
        meta.grammar_id,
        meta.ranker
    );
    if !meta.config.is_empty() {
        let _ = writeln!(out, "- 構成: {}", meta.config);
    }
    if !meta.split.is_empty() {
        let _ = writeln!(
            out,
            "- split: {} / 除外（unreachable）: {} 件",
            meta.split, meta.excluded_unreachable
        );
    }
    let rss = meta.max_rss_bytes.map_or_else(
        || "未計測".to_owned(),
        |b| format!("{:.1} MiB", b as f64 / BYTES_PER_MIB),
    );
    let _ = writeln!(
        out,
        "- build: {} / repeat: {} / max_rank: {} / 最大RSS: {rss}",
        meta.build, meta.repeat, meta.max_rank
    );
    if let Some(lm) = meta.lm {
        let _ = writeln!(
            out,
            "- LM: 呼び出し {} / 時間切れ {} ({:.1}%) / 遅れて届いた結果の利用 {}",
            lm.calls,
            lm.timeouts,
            lm.timeout_rate() * 100.0,
            lm.late_hits
        );
    }
    if meta.build != "release" {
        let _ = writeln!(out, "- 注意: {} ビルドなので、応答時間は参考値", meta.build);
    }

    out.push_str("\n## 正解率\n");
    out.push_str("| 区分 | 件数 | 1位正解 | 1位正解率 | MRR |\n|---|---|---|---|---|\n");
    out.push_str(&group_row("全体", &summary.overall));
    for (k, g) in &summary.by_kind {
        out.push_str(&group_row(&format!("kind:{k}"), g));
    }
    for (t, g) in &summary.by_tag {
        out.push_str(&group_row(&format!("tag:{t}"), g));
    }

    out.push_str("\n## 応答時間（convert〜1ページ目, ms）\n");
    out.push_str("| 対象 | サンプル数 | p50 | p99 | 最悪 |\n|---|---|---|---|---|\n");
    out.push_str(&latency_row("warm", &summary.warm));
    out.push_str(&latency_row("cold（各ケースの初回）", &summary.cold));

    out.push_str("\n## ケース別\n");
    out.push_str(
        "| id | kind | 文脈 | 読み | 期待 | 1位 | 順位 | warm p99 |\n|---|---|---|---|---|---|---|---|\n",
    );
    for r in results {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            cell(&r.case.id),
            cell(&r.case.kind),
            cell(&r.case.context),
            cell(&r.case.reading),
            cell(&r.case.expected),
            cell(r.top1.as_deref().unwrap_or("-")),
            r.rank.map_or_else(|| "-".to_owned(), |n| n.to_string()),
            ms(case_warm_p99(r)),
        );
    }
    out
}

/// `render_cases_tsv` の列（`--baseline` と `compare` が名前で読む）
pub(crate) const CASES_TSV_HEADER: &str =
    "id\tkind\tcontext\treading\texpected\ttop1\trank\tcold_ms\twarm_p99_ms\ttags\tsplit";

/// ケース別の結果を TSV にする（ヘッダ行あり。時間は ms）
pub fn render_cases_tsv(results: &[CaseResult]) -> String {
    let mut out = String::from(CASES_TSV_HEADER);
    out.push('\n');
    for r in results {
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            r.case.id,
            r.case.kind,
            r.case.context,
            r.case.reading,
            r.case.expected,
            r.top1.as_deref().unwrap_or(""),
            r.rank.map_or_else(String::new, |n| n.to_string()),
            ms(r.cold),
            ms(case_warm_p99(r)),
            r.case.tags.join(","),
            r.case.split.map_or("", Split::as_str),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{d, mock_cases, mock_converter, result};
    use crate::{EvalConfig, run};
    #[test]
    fn nearest_rank_percentiles() {
        let v: Vec<Duration> = (1..=100).map(d).collect();
        assert_eq!(nearest_rank(&v, 99), d(99));
        assert_eq!(nearest_rank(&v, 50), d(50));
        let v: Vec<Duration> = (1..=10).map(d).collect();
        // ceil(0.99 * 10) - 1 = 9 番目（0始まり）= 最大値
        assert_eq!(nearest_rank(&v, 99), d(10));
        assert_eq!(nearest_rank(&v, 50), d(5));
        assert_eq!(nearest_rank(&[d(7)], 99), d(7));
        assert_eq!(nearest_rank(&[], 99), Duration::ZERO);
        // 101件では ceil(99.99) - 1 = 99 番目（0始まり）
        let v: Vec<Duration> = (1..=101).map(d).collect();
        assert_eq!(nearest_rank(&v, 99), d(100));
    }

    #[test]
    fn summarize_computes_top1_and_mrr() {
        let results = vec![
            result("a", "話題", &["怒り"], Some(1)),
            result("b", "話題", &[], Some(2)),
            result("c", "否定", &["怒り"], None),
            result("d", "否定", &[], Some(4)),
        ];
        let s = summarize(&results);
        assert_eq!((s.overall.n, s.overall.top1), (4, 1));
        assert!((s.overall.mrr - (1.0 + 0.5 + 0.0 + 0.25) / 4.0).abs() < 1e-12);
        assert!((s.overall.top1_rate() - 0.25).abs() < 1e-12);
        assert_eq!(s.by_kind["話題"].n, 2);
        assert_eq!(s.by_kind["話題"].top1, 1);
        assert!((s.by_kind["話題"].mrr - 0.75).abs() < 1e-12);
        assert_eq!(s.by_kind["否定"].top1, 0);
        assert_eq!(s.by_tag["怒り"].n, 2);
        assert!((s.by_tag["怒り"].mrr - 0.5).abs() < 1e-12);
        assert_eq!(s.warm.n, 12);
        assert_eq!(s.warm.max, d(3));
        assert_eq!(s.warm.p50, d(2));
        assert_eq!(s.warm.p99, d(3));
        assert_eq!(s.cold.n, 4);
        assert_eq!(s.cold.p99, d(5));
    }

    #[test]
    fn summarize_empty_is_zero() {
        let s = summarize(&[]);
        assert_eq!(s.overall.n, 0);
        assert_eq!(s.overall.mrr, 0.0);
        assert_eq!(s.overall.top1_rate(), 0.0);
        assert_eq!(s.warm.n, 0);
    }

    fn meta() -> ReportMeta {
        ReportMeta {
            set_path: "eval/sets/homophones.tsv".into(),
            dict_source: "test".into(),
            grammar_id: 0xabc,
            ranker: "cost".into(),
            build: "release".into(),
            repeat: 3,
            max_rank: 50,
            ..ReportMeta::default()
        }
    }

    #[test]
    fn report_has_required_sections() {
        let cfg = EvalConfig {
            max_rank: 50,
            repeat: 3,
        };
        let results = run(&mock_converter(), &mock_cases(), &cfg);
        let s = summarize(&results);
        let md = render_markdown(&s, &results, &meta());
        assert!(md.starts_with("# ikari-eval report\n"));
        assert!(md.contains("(4 cases)"));
        assert!(md.contains("grammar_id=0x0000000000000abc"));
        assert!(md.contains("最大RSS: 未計測"));
        assert!(md.contains("## 正解率"));
        assert!(md.contains("| 全体 | 4 | 1 |"));
        assert!(md.contains("| kind:話題 | 2 | 1 | 50.0% |"));
        assert!(md.contains("| tag:怒り | 1 | 0 | 0.0% |"));
        assert!(md.contains("## 応答時間"));
        assert!(md.contains("| warm | 12 |"));
        assert!(md.contains("| cold（各ケースの初回） | 4 |"));
        assert!(md.contains("## ケース別"));
        assert!(md.contains("| m-2 | 話題 | ctx | ひんし | 瀕死 | 品詞 | 2 |"));
        assert!(!md.contains("参考値"));
        // 任意の行は、情報があるときだけ出す
        assert!(!md.contains("- 構成:"));
        assert!(!md.contains("- split:"));
        assert!(!md.contains("- LM:"));

        let mut debug = meta();
        debug.build = "debug".into();
        assert!(render_markdown(&s, &results, &debug).contains("参考値"));
    }

    #[test]
    fn report_shows_config_split_rss_and_lm_timeout_rate() {
        let results = vec![result("a", "k", &[], Some(1))];
        let mut m = meta();
        m.config = "ranker=full analyzer=lattice lm=zenz.gguf".into();
        m.split = "dev".into();
        m.excluded_unreachable = 7;
        m.max_rss_bytes = Some(256 * 1024 * 1024);
        m.lm = Some(LmReport {
            calls: 200,
            timeouts: 10,
            late_hits: 3,
        });
        let md = render_markdown(&summarize(&results), &results, &m);
        assert!(
            md.contains("- 構成: ranker=full analyzer=lattice lm=zenz.gguf"),
            "{md}"
        );
        assert!(
            md.contains("- split: dev / 除外（unreachable）: 7 件"),
            "{md}"
        );
        assert!(md.contains("最大RSS: 256.0 MiB"), "{md}");
        assert!(
            md.contains("- LM: 呼び出し 200 / 時間切れ 10 (5.0%) / 遅れて届いた結果の利用 3"),
            "{md}"
        );
    }

    #[test]
    fn lm_report_timeout_rate_handles_zero() {
        assert_eq!(LmReport::default().timeout_rate(), 0.0);
    }

    #[test]
    fn markdown_escapes_pipes() {
        let results = vec![result("a", "k", &[], Some(1))];
        let md = render_markdown(&summarize(&results), &results, &meta());
        assert!(md.contains("ctx\\|x"));
    }

    #[test]
    fn cases_tsv_has_one_row_per_case() {
        let results = vec![
            result("a", "k", &["t1", "t2"], Some(3)),
            result("b", "k", &[], None),
        ];
        let tsv = render_cases_tsv(&results);
        let lines: Vec<&str> = tsv.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("id\tkind"));
        let a: Vec<&str> = lines[1].split('\t').collect();
        assert_eq!(a[0], "a");
        assert_eq!(a[6], "3");
        assert_eq!(a[7], "5.000");
        assert_eq!(a[8], "3.000");
        assert_eq!(a[9], "t1,t2");
        assert_eq!(a[10], "");
        let b: Vec<&str> = lines[2].split('\t').collect();
        assert_eq!(b[6], "");
    }
}
