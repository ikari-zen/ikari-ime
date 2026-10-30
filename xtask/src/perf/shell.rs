//! `cargo xtask perf shell`: P1（キー 1 回）、P3（殻の起動）、P4（メモリ）（ADR 0041 §6.3）。
//!
//! 共通のシナリオ（`shell-core/scenarios/`）を替え玉のエンジンで再生する測り手を、実装ごとに呼ぶ。
//! - Swift 版: `macos/Sources/IkariShellBench/`（macOS のみ。移行の間だけ。M5 で消す）。`swift build` で作って流す
//! - Rust 版: `shell-core/benches/shell_perf.rs`。`Shell` が stub の間（`"stub": true`）は、結果に入れない
//!
//! 1 回の計測 = P1 の測り手を 1 プロセス（キー全部を `--repeat` 回）+ P3 の測り手を 1 プロセス（初期化を `--repeat` 回）。
//! それを `--runs` 回（既定 5）くり返し、指標ごとに中央値と最小・最大を記録する。P4 は P1 のプロセスの最大 RSS。

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use super::record::{self, Collector, LoadWatch, capture, err, repo_root};
use crate::XtaskError;

pub const DEFAULT_RUNS: usize = 5;
pub const DEFAULT_REPEAT: usize = 100;
pub const DEFAULT_SHORTCODE_ENTRIES: usize = 2000;

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
    /// 測る実装（くり返して指定。既定は両方。Swift は macOS のみ）
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

/// 実装ごとの測り手を呼ぶ口。
struct Bench {
    implementation: Implementation,
    swift_binary: Option<PathBuf>,
}

impl Bench {
    fn prepare(implementation: Implementation) -> Result<Self, XtaskError> {
        let swift_binary = match implementation {
            Implementation::Swift => Some(build_swift_bench()?),
            Implementation::Rust => {
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
                None
            }
        };
        Ok(Self {
            implementation,
            swift_binary,
        })
    }

    fn run(&self, metric: &str, args: &Args) -> Result<BenchResult, XtaskError> {
        let repeat = args.repeat.to_string();
        let entries = args.shortcode_entries.to_string();
        let stdout = match (&self.swift_binary, self.implementation) {
            (Some(binary), Implementation::Swift) => capture(
                Command::new(binary).args([
                    "--metric",
                    metric,
                    "--scenarios",
                    &repo_root().join("shell-core/scenarios").to_string_lossy(),
                    "--resources",
                    &repo_root().join("macos/Resources").to_string_lossy(),
                    "--repeat",
                    &repeat,
                    "--shortcode-entries",
                    &entries,
                ]),
                "ikari-shell-bench",
            )?,
            _ => capture(
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
            )?,
        };
        parse_bench_output(&stdout)
    }
}

/// Swift 版の測り手をリリースでビルドし、実行ファイルの場所を返す。
/// `Doubles.swift` が `@testable import` を使うので、`-enable-testing` を付ける（最適化は効いたまま）。
fn build_swift_bench() -> Result<PathBuf, XtaskError> {
    if !cfg!(target_os = "macos") {
        return Err(err(
            "Swift 版の測り手は macOS でだけ流せます（--impl rust を使う）",
        ));
    }
    let macos = repo_root().join("macos");
    let build = |extra: &[&str]| {
        let mut command = Command::new("swift");
        command.current_dir(&macos).args([
            "build",
            "-c",
            "release",
            "--product",
            "ikari-shell-bench",
            "-Xswiftc",
            "-enable-testing",
        ]);
        command.args(extra);
        capture(&mut command, "swift build")
    };
    build(&[])?;
    let bin_path = build(&["--show-bin-path"])?;
    let binary = Path::new(bin_path.trim()).join("ikari-shell-bench");
    if !binary.exists() {
        return Err(err(format!("{} がありません", binary.display())));
    }
    Ok(binary)
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    if args.runs == 0 || args.repeat == 0 {
        return Err(err("--runs と --repeat は 1 以上"));
    }
    let implementations = if args.implementations.is_empty() {
        if cfg!(target_os = "macos") {
            vec![Implementation::Swift, Implementation::Rust]
        } else {
            vec![Implementation::Rust]
        }
    } else {
        args.implementations.clone()
    };

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
    fn formats_units() {
        assert_eq!(format_value(500.0, "ns"), "500 ns");
        assert_eq!(format_value(3041.0, "ns"), "3.04 us");
        assert_eq!(format_value(2_500_000.0, "ns"), "2.500 ms");
        assert_eq!(format_value(1024.0 * 1024.0 * 12.0, "bytes"), "12.00 MiB");
    }
}
