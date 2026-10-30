//! 計測の結果のファイル（`eval/reports/shell-perf/baseline.json` と同じ形）と、環境の記録、統計。
//!
//! ```json
//! {
//!   "schema": 1,
//!   "meta": { "sha": "…", "base_sha": "…", "dirty": false, "machine": {…}, "os": "…",
//!             "sections": { "shell": { "measured_at": "…", "runs": 5, "load_average_start": 3.5, "busy_processes_max": 0, … } } },
//!   "metrics": { "swift.p1.p50_ns": { "unit": "ns", "median": 500, "min": 500, "max": 541, "runs": [500, …] } }
//! }
//! ```
//!
//! 指標の名前は `<実装>.<指標>` （`swift.p1.p50_ns`、`rust.p3.p99_ns`）。実装を持たない指標（P2、P5）は `p5.…`、`p2.…`。

use std::path::Path;
use std::process::Command;

use serde_json::{Map, Value, json};

use crate::XtaskError;

pub const SCHEMA: u64 = 1;

pub fn err(message: impl Into<String>) -> XtaskError {
    XtaskError(message.into())
}

/// 子プロセスを流し、標準出力を返す。失敗（起動できない、終了コードが 0 以外）は標準エラーつきの誤りにする。
pub fn capture(command: &mut Command, what: &str) -> Result<String, XtaskError> {
    let output = command
        .output()
        .map_err(|error| err(format!("{what}: 起動できません: {error}")))?;
    if !output.status.success() {
        return Err(err(format!(
            "{what}: 終了コード {:?}\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// 出力のコマンドを流し、失敗したら `None`（環境の記録に使う。なくても計測は続ける）。
fn try_capture(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// リポジトリの根（`xtask` の 1 つ上）。
pub fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(Path::new("."))
}

// ---------------------------------------------------------------- 統計

/// 昇順に並べた値の中央値（偶数個なら真ん中の 2 つの平均）。空なら `None`。
pub fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    Some(if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        f64::midpoint(sorted[mid - 1], sorted[mid])
    })
}

/// 複数回の値から、指標の欄（中央値、最小、最大、各回の値）を作る。
pub fn metric(unit: &str, runs: &[f64]) -> Option<Value> {
    let median = median(runs)?;
    let min = runs.iter().copied().fold(f64::INFINITY, f64::min);
    let max = runs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Some(json!({ "unit": unit, "median": median, "min": min, "max": max, "runs": runs }))
}

// ---------------------------------------------------------------- 環境

/// 計測する機械と OS（結果のファイルの `meta`）。
pub fn machine() -> Value {
    let cpu = try_capture("sysctl", &["-n", "machdep.cpu.brand_string"]);
    let memory: Option<u64> =
        try_capture("sysctl", &["-n", "hw.memsize"]).and_then(|v| v.parse().ok());
    json!({
        "cpu": cpu,
        "memory_bytes": memory,
        "arch": std::env::consts::ARCH,
        "os": std::env::consts::OS,
        "os_version": try_capture("sw_vers", &["-productVersion"]),
        "os_build": try_capture("sw_vers", &["-buildVersion"]),
        "kernel": try_capture("uname", &["-sr"]),
    })
}

/// 計測中のビルドの重さに効く、ほかのプロセス（cargo、rustc、swift-build など）。
const BUSY_NAMES: &[&str] = &[
    "cargo",
    "rustc",
    "swift-build",
    "swift-frontend",
    "swiftc",
    "swift-driver",
    "clang",
    "cc1",
    "ld",
];

/// 自分の祖先（`cargo xtask` の親の cargo など）を除いた、重いプロセスの名前。
pub fn busy_processes() -> Vec<String> {
    let Some(table) = try_capture("ps", &["-A", "-o", "pid=,ppid=,comm="]) else {
        return Vec::new();
    };
    let rows: Vec<(u32, u32, String)> = table
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse().ok()?;
            let ppid = parts.next()?.parse().ok()?;
            let command = parts.collect::<Vec<_>>().join(" ");
            Some((pid, ppid, command))
        })
        .collect();
    let mut ancestors = std::collections::HashSet::new();
    let mut pid = std::process::id();
    while ancestors.insert(pid) {
        match rows.iter().find(|row| row.0 == pid) {
            Some(row) if row.1 != 0 => pid = row.1,
            _ => break,
        }
    }
    rows.iter()
        .filter(|(pid, _, _)| !ancestors.contains(pid))
        .filter_map(|(_, _, command)| {
            let name = command.rsplit('/').next().unwrap_or(command);
            BUSY_NAMES.contains(&name).then(|| name.to_owned())
        })
        .collect()
}

/// 直近 1 分の負荷平均。
pub fn load_average() -> Option<f64> {
    let text = try_capture("sysctl", &["-n", "vm.loadavg"])
        .or_else(|| std::fs::read_to_string("/proc/loadavg").ok())?;
    text.split(|c: char| c == '{' || c.is_whitespace())
        .find_map(|token| token.parse().ok())
}

/// 計測の区間の負荷の記録（区間の最初と最後の負荷平均、途中で見えた重いプロセスの最大の数と名前）。
#[derive(Debug, Default)]
pub struct LoadWatch {
    pub load_start: Option<f64>,
    pub busy_max: usize,
    pub busy_names: std::collections::BTreeSet<String>,
}

impl LoadWatch {
    pub fn start() -> Self {
        let mut watch = Self {
            load_start: load_average(),
            ..Self::default()
        };
        watch.sample();
        watch
    }

    /// 1 回ごとの計測の前に呼ぶ。
    pub fn sample(&mut self) {
        let busy = busy_processes();
        self.busy_max = self.busy_max.max(busy.len());
        self.busy_names.extend(busy);
    }

    pub fn finish(&self) -> Value {
        json!({
            "load_average_start": self.load_start,
            "load_average_end": load_average(),
            "busy_processes_max": self.busy_max,
            "busy_process_names": self.busy_names,
        })
    }
}

/// 今の HEAD の SHA、`origin/main` との共通の祖先、作業ツリーに変更があるか。
pub fn git_state() -> Value {
    let root = repo_root().to_string_lossy().into_owned();
    let git = |args: &[&str]| {
        let mut full = vec!["-C", root.as_str()];
        full.extend_from_slice(args);
        try_capture("git", &full)
    };
    json!({
        "sha": git(&["rev-parse", "HEAD"]),
        "base_sha": git(&["merge-base", "HEAD", "origin/main"]),
        "branch": git(&["rev-parse", "--abbrev-ref", "HEAD"]),
        "dirty": git(&["status", "--porcelain"]).is_some_and(|text| !text.is_empty()),
    })
}

/// 今の時刻（UTC。RFC 3339）。
pub fn now_utc() -> String {
    try_capture("date", &["-u", "+%Y-%m-%dT%H:%M:%SZ"]).unwrap_or_default()
}

// ---------------------------------------------------------------- ファイル

/// 結果のファイルを読む。なければ空の結果。
pub fn load(path: &Path) -> Result<Value, XtaskError> {
    if !path.exists() {
        return Ok(json!({ "schema": SCHEMA, "meta": {}, "metrics": {} }));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|error| err(format!("{}: {error}", path.display())))?;
    let value: Value = serde_json::from_str(&text).map_err(|error| {
        err(format!(
            "{}: JSON として読めません: {error}",
            path.display()
        ))
    })?;
    if value.get("schema").and_then(Value::as_u64) != Some(SCHEMA) {
        return Err(err(format!(
            "{}: schema が {SCHEMA} ではありません",
            path.display()
        )));
    }
    Ok(value)
}

/// `section`（shell、engine、smoke）の結果を `path` に混ぜて書く。ほかの区間の指標は残す。
pub fn merge_into(
    path: &Path,
    section: &str,
    section_meta: Value,
    metrics: Map<String, Value>,
) -> Result<(), XtaskError> {
    let mut file = load(path)?;
    let git = git_state();
    if let Some(meta) = file["meta"].as_object_mut() {
        if let Some(git) = git.as_object() {
            meta.extend(git.clone());
        }
        meta.insert("machine".into(), machine());
        let sections = meta
            .entry("sections")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(sections) = sections.as_object_mut() {
            sections.insert(section.into(), section_meta);
        }
    }
    if let Some(existing) = file["metrics"].as_object_mut() {
        existing.extend(metrics);
    }
    let mut text = serde_json::to_string_pretty(&file)
        .map_err(|error| err(format!("JSON に書けません: {error}")))?;
    text.push('\n');
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| err(format!("{}: {error}", parent.display())))?;
    }
    std::fs::write(path, text).map_err(|error| err(format!("{}: {error}", path.display())))
}

/// 結果の 1 回分（指標名 → 値）を、回をまたいで集める。
#[derive(Debug, Default)]
pub struct Collector {
    units: std::collections::BTreeMap<String, &'static str>,
    runs: std::collections::BTreeMap<String, Vec<f64>>,
}

impl Collector {
    pub fn push(&mut self, name: impl Into<String>, unit: &'static str, value: f64) {
        let name = name.into();
        self.units.insert(name.clone(), unit);
        self.runs.entry(name).or_default().push(value);
    }

    pub fn into_metrics(self) -> Map<String, Value> {
        self.runs
            .into_iter()
            .filter_map(|(name, values)| {
                let unit = self.units.get(&name).copied().unwrap_or("");
                Some((name, metric(unit, &values)?))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_of_odd_and_even() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
        assert_eq!(median(&[]), None);
    }

    #[test]
    fn metric_records_spread() {
        let value = metric("ns", &[500.0, 541.0, 510.0]).unwrap();
        assert_eq!(value["median"], 510.0);
        assert_eq!(value["min"], 500.0);
        assert_eq!(value["max"], 541.0);
    }

    #[test]
    fn collector_groups_runs_by_name() {
        let mut collector = Collector::default();
        collector.push("a", "ns", 1.0);
        collector.push("a", "ns", 3.0);
        let metrics = collector.into_metrics();
        assert_eq!(metrics["a"]["median"], 2.0);
    }
}
