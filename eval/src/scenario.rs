//! `--scenario <tsv>`: 学習のシナリオ（ADR 0002 §15.1）。
//!
//! 列は `step`（`commit` / `probe` / `wait`。`wait` は `surface` 列の秒数だけ手動の時計を進める。
//! `reading` と `context` は空。ADR 0045）、`context`、`reading`、`surface`。省略可能な列 `expect`
//! （`top1`（既定）/ `not-top1`）は probe の期待を表す。
//! 一時的なユーザーディレクトリで、`commit` を順に実行して学習させ、`probe` の順位を調べる。
//! 省略可能な列 `field`（`none` / `available`）と `history` は、左文脈を `compose_left` で作る（ADR 0046。
//! [`crate::carry`]）。`field` がなければ `context` をそのまま左文脈にする。
//! 学習の実体（LearningStore と LearningRanker）は Converter の側にあり、
//! ここでは [`ScenarioHost`] を通して呼ぶ（モックでテストできる）。

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ikari_core::{CarryParams, CommitEvent, Converter};

use crate::carry::{FieldMode, left_context, parse_field};
use crate::run::find_rank_until;
use crate::{EvalSetError, cell};

const COMMENT_PREFIX: char = '#';
const COL_STEP: &str = "step";
const COL_CONTEXT: &str = "context";
const COL_READING: &str = "reading";
const COL_SURFACE: &str = "surface";
const COL_FIELD: &str = "field";
const COL_HISTORY: &str = "history";
const COL_EXPECT: &str = "expect";
const STEP_COMMIT: &str = "commit";
const STEP_PROBE: &str = "probe";
const STEP_WAIT: &str = "wait";
const MILLIS_PER_SEC: u64 = 1000;
const EXPECT_TOP1: &str = "top1";
const EXPECT_NOT_TOP1: &str = "not-top1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    Commit,
    Probe,
    /// 手動の時計を進める（秒。`surface` 列）
    Wait(u64),
}

/// probe が満たすべき条件
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
    /// `surface` が1位（既定）
    Top1,
    /// `surface` が1位ではない（別の文脈では学習が効かないことの確認）
    NotTop1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioStep {
    /// ファイルの行番号（1始まり）
    pub line: usize,
    pub kind: StepKind,
    pub context: String,
    pub reading: String,
    pub surface: String,
    /// `field` 列（ADR 0046）。なければ None で、`context` をそのまま左文脈にする
    pub field: Option<FieldMode>,
    /// `history` 列（ADR 0046）。同じセッションの確定の表記の連結
    pub history: String,
    pub expect: Expect,
}

fn err(line: usize, message: impl Into<String>) -> EvalSetError {
    EvalSetError {
        line,
        message: message.into(),
    }
}

pub fn load_scenario(tsv: &str) -> Result<Vec<ScenarioStep>, EvalSetError> {
    let mut header: Option<BTreeMap<String, usize>> = None;
    let mut steps = Vec::new();
    for (i, raw) in tsv.lines().enumerate() {
        let line_no = i + 1;
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with(COMMENT_PREFIX) {
            continue;
        }
        let cells: Vec<&str> = line.split('\t').collect();
        let Some(cols) = &header else {
            let mut map = BTreeMap::new();
            for (idx, name) in cells.iter().enumerate() {
                if map.insert(name.trim().to_owned(), idx).is_some() {
                    return Err(err(line_no, format!("duplicate column `{}`", name.trim())));
                }
            }
            for req in [COL_STEP, COL_CONTEXT, COL_READING, COL_SURFACE] {
                if !map.contains_key(req) {
                    return Err(err(line_no, format!("missing required column `{req}`")));
                }
            }
            header = Some(map);
            continue;
        };
        if cells.len() > cols.len() {
            return Err(err(
                line_no,
                format!(
                    "{} cells but header has {} columns",
                    cells.len(),
                    cols.len()
                ),
            ));
        }
        let get = |name: &str| {
            cols.get(name)
                .and_then(|&i| cells.get(i))
                .copied()
                .unwrap_or("")
        };
        let kind = match get(COL_STEP).trim() {
            STEP_COMMIT => StepKind::Commit,
            STEP_PROBE => StepKind::Probe,
            STEP_WAIT => {
                let secs = get(COL_SURFACE)
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| err(line_no, "`wait` needs a number of seconds in `surface`"))?;
                StepKind::Wait(secs)
            }
            other => {
                return Err(err(
                    line_no,
                    format!(
                        "step must be `{STEP_COMMIT}`, `{STEP_PROBE}` or `{STEP_WAIT}`, got `{other}`"
                    ),
                ));
            }
        };
        let required: &[&str] = if matches!(kind, StepKind::Wait(_)) {
            &[]
        } else {
            &[COL_READING, COL_SURFACE]
        };
        for req in required {
            if get(req).is_empty() {
                return Err(err(line_no, format!("empty required value `{req}`")));
            }
        }
        let field = parse_field(get(COL_FIELD)).map_err(|m| err(line_no, m))?;
        let expect = match get(COL_EXPECT).trim() {
            "" | EXPECT_TOP1 => Expect::Top1,
            EXPECT_NOT_TOP1 => Expect::NotTop1,
            other => {
                return Err(err(
                    line_no,
                    format!("expect must be `{EXPECT_TOP1}` or `{EXPECT_NOT_TOP1}`, got `{other}`"),
                ));
            }
        };
        if kind != StepKind::Probe && expect != Expect::Top1 {
            return Err(err(line_no, "`expect` applies to probe steps only"));
        }
        steps.push(ScenarioStep {
            line: line_no,
            kind,
            context: get(COL_CONTEXT).to_owned(),
            reading: get(COL_READING).to_owned(),
            surface: get(COL_SURFACE).to_owned(),
            field,
            history: get(COL_HISTORY).to_owned(),
            expect,
        });
    }
    if header.is_none() {
        return Err(err(1, "missing header line"));
    }
    Ok(steps)
}

/// シナリオを実行する相手（Converter と学習の組。テストではモック）
pub trait ScenarioHost {
    /// `step.surface` の候補を確定して学習させる。候補が見つからないなどの失敗は Err。
    fn commit(&mut self, step: &ScenarioStep, max_rank: usize) -> Result<(), String>;
    /// `step.surface` の順位と、1位の表記
    fn probe(&mut self, step: &ScenarioStep, max_rank: usize) -> (Option<usize>, Option<String>);
    /// 手動の時計を `secs` 秒進める（直前ボーナスの窓。ADR 0045）
    fn wait(&mut self, secs: u64);
}

#[derive(Clone, Debug)]
pub struct StepOutcome {
    pub step: ScenarioStep,
    /// probe の順位
    pub rank: Option<usize>,
    pub top1: Option<String>,
    /// commit の失敗
    pub error: Option<String>,
}

impl StepOutcome {
    /// probe の合否。commit は失敗していなければ None（合否なし）、失敗していれば Some(false)
    pub fn passed(&self) -> Option<bool> {
        match self.step.kind {
            StepKind::Commit => self.error.as_ref().map(|_| false),
            StepKind::Wait(_) => None,
            StepKind::Probe => Some(match self.step.expect {
                Expect::Top1 => self.rank == Some(1),
                Expect::NotTop1 => self.rank != Some(1),
            }),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScenarioReport {
    pub outcomes: Vec<StepOutcome>,
}

impl ScenarioReport {
    pub fn failures(&self) -> Vec<&StepOutcome> {
        self.outcomes
            .iter()
            .filter(|o| o.passed() == Some(false))
            .collect()
    }

    pub fn probe_count(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| o.step.kind == StepKind::Probe)
            .count()
    }
}

pub fn run_scenario(
    host: &mut dyn ScenarioHost,
    steps: &[ScenarioStep],
    max_rank: usize,
) -> ScenarioReport {
    let outcomes = steps
        .iter()
        .map(|step| match step.kind {
            StepKind::Commit => StepOutcome {
                error: host.commit(step, max_rank).err(),
                rank: None,
                top1: None,
                step: step.clone(),
            },
            StepKind::Probe => {
                let (rank, top1) = host.probe(step, max_rank);
                StepOutcome {
                    step: step.clone(),
                    rank,
                    top1,
                    error: None,
                }
            }
            StepKind::Wait(secs) => {
                host.wait(secs);
                StepOutcome {
                    step: step.clone(),
                    rank: None,
                    top1: None,
                    error: None,
                }
            }
        })
        .collect();
    ScenarioReport { outcomes }
}

pub fn render_scenario(report: &ScenarioReport) -> String {
    let mut out = String::from("# ikari-eval scenario\n");
    let failed = report.failures().len();
    let _ = writeln!(
        out,
        "- steps: {} / probe: {} / 失敗: {failed}",
        report.outcomes.len(),
        report.probe_count()
    );
    out.push_str("\n| 行 | step | 文脈 | 読み | 表記 | 期待 | 順位 | 1位 | 結果 |\n|---|---|---|---|---|---|---|---|---|\n");
    for o in &report.outcomes {
        let s = &o.step;
        let expect = match (s.kind, s.expect) {
            (StepKind::Commit | StepKind::Wait(_), _) => "-",
            (StepKind::Probe, Expect::Top1) => EXPECT_TOP1,
            (StepKind::Probe, Expect::NotTop1) => EXPECT_NOT_TOP1,
        };
        let result = match (&o.error, o.passed()) {
            (Some(e), _) => format!("失敗: {}", cell(e)),
            (None, Some(true)) => "OK".to_owned(),
            (None, Some(false)) => "**NG**".to_owned(),
            (None, None) => "確定".to_owned(),
        };
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {expect} | {} | {} | {result} |",
            s.line,
            match s.kind {
                StepKind::Commit => STEP_COMMIT,
                StepKind::Probe => STEP_PROBE,
                StepKind::Wait(_) => STEP_WAIT,
            },
            cell(&s.context),
            cell(&s.reading),
            cell(&s.surface),
            o.rank.map_or_else(|| "-".to_owned(), |n| n.to_string()),
            cell(o.top1.as_deref().unwrap_or("-")),
        );
    }
    out
}

/// 実際の Converter に対して実行する。確定のイベントは `on_commit` に渡す（学習の記録は呼び出し側）。
pub struct ConverterHost<'a, F: FnMut(CommitEvent)> {
    pub converter: &'a Converter,
    pub on_commit: F,
    /// 変換1回あたりの LM の締め切り（`--lm-deadline-ms`）。`None` なら Converter の既定の予算。
    /// 既定の予算だと LM が間に合うかどうかで結果が揺れるので、LM ありの評価では固定する
    pub lm_deadline: Option<Duration>,
    /// `field` 列のあるステップの左文脈の組み立て（`--carry-fill`・`--carry-joint`。ADR 0046）。
    /// `None` なら足さない
    pub carry: Option<CarryParams>,
    /// `wait` が進める手動の時計（学習の `RecentChoices` と共有する。ADR 0045）。`None` なら `wait` は何もしない
    pub manual_clock: Option<Arc<AtomicU64>>,
}

impl<F: FnMut(CommitEvent)> ConverterHost<'_, F> {
    /// 変換して候補を探す。commit にも使えるよう、変換結果も返す
    fn convert_and_find(
        &self,
        step: &ScenarioStep,
        max_rank: usize,
    ) -> (ikari_core::Conversion, Option<usize>, Option<String>) {
        let deadline = self.lm_deadline.map(|d| Instant::now() + d);
        // `field` 列があれば compose_left で左文脈を作り直す（なければ context のまま。ADR 0046）
        let ctx = left_context(&step.context, step.field, &step.history, self.carry);
        let mut conversion = match deadline {
            Some(d) => self.converter.convert_until(&step.reading, &ctx, Some(d)),
            None => self.converter.convert(&step.reading, &ctx),
        };
        let (rank, top1) =
            find_rank_until(&mut conversion, max_rank, &|s| s == step.surface, deadline);
        (conversion, rank, top1)
    }
}

impl<F: FnMut(CommitEvent)> ScenarioHost for ConverterHost<'_, F> {
    fn commit(&mut self, step: &ScenarioStep, max_rank: usize) -> Result<(), String> {
        let (conversion, rank, _) = self.convert_and_find(step, max_rank);
        let rank = rank.ok_or_else(|| {
            format!(
                "`{}` is not among the top {max_rank} candidates of `{}`",
                step.surface, step.reading
            )
        })?;
        let event = conversion
            .commit(rank - 1)
            .ok_or_else(|| "commit produced no event".to_owned())?;
        (self.on_commit)(event);
        Ok(())
    }

    fn probe(&mut self, step: &ScenarioStep, max_rank: usize) -> (Option<usize>, Option<String>) {
        let (_, rank, top1) = self.convert_and_find(step, max_rank);
        (rank, top1)
    }

    fn wait(&mut self, secs: u64) {
        if let Some(clock) = &self.manual_clock {
            clock.fetch_add(secs.saturating_mul(MILLIS_PER_SEC), Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 学習を模したホスト: commit した (読み, 表記) を、同じ文脈キーの probe で1位にする
    #[derive(Default)]
    struct FakeHost {
        learned: Vec<(String, String, String)>,
        fail_commit_for: Option<String>,
        waited: u64,
    }

    impl ScenarioHost for FakeHost {
        fn commit(&mut self, step: &ScenarioStep, _max_rank: usize) -> Result<(), String> {
            if self.fail_commit_for.as_deref() == Some(step.surface.as_str()) {
                return Err("not a candidate".to_owned());
            }
            self.learned.push((
                step.context.clone(),
                step.reading.clone(),
                step.surface.clone(),
            ));
            Ok(())
        }

        fn probe(
            &mut self,
            step: &ScenarioStep,
            _max_rank: usize,
        ) -> (Option<usize>, Option<String>) {
            let hit = self
                .learned
                .iter()
                .any(|(c, r, s)| *c == step.context && *r == step.reading && *s == step.surface);
            if hit {
                (Some(1), Some(step.surface.clone()))
            } else {
                (Some(3), Some("別".to_owned()))
            }
        }

        fn wait(&mut self, secs: u64) {
            self.waited += secs;
        }
    }

    const SCENARIO: &str = "\
# a-02 の再現
step\tcontext\treading\tsurface\texpect
commit\t技術文書で\tひんし\t品詞
probe\t技術文書で\tひんし\t品詞
probe\t小説で\tひんし\t品詞\tnot-top1
probe\t技術文書で\tひんし\t瀕死\tnot-top1
";

    #[test]
    fn loads_steps_in_order() {
        let steps = load_scenario(SCENARIO).unwrap();
        assert_eq!(steps.len(), 4);
        assert_eq!(steps[0].kind, StepKind::Commit);
        assert_eq!(steps[0].line, 3);
        assert_eq!(steps[1].expect, Expect::Top1);
        assert_eq!(steps[2].expect, Expect::NotTop1);
        assert_eq!(steps[3].surface, "瀕死");
    }

    #[test]
    fn expect_column_is_optional() {
        let steps = load_scenario("step\tcontext\treading\tsurface\nprobe\t\tあ\t亜\n").unwrap();
        assert_eq!(steps[0].expect, Expect::Top1);
        assert_eq!(steps[0].context, "");
    }

    #[test]
    fn field_and_history_columns_are_optional() {
        let head = "step\tcontext\treading\tsurface\tfield\thistory\n";
        let steps = load_scenario(&format!(
            "{head}probe\t\tr\ts\tavailable\t前の文\nprobe\t\tr\ts\tnone\t前の文\nprobe\tc\tr\ts\t\t\n"
        ))
        .unwrap();
        assert_eq!(steps[0].field, Some(FieldMode::Available));
        assert_eq!(steps[0].history, "前の文");
        assert_eq!(steps[1].field, Some(FieldMode::None));
        assert_eq!(steps[2].field, None);
        let without = load_scenario("step\tcontext\treading\tsurface\nprobe\tc\tr\ts\n").unwrap();
        assert_eq!((without[0].field, without[0].history.as_str()), (None, ""));
        assert!(load_scenario(&format!("{head}probe\tc\tr\ts\tmaybe\t\n")).is_err());
    }

    #[test]
    fn wait_step_takes_seconds_in_surface_and_advances_the_host() {
        let head = "step\tcontext\treading\tsurface\n";
        let steps = load_scenario(&format!("{head}wait\t\t\t3600\nprobe\t\tあ\t亜\n")).unwrap();
        assert_eq!(steps[0].kind, StepKind::Wait(3600));
        let mut host = FakeHost::default();
        let report = run_scenario(&mut host, &steps, 50);
        assert_eq!(host.waited, 3600);
        assert_eq!(report.outcomes[0].passed(), None);
        assert_eq!(report.probe_count(), 1);
        assert!(render_scenario(&report).contains("| 2 | wait |"));
        for bad in ["wait\t\t\t\n", "wait\t\t\tx\n", "wait\t\t\t-1\n"] {
            assert_eq!(load_scenario(&format!("{head}{bad}")).unwrap_err().line, 2);
        }
        assert!(
            load_scenario(&format!(
                "step\tcontext\treading\tsurface\texpect\nwait\t\t\t1\tnot-top1\n"
            ))
            .is_err()
        );
    }

    #[test]
    fn rejects_bad_scenarios() {
        let head = "step\tcontext\treading\tsurface\texpect\n";
        for (tsv, line) in [
            ("", 1),
            ("step\tcontext\treading\n", 1),
            (&format!("{head}jump\tc\tr\ts\n"), 2),
            (&format!("{head}commit\tc\t\ts\n"), 2),
            (&format!("{head}commit\tc\tr\t\n"), 2),
            (&format!("{head}probe\tc\tr\ts\tmaybe\n"), 2),
            (&format!("{head}commit\tc\tr\ts\tnot-top1\n"), 2),
            (&format!("{head}probe\tc\tr\ts\ttop1\textra\n"), 2),
        ] {
            let e = load_scenario(tsv).expect_err(tsv);
            assert_eq!(e.line, line, "{tsv:?}: {e}");
        }
    }

    #[test]
    fn learning_scenario_passes_when_commit_only_helps_the_same_context() {
        let steps = load_scenario(SCENARIO).unwrap();
        let mut host = FakeHost::default();
        let report = run_scenario(&mut host, &steps, 50);
        assert_eq!(report.outcomes.len(), 4);
        assert!(report.failures().is_empty(), "{:?}", report.failures());
        assert_eq!(report.probe_count(), 3);
        // commit のステップは合否なし
        assert_eq!(report.outcomes[0].passed(), None);
        assert_eq!(report.outcomes[1].rank, Some(1));
        assert_eq!(report.outcomes[2].rank, Some(3));
    }

    #[test]
    fn a_probe_that_misses_its_expectation_fails() {
        let steps = load_scenario("step\tcontext\treading\tsurface\nprobe\tc\tr\ts\n").unwrap();
        let report = run_scenario(&mut FakeHost::default(), &steps, 50);
        assert_eq!(report.failures().len(), 1);
        assert!(render_scenario(&report).contains("**NG**"));
    }

    #[test]
    fn a_failed_commit_is_reported_as_a_failure() {
        let steps = load_scenario(SCENARIO).unwrap();
        let mut host = FakeHost {
            fail_commit_for: Some("品詞".to_owned()),
            ..Default::default()
        };
        let report = run_scenario(&mut host, &steps, 50);
        assert_eq!(report.outcomes[0].error.as_deref(), Some("not a candidate"));
        assert_eq!(report.outcomes[0].passed(), Some(false));
        // commit できなかったので、同じ文脈の probe も 1位にならない
        assert!(report.failures().len() >= 2);
        let md = render_scenario(&report);
        assert!(md.contains("失敗: not a candidate"), "{md}");
    }

    #[test]
    fn render_summarizes_steps() {
        let steps = load_scenario(SCENARIO).unwrap();
        let report = run_scenario(&mut FakeHost::default(), &steps, 50);
        let md = render_scenario(&report);
        assert!(md.contains("steps: 4 / probe: 3 / 失敗: 0"), "{md}");
        assert!(
            md.contains("| 4 | probe | 技術文書で | ひんし | 品詞 | top1 | 1 | 品詞 | OK |"),
            "{md}"
        );
        assert!(
            md.contains("| 3 | commit | 技術文書で | ひんし | 品詞 | - | - | - | 確定 |"),
            "{md}"
        );
    }
}
