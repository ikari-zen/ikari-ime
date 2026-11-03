//! 評価セットの実行（順位と応答時間の計測）。

use std::sync::Arc;
use std::time::{Duration, Instant};

use ikari_core::{CarryParams, Context, Converter, Profile};

use crate::EvalCase;
use crate::carry::left_text;

/// 既定: max_rank 50, repeat 20
#[derive(Clone, Debug)]
pub struct EvalConfig {
    pub max_rank: usize,
    pub repeat: usize,
}

impl Default for EvalConfig {
    fn default() -> Self {
        Self {
            max_rank: 50,
            repeat: 20,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CaseResult {
    pub case: EvalCase,
    /// 1始まり。max_rank 以内に見つからなければ None
    pub rank: Option<usize>,
    pub top1: Option<String>,
    pub cold: Duration,
    pub warm: Vec<Duration>,
}

/// profile 列の名前から `Profile` を作る関数（プリセットの実体はユーザーデータ側が持つ）
pub type ProfileResolver<'a> = &'a dyn Fn(&str) -> Option<Arc<Profile>>;

/// ケースに使うプロファイルの決め方
#[derive(Clone, Copy, Default)]
pub struct RunOptions<'a> {
    /// None なら、どのケースにもプロファイルを付けない（フェーズ0の動作）
    pub resolver: Option<ProfileResolver<'a>>,
    /// 指定すると、ケースの profile 列の代わりに全ケースへこの名前を使う
    pub forced_profile: Option<&'a str>,
    /// 指定すると、変換1回の締め切りを「開始 + この時間」にする（`convert_until` / `page_until`）。
    /// 先行計算が済んでいる状況（エンジンの Prefetch は最大 300ms を使う）の再現に使う。
    /// None なら `Converter::convert` / `page` の既定（`lm_budget`）。
    pub lm_deadline: Option<Duration>,
    /// `field` 列のあるケースの左文脈の組み立て（`--carry-fill`・`--carry-joint`。ADR 0046）。
    /// None なら足さない。`field` 列のないケースには影響しない
    pub carry: Option<CarryParams>,
}

impl RunOptions<'_> {
    fn profile_for(&self, case: &EvalCase) -> Option<Arc<Profile>> {
        let resolver = self.resolver?;
        let name = self.forced_profile.or(case.profile.as_deref())?;
        resolver(name)
    }
}

fn case_context(
    case: &EvalCase,
    profile: Option<Arc<Profile>>,
    carry: Option<CarryParams>,
) -> Context {
    Context::new()
        .with_left_text(left_text(&case.context, case.field, &case.history, carry))
        .with_right_text(case.right.as_str())
        .with_profile(profile)
}

/// `convert()` から `page(0)` が返るまでの時間を測る。
fn time_first_page(
    conv: &Converter,
    reading: &str,
    ctx: &Context,
    lm_deadline: Option<Duration>,
) -> (Duration, ikari_core::Conversion) {
    let start = Instant::now();
    let deadline = lm_deadline.map(|d| start + d);
    let mut conversion = match deadline {
        Some(d) => conv.convert_until(reading, ctx, Some(d)),
        None => conv.convert(reading, ctx),
    };
    let _ = match deadline {
        Some(d) => conversion.page_until(0, Some(d)),
        None => conversion.page(0),
    };
    (start.elapsed(), conversion)
}

/// 変換結果をページ順にたどり、`is_match` を満たす表記の順位（1始まり）と、1位の表記を返す。
/// `max_rank` 位まで探す。`deadline` が `Some` なら、ページの生成にその締め切りを使う。
pub(crate) fn find_rank_until(
    conversion: &mut ikari_core::Conversion,
    max_rank: usize,
    is_match: &dyn Fn(&str) -> bool,
    deadline: Option<Instant>,
) -> (Option<usize>, Option<String>) {
    let mut top1 = None;
    let mut seen = 0;
    let mut page = 0;
    while seen < max_rank {
        let page_result = match deadline {
            Some(d) => conversion.page_until(page, Some(d)),
            None => conversion.page(page),
        };
        let Some(p) = page_result else {
            break;
        };
        for cand in p.candidates {
            seen += 1;
            if top1.is_none() {
                top1 = Some(cand.surface.clone());
            }
            if is_match(&cand.surface) {
                return (Some(seen), top1);
            }
            if seen >= max_rank {
                return (None, top1);
            }
        }
        page += 1;
    }
    (None, top1)
}

/// プロファイルを付けずに各ケースを実行する（フェーズ0と同じ）。
pub fn run(conv: &Converter, cases: &[EvalCase], cfg: &EvalConfig) -> Vec<CaseResult> {
    run_with(conv, cases, cfg, &RunOptions::default())
}

/// 各ケースを実行する。初回を cold として記録し、続けて `cfg.repeat` 回の warm を測る。
/// 順位はページを順にたどって、表記が `expected`（別解のどれか）と完全一致する位置（`cfg.max_rank` まで）を探す。
pub fn run_with(
    conv: &Converter,
    cases: &[EvalCase],
    cfg: &EvalConfig,
    opts: &RunOptions<'_>,
) -> Vec<CaseResult> {
    cases
        .iter()
        .map(|case| {
            let ctx = case_context(case, opts.profile_for(case), opts.carry);
            let started = Instant::now();
            let (cold, mut conversion) =
                time_first_page(conv, &case.reading, &ctx, opts.lm_deadline);
            let warm = (0..cfg.repeat)
                .map(|_| time_first_page(conv, &case.reading, &ctx, opts.lm_deadline).0)
                .collect();
            let (rank, top1) = find_rank_until(
                &mut conversion,
                cfg.max_rank,
                &|s| case.matches(s),
                opts.lm_deadline.map(|d| started + d),
            );
            CaseResult {
                case: case.clone(),
                rank,
                top1,
                cold,
                warm,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_cases;
    use crate::testutil::{
        ProfileBoostRanker, mock_cases, mock_converter, mock_converter_with, test_profile,
    };

    #[test]
    fn run_finds_ranks_with_real_converter() {
        let cfg = EvalConfig {
            max_rank: 50,
            repeat: 3,
        };
        let results = run(&mock_converter(), &mock_cases(), &cfg);
        let ranks: Vec<_> = results.iter().map(|r| r.rank).collect();
        assert_eq!(ranks[0], Some(1));
        assert_eq!(ranks[1], Some(2));
        assert_eq!(ranks[2], None);
        assert!(ranks[3].is_some());
        assert!(results.iter().all(|r| r.top1.as_deref() == Some("品詞")));
        assert!(results.iter().all(|r| r.warm.len() == 3));
    }

    #[test]
    fn run_respects_max_rank() {
        let cfg = EvalConfig {
            max_rank: 1,
            repeat: 0,
        };
        let results = run(&mock_converter(), &mock_cases(), &cfg);
        assert_eq!(results[0].rank, Some(1));
        assert_eq!(results[1].rank, None);
        assert!(results[0].warm.is_empty());
    }

    #[test]
    fn run_matches_any_alternative() {
        let tsv = "id\tkind\tcontext\treading\texpected\n\
                   alt-1\tk\tc\tひんし\t存在しない|瀕死\n\
                   alt-2\tk\tc\tひんし\t瀕死|品詞\n";
        let cases = load_cases(tsv).unwrap();
        let cfg = EvalConfig {
            max_rank: 50,
            repeat: 0,
        };
        let r = run(&mock_converter(), &cases, &cfg);
        assert_eq!(r[0].rank, Some(2));
        // 別解のうち上位にある方で数える
        assert_eq!(r[1].rank, Some(1));
    }

    #[test]
    fn run_with_applies_case_profile_through_resolver() {
        let conv = mock_converter_with(Arc::new(ProfileBoostRanker { boost: "瀕死" }));
        let tsv = "id\tkind\tprofile\tcontext\treading\texpected\n\
                   p-1\t話題\tit\tc\tひんし\t瀕死\n\
                   p-2\t話題\t\tc\tひんし\t瀕死\n";
        let cases = load_cases(tsv).unwrap();
        let cfg = EvalConfig {
            max_rank: 50,
            repeat: 0,
        };
        let resolver = |name: &str| Some(test_profile(name));

        // resolver なし: プロファイルは付かない
        let r = run_with(&conv, &cases, &cfg, &RunOptions::default());
        assert_eq!((r[0].rank, r[1].rank), (Some(2), Some(2)));

        // case の profile 列だけが使われる
        let opts = RunOptions {
            resolver: Some(&resolver),
            forced_profile: None,
            ..RunOptions::default()
        };
        let r = run_with(&conv, &cases, &cfg, &opts);
        assert_eq!((r[0].rank, r[1].rank), (Some(1), Some(2)));

        // forced_profile は全ケースに使われる
        let opts = RunOptions {
            resolver: Some(&resolver),
            forced_profile: Some("novel"),
            ..RunOptions::default()
        };
        let r = run_with(&conv, &cases, &cfg, &opts);
        assert_eq!((r[0].rank, r[1].rank), (Some(1), Some(1)));

        // resolver が None を返す名前（未知のプロファイル）はプロファイルなし
        let none = |_: &str| None;
        let opts = RunOptions {
            resolver: Some(&none),
            forced_profile: None,
            ..RunOptions::default()
        };
        let r = run_with(&conv, &cases, &cfg, &opts);
        assert_eq!((r[0].rank, r[1].rank), (Some(2), Some(2)));
    }
}
