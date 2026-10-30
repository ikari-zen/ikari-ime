//! `cargo xtask perf engine`: P5 エンジンとコアの性能（ADR 0041 §6.3）。今の `scripts/check.sh --perf`（`run_perf`）を包む。
//!
//! 流すもの（`run_perf` と同じ）:
//! 1. `cargo test --release -p ikari-core -- --ignored --nocapture perf`（解析器、変換、郵便番号の表）
//! 2. `IKARI_DICT_DIR=… cargo test --release -p ikari-engine -- --ignored --nocapture segment_perf`（文節と再変換。p99 2ms 以下、LM なし）
//!
//! テストの出力の行から時間を読み、`--runs` 回の中央値と最小・最大を記録する。テストが落ちたら（予算を超えたら）誤りで終わる。
//! 辞書の場所は `IKARI_DICT_DIR`、なければ `data/build/dict`。読むだけで、書き換えない。

use std::path::PathBuf;
use std::process::Command;

use serde_json::json;

use super::record::{self, Collector, LoadWatch, err, repo_root};
use super::shell::print_table;
use crate::XtaskError;

const DEFAULT_RUNS: usize = 5;
const DICT_FILE: &str = "system.dict";
const DEFAULT_DICT_DIR: &str = "data/build/dict";

#[derive(Debug, clap::Args)]
pub struct Args {
    /// 計測をくり返す回数
    #[arg(long, default_value_t = DEFAULT_RUNS)]
    pub runs: usize,
    /// 結果の書き出し先（あれば、ほかの区間の指標を残して混ぜる）
    #[arg(long)]
    pub out: Option<PathBuf>,
}

/// 時間の表記（Rust の `Duration` の Debug: `1.5ms`、`12µs`、`345ns`、`2s`。と、`123us`）をナノ秒に。
pub fn parse_duration_ns(text: &str) -> Option<f64> {
    let text = text.trim_matches(|c: char| c == ',' || c == ';' || c.is_whitespace());
    let split = text.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (number, unit) = text.split_at(split);
    let number: f64 = number.parse().ok()?;
    let scale = match unit {
        "ns" => 1.0,
        "µs" | "us" => 1_000.0,
        "ms" => 1_000_000.0,
        "s" => 1_000_000_000.0,
        _ => return None,
    };
    Some((number * scale).round())
}

/// `line` の中の `key` の直後（`key` は `=` を含む）の時間。次の空白・`,`・`;` までを読む。
fn value_after(line: &str, key: &str) -> Option<f64> {
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .unwrap_or(rest.len());
    parse_duration_ns(&rest[..end])
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// テストの出力の 1 行から、指標（名前、ナノ秒）を取り出す。読めない行は空。
pub fn parse_line(line: &str) -> Vec<(String, f64)> {
    let mut found = Vec::new();
    let mut take = |name: String, value: Option<f64>| {
        if let Some(value) = value {
            found.push((name, value));
        }
    };
    if let Some(rest) = line.strip_prefix("segment_perf ") {
        if let Some((name, _)) = rest.split_once(':') {
            let name = sanitize(name);
            for key in ["p50", "p99", "max"] {
                take(
                    format!("p5.segment.{name}.{key}_ns"),
                    value_after(line, &format!("{key}=")),
                );
            }
        }
    } else if line.starts_with("perf_smoke:") {
        for key in ["p50", "p99", "max"] {
            take(
                format!("p5.convert.{key}_ns"),
                value_after(line, &format!("{key}=")),
            );
        }
    } else if line.starts_with("full(no cache)") {
        for (name, key) in [
            ("full_median", "median="),
            ("full_min", "min="),
            ("full_max", "max="),
            ("cache_hit", "hit="),
            ("sliding_append", "append="),
        ] {
            take(format!("p5.lattice.{name}_ns"), value_after(line, key));
        }
    } else if line.starts_with("text ") && line.contains(" lookups avg ") {
        take("p5.postal.load_ns".into(), value_after(line, "load "));
        take(
            "p5.postal.lookup_avg_ns".into(),
            value_after(line, "lookups avg "),
        );
    }
    found
}

/// 出力全体（標準出力と標準エラー）から指標を集める。
pub fn parse_output(output: &str) -> Vec<(String, f64)> {
    output.lines().flat_map(parse_line).collect()
}

fn run_tests(args: &[&str], dict_dir: &str) -> Result<String, XtaskError> {
    let output = Command::new("cargo")
        .current_dir(repo_root())
        .env("IKARI_DICT_DIR", dict_dir)
        .args(args)
        .output()
        .map_err(|error| err(format!("cargo test: 起動できません: {error}")))?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if !output.status.success() {
        return Err(err(format!(
            "cargo {} が失敗しました（予算を超えた可能性）:\n{text}",
            args.join(" ")
        )));
    }
    Ok(text)
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    if args.runs == 0 {
        return Err(err("--runs は 1 以上"));
    }
    let dict_dir = std::env::var("IKARI_DICT_DIR").unwrap_or_else(|_| {
        repo_root()
            .join(DEFAULT_DICT_DIR)
            .to_string_lossy()
            .into_owned()
    });
    if !std::path::Path::new(&dict_dir).join(DICT_FILE).is_file() {
        return Err(err(format!(
            "{dict_dir}/{DICT_FILE} がありません（IKARI_DICT_DIR で辞書の場所を渡す。読むだけ）"
        )));
    }

    let mut collector = Collector::default();
    let mut watch = LoadWatch::start();
    for run in 1..=args.runs {
        watch.sample();
        eprintln!("perf engine: {run}/{}", args.runs);
        let core = run_tests(
            &[
                "test",
                "--release",
                "-p",
                "ikari-core",
                "--",
                "--ignored",
                "--nocapture",
                "perf",
            ],
            &dict_dir,
        )?;
        let engine = run_tests(
            &[
                "test",
                "--release",
                "-p",
                "ikari-engine",
                "--",
                "--ignored",
                "--nocapture",
                "segment_perf",
            ],
            &dict_dir,
        )?;
        let mut seen = std::collections::BTreeMap::new();
        for (name, value) in parse_output(&core).into_iter().chain(parse_output(&engine)) {
            seen.insert(name, value);
        }
        if seen.is_empty() {
            return Err(err(
                "テストの出力から時間を読めませんでした（出力の形が変わった？）",
            ));
        }
        for (name, value) in seen {
            collector.push(name, "ns", value);
        }
    }

    let metrics = collector.into_metrics();
    print_table(&metrics);
    if let Some(out) = &args.out {
        let mut section = watch.finish();
        section["measured_at"] = record::now_utc().into();
        section["runs"] = args.runs.into();
        section["dict_dir_has_system_dict"] = json!(true);
        record::merge_into(out, "engine", section, metrics)?;
        eprintln!("perf engine: {} に書きました", out.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration_ns("1.5ms"), Some(1_500_000.0));
        assert_eq!(parse_duration_ns("12µs"), Some(12_000.0));
        assert_eq!(parse_duration_ns("123us,"), Some(123_000.0));
        assert_eq!(parse_duration_ns("345ns"), Some(345.0));
        assert_eq!(parse_duration_ns("2s"), Some(2e9));
        assert_eq!(parse_duration_ns("abc"), None);
    }

    #[test]
    fn parses_segment_perf() {
        let line = "segment_perf page (3 chars): n=40 p50=210us p99=900us max=1500us";
        let found = parse_line(line);
        assert_eq!(
            found,
            [
                ("p5.segment.page__3_chars_.p50_ns".to_owned(), 210_000.0),
                ("p5.segment.page__3_chars_.p99_ns".to_owned(), 900_000.0),
                ("p5.segment.page__3_chars_.max_ns".to_owned(), 1_500_000.0),
            ]
        );
    }

    #[test]
    fn parses_core_convert() {
        let found = parse_line("perf_smoke: 20 chars, p50=1.2ms p99=3.4ms max=5ms");
        assert_eq!(found.len(), 3);
        assert_eq!(found[1], ("p5.convert.p99_ns".to_owned(), 3_400_000.0));
    }

    #[test]
    fn parses_lattice() {
        let line = "full(no cache) median=4.1ms min=3.9ms max=5.2ms; cache hit=800ns; sliding append=1.5µs";
        let found: std::collections::BTreeMap<_, _> = parse_line(line).into_iter().collect();
        assert_eq!(found["p5.lattice.full_median_ns"], 4_100_000.0);
        assert_eq!(found["p5.lattice.cache_hit_ns"], 800.0);
        assert_eq!(found["p5.lattice.sliding_append_ns"], 1_500.0);
    }

    #[test]
    fn parses_postal() {
        let line = "text 1000 bytes, rows 500, load 1.2ms, 10 lookups avg 3.5µs, hits 9";
        let found: std::collections::BTreeMap<_, _> = parse_line(line).into_iter().collect();
        assert_eq!(found["p5.postal.load_ns"], 1_200_000.0);
        assert_eq!(found["p5.postal.lookup_avg_ns"], 3_500.0);
    }

    #[test]
    fn ignores_other_lines() {
        assert!(parse_line("test result: ok. 1 passed").is_empty());
    }
}
