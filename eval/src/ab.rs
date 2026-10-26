//! `--profile-ab`: profile 列のあるケースを、プロファイルの有無の両方で実行して差分を出す
//! （ADR 0002 §9.2-5, §15.1）。否定ケース・文法ケースがプロファイルで下がれば、レポートで目立たせて終了コード 1。

use std::fmt::Write as _;

use ikari_core::Converter;

use crate::{EvalCase, EvalConfig, ProfileResolver, RunOptions, cell, run_with};

/// 順位の変化（小さい方が良い。圏外は最悪）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Improved,
    Same,
    Worse,
}

impl Change {
    pub fn between(without: Option<usize>, with: Option<usize>) -> Change {
        // 圏外（None）は、どの順位より悪い
        let key = |r: Option<usize>| r.unwrap_or(usize::MAX);
        match key(with).cmp(&key(without)) {
            std::cmp::Ordering::Less => Change::Improved,
            std::cmp::Ordering::Equal => Change::Same,
            std::cmp::Ordering::Greater => Change::Worse,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AbRow {
    pub case: EvalCase,
    pub without: Option<usize>,
    pub with: Option<usize>,
    pub top1_without: Option<String>,
    pub top1_with: Option<String>,
}

impl AbRow {
    pub fn change(&self) -> Change {
        Change::between(self.without, self.with)
    }

    /// 否定・文法ケースが、プロファイルを付けて悪化した
    pub fn is_guard_regression(&self) -> bool {
        self.case.is_guard() && self.change() == Change::Worse
    }
}

#[derive(Clone, Debug, Default)]
pub struct AbReport {
    pub rows: Vec<AbRow>,
}

impl AbReport {
    pub fn guard_regressions(&self) -> Vec<&AbRow> {
        self.rows
            .iter()
            .filter(|r| r.is_guard_regression())
            .collect()
    }

    pub fn count(&self, change: Change) -> usize {
        self.rows.iter().filter(|r| r.change() == change).count()
    }
}

/// profile 列のあるケース（`forced` を指定したときは全ケース）を、プロファイルなしとありで実行する。
/// 時間は測らない（順位だけ）。
pub fn run_profile_ab(
    conv: &Converter,
    cases: &[EvalCase],
    cfg: &EvalConfig,
    resolver: ProfileResolver<'_>,
    forced: Option<&str>,
) -> AbReport {
    let targets: Vec<EvalCase> = cases
        .iter()
        .filter(|c| forced.is_some() || c.profile.is_some())
        .cloned()
        .collect();
    let quick = EvalConfig {
        repeat: 0,
        ..cfg.clone()
    };
    let without = run_with(conv, &targets, &quick, &RunOptions::default());
    let with = run_with(
        conv,
        &targets,
        &quick,
        &RunOptions {
            resolver: Some(resolver),
            forced_profile: forced,
            ..RunOptions::default()
        },
    );
    AbReport {
        rows: without
            .into_iter()
            .zip(with)
            .map(|(a, b)| AbRow {
                case: a.case,
                without: a.rank,
                with: b.rank,
                top1_without: a.top1,
                top1_with: b.top1,
            })
            .collect(),
    }
}

fn rank_text(r: Option<usize>) -> String {
    r.map_or_else(|| "-".to_owned(), |n| n.to_string())
}

/// 差分の表（Markdown）。否定・文法ケースの悪化は **回帰** と書く。
pub fn render_profile_ab(report: &AbReport) -> String {
    let mut out = String::from("\n## プロファイルあり・なしの差分（--profile-ab）\n");
    if report.rows.is_empty() {
        out.push_str("- 対象のケースがありません（profile 列のあるケースがない）\n");
        return out;
    }
    let guards = report.guard_regressions();
    let _ = writeln!(
        out,
        "- 対象 {} 件 / 改善 {} / 変化なし {} / 悪化 {} / **否定・文法ケースの回帰 {} 件**",
        report.rows.len(),
        report.count(Change::Improved),
        report.count(Change::Same),
        report.count(Change::Worse),
        guards.len()
    );
    if !guards.is_empty() {
        out.push_str("\n**回帰: 否定・文法ケースがプロファイルを付けると悪化しました**\n");
        for g in &guards {
            let _ = writeln!(
                out,
                "- **{}**（{}）: {} 位 -> {} 位（1位: {} -> {}）",
                g.case.id,
                g.case.kind,
                rank_text(g.without),
                rank_text(g.with),
                g.top1_without.as_deref().unwrap_or("-"),
                g.top1_with.as_deref().unwrap_or("-")
            );
        }
    }
    out.push_str(
        "\n| id | kind | profile | 期待 | なし | あり | 変化 |\n|---|---|---|---|---|---|---|\n",
    );
    for r in &report.rows {
        let mark = match (r.change(), r.is_guard_regression()) {
            (_, true) => "**回帰**",
            (Change::Improved, _) => "改善",
            (Change::Same, _) => "-",
            (Change::Worse, _) => "悪化",
        };
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} |",
            cell(&r.case.id),
            cell(&r.case.kind),
            cell(r.case.profile.as_deref().unwrap_or("(強制)")),
            cell(&r.case.expected),
            rank_text(r.without),
            rank_text(r.with),
            mark
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::load_cases;
    use crate::testutil::{ProfileBoostRanker, mock_converter_with, test_profile};

    fn cases() -> Vec<EvalCase> {
        load_cases(
            "id\tkind\tprofile\tcontext\treading\texpected\n\
             p-topic\t話題\tit\tc\tひんし\t瀕死\n\
             p-guard-ok\t否定\tit\tc\tひんし\t品詞\n\
             p-no-profile\t話題\t\tc\tひんし\t瀕死\n",
        )
        .unwrap()
    }

    fn cfg() -> EvalConfig {
        EvalConfig {
            max_rank: 50,
            repeat: 0,
        }
    }

    #[test]
    fn change_ordering_treats_none_as_worst() {
        assert_eq!(Change::between(Some(2), Some(1)), Change::Improved);
        assert_eq!(Change::between(Some(1), Some(1)), Change::Same);
        assert_eq!(Change::between(Some(1), Some(2)), Change::Worse);
        assert_eq!(Change::between(None, Some(9)), Change::Improved);
        assert_eq!(Change::between(Some(9), None), Change::Worse);
        assert_eq!(Change::between(None, None), Change::Same);
    }

    #[test]
    fn only_cases_with_a_profile_column_are_compared() {
        let conv = mock_converter_with(Arc::new(ProfileBoostRanker { boost: "瀕死" }));
        let resolver = |n: &str| Some(test_profile(n));
        let ab = run_profile_ab(&conv, &cases(), &cfg(), &resolver, None);
        assert_eq!(ab.rows.len(), 2);
        let topic = &ab.rows[0];
        assert_eq!((topic.without, topic.with), (Some(2), Some(1)));
        assert_eq!(topic.change(), Change::Improved);
        // 否定ケース（品詞が1位のまま）は、プロファイルで 品詞 が 2位に下がる
        let guard = &ab.rows[1];
        assert_eq!((guard.without, guard.with), (Some(1), Some(2)));
        assert!(guard.is_guard_regression());
        assert_eq!(ab.guard_regressions().len(), 1);
    }

    #[test]
    fn a_neutral_profile_has_no_regression() {
        // プロファイルの有無で順位が変わらない Ranker
        let conv = mock_converter_with(Arc::new(ikari_core::CostRanker));
        let resolver = |n: &str| Some(test_profile(n));
        let ab = run_profile_ab(&conv, &cases(), &cfg(), &resolver, None);
        assert!(ab.guard_regressions().is_empty());
        assert_eq!(ab.count(Change::Same), 2);
    }

    #[test]
    fn forced_profile_compares_every_case() {
        let conv = mock_converter_with(Arc::new(ProfileBoostRanker { boost: "瀕死" }));
        let resolver = |n: &str| Some(test_profile(n));
        let ab = run_profile_ab(&conv, &cases(), &cfg(), &resolver, Some("novel"));
        assert_eq!(ab.rows.len(), 3);
    }

    #[test]
    fn improvements_of_guard_cases_are_not_regressions() {
        let row = AbRow {
            case: cases().remove(1),
            without: Some(3),
            with: Some(1),
            top1_without: None,
            top1_with: None,
        };
        assert!(!row.is_guard_regression());
        let row = AbRow {
            without: Some(1),
            with: None,
            ..row
        };
        assert!(
            row.is_guard_regression(),
            "dropping out of range is a regression"
        );
    }

    #[test]
    fn non_guard_regressions_are_reported_but_not_fatal() {
        let row = AbRow {
            case: cases().remove(0),
            without: Some(1),
            with: Some(2),
            top1_without: None,
            top1_with: None,
        };
        assert_eq!(row.change(), Change::Worse);
        assert!(!row.is_guard_regression());
    }

    #[test]
    fn report_highlights_guard_regressions() {
        let conv = mock_converter_with(Arc::new(ProfileBoostRanker { boost: "瀕死" }));
        let resolver = |n: &str| Some(test_profile(n));
        let ab = run_profile_ab(&conv, &cases(), &cfg(), &resolver, None);
        let md = render_profile_ab(&ab);
        assert!(md.contains("否定・文法ケースの回帰 1 件"), "{md}");
        assert!(md.contains("**回帰**"), "{md}");
        assert!(
            md.contains("| p-topic | 話題 | it | 瀕死 | 2 | 1 | 改善 |"),
            "{md}"
        );
        assert!(
            md.contains("| p-guard-ok | 否定 | it | 品詞 | 1 | 2 | **回帰** |"),
            "{md}"
        );

        let empty = render_profile_ab(&AbReport::default());
        assert!(empty.contains("対象のケースがありません"));
    }
}
