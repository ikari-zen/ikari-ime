//! `ikari-eval`: 評価セットを実行して Markdown のレポートを stdout に出す（ADR 0001 §6.2、ADR 0002 §15）。
//!
//! 終了コード: 0 = 成功、1 = 基準を満たさない（`--min-top1`、`--fail-on-regression`、`--profile-ab` の
//! 回帰、`--scenario` の失敗）、2 = 実行できなかった（ファイルが読めない、設定が不正など）。

use std::collections::HashSet;
use std::error::Error;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use ikari_core::user::today_unix_day;
use ikari_core::{Grammar, Profile, open_dir};
use ikari_eval::ab::{render_profile_ab, run_profile_ab};
use ikari_eval::ajimee::load_ajimee;
use ikari_eval::boundary::{evaluate_boundaries, render_boundaries};
use ikari_eval::compare::{compare_tables, load_table, render_comparison};
use ikari_eval::gates::{
    check_min_top1, find_regressions, load_baseline, parse_threshold, render_regressions,
};
use ikari_eval::holdout::{DEFAULT_CONTEXT_CHARS, holdout_cases, read_holdout};
use ikari_eval::ops::{OpsCase, OpsConfig, evaluate_ops, load_ops_cases, render_ops, worse_cases};
use ikari_eval::reach::{mark_unreachable, unreachable_ids};
use ikari_eval::reading::{evaluate_readings, pairs_from_cases, render_readings};
use ikari_eval::scenario::{ConverterHost, load_scenario, render_scenario, run_scenario};
use ikari_eval::segment_engine::CoreSegmentEngine;
use ikari_eval::setup::{
    ProfileChoice, Setup, SetupOptions, build, default_postal_path, default_vectors_path,
    preset_profile,
};
use ikari_eval::{
    EvalCase, EvalConfig, ReportMeta, RunOptions, SegmentCase, SplitSel, filter_cases, load_cases,
    load_segment_cases, render_cases_tsv, render_markdown, rss, run_with, summarize,
};
use ikari_stack::RankerSet;

mod args;

use args::{
    Cli, Command, EngineArgs, OpsArgs, ReachArgs, ReadingArgs, RunArgs, SegmentsArgs, SetFormat,
    SplitArg,
};

const DICT_DIR_ENV: &str = "IKARI_DICT_DIR";
const DEFAULT_DICT_DIR: &str = "data/build/dict";
const DEFAULT_SET_PATH: &str = "eval/sets/homophones.tsv";
const DEFAULT_AJIMEE_PATH: &str = "data/raw/ajimee/evaluation_items.json";
const META_SOURCE_KEY: &str = "source=";
const UNKNOWN_SOURCE: &str = "(unknown)";
/// `reading` の既定のセット（表記と読みの組が取れる既存のセット）
const DEFAULT_READING_SETS: [&str; 4] = [
    "eval/sets/homophones.tsv",
    "eval/sets/homophones-ext.tsv",
    "eval/sets/hiragana.tsv",
    "eval/sets/katakana-suffix.tsv",
];
/// 候補ウィンドウの番号キーの数（`ConvertOptions.page_size` の上限）
const MAX_PAGE_SIZE: usize = 9;
const EXIT_GATE_FAILED: u8 = 1;
const EXIT_ERROR: u8 = 2;

fn dict_dir(arg: Option<PathBuf>) -> PathBuf {
    arg.or_else(|| std::env::var_os(DICT_DIR_ENV).map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DICT_DIR))
}

/// 辞書の meta（`key=value\n`）から source を取り出す
fn source_of(meta: &str) -> String {
    meta.lines()
        .find_map(|l| l.strip_prefix(META_SOURCE_KEY))
        .unwrap_or(UNKNOWN_SOURCE)
        .to_owned()
}

fn read_file(path: &std::path::Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn main() -> ExitCode {
    match run_main() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

fn run_main() -> Result<ExitCode, Box<dyn Error>> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Compare { a, b }) => {
            let ta = load_table(&read_file(&a)?).map_err(|e| format!("{}: {e}", a.display()))?;
            let tb = load_table(&read_file(&b)?).map_err(|e| format!("{}: {e}", b.display()))?;
            print!(
                "{}",
                render_comparison(
                    &compare_tables(&ta, &tb),
                    &a.display().to_string(),
                    &b.display().to_string()
                )
            );
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Reach(args)) => run_reach(args),
        Some(Command::Segments(args)) => run_segments(args),
        Some(Command::Ops(args)) => run_ops(args),
        Some(Command::Reading(args)) => run_reading(args),
        None => run_eval(cli.run),
    }
}

fn open_setup(
    dict_dir: &std::path::Path,
    args: &RunArgs,
    description_suffix: &mut String,
) -> Result<(Setup, String, u64), Box<dyn Error>> {
    let loaded = open_dir(dict_dir)
        .map_err(|e| format!("cannot open dictionary {}: {e}", dict_dir.display()))?;
    let source = source_of(loaded.dict.meta());
    let grammar_id = loaded.grammar.grammar_id();
    let opts = SetupOptions {
        ranker: Some(args.ranker),
        no_analyzer: args.no_analyzer,
        vectors: args.vectors.clone(),
        default_vectors: Some(default_vectors_path(dict_dir)),
        ranker_config: args.ranker_config.clone(),
        lm: args.lm.clone(),
        lm_budget_ms: args.lm_budget_ms,
        user_dir: args.user_dir.clone(),
        clock: None,
        packs: !args.no_packs,
        postal_tsv: Some(
            args.postal
                .clone()
                .unwrap_or_else(|| default_postal_path(dict_dir)),
        ),
    };
    let setup = build(loaded.dict, loaded.grammar, &opts)?;
    description_suffix.push_str(&setup.description);
    Ok((setup, source, grammar_id))
}

fn load_case_set(
    args: &RunArgs,
    dict_dir: &std::path::Path,
) -> Result<(Vec<EvalCase>, String), Box<dyn Error>> {
    if let Some(dir) = &args.ingest_holdout {
        // アライメントはシステム辞書で行う（取り込み時のユーザー辞書の語は未知語になり、その文は除かれる）
        let loaded = open_dir(dict_dir)
            .map_err(|e| format!("cannot open dictionary {}: {e}", dict_dir.display()))?;
        let sentences = read_holdout(dir, &*loaded.dict, &*loaded.grammar)?;
        return Ok((
            holdout_cases(&sentences, DEFAULT_CONTEXT_CHARS),
            format!("ingest-holdout:{}", dir.display()),
        ));
    }
    let (path, loaded) = match args.set_format {
        SetFormat::Ikari => {
            let p = args
                .set
                .clone()
                .unwrap_or_else(|| PathBuf::from(DEFAULT_SET_PATH));
            let cases = load_cases(&read_file(&p)?)?;
            (p, cases)
        }
        SetFormat::Ajimee => {
            let p = args
                .set
                .clone()
                .unwrap_or_else(|| PathBuf::from(DEFAULT_AJIMEE_PATH));
            let cases =
                load_ajimee(&read_file(&p)?).map_err(|e| format!("{}: {e}", p.display()))?;
            (p, cases)
        }
    };
    Ok((loaded, path.display().to_string()))
}

fn run_eval(args: RunArgs) -> Result<ExitCode, Box<dyn Error>> {
    let is_debug = cfg!(debug_assertions);
    if is_debug {
        eprintln!(
            "warning: debug ビルドです。応答時間は参考値です（--release で実行してください）"
        );
    }
    let min_top1 = args
        .min_top1
        .as_deref()
        .map(parse_threshold)
        .transpose()
        .map_err(|e| format!("--min-top1: {e}"))?;
    let baseline = match &args.baseline {
        Some(p) => {
            Some(load_baseline(&read_file(p)?).map_err(|e| format!("{}: {e}", p.display()))?)
        }
        None => None,
    };

    // シナリオは自前のステップを持つので、評価セット（既定は cwd 相対のパス）は読まない
    let dir = dict_dir(args.dict.clone());
    let (all_cases, set_label) = if args.scenario.is_some() {
        (Vec::new(), String::new())
    } else {
        load_case_set(&args, &dir)?
    };
    let split = match args.split {
        SplitArg::Dev => SplitSel::Dev,
        SplitArg::Test => SplitSel::Test,
        SplitArg::All => SplitSel::All,
    };
    let filtered = filter_cases(all_cases, split, args.include_unreachable);

    let mut config_line = String::new();
    let (setup, source, grammar_id) = open_setup(&dir, &args, &mut config_line)?;

    let profile = match args.profile.as_deref() {
        None => ProfileChoice::PerCase,
        Some(p) => ProfileChoice::parse(p),
    };
    // 同梱プリセットの読み込みは、必要なときだけ行う（cost のときはケースの profile 列を使わない）
    let use_profiles = match (&profile, args.ranker) {
        (ProfileChoice::None, _) => false,
        (ProfileChoice::Named(_), _) => true,
        (ProfileChoice::PerCase, RankerSet::Full) => true,
        (ProfileChoice::PerCase, _) => false,
    } || args.profile_ab;
    let resolver = |name: &str| -> Option<Arc<Profile>> { preset_profile(name) };
    let forced = match &profile {
        ProfileChoice::Named(n) => Some(n.as_str()),
        _ => None,
    };
    let profile_label = match &profile {
        ProfileChoice::None => "none".to_owned(),
        ProfileChoice::Named(n) => format!("forced:{n}"),
        ProfileChoice::PerCase if use_profiles => "per-case".to_owned(),
        ProfileChoice::PerCase => "off".to_owned(),
    };
    config_line.push_str(&format!(" profile={profile_label}"));

    let cfg = EvalConfig {
        max_rank: args.max_rank,
        repeat: args.repeat,
    };

    if let Some(path) = &args.scenario {
        return run_scenario_mode(&args, &setup, path, &cfg);
    }

    let opts = RunOptions {
        resolver: use_profiles.then_some(&resolver as ikari_eval::ProfileResolver<'_>),
        forced_profile: forced,
        lm_deadline: args.lm_deadline_ms.map(std::time::Duration::from_millis),
        carry: Some(args.carry_params()),
    };
    let results = run_with(&setup.converter, &filtered.cases, &cfg, &opts);
    let summary = summarize(&results);
    let meta = ReportMeta {
        set_path: set_label,
        dict_source: source,
        grammar_id,
        ranker: setup.ranker_name.clone(),
        build: if is_debug { "debug" } else { "release" }.to_owned(),
        repeat: cfg.repeat,
        max_rank: cfg.max_rank,
        config: config_line,
        split: split.as_str().to_owned(),
        excluded_unreachable: filtered.excluded_unreachable,
        max_rss_bytes: rss::max_rss_bytes(),
        lm: setup.lm_report.as_ref().map(|f| f()),
    };
    print!("{}", render_markdown(&summary, &results, &meta));

    let mut failures: Vec<String> = Vec::new();

    if args.profile_ab {
        let ab = run_profile_ab(&setup.converter, &filtered.cases, &cfg, &resolver, forced);
        print!("{}", render_profile_ab(&ab));
        let guards = ab.guard_regressions();
        if !guards.is_empty() {
            failures.push(format!(
                "profile-ab: {} negative/grammar case(s) got worse with a profile: {}",
                guards.len(),
                guards
                    .iter()
                    .map(|g| g.case.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    if let Some(b) = &baseline {
        let report = find_regressions(b, &results);
        print!("{}", render_regressions(&report));
        if args.fail_on_regression && !report.regressions.is_empty() {
            failures.push(format!(
                "regression: {} case(s) were first in the baseline but are not now: {}",
                report.regressions.len(),
                report
                    .regressions
                    .iter()
                    .map(|r| r.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    if let Some(path) = &args.out_cases {
        std::fs::write(path, render_cases_tsv(&results))
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }

    if let Some(min) = min_top1
        && let Err(m) = check_min_top1(&summary.overall, min)
    {
        failures.push(m);
    }

    if failures.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        for f in &failures {
            eprintln!("FAIL: {f}");
        }
        Ok(ExitCode::from(EXIT_GATE_FAILED))
    }
}

fn run_scenario_mode(
    args: &RunArgs,
    setup: &Setup,
    path: &std::path::Path,
    cfg: &EvalConfig,
) -> Result<ExitCode, Box<dyn Error>> {
    let steps = load_scenario(&read_file(path)?).map_err(|e| format!("{}: {e}", path.display()))?;
    let store = setup.store.clone();
    let vectors = setup.vectors.clone();
    let today = today_unix_day();
    let mut host = ConverterHost {
        converter: &setup.converter,
        lm_deadline: args.lm_deadline_ms.map(std::time::Duration::from_millis),
        carry: Some(args.carry_params()),
        on_commit: move |ev| {
            if let Some(store) = &store {
                store.record(&ev, vectors.as_deref(), today);
            }
        },
    };
    if setup.store.is_none() {
        eprintln!(
            "warning: --ranker cost has no learning, so commit steps are not recorded (use --ranker full)"
        );
    }
    let report = run_scenario(&mut host, &steps, cfg.max_rank);
    print!("{}", render_scenario(&report));
    let failed = report.failures();
    if failed.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        eprintln!(
            "FAIL: scenario: {} step(s) failed (lines {})",
            failed.len(),
            failed
                .iter()
                .map(|o| o.step.line.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(ExitCode::from(EXIT_GATE_FAILED))
    }
}

/// 文節の評価の subcommand が使う構成を作る（辞書を開いて Converter を組む）
fn open_engine_setup(args: &EngineArgs) -> Result<Setup, Box<dyn Error>> {
    let dir = dict_dir(args.dict.clone());
    let loaded =
        open_dir(&dir).map_err(|e| format!("cannot open dictionary {}: {e}", dir.display()))?;
    let opts = SetupOptions {
        ranker: Some(args.ranker),
        no_analyzer: args.no_analyzer,
        vectors: args.vectors.clone(),
        default_vectors: Some(default_vectors_path(&dir)),
        ranker_config: args.ranker_config.clone(),
        lm: args.lm.clone(),
        lm_budget_ms: args.lm_budget_ms,
        user_dir: args.user_dir.clone(),
        clock: None,
        packs: !args.no_packs,
        postal_tsv: Some(
            args.postal
                .clone()
                .unwrap_or_else(|| default_postal_path(&dir)),
        ),
    };
    Ok(build(loaded.dict, loaded.grammar, &opts)?)
}

fn split_sel(arg: SplitArg) -> SplitSel {
    match arg {
        SplitArg::Dev => SplitSel::Dev,
        SplitArg::Test => SplitSel::Test,
        SplitArg::All => SplitSel::All,
    }
}

fn run_segments(args: SegmentsArgs) -> Result<ExitCode, Box<dyn Error>> {
    let sel = split_sel(args.split);
    let cases: Vec<SegmentCase> = load_segment_cases(&read_file(&args.set)?)
        .map_err(|e| format!("{}: {e}", args.set.display()))?
        .into_iter()
        .filter(|c| sel.includes(c.case.split))
        .collect();
    let setup = open_engine_setup(&args.engine)?;
    let engine = CoreSegmentEngine::new(
        &setup.converter,
        args.engine.lm_deadline_ms.map(Duration::from_millis),
    );
    println!(
        "# 文節の区切り: {}（split={}、ranker={}）\n",
        args.set.display(),
        sel.as_str(),
        setup.ranker_name
    );
    let results = evaluate_boundaries(&engine, &cases);
    print!("{}", render_boundaries(&results, &cases));
    Ok(ExitCode::SUCCESS)
}

fn run_ops(args: OpsArgs) -> Result<ExitCode, Box<dyn Error>> {
    if !(1..=MAX_PAGE_SIZE).contains(&args.page_size) {
        return Err(format!("--page-size must be 1..={MAX_PAGE_SIZE}").into());
    }
    let sel = split_sel(args.split);
    let cases: Vec<OpsCase> = load_ops_cases(&read_file(&args.set)?)
        .map_err(|e| format!("{}: {e}", args.set.display()))?
        .into_iter()
        .filter(|c| sel.includes(c.split))
        .collect();
    let setup = open_engine_setup(&args.engine)?;
    let engine = CoreSegmentEngine::new(
        &setup.converter,
        args.engine.lm_deadline_ms.map(Duration::from_millis),
    );
    let cfg = OpsConfig {
        max_rank: args.max_rank,
        page_size: args.page_size,
        max_keys: args.max_keys,
        ..OpsConfig::default()
    };
    println!(
        "# 打鍵数: {}（split={}、ranker={}、max_rank={}、page_size={}）\n",
        args.set.display(),
        sel.as_str(),
        setup.ranker_name,
        cfg.max_rank,
        cfg.page_size
    );
    let results = evaluate_ops(&engine, &cases, &cfg);
    print!("{}", render_ops(&results));
    let worse = worse_cases(&results);
    if args.fail_on_worse && !worse.is_empty() {
        eprintln!(
            "FAIL: ops: {} case(s) need more keys with segments: {}",
            worse.len(),
            worse
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Ok(ExitCode::from(EXIT_GATE_FAILED));
    }
    Ok(ExitCode::SUCCESS)
}

fn run_reading(args: ReadingArgs) -> Result<ExitCode, Box<dyn Error>> {
    let sel = split_sel(args.split);
    let paths: Vec<PathBuf> = if args.sets.is_empty() {
        DEFAULT_READING_SETS.iter().map(PathBuf::from).collect()
    } else {
        args.sets.clone()
    };
    let mut pairs = Vec::new();
    for path in &paths {
        let cases: Vec<EvalCase> = load_cases(&read_file(path)?)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .into_iter()
            .filter(|c| sel.includes(c.split))
            .collect();
        let label = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        pairs.extend(pairs_from_cases(&label, &cases));
    }
    let setup = open_engine_setup(&args.engine)?;
    let engine = CoreSegmentEngine::new(
        &setup.converter,
        args.engine.lm_deadline_ms.map(Duration::from_millis),
    );
    println!(
        "# 再変換の読みの推定（split={}、ranker={}）\n",
        sel.as_str(),
        setup.ranker_name
    );
    let results = evaluate_readings(&engine, &pairs);
    print!("{}", render_readings(&results, args.list));
    Ok(ExitCode::SUCCESS)
}

fn run_reach(args: ReachArgs) -> Result<ExitCode, Box<dyn Error>> {
    let tsv = read_file(&args.set)?;
    let cases = load_cases(&tsv).map_err(|e| format!("{}: {e}", args.set.display()))?;
    let dir = dict_dir(args.dict);
    let loaded =
        open_dir(&dir).map_err(|e| format!("cannot open dictionary {}: {e}", dir.display()))?;
    let setup = build(loaded.dict, loaded.grammar, &SetupOptions::default())?;
    let cfg = EvalConfig {
        max_rank: args.max_rank,
        repeat: 0,
    };
    let results = run_with(&setup.converter, &cases, &cfg, &RunOptions::default());
    let ids = unreachable_ids(&results);
    println!(
        "{} of {} cases do not have the expected surface within the top {}",
        ids.len(),
        results.len(),
        cfg.max_rank
    );
    for r in results.iter().filter(|r| r.rank.is_none()) {
        println!(
            "{}\t{}\t{}\t{}",
            r.case.id, r.case.reading, r.case.expected, r.case.context
        );
    }
    if args.write {
        let set: HashSet<String> = ids.into_iter().collect();
        let out = mark_unreachable(&tsv, &set)?;
        std::fs::write(&args.set, out)
            .map_err(|e| format!("cannot write {}: {e}", args.set.display()))?;
        eprintln!(
            "marked {} case(s) as unreachable in {}",
            set.len(),
            args.set.display()
        );
    }
    Ok(ExitCode::SUCCESS)
}
