//! `ikari-eval compare <a.tsv> <b.tsv>`: 2つの結果（`--out-cases` の TSV）のケースごとの順位の差分表
//! （ADR 0002 §15.1）。

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::ab::Change;
use crate::cell;

const COMMENT_PREFIX: char = '#';
const COL_ID: &str = "id";
const COL_RANK: &str = "rank";
const COL_KIND: &str = "kind";
const COL_EXPECTED: &str = "expected";
const COL_TOP1: &str = "top1";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    kind: String,
    expected: String,
    top1: String,
    rank: Option<usize>,
}

/// 1つの結果ファイル。id の昇順
#[derive(Clone, Debug, Default)]
pub struct CaseTable {
    rows: BTreeMap<String, Row>,
}

impl CaseTable {
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    fn top1_count(&self) -> usize {
        self.rows.values().filter(|r| r.rank == Some(1)).count()
    }
}

/// `--out-cases` の TSV を読む。`id` と `rank` は必須。`kind` `expected` `top1` があれば読む。
pub fn load_table(tsv: &str) -> Result<CaseTable, String> {
    let mut lines = tsv
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty() && !l.starts_with(COMMENT_PREFIX));
    let header = lines.next().ok_or("empty file")?;
    let cols: Vec<&str> = header.split('\t').map(str::trim).collect();
    let find = |name: &str| cols.iter().position(|c| *c == name);
    let id_idx = find(COL_ID).ok_or("missing column `id`")?;
    let rank_idx = find(COL_RANK).ok_or("missing column `rank`")?;
    let (kind_idx, expected_idx, top1_idx) = (find(COL_KIND), find(COL_EXPECTED), find(COL_TOP1));

    let mut rows = BTreeMap::new();
    for (i, line) in lines.enumerate() {
        let cells: Vec<&str> = line.split('\t').collect();
        let get = |idx: Option<usize>| idx.and_then(|i| cells.get(i)).copied().unwrap_or("");
        let id = get(Some(id_idx));
        if id.is_empty() {
            return Err(format!("row {} has no id", i + 1));
        }
        let rank = match get(Some(rank_idx)).trim() {
            "" => None,
            s => Some(
                s.parse::<usize>()
                    .ok()
                    .filter(|r| *r >= 1)
                    .ok_or_else(|| format!("row {} has a bad rank `{s}`", i + 1))?,
            ),
        };
        let row = Row {
            kind: get(kind_idx).to_owned(),
            expected: get(expected_idx).to_owned(),
            top1: get(top1_idx).to_owned(),
            rank,
        };
        if rows.insert(id.to_owned(), row).is_some() {
            return Err(format!("duplicate id `{id}`"));
        }
    }
    Ok(CaseTable { rows })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Presence {
    Both { a: Option<usize>, b: Option<usize> },
    OnlyA(Option<usize>),
    OnlyB(Option<usize>),
}

#[derive(Clone, Debug)]
pub struct CompareRow {
    pub id: String,
    pub kind: String,
    pub expected: String,
    pub top1_a: String,
    pub top1_b: String,
    pub presence: Presence,
}

impl CompareRow {
    /// 両方にあるケースの変化。片方にしかなければ None
    pub fn change(&self) -> Option<Change> {
        match self.presence {
            Presence::Both { a, b } => Some(Change::between(a, b)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Comparison {
    pub rows: Vec<CompareRow>,
    /// (件数, 1位正解の数)
    pub a: (usize, usize),
    pub b: (usize, usize),
}

impl Comparison {
    pub fn count(&self, change: Change) -> usize {
        self.rows
            .iter()
            .filter(|r| r.change() == Some(change))
            .count()
    }
}

/// a を基準に b を比べる。順位が小さい方が良い（`Improved` は b の方が良い）。
pub fn compare_tables(a: &CaseTable, b: &CaseTable) -> Comparison {
    let mut rows = Vec::new();
    for (id, ra) in &a.rows {
        let (presence, top1_b) = match b.rows.get(id) {
            Some(rb) => (
                Presence::Both {
                    a: ra.rank,
                    b: rb.rank,
                },
                rb.top1.clone(),
            ),
            None => (Presence::OnlyA(ra.rank), String::new()),
        };
        rows.push(CompareRow {
            id: id.clone(),
            kind: ra.kind.clone(),
            expected: ra.expected.clone(),
            top1_a: ra.top1.clone(),
            top1_b,
            presence,
        });
    }
    for (id, rb) in &b.rows {
        if !a.rows.contains_key(id) {
            rows.push(CompareRow {
                id: id.clone(),
                kind: rb.kind.clone(),
                expected: rb.expected.clone(),
                top1_a: String::new(),
                top1_b: rb.top1.clone(),
                presence: Presence::OnlyB(rb.rank),
            });
        }
    }
    Comparison {
        rows,
        a: (a.len(), a.top1_count()),
        b: (b.len(), b.top1_count()),
    }
}

fn rank_text(r: Option<usize>) -> String {
    r.map_or_else(|| "-".to_owned(), |n| n.to_string())
}

fn rate(n: usize, top1: usize) -> f64 {
    if n == 0 {
        0.0
    } else {
        top1 as f64 * 100.0 / n as f64
    }
}

/// 差分の表（Markdown）。順位が変わったケースと、片方にしかないケースだけを出す。
pub fn render_comparison(c: &Comparison, name_a: &str, name_b: &str) -> String {
    let mut out = String::from("# ikari-eval compare\n");
    let _ = writeln!(
        out,
        "- a: {name_a}（{} 件、1位 {} = {:.1}%）",
        c.a.0,
        c.a.1,
        rate(c.a.0, c.a.1)
    );
    let _ = writeln!(
        out,
        "- b: {name_b}（{} 件、1位 {} = {:.1}%）",
        c.b.0,
        c.b.1,
        rate(c.b.0, c.b.1)
    );
    let only_a = c
        .rows
        .iter()
        .filter(|r| matches!(r.presence, Presence::OnlyA(_)))
        .count();
    let only_b = c
        .rows
        .iter()
        .filter(|r| matches!(r.presence, Presence::OnlyB(_)))
        .count();
    let _ = writeln!(
        out,
        "- 改善（b が上位）{} / 悪化 {} / 変化なし {} / a にだけある {only_a} / b にだけある {only_b}",
        c.count(Change::Improved),
        c.count(Change::Worse),
        c.count(Change::Same)
    );
    out.push_str("\n| id | kind | 期待 | a の順位 | b の順位 | a の1位 | b の1位 | 変化 |\n|---|---|---|---|---|---|---|---|\n");
    let mut shown = 0;
    for r in &c.rows {
        let (ra, rb, mark) = match r.presence {
            Presence::Both { a, b } => match Change::between(a, b) {
                Change::Same => continue,
                Change::Improved => (rank_text(a), rank_text(b), "改善"),
                Change::Worse => (rank_text(a), rank_text(b), "悪化"),
            },
            Presence::OnlyA(a) => (rank_text(a), "(なし)".to_owned(), "a のみ"),
            Presence::OnlyB(b) => ("(なし)".to_owned(), rank_text(b), "b のみ"),
        };
        shown += 1;
        let _ = writeln!(
            out,
            "| {} | {} | {} | {ra} | {rb} | {} | {} | {mark} |",
            cell(&r.id),
            cell(&r.kind),
            cell(&r.expected),
            cell(&r.top1_a),
            cell(&r.top1_b),
        );
    }
    if shown == 0 {
        out.push_str("| (差分なし) | | | | | | | |\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "\
id\tkind\tcontext\treading\texpected\ttop1\trank
x1\t話題\tc\tr\t品詞\t瀕死\t2
x2\t話題\tc\tr\t構成\t構成\t1
x3\t否定\tc\tr\tしよう\t仕様\t3
x4\t話題\tc\tr\t校正\t構成\t
x5\t話題\tc\tr\t回答\t回答\t1
";
    const B: &str = "\
id\tkind\tcontext\treading\texpected\ttop1\trank
x1\t話題\tc\tr\t品詞\t品詞\t1
x2\t話題\tc\tr\t構成\t構成\t1
x3\t否定\tc\tr\tしよう\t仕様\t9
x4\t話題\tc\tr\t校正\t校正\t1
x6\t話題\tc\tr\t解凍\t解凍\t1
";

    #[test]
    fn compares_ranks_per_case() {
        let a = load_table(A).unwrap();
        let b = load_table(B).unwrap();
        let c = compare_tables(&a, &b);
        assert_eq!(c.a, (5, 2));
        assert_eq!(c.b, (5, 4));
        let by_id = |id: &str| c.rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!(by_id("x1").change(), Some(Change::Improved));
        assert_eq!(by_id("x2").change(), Some(Change::Same));
        assert_eq!(by_id("x3").change(), Some(Change::Worse));
        // 圏外から1位への改善
        assert_eq!(by_id("x4").change(), Some(Change::Improved));
        assert_eq!(by_id("x5").presence, Presence::OnlyA(Some(1)));
        assert_eq!(by_id("x6").presence, Presence::OnlyB(Some(1)));
        assert_eq!(c.count(Change::Improved), 2);
        assert_eq!(c.count(Change::Worse), 1);
        assert_eq!(c.count(Change::Same), 1);
    }

    #[test]
    fn render_shows_only_differences() {
        let c = compare_tables(&load_table(A).unwrap(), &load_table(B).unwrap());
        let md = render_comparison(&c, "a.tsv", "b.tsv");
        assert!(md.contains("a: a.tsv（5 件、1位 2 = 40.0%）"), "{md}");
        assert!(md.contains("b: b.tsv（5 件、1位 4 = 80.0%）"), "{md}");
        assert!(
            md.contains("| x1 | 話題 | 品詞 | 2 | 1 | 瀕死 | 品詞 | 改善 |"),
            "{md}"
        );
        assert!(
            md.contains("| x3 | 否定 | しよう | 3 | 9 | 仕様 | 仕様 | 悪化 |"),
            "{md}"
        );
        assert!(md.contains("| x4 | 話題 | 校正 | - | 1 |"), "{md}");
        assert!(md.contains("a のみ") && md.contains("b のみ"), "{md}");
        assert!(
            !md.contains("| x2 |"),
            "unchanged rows are not listed: {md}"
        );
    }

    #[test]
    fn identical_tables_have_no_differences() {
        let t = load_table(A).unwrap();
        let md = render_comparison(&compare_tables(&t, &t), "a", "a");
        assert!(md.contains("差分なし"), "{md}");
    }

    #[test]
    fn rejects_bad_tables() {
        for bad in [
            "",
            "kind\trank\nk\t1\n",
            "id\tkind\nx\tk\n",
            "id\trank\nx\t0\n",
            "id\trank\nx\tz\n",
            "id\trank\nx\t1\nx\t2\n",
            "id\trank\n\t1\n",
        ] {
            assert!(load_table(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn minimal_columns_are_enough() {
        let t = load_table("id\trank\na\t1\nb\t\n").unwrap();
        assert_eq!(t.len(), 2);
        let c = compare_tables(&t, &t);
        assert_eq!(c.a, (2, 1));
    }
}
