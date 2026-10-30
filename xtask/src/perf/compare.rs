//! `cargo xtask perf compare --baseline <file>`: 基準値との比較の表（ADR 0041 §6.3）。
//!
//! - 比べる値は各指標の中央値（5 回流した中央値。ばらつきの最小・最大も表に出す）
//! - 許す幅は `eval/reports/shell-perf/tolerance.toml`（`--tolerance` で変えられる）。**許す幅はユーザーが基準値を見て決める**ので、
//!   ファイルがなければ表を出すだけで終了コード 0。ファイルがあって、指標が幅を超えたら 1
//! - 指標の名前は `<実装>.<指標>`。基準値の実装（`--baseline-impl`。既定 swift）と今の実装（`--current-impl`。既定 rust）の
//!   同じ指標を比べる。実装を持たない指標（`p2.*`、`p5.*`）は同じ名前どうしを比べる
//!   Swift 版の測り手は M5 で消したので、`swift.*` は `baseline.json` に固定で残る移行の前の値だけ（取り直せない）
//! - `--current` がなければ、`perf shell` を今流して比べる（`--current-impl` の実装だけ）
//!
//! # tolerance.toml の形
//! 1 行 1 指標。`<実装なしの指標名> = <倍率>`。倍率は「今の中央値 ÷ 基準値の中央値」の上限（1.25 なら 25% まで増えてよい）。`#` から行末はコメント。
//! ```toml
//! p1.p99_ns = 1.5
//! p4.max_rss_bytes = 1.2
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::record::{self, err, repo_root};
use super::shell::{self, Implementation, format_value};
use crate::XtaskError;

const TOLERANCE_PATH: &str = "eval/reports/shell-perf/tolerance.toml";
const CURRENT_SCRATCH: &str = "target/perf-current.json";

#[derive(Debug, clap::Args)]
pub struct Args {
    /// 基準値のファイル（`eval/reports/shell-perf/baseline.json`）
    #[arg(long)]
    pub baseline: PathBuf,
    /// 今の結果のファイル（`perf shell --out` などで作ったもの）。なければ `perf shell` を流す
    #[arg(long)]
    pub current: Option<PathBuf>,
    /// 許す幅のファイル
    #[arg(long)]
    pub tolerance: Option<PathBuf>,
    /// 基準値の側の実装
    #[arg(long, value_enum, default_value = "swift")]
    pub baseline_impl: Implementation,
    /// 今の側の実装
    #[arg(long, value_enum, default_value = "rust")]
    pub current_impl: Implementation,
    /// `--current` がないときの計測の回数
    #[arg(long, default_value_t = shell::DEFAULT_RUNS)]
    pub runs: usize,
}

/// 実装の接頭辞（`swift.`・`rust.`）を外した指標の名前。実装を持たなければ `None`。
fn split_impl(name: &str) -> (Option<&str>, &str) {
    for prefix in ["swift", "rust"] {
        if let Some(rest) = name
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix('.'))
        {
            return (Some(prefix), rest);
        }
    }
    (None, name)
}

/// 指標の欄（中央値、最小、最大）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stat {
    pub median: f64,
    pub min: f64,
    pub max: f64,
}

/// 結果のファイルから、`implementation` の指標を実装の接頭辞なしの名前で取り出す（実装を持たない指標も含める）。
fn stats_for(file: &Value, implementation: &str) -> BTreeMap<String, (String, Stat)> {
    let mut found = BTreeMap::new();
    let Some(metrics) = file["metrics"].as_object() else {
        return found;
    };
    for (name, value) in metrics {
        let (owner, id) = split_impl(name);
        if owner.is_some_and(|owner| owner != implementation) {
            continue;
        }
        let (Some(median), Some(min), Some(max)) = (
            value["median"].as_f64(),
            value["min"].as_f64(),
            value["max"].as_f64(),
        ) else {
            continue;
        };
        let unit = value["unit"].as_str().unwrap_or("").to_owned();
        found.insert(id.to_owned(), (unit, Stat { median, min, max }));
    }
    found
}

/// tolerance.toml を読む（`指標 = 倍率` の行だけ。`#` はコメント。`[…]` の見出しは読み飛ばす）。
pub fn parse_tolerance(text: &str) -> Result<BTreeMap<String, f64>, XtaskError> {
    let mut limits = BTreeMap::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('[') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(|| {
            err(format!(
                "tolerance.toml {} 行目: `指標 = 倍率` の形ではありません",
                index + 1
            ))
        })?;
        let key = key.trim().trim_matches('"').to_owned();
        let ratio: f64 = value.trim().parse().map_err(|_| {
            err(format!(
                "tolerance.toml {} 行目: 倍率が数ではありません",
                index + 1
            ))
        })?;
        if ratio.is_nan() || ratio <= 0.0 {
            return Err(err(format!(
                "tolerance.toml {} 行目: 倍率は 0 より大きい数",
                index + 1
            )));
        }
        limits.insert(key, ratio);
    }
    Ok(limits)
}

/// 比較の 1 行。
#[derive(Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub unit: String,
    pub baseline: Stat2,
    pub current: Stat2,
    pub ratio: Option<f64>,
    pub limit: Option<f64>,
    pub exceeded: bool,
}

/// `Stat` の公開用の写し（テスト用に比較できる形）。
pub type Stat2 = (f64, f64, f64);

/// 基準値と今の値を突き合わせる。どちらかにしかない指標は表に出さない。
pub fn build_rows(
    baseline: &BTreeMap<String, (String, Stat)>,
    current: &BTreeMap<String, (String, Stat)>,
    limits: &BTreeMap<String, f64>,
) -> Vec<Row> {
    baseline
        .iter()
        .filter_map(|(id, (unit, base))| {
            let (_, now) = current.get(id)?;
            let ratio = (base.median > 0.0).then(|| now.median / base.median);
            let limit = limits.get(id).copied();
            let exceeded = matches!((ratio, limit), (Some(ratio), Some(limit)) if ratio > limit);
            Some(Row {
                id: id.clone(),
                unit: unit.clone(),
                baseline: (base.median, base.min, base.max),
                current: (now.median, now.min, now.max),
                ratio,
                limit,
                exceeded,
            })
        })
        .collect()
}

fn print_rows(rows: &[Row]) {
    println!(
        "{:<24} {:>26} {:>26} {:>7} {:>6}  status",
        "metric", "baseline (min-max)", "current (min-max)", "ratio", "limit"
    );
    for row in rows {
        let cell = |(median, min, max): Stat2| {
            format!(
                "{} ({}-{})",
                format_value(median, &row.unit),
                format_value(min, &row.unit),
                format_value(max, &row.unit)
            )
        };
        let status = match (row.limit, row.exceeded) {
            (None, _) => "no limit",
            (Some(_), true) => "EXCEEDED",
            (Some(_), false) => "ok",
        };
        println!(
            "{:<24} {:>26} {:>26} {:>7} {:>6}  {status}",
            row.id,
            cell(row.baseline),
            cell(row.current),
            row.ratio
                .map_or("-".to_owned(), |ratio| format!("{ratio:.2}x")),
            row.limit
                .map_or("-".to_owned(), |limit| format!("{limit:.2}")),
        );
    }
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    let baseline_file = record::load(&args.baseline)?;
    let current_path = match &args.current {
        Some(path) => path.clone(),
        None => {
            let scratch = repo_root().join(CURRENT_SCRATCH);
            let _ = std::fs::remove_file(&scratch);
            shell::run(shell::Args {
                implementations: vec![args.current_impl],
                runs: args.runs,
                repeat: shell::DEFAULT_REPEAT,
                shortcode_entries: shell::DEFAULT_SHORTCODE_ENTRIES,
                out: Some(scratch.clone()),
            })?;
            scratch
        }
    };
    let current_file = record::load(&current_path)?;

    let tolerance_path = args
        .tolerance
        .clone()
        .unwrap_or_else(|| repo_root().join(TOLERANCE_PATH));
    let limits = read_tolerance(&tolerance_path)?;

    let baseline = stats_for(&baseline_file, impl_name(args.baseline_impl));
    let current = stats_for(&current_file, impl_name(args.current_impl));
    let rows = build_rows(&baseline, &current, &limits);
    if rows.is_empty() {
        println!(
            "比べられる指標がありません（基準値: {}、今: {}。Rust 版の Shell が stub の間は、Rust 側の指標が入りません）",
            args.baseline_impl_label(),
            args.current_impl_label()
        );
        return Ok(());
    }
    print_rows(&rows);
    if limits.is_empty() {
        println!(
            "許す幅（{}）がないので、表だけです",
            tolerance_path.display()
        );
    }
    let exceeded: Vec<&str> = rows
        .iter()
        .filter(|row| row.exceeded)
        .map(|row| row.id.as_str())
        .collect();
    if exceeded.is_empty() {
        Ok(())
    } else {
        Err(err(format!("許す幅を超えた指標: {}", exceeded.join(", "))))
    }
}

fn read_tolerance(path: &Path) -> Result<BTreeMap<String, f64>, XtaskError> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let text = std::fs::read_to_string(path)
        .map_err(|error| err(format!("{}: {error}", path.display())))?;
    parse_tolerance(&text)
}

fn impl_name(implementation: Implementation) -> &'static str {
    match implementation {
        Implementation::Swift => "swift",
        Implementation::Rust => "rust",
    }
}

impl Args {
    fn baseline_impl_label(&self) -> &'static str {
        impl_name(self.baseline_impl)
    }
    fn current_impl_label(&self) -> &'static str {
        impl_name(self.current_impl)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn file(entries: &[(&str, f64, f64, f64)]) -> Value {
        let metrics: serde_json::Map<String, Value> = entries
            .iter()
            .map(|(name, median, min, max)| {
                (
                    (*name).to_owned(),
                    json!({"unit": "ns", "median": median, "min": min, "max": max}),
                )
            })
            .collect();
        json!({"schema": 1, "meta": {}, "metrics": metrics})
    }

    #[test]
    fn splits_implementation_prefix() {
        assert_eq!(split_impl("swift.p1.p50_ns"), (Some("swift"), "p1.p50_ns"));
        assert_eq!(split_impl("p5.convert.p50_ns"), (None, "p5.convert.p50_ns"));
    }

    #[test]
    fn compares_across_implementations() {
        let baseline = file(&[
            ("swift.p1.p99_ns", 1000.0, 900.0, 1100.0),
            ("p5.x", 10.0, 9.0, 11.0),
        ]);
        let current = file(&[
            ("rust.p1.p99_ns", 1600.0, 1500.0, 1700.0),
            ("p5.x", 10.0, 9.0, 11.0),
        ]);
        let limits = parse_tolerance("p1.p99_ns = 1.5 # 仮\n").unwrap();
        let rows = build_rows(
            &stats_for(&baseline, "swift"),
            &stats_for(&current, "rust"),
            &limits,
        );
        assert_eq!(rows.len(), 2);
        let p1 = rows.iter().find(|row| row.id == "p1.p99_ns").unwrap();
        assert!(p1.exceeded);
        assert_eq!(p1.ratio, Some(1.6));
        let p5 = rows.iter().find(|row| row.id == "p5.x").unwrap();
        assert!(!p5.exceeded);
        assert_eq!(p5.limit, None);
    }

    #[test]
    fn without_limits_nothing_is_exceeded() {
        let baseline = file(&[("swift.p1.p50_ns", 100.0, 90.0, 110.0)]);
        let current = file(&[("rust.p1.p50_ns", 10_000.0, 9_000.0, 11_000.0)]);
        let rows = build_rows(
            &stats_for(&baseline, "swift"),
            &stats_for(&current, "rust"),
            &BTreeMap::new(),
        );
        assert!(rows.iter().all(|row| !row.exceeded));
    }

    #[test]
    fn rejects_bad_tolerance_lines() {
        assert!(parse_tolerance("p1.p50_ns").is_err());
        assert!(parse_tolerance("p1.p50_ns = fast").is_err());
        assert!(parse_tolerance("p1.p50_ns = 0").is_err());
        assert_eq!(parse_tolerance("# only a comment\n[x]\n").unwrap().len(), 0);
    }
}
