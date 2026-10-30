//! `cargo llvm-cov report --json --summary-only` の出力の解釈と、表の整形。
//!
//! JSON を選んだ理由: 表（テキスト）は列の幅や桁がバージョンで変わりうるが、JSON は `data[0].files[].filename` と
//! `summary.lines.{count,covered}` という名前つきの欄で、壊れにくい。行カバレッジだけを使う。

use std::fmt::Write as _;

use serde_json::Value;

/// 変換を担うロジックの crate（ディレクトリ名）。グループの定義はここだけ（`--group conversion`、小計の行）。
pub const CONVERSION_CRATES: &[&str] = &["core", "engine", "input", "lm", "dict-format"];

/// 1 ファイルの行カバレッジ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCov {
    /// リポジトリの根からの相対パス（区切りは `/`）
    pub path: String,
    pub lines: u64,
    pub covered: u64,
}

impl FileCov {
    pub fn uncovered(&self) -> u64 {
        self.lines - self.covered
    }

    pub fn percent(&self) -> f64 {
        percent(self.covered, self.lines)
    }

    /// crate のディレクトリ名（パスの最初の要素）。
    pub fn krate(&self) -> &str {
        self.path.split('/').next().unwrap_or(&self.path)
    }
}

pub fn percent(covered: u64, lines: u64) -> f64 {
    if lines == 0 {
        100.0
    } else {
        covered as f64 * 100.0 / lines as f64
    }
}

/// 対象の絞り込み。
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Group {
    /// すべての crate
    All,
    /// 変換のロジック（core、engine、input、lm、dict-format）だけ
    Conversion,
}

/// 低いファイルの目安。
#[derive(Debug, Clone, Copy)]
pub struct LowCriteria {
    pub min_lines: u64,
    pub below_percent: f64,
}

/// llvm-cov の JSON を読む。`root` はリポジトリの根（絶対パス。ファイル名から引く）。
pub fn parse(json: &str, root: &str) -> Result<Vec<FileCov>, String> {
    let value: Value =
        serde_json::from_str(json).map_err(|e| format!("llvm-cov の JSON を読めません: {e}"))?;
    let files = value
        .pointer("/data/0/files")
        .and_then(Value::as_array)
        .ok_or("llvm-cov の JSON に data[0].files がありません")?;
    let root = normalize(root);
    let root = root.trim_end_matches('/');
    let mut out = Vec::new();
    for file in files {
        let name = file
            .get("filename")
            .and_then(Value::as_str)
            .ok_or("files[].filename がありません")?;
        let name = normalize(name);
        // 根の外（依存の crate など）は数えない
        let Some(rel) = name
            .strip_prefix(root)
            .and_then(|rest| rest.strip_prefix('/'))
        else {
            continue;
        };
        let count = |key: &str| {
            file.pointer(&format!("/summary/lines/{key}"))
                .and_then(Value::as_u64)
        };
        let (Some(lines), Some(covered)) = (count("count"), count("covered")) else {
            return Err(format!("{name}: summary.lines がありません"));
        };
        if covered > lines {
            return Err(format!("{name}: covered が count より大きい"));
        }
        out.push(FileCov {
            path: rel.to_owned(),
            lines,
            covered,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Windows の区切りを `/` にそろえる。
fn normalize(path: &str) -> String {
    path.replace('\\', "/")
}

#[derive(Debug, PartialEq)]
pub struct CrateRow {
    pub name: String,
    pub lines: u64,
    pub covered: u64,
    pub conversion: bool,
}

impl CrateRow {
    pub fn percent(&self) -> f64 {
        percent(self.covered, self.lines)
    }
}

pub fn is_conversion(krate: &str) -> bool {
    CONVERSION_CRATES.contains(&krate)
}

fn selected(file: &FileCov, group: Group) -> bool {
    group == Group::All || is_conversion(file.krate())
}

/// crate ごとの集計（カバレッジの低い順）。
pub fn by_crate(files: &[FileCov], group: Group) -> Vec<CrateRow> {
    let mut rows: Vec<CrateRow> = Vec::new();
    for file in files.iter().filter(|f| selected(f, group)) {
        let name = file.krate();
        match rows.iter_mut().find(|r| r.name == name) {
            Some(row) => {
                row.lines += file.lines;
                row.covered += file.covered;
            }
            None => rows.push(CrateRow {
                name: name.to_owned(),
                lines: file.lines,
                covered: file.covered,
                conversion: is_conversion(name),
            }),
        }
    }
    rows.sort_by(|a, b| {
        a.percent()
            .total_cmp(&b.percent())
            .then(a.name.cmp(&b.name))
    });
    rows
}

/// 低いファイル（未実行の行が多い順）。
pub fn low_files(files: &[FileCov], group: Group, criteria: LowCriteria) -> Vec<&FileCov> {
    let mut low: Vec<&FileCov> = files
        .iter()
        .filter(|f| selected(f, group))
        .filter(|f| f.lines >= criteria.min_lines && f.percent() < criteria.below_percent)
        .collect();
    low.sort_by(|a, b| b.uncovered().cmp(&a.uncovered()).then(a.path.cmp(&b.path)));
    low
}

/// 合計（対象のグループの）。
pub fn total(files: &[FileCov], group: Group) -> (u64, u64) {
    files
        .iter()
        .filter(|f| selected(f, group))
        .fold((0, 0), |(l, c), f| (l + f.lines, c + f.covered))
}

/// Markdown の表を作る。
pub fn render(files: &[FileCov], group: Group, criteria: LowCriteria) -> String {
    let mut text = String::new();
    let rows = by_crate(files, group);
    let _ = writeln!(text, "## crate ごと（行カバレッジ。低い順）\n");
    let _ = writeln!(text, "| crate | グループ | 行数 | 未実行 | カバレッジ |");
    let _ = writeln!(text, "| --- | --- | ---: | ---: | ---: |");
    for row in &rows {
        let label = if row.conversion { "変換" } else { "" };
        let _ = writeln!(
            text,
            "| {} | {} | {} | {} | {:.1}% |",
            row.name,
            label,
            row.lines,
            row.lines - row.covered,
            row.percent()
        );
    }
    let (lines, covered) = total(files, Group::Conversion);
    if group == Group::All {
        let _ = writeln!(
            text,
            "| **変換のロジックの小計**（{}） |  | {} | {} | {:.1}% |",
            CONVERSION_CRATES.join("、"),
            lines,
            lines - covered,
            percent(covered, lines)
        );
    }
    let (lines, covered) = total(files, group);
    let _ = writeln!(
        text,
        "| **{}** |  | {} | {} | {:.1}% |",
        match group {
            Group::All => "全体",
            Group::Conversion => "変換のロジックの合計",
        },
        lines,
        lines - covered,
        percent(covered, lines)
    );

    let low = low_files(files, group, criteria);
    let _ = writeln!(
        text,
        "\n## 低いファイル（{} 行以上で {}% 未満。未実行の行が多い順）\n",
        criteria.min_lines, criteria.below_percent
    );
    if low.is_empty() {
        let _ = writeln!(text, "なし");
    } else {
        let _ = writeln!(text, "| ファイル | 行数 | 未実行 | カバレッジ |");
        let _ = writeln!(text, "| --- | ---: | ---: | ---: |");
        for file in low {
            let _ = writeln!(
                text,
                "| {} | {} | {} | {:.1}% |",
                file.path,
                file.lines,
                file.uncovered(),
                file.percent()
            );
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "type": "llvm.coverage.json.export", "version": "2.0.1",
      "data": [{
        "files": [
          {"filename": "/repo/core/src/a.rs", "summary": {"lines": {"count": 100, "covered": 90, "percent": 90.0}}},
          {"filename": "/repo/core/src/b.rs", "summary": {"lines": {"count": 100, "covered": 50, "percent": 50.0}}},
          {"filename": "/repo/engine-ffi/src/c.rs", "summary": {"lines": {"count": 80, "covered": 40, "percent": 50.0}}},
          {"filename": "/repo/shell-core/src/d.rs", "summary": {"lines": {"count": 10, "covered": 0, "percent": 0.0}}},
          {"filename": "/elsewhere/registry/x.rs", "summary": {"lines": {"count": 99, "covered": 0, "percent": 0.0}}}
        ],
        "totals": {}
      }]
    }"#;

    const CRITERIA: LowCriteria = LowCriteria {
        min_lines: 60,
        below_percent: 70.0,
    };

    #[test]
    fn parse_keeps_only_files_under_root() {
        let files = parse(SAMPLE, "/repo").unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "core/src/a.rs",
                "core/src/b.rs",
                "engine-ffi/src/c.rs",
                "shell-core/src/d.rs"
            ]
        );
    }

    #[test]
    fn parse_accepts_windows_paths() {
        let json = r#"{"data":[{"files":[{"filename":"C:\\repo\\core\\src\\a.rs","summary":{"lines":{"count":4,"covered":3}}}]}]}"#;
        let files = parse(json, "C:\\repo").unwrap();
        assert_eq!(files[0].path, "core/src/a.rs");
        assert_eq!(files[0].krate(), "core");
    }

    #[test]
    fn parse_rejects_broken_input() {
        assert!(parse("not json", "/repo").is_err());
        assert!(parse("{}", "/repo").is_err());
        let json = r#"{"data":[{"files":[{"filename":"/repo/a.rs","summary":{"lines":{"count":1,"covered":2}}}]}]}"#;
        assert!(parse(json, "/repo").is_err());
    }

    #[test]
    fn crates_are_sorted_low_first_and_grouped() {
        let files = parse(SAMPLE, "/repo").unwrap();
        let rows = by_crate(&files, Group::All);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["shell-core", "engine-ffi", "core"]);
        assert!(rows[2].conversion);
        assert!(!rows[1].conversion);
        assert_eq!((rows[2].lines, rows[2].covered), (200, 140));

        let rows = by_crate(&files, Group::Conversion);
        assert_eq!(rows.len(), 1);
        assert_eq!(total(&files, Group::Conversion), (200, 140));
        assert_eq!(total(&files, Group::All), (290, 180));
    }

    #[test]
    fn low_files_use_thresholds_and_sort_by_uncovered() {
        let files = parse(SAMPLE, "/repo").unwrap();
        let low = low_files(&files, Group::All, CRITERIA);
        let paths: Vec<&str> = low.iter().map(|f| f.path.as_str()).collect();
        // d.rs は 10 行しかないので除く。b.rs（未実行 50）、c.rs（未実行 40）の順
        assert_eq!(paths, ["core/src/b.rs", "engine-ffi/src/c.rs"]);
        let strict = LowCriteria {
            min_lines: 60,
            below_percent: 50.0,
        };
        assert!(low_files(&files, Group::All, strict).is_empty());
    }

    #[test]
    fn render_has_subtotal_and_low_table() {
        let files = parse(SAMPLE, "/repo").unwrap();
        let text = render(&files, Group::All, CRITERIA);
        assert!(
            text.contains("| core | 変換 | 200 | 60 | 70.0% |"),
            "{text}"
        );
        assert!(text.contains("**変換のロジックの小計**"));
        assert!(
            text.contains("| **全体** |  | 290 | 110 | 62.1% |"),
            "{text}"
        );
        assert!(text.contains("| core/src/b.rs | 100 | 50 | 50.0% |"));
        let text = render(&files, Group::Conversion, CRITERIA);
        assert!(!text.contains("engine-ffi"));
        assert!(text.contains("変換のロジックの合計"));
    }

    #[test]
    fn empty_file_counts_as_fully_covered() {
        assert_eq!(percent(0, 0), 100.0);
    }
}
