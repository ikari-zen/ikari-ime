//! 合否の判定（ADR 0002 §15.1 `--min-top1`、`--baseline` と `--fail-on-regression`）。

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::{CaseResult, Group, cell};

const COL_ID: &str = "id";
const COL_RANK: &str = "rank";
const COMMENT_PREFIX: char = '#';

/// `--min-top1` の値を確かめる（0 以上 1 以下の有限の数）
pub fn parse_threshold(s: &str) -> Result<f64, String> {
    let v: f64 = s
        .trim()
        .parse()
        .map_err(|_| format!("`{s}` is not a number"))?;
    if !v.is_finite() || !(0.0..=1.0).contains(&v) {
        return Err(format!("`{s}` must be between 0 and 1"));
    }
    Ok(v)
}

/// 全体の1位正解率が `min` を下回れば Err（メッセージ付き）。ケースが0件のときも Err
/// （何も測れていないのに合格させない）。
pub fn check_min_top1(overall: &Group, min: f64) -> Result<(), String> {
    if overall.n == 0 {
        return Err("min-top1: no cases were evaluated".to_owned());
    }
    let rate = overall.top1_rate();
    // 浮動小数点の誤差で、ちょうど基準どおりの正解率が不合格にならないようにする
    if rate + f64::EPSILON < min {
        return Err(format!(
            "min-top1: top-1 accuracy {:.1}% ({}/{}) is below the required {:.1}%",
            rate * 100.0,
            overall.top1,
            overall.n,
            min * 100.0
        ));
    }
    Ok(())
}

/// 基準の結果（`--out-cases` で書いた TSV）。id -> 順位（見つからなければ None）
#[derive(Clone, Debug, Default)]
pub struct Baseline {
    ranks: HashMap<String, Option<usize>>,
}

impl Baseline {
    pub fn len(&self) -> usize {
        self.ranks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ranks.is_empty()
    }

    pub fn rank_of(&self, id: &str) -> Option<Option<usize>> {
        self.ranks.get(id).copied()
    }
}

/// `--out-cases` の TSV から基準を読む。`id` 列と `rank` 列を名前で探す。
pub fn load_baseline(tsv: &str) -> Result<Baseline, String> {
    let mut lines = tsv
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty() && !l.starts_with(COMMENT_PREFIX));
    let header = lines.next().ok_or("baseline: empty file")?;
    let cols: Vec<&str> = header.split('\t').map(str::trim).collect();
    let find = |name: &str| {
        cols.iter()
            .position(|c| *c == name)
            .ok_or_else(|| format!("baseline: missing column `{name}`"))
    };
    let (id_idx, rank_idx) = (find(COL_ID)?, find(COL_RANK)?);

    let mut ranks = HashMap::new();
    for (i, line) in lines.enumerate() {
        let cells: Vec<&str> = line.split('\t').collect();
        let id = cells
            .get(id_idx)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("baseline: row {} has no id", i + 1))?;
        let rank = match cells.get(rank_idx).map(|s| s.trim()) {
            None | Some("") => None,
            Some(s) => Some(
                s.parse::<usize>()
                    .ok()
                    .filter(|r| *r >= 1)
                    .ok_or_else(|| format!("baseline: row {} has a bad rank `{s}`", i + 1))?,
            ),
        };
        if ranks.insert((*id).to_owned(), rank).is_some() {
            return Err(format!("baseline: duplicate id `{id}`"));
        }
    }
    Ok(Baseline { ranks })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Regression {
    pub id: String,
    pub kind: String,
    pub expected: String,
    /// 今回の順位（max_rank 以内に見つからなければ None）
    pub now_rank: Option<usize>,
    pub now_top1: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct RegressionReport {
    pub regressions: Vec<Regression>,
    /// 基準で1位だったケースの数（今回も実行したもの）
    pub baseline_top1: usize,
    /// 基準にあるが、今回は実行しなかったケース（`--split` などで除外）
    pub not_run: usize,
    /// 今回あるが、基準にないケース（比べられない）
    pub new_cases: usize,
}

/// 基準で1位だったケースが、今回は1位でなくなったものを集める。
pub fn find_regressions(baseline: &Baseline, results: &[CaseResult]) -> RegressionReport {
    let mut report = RegressionReport::default();
    let mut compared = 0;
    for r in results {
        match baseline.rank_of(&r.case.id) {
            None => report.new_cases += 1,
            Some(base) => {
                compared += 1;
                if base == Some(1) {
                    report.baseline_top1 += 1;
                    if r.rank != Some(1) {
                        report.regressions.push(Regression {
                            id: r.case.id.clone(),
                            kind: r.case.kind.clone(),
                            expected: r.case.expected.clone(),
                            now_rank: r.rank,
                            now_top1: r.top1.clone(),
                        });
                    }
                }
            }
        }
    }
    report.not_run = baseline.len().saturating_sub(compared);
    report
}

/// 回帰の一覧（Markdown）。回帰がなければ「なし」と書く。
pub fn render_regressions(r: &RegressionReport) -> String {
    let mut out = String::from("\n## 回帰（基準で1位だったケース）\n");
    let _ = writeln!(
        out,
        "- 基準で1位: {} 件 / 回帰: {} 件 / 基準にあって今回未実行: {} 件 / 基準にない新規: {} 件",
        r.baseline_top1,
        r.regressions.len(),
        r.not_run,
        r.new_cases
    );
    if r.regressions.is_empty() {
        out.push_str("- 回帰なし\n");
        return out;
    }
    out.push_str("| id | kind | 期待 | 今回の1位 | 今回の順位 |\n|---|---|---|---|---|\n");
    for x in &r.regressions {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} |",
            cell(&x.id),
            cell(&x.kind),
            cell(&x.expected),
            cell(x.now_top1.as_deref().unwrap_or("-")),
            x.now_rank.map_or_else(|| "-".to_owned(), |n| n.to_string()),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::result;

    fn group(n: usize, top1: usize) -> Group {
        Group { n, top1, mrr: 0.0 }
    }

    #[test]
    fn threshold_parsing() {
        assert_eq!(parse_threshold("0.5"), Ok(0.5));
        assert_eq!(parse_threshold("0"), Ok(0.0));
        assert_eq!(parse_threshold("1"), Ok(1.0));
        assert_eq!(parse_threshold(" 0.25 "), Ok(0.25));
        for bad in ["", "x", "-0.1", "1.01", "NaN", "inf", "50%"] {
            assert!(parse_threshold(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn min_top1_passes_at_and_above_the_threshold() {
        assert!(check_min_top1(&group(10, 5), 0.5).is_ok());
        assert!(check_min_top1(&group(10, 6), 0.5).is_ok());
        assert!(check_min_top1(&group(10, 10), 1.0).is_ok());
        assert!(check_min_top1(&group(3, 1), 1.0 / 3.0).is_ok());
        assert!(check_min_top1(&group(10, 0), 0.0).is_ok());
    }

    #[test]
    fn min_top1_fails_below_the_threshold() {
        let e = check_min_top1(&group(14, 3), 0.5).unwrap_err();
        assert!(
            e.contains("21.4%") && e.contains("3/14") && e.contains("50.0%"),
            "{e}"
        );
        assert!(check_min_top1(&group(10, 9), 1.0).is_err());
    }

    #[test]
    fn min_top1_fails_when_nothing_was_evaluated() {
        assert!(check_min_top1(&group(0, 0), 0.0).is_err());
    }

    const BASELINE: &str = "\
id\tkind\tcontext\treading\texpected\ttop1\trank\tcold_ms\twarm_p99_ms
a\tk\tc\tr\te\tx\t1\t0.1\t0.1
b\tk\tc\tr\te\tx\t2\t0.1\t0.1
c\tk\tc\tr\te\t\t\t0.1\t0.1
d\tk\tc\tr\te\tx\t1\t0.1\t0.1
";

    #[test]
    fn baseline_loads_rank_by_column_name() {
        let b = load_baseline(BASELINE).unwrap();
        assert_eq!(b.len(), 4);
        assert_eq!(b.rank_of("a"), Some(Some(1)));
        assert_eq!(b.rank_of("b"), Some(Some(2)));
        assert_eq!(b.rank_of("c"), Some(None));
        assert_eq!(b.rank_of("zzz"), None);
        // 列の順序が違っても名前で読む
        let b = load_baseline("rank\tid\n3\tx\n").unwrap();
        assert_eq!(b.rank_of("x"), Some(Some(3)));
    }

    #[test]
    fn baseline_rejects_bad_files() {
        for bad in [
            "",
            "# only\n",
            "id\tkind\na\tk\n",
            "id\trank\na\t0\n",
            "id\trank\na\tx\n",
            "id\trank\na\t1\na\t2\n",
            "id\trank\n\t1\n",
        ] {
            assert!(load_baseline(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn regressions_are_cases_that_were_first_and_no_longer_are() {
        let b = load_baseline(BASELINE).unwrap();
        let results = vec![
            result("a", "話題", &[], Some(1)), // 維持
            result("b", "話題", &[], Some(1)), // 改善（基準は2位）
            result("c", "話題", &[], Some(1)), // 改善（基準は圏外）
            result("d", "否定", &[], Some(3)), // 回帰
            result("zz", "話題", &[], None),   // 基準にない
        ];
        let r = find_regressions(&b, &results);
        assert_eq!(r.regressions.len(), 1);
        assert_eq!(r.regressions[0].id, "d");
        assert_eq!(r.regressions[0].now_rank, Some(3));
        assert_eq!(r.baseline_top1, 2);
        assert_eq!(r.new_cases, 1);
        assert_eq!(r.not_run, 0);
    }

    #[test]
    fn a_case_that_dropped_out_of_range_is_a_regression() {
        let b = load_baseline(BASELINE).unwrap();
        let r = find_regressions(&b, &[result("a", "k", &[], None)]);
        assert_eq!(r.regressions.len(), 1);
        assert_eq!(r.regressions[0].now_rank, None);
        // 基準にあって今回は実行していないケースは、回帰ではなく未実行として数える
        assert_eq!(r.not_run, 3);
    }

    #[test]
    fn render_lists_regressions_or_says_none() {
        let b = load_baseline(BASELINE).unwrap();
        let none = find_regressions(&b, &[result("a", "k", &[], Some(1))]);
        assert!(render_regressions(&none).contains("回帰なし"));
        let some = find_regressions(&b, &[result("d", "否定", &[], Some(3))]);
        let md = render_regressions(&some);
        assert!(md.contains("| d | 否定 | 期待 | 一位 | 3 |"), "{md}");
        assert!(!md.contains("回帰なし"));
    }
}
