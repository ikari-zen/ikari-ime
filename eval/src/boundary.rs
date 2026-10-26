//! 区切りの指標（ADR 0006 §7.1-2）: 文節の境界の適合率・再現率・F1 と、区切り全体の完全一致率。
//!
//! 境界は「読みの文字位置」（先頭と末尾を除く）の集合。ケース全体の境界を数え上げて集計する（マイクロ平均）。
//! 境界が 1 つもないケース（1 文節が正解で、1 文節と答えた）は、完全一致率に数え、F1 には寄与しない。

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fmt::Write as _;

use crate::SegmentCase;
use crate::segment_engine::SegmentEngine;
use crate::segments::boundaries_of;

/// 1 ケースの区切りの結果
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundaryResult {
    pub id: String,
    pub kind: String,
    pub gold: Vec<usize>,
    /// エンジンが答えた境界。区切れなかった（エラー）ときは None
    pub predicted: Option<Vec<usize>>,
    /// 答えた区切り（読み）。表示用
    pub predicted_segments: Vec<String>,
}

impl BoundaryResult {
    pub fn exact(&self) -> bool {
        self.predicted.as_deref() == Some(self.gold.as_slice())
    }
}

/// 境界の数え上げ
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BoundaryCounts {
    /// 正解と答えの両方にある境界
    pub tp: usize,
    /// 答えにだけある境界
    pub fp: usize,
    /// 正解にだけある境界
    pub fn_: usize,
}

impl BoundaryCounts {
    pub fn of(gold: &[usize], predicted: &[usize]) -> Self {
        let g: HashSet<usize> = gold.iter().copied().collect();
        let p: HashSet<usize> = predicted.iter().copied().collect();
        let tp = g.intersection(&p).count();
        Self {
            tp,
            fp: p.len() - tp,
            fn_: g.len() - tp,
        }
    }

    pub fn add(&mut self, other: BoundaryCounts) {
        self.tp += other.tp;
        self.fp += other.fp;
        self.fn_ += other.fn_;
    }

    /// 答えた境界が 0 件なら 1.0（間違った境界を出していない）
    pub fn precision(&self) -> f64 {
        ratio(self.tp, self.tp + self.fp)
    }

    /// 正解の境界が 0 件なら 1.0（取り逃した境界がない）
    pub fn recall(&self) -> f64 {
        ratio(self.tp, self.tp + self.fn_)
    }

    /// 適合率と再現率の調和平均。両方 0 なら 0
    pub fn f1(&self) -> f64 {
        let (p, r) = (self.precision(), self.recall());
        if p + r == 0.0 {
            0.0
        } else {
            2.0 * p * r / (p + r)
        }
    }
}

fn ratio(num: usize, den: usize) -> f64 {
    if den == 0 {
        1.0
    } else {
        num as f64 / den as f64
    }
}

/// 集計 1 行分
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoundaryGroup {
    pub cases: usize,
    pub exact: usize,
    pub counts: BoundaryCounts,
}

impl BoundaryGroup {
    fn push(&mut self, r: &BoundaryResult) {
        self.cases += 1;
        self.exact += usize::from(r.exact());
        self.counts.add(BoundaryCounts::of(
            &r.gold,
            r.predicted.as_deref().unwrap_or(&[]),
        ));
    }

    pub fn exact_rate(&self) -> f64 {
        ratio(self.exact, self.cases)
    }
}

/// 全ケースの区切りを評価する。`open` に失敗したケースは `predicted: None`（境界 0 件の答えとして数える）
pub fn evaluate_boundaries<E: SegmentEngine>(
    engine: &E,
    cases: &[SegmentCase],
) -> Vec<BoundaryResult> {
    cases
        .iter()
        .map(|c| {
            let session = engine.open(&c.case.reading, &c.case.context).ok();
            let spans = session.as_ref().map(|s| &engine.layout(s).spans);
            BoundaryResult {
                id: c.case.id.clone(),
                kind: c.case.kind.clone(),
                gold: c.gold_boundaries(),
                predicted: spans.map(|sp| boundaries_of(sp.iter().map(|s| s.reading.as_str()))),
                predicted_segments: spans
                    .map(|sp| sp.iter().map(|s| s.reading.clone()).collect())
                    .unwrap_or_default(),
            }
        })
        .collect()
}

/// 種別ごとと全体の集計（種別は名前順）
pub fn summarize_boundaries(
    results: &[BoundaryResult],
) -> (BoundaryGroup, BTreeMap<String, BoundaryGroup>) {
    let mut all = BoundaryGroup::default();
    let mut by_kind: BTreeMap<String, BoundaryGroup> = BTreeMap::new();
    for r in results {
        all.push(r);
        by_kind.entry(r.kind.clone()).or_default().push(r);
    }
    (all, by_kind)
}

/// Markdown の表と、完全一致しなかったケースの一覧
pub fn render_boundaries(results: &[BoundaryResult], gold_segments: &[SegmentCase]) -> String {
    let (all, by_kind) = summarize_boundaries(results);
    let mut out = String::new();
    out.push_str("## 区切り（文節の境界）\n\n");
    out.push_str(
        "| 種別 | 件数 | 完全一致 | 適合率 | 再現率 | F1 |\n|---|---:|---:|---:|---:|---:|\n",
    );
    let row = |out: &mut String, name: &str, g: &BoundaryGroup| {
        let _ = writeln!(
            out,
            "| {name} | {} | {:.1}% ({}) | {:.3} | {:.3} | {:.3} |",
            g.cases,
            g.exact_rate() * 100.0,
            g.exact,
            g.counts.precision(),
            g.counts.recall(),
            g.counts.f1()
        );
    };
    for (kind, g) in &by_kind {
        row(&mut out, kind, g);
    }
    row(&mut out, "全体", &all);

    let misses: Vec<&BoundaryResult> = results.iter().filter(|r| !r.exact()).collect();
    if !misses.is_empty() {
        out.push_str("\n### 区切りが違うケース\n\n| id | 正解 | 答え |\n|---|---|---|\n");
        for r in misses {
            let gold = gold_segments
                .iter()
                .find(|c| c.case.id == r.id)
                .map(|c| c.expected_segments.join("/"))
                .unwrap_or_default();
            let got = if r.predicted.is_some() {
                r.predicted_segments.join("/")
            } else {
                "(区切れない)".to_owned()
            };
            let _ = writeln!(out, "| {} | {gold} | {got} |", r.id);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_segment_cases;
    use crate::segment_engine::mock::MockEngine;

    fn cases() -> Vec<SegmentCase> {
        load_segment_cases(
            "id\tkind\tcontext\treading\texpected\texpected_segments\n\
             a\t区切り\t\tはじめてしった\t初めて知った\tはじめて|しった\n\
             b\t区切り\t\tりぽじとりめい\tリポジトリ名\tりぽじとりめい\n\
             c\t区切り\t\tきょうはあつい\t今日は暑い\tきょうは|あつい\n\
             d\t区切り\t\tほんをよむ\t本を読む\tほんを|よむ\n",
        )
        .unwrap()
    }

    fn engine() -> MockEngine {
        let mut e = MockEngine::default();
        let mut put = |reading: &str, segs: &[&str]| {
            e.initial.insert(
                reading.to_owned(),
                segs.iter().map(|s| (*s).to_owned()).collect(),
            );
        };
        put("はじめてしった", &["はじめて", "しった"]); // 一致
        put("りぽじとりめい", &["りぽじとり", "めい"]); // 余計な境界 1
        put("きょうはあつい", &["きょうはあつい"]); // 取り逃し 1
        // d は open できない
        e
    }

    #[test]
    fn counts_tp_fp_fn_per_case() {
        assert_eq!(
            BoundaryCounts::of(&[4], &[4]),
            BoundaryCounts {
                tp: 1,
                fp: 0,
                fn_: 0
            }
        );
        assert_eq!(
            BoundaryCounts::of(&[], &[5]),
            BoundaryCounts {
                tp: 0,
                fp: 1,
                fn_: 0
            }
        );
        assert_eq!(
            BoundaryCounts::of(&[3, 6], &[3, 5]),
            BoundaryCounts {
                tp: 1,
                fp: 1,
                fn_: 1
            }
        );
    }

    #[test]
    fn f1_is_micro_averaged_with_documented_edges() {
        let c = BoundaryCounts {
            tp: 1,
            fp: 1,
            fn_: 2,
        };
        assert!((c.precision() - 0.5).abs() < 1e-9);
        assert!((c.recall() - 1.0 / 3.0).abs() < 1e-9);
        assert!((c.f1() - 0.4).abs() < 1e-9);
        // 境界が 1 つもない集計は、間違っていないので 1.0
        assert!((BoundaryCounts::default().f1() - 1.0).abs() < 1e-9);
        // 当たりが 0 で外れだけなら 0
        assert_eq!(
            BoundaryCounts {
                tp: 0,
                fp: 2,
                fn_: 0
            }
            .f1(),
            0.0
        );
    }

    #[test]
    fn evaluates_against_the_mock_engine() {
        let results = evaluate_boundaries(&engine(), &cases());
        assert_eq!(results.len(), 4);
        assert!(results[0].exact());
        assert_eq!(results[1].predicted, Some(vec![5]));
        assert!(!results[1].exact());
        assert_eq!(results[2].predicted, Some(vec![]));
        assert_eq!(results[3].predicted, None, "open failed");

        let (all, by_kind) = summarize_boundaries(&results);
        assert_eq!(all.cases, 4);
        assert_eq!(all.exact, 1);
        // a: tp 1 / b: fp 1 / c: fn 1 / d: fn 1（答えなし）
        assert_eq!(
            all.counts,
            BoundaryCounts {
                tp: 1,
                fp: 1,
                fn_: 2
            }
        );
        assert_eq!(by_kind.len(), 1);
        assert!((all.exact_rate() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn renders_a_table_and_the_misses() {
        let cs = cases();
        let md = render_boundaries(&evaluate_boundaries(&engine(), &cs), &cs);
        assert!(
            md.contains("| 全体 | 4 | 25.0% (1) | 0.500 | 0.333 | 0.400 |"),
            "{md}"
        );
        assert!(
            md.contains("| b | りぽじとりめい | りぽじとり/めい |"),
            "{md}"
        );
        assert!(md.contains("(区切れない)"), "{md}");
        assert!(!md.contains("| a |"), "exact cases are not listed: {md}");
    }
}
