//! `cargo xtask perf shell`: P1（キー 1 回）、P3（殻の起動）、P4（メモリ）（ADR 0041 §6.3）。
//!
//! 共通のシナリオ（`shell-core/scenarios/`）を替え玉のエンジンで再生する測り手を呼ぶ。
//! - Rust 版: `shell-core/benches/shell_perf.rs`。`Shell` が stub の間（`"stub": true`）は、結果に入れない
//! - Swift 版: 測り手（`IkariShellBench`）は移行の片付け（M5）で Swift の状態機械と一緒に消した。もう測れない。
//!   `eval/reports/shell-perf/baseline.json` の `swift.*` は、移行の前に取った基準値として固定で残す（`perf compare` の既定の比較相手）
//!
//! 1 回の計測 = P1 の測り手を 1 プロセス（キー全部を `--repeat` 回）+ P3 の測り手を 1 プロセス（初期化を `--repeat` 回）。
//! それを `--runs` 回（既定 5）くり返し、指標ごとに中央値と最小・最大を記録する。P4 は P1 のプロセスの最大 RSS。

use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};

use super::record::{self, Collector, LoadWatch, capture, err, repo_root};
use crate::XtaskError;

pub const DEFAULT_RUNS: usize = 5;
pub const DEFAULT_REPEAT: usize = 100;
pub const DEFAULT_SHORTCODE_ENTRIES: usize = 2000;

/// 指標の名前の接頭辞（`swift.`・`rust.`）になる実装。`Swift` は基準値のファイルを読むときの名前としてだけ残す
/// （測り手は M5 で消した。`perf shell --impl swift` は誤りになる）
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Implementation {
    Swift,
    Rust,
}

impl Implementation {
    fn name(self) -> &'static str {
        match self {
            Self::Swift => "swift",
            Self::Rust => "rust",
        }
    }
}

#[derive(Debug, clap::Args)]
pub struct Args {
    /// 測る実装（既定は rust。swift の測り手は M5 で消したので、指定すると誤りになる）
    #[arg(long = "impl", value_enum)]
    pub implementations: Vec<Implementation>,
    /// 計測をくり返す回数（指標ごとに中央値と最小・最大を記録する）
    #[arg(long, default_value_t = DEFAULT_RUNS)]
    pub runs: usize,
    /// 1 回の計測の中で、シナリオ全部（P1）と初期化（P3）をくり返す回数
    #[arg(long, default_value_t = DEFAULT_REPEAT)]
    pub repeat: usize,
    /// P3 のショートコードの表の大きさ
    #[arg(long, default_value_t = DEFAULT_SHORTCODE_ENTRIES)]
    pub shortcode_entries: usize,
    /// 結果の書き出し先（あれば、ほかの区間の指標を残して混ぜる）。なければ標準出力に表を出すだけ
    #[arg(long)]
    pub out: Option<PathBuf>,
}

/// 測り手の 1 回の出力（JSON）から取り出した値。
#[derive(Debug, PartialEq)]
pub struct BenchResult {
    pub p50_ns: f64,
    pub p99_ns: f64,
    pub max_ns: f64,
    pub max_rss_bytes: Option<f64>,
    pub stub: bool,
}

/// 測り手の標準出力（最後の JSON の行）を読む。
pub fn parse_bench_output(stdout: &str) -> Result<BenchResult, XtaskError> {
    let line = stdout
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with('{'))
        .ok_or_else(|| err(format!("測り手の出力に JSON がありません: {stdout:?}")))?;
    let value: Value = serde_json::from_str(line).map_err(|error| {
        err(format!(
            "測り手の出力が JSON ではありません: {error}: {line}"
        ))
    })?;
    let number = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_f64)
            .ok_or_else(|| err(format!("測り手の出力に {key} がありません: {line}")))
    };
    Ok(BenchResult {
        p50_ns: number("p50_ns")?,
        p99_ns: number("p99_ns")?,
        max_ns: number("max_ns")?,
        max_rss_bytes: value.get("max_rss_bytes").and_then(Value::as_f64),
        stub: value.get("stub").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// Rust 版の測り手を呼ぶ口。
struct Bench;

impl Bench {
    fn prepare(implementation: Implementation) -> Result<Self, XtaskError> {
        if implementation == Implementation::Swift {
            return Err(swift_bench_removed());
        }
        // ビルドは先に済ませる（計測の中に入れない）
        capture(
            Command::new("cargo").current_dir(repo_root()).args([
                "bench",
                "-p",
                "ikari-shell-core",
                "--bench",
                "shell_perf",
                "--no-run",
            ]),
            "cargo bench --no-run",
        )?;
        Ok(Self)
    }

    fn run(&self, metric: &str, args: &Args) -> Result<BenchResult, XtaskError> {
        let repeat = args.repeat.to_string();
        let entries = args.shortcode_entries.to_string();
        let stdout = capture(
            Command::new("cargo").current_dir(repo_root()).args([
                "bench",
                "-p",
                "ikari-shell-core",
                "--bench",
                "shell_perf",
                "--",
                "--metric",
                metric,
                "--repeat",
                &repeat,
                "--shortcode-entries",
                &entries,
            ]),
            "shell_perf",
        )?;
        parse_bench_output(&stdout)
    }
}

/// Swift 版の測り手はもうない（M5）。基準値は `baseline.json` の `swift.*` に固定で残っている
fn swift_bench_removed() -> XtaskError {
    err(
        "Swift 版の測り手（IkariShellBench）は M5 で消したので測れません。基準値は \
         eval/reports/shell-perf/baseline.json の swift.* に固定で残っています（--impl rust を使う）",
    )
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    if args.runs == 0 || args.repeat == 0 {
        return Err(err("--runs と --repeat は 1 以上"));
    }
    let implementations = if args.implementations.is_empty() {
        vec![Implementation::Rust]
    } else {
        args.implementations.clone()
    };
    if implementations.contains(&Implementation::Swift) {
        return Err(swift_bench_removed());
    }

    let mut collector = Collector::default();
    let mut skipped = Vec::new();
    let mut watch = LoadWatch::start();
    for implementation in implementations {
        let bench = Bench::prepare(implementation)?;
        let name = implementation.name();
        let mut stub = false;
        for run in 1..=args.runs {
            watch.sample();
            eprintln!("perf shell: {name} {run}/{}", args.runs);
            let p1 = bench.run("p1", &args)?;
            let p3 = bench.run("p3", &args)?;
            stub |= p1.stub || p3.stub;
            if stub {
                break;
            }
            for (metric, result) in [("p1", &p1), ("p3", &p3)] {
                collector.push(format!("{name}.{metric}.p50_ns"), "ns", result.p50_ns);
                collector.push(format!("{name}.{metric}.p99_ns"), "ns", result.p99_ns);
                collector.push(format!("{name}.{metric}.max_ns"), "ns", result.max_ns);
            }
            // P4: P1 のプロセスの最大 RSS
            if let Some(rss) = p1.max_rss_bytes {
                collector.push(format!("{name}.p4.max_rss_bytes"), "bytes", rss);
            }
        }
        if stub {
            eprintln!("perf shell: {name} は stub（動作を返さない）なので、結果に入れません");
            skipped.push(name);
        }
    }

    let metrics = collector.into_metrics();
    print_table(&metrics);
    if let Some(out) = &args.out {
        let mut section = watch.finish();
        section["measured_at"] = record::now_utc().into();
        section["runs"] = args.runs.into();
        section["repeat"] = args.repeat.into();
        section["shortcode_entries"] = args.shortcode_entries.into();
        section["skipped_stub"] = json!(skipped);
        record::merge_into(out, "shell", section, metrics)?;
        eprintln!("perf shell: {} に書きました", out.display());
    }
    Ok(())
}

pub fn print_table(metrics: &serde_json::Map<String, Value>) {
    println!(
        "{:<28} {:>14} {:>14} {:>14}",
        "metric", "median", "min", "max"
    );
    for (name, value) in metrics {
        let unit = value["unit"].as_str().unwrap_or("");
        let cell = |key: &str| format_value(value[key].as_f64().unwrap_or(f64::NAN), unit);
        println!(
            "{name:<28} {:>14} {:>14} {:>14}",
            cell("median"),
            cell("min"),
            cell("max")
        );
    }
}

/// 単位つきの短い表記（ns は µs・ms に、bytes は MiB に）。
pub fn format_value(value: f64, unit: &str) -> String {
    match unit {
        "ns" if value >= 1_000_000.0 => format!("{:.3} ms", value / 1_000_000.0),
        "ns" if value >= 1_000.0 => format!("{:.2} us", value / 1_000.0),
        "ns" => format!("{value:.0} ns"),
        "bytes" => format!("{:.2} MiB", value / (1024.0 * 1024.0)),
        "" => format!("{value}"),
        other => format!("{value:.3} {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_last_json_line() {
        let output = "sink=1\n{\"p50_ns\":500,\"p99_ns\":3041,\"max_ns\":112292,\"max_rss_bytes\":12419072}\n";
        let result = parse_bench_output(output).unwrap();
        assert_eq!(result.p99_ns, 3041.0);
        assert_eq!(result.max_rss_bytes, Some(12419072.0));
        assert!(!result.stub);
    }

    #[test]
    fn stub_flag_is_read() {
        let result =
            parse_bench_output("{\"p50_ns\":1,\"p99_ns\":2,\"max_ns\":3,\"stub\":true}").unwrap();
        assert!(result.stub);
    }

    #[test]
    fn missing_fields_are_errors() {
        assert!(parse_bench_output("{\"p50_ns\":1}").is_err());
        assert!(parse_bench_output("no json").is_err());
    }

    #[test]
    fn the_swift_bench_is_gone() {
        let args = Args {
            implementations: vec![Implementation::Swift],
            runs: 1,
            repeat: 1,
            shortcode_entries: 1,
            out: None,
        };
        let message = run(args).unwrap_err().to_string();
        assert!(message.contains("M5"), "{message}");
    }

    #[test]
    fn formats_units() {
        assert_eq!(format_value(500.0, "ns"), "500 ns");
        assert_eq!(format_value(3041.0, "ns"), "3.04 us");
        assert_eq!(format_value(2_500_000.0, "ns"), "2.500 ms");
        assert_eq!(format_value(1024.0 * 1024.0 * 12.0, "bytes"), "12.00 MiB");
    }
}
