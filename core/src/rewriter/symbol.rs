//! 読みから記号（`core/assets/symbols.tsv`。ADR 0002 §12.2）。

use std::collections::{HashMap, HashSet};
use std::fmt;

use ikari_dict_format::reading::normalize_key;

use super::{RewriteCandidate, RewriteInput, Rewriter};

const BUNDLED_SYMBOLS: &str = include_str!("../../assets/symbols.tsv");
const COMMENT_PREFIX: char = '#';
const COLUMN_COUNT: usize = 3;

/// 読みごとの候補表（`読み<TAB>表記<TAB>書式ID` の TSV）。symbol と emoticon で共有する。
#[derive(Clone, Debug, Default)]
pub(super) struct CandidateTable {
    by_reading: HashMap<String, Vec<(String, u16)>>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TableParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for TableParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for TableParseError {}

fn err(line: usize, message: impl Into<String>) -> TableParseError {
    TableParseError {
        line,
        message: message.into(),
    }
}

impl CandidateTable {
    /// 空行と `#` で始まる行は読み飛ばす。読みは正規化済み（ひらがな）であること。
    /// 書式IDはファイル内で一意でなければならない（学習が書式IDで行われるため）。
    pub(super) fn parse(tsv: &str) -> Result<Self, TableParseError> {
        let mut table = Self::default();
        let mut ids = HashSet::new();
        let mut pairs = HashSet::new();
        for (i, raw) in tsv.lines().enumerate() {
            let line_no = i + 1;
            let line = raw.trim_end_matches('\r');
            if line.trim().is_empty() || line.starts_with(COMMENT_PREFIX) {
                continue;
            }
            let cells: Vec<&str> = line.split('\t').collect();
            if cells.len() != COLUMN_COUNT {
                return Err(err(
                    line_no,
                    format!("expected {COLUMN_COUNT} columns, got {}", cells.len()),
                ));
            }
            let (reading, surface) = (cells[0], cells[1]);
            if reading.is_empty() || surface.is_empty() {
                return Err(err(line_no, "empty reading or surface"));
            }
            if normalize_key(reading) != reading {
                return Err(err(
                    line_no,
                    format!("reading `{reading}` is not normalized"),
                ));
            }
            let format: u16 = cells[2]
                .parse()
                .map_err(|_| err(line_no, format!("bad format id `{}`", cells[2])))?;
            if !ids.insert(format) {
                return Err(err(line_no, format!("duplicate format id {format}")));
            }
            if !pairs.insert((reading, surface)) {
                return Err(err(
                    line_no,
                    format!("duplicate row `{reading}` `{surface}`"),
                ));
            }
            table
                .by_reading
                .entry(reading.to_owned())
                .or_default()
                .push((surface.to_owned(), format));
        }
        Ok(table)
    }

    pub(super) fn lookup(&self, reading: &str, out: &mut Vec<RewriteCandidate>) {
        if let Some(rows) = self.by_reading.get(reading) {
            out.extend(
                rows.iter()
                    .map(|(surface, format)| RewriteCandidate::new(surface.clone(), *format)),
            );
        }
    }

    #[cfg(test)]
    pub(super) fn reading_count(&self) -> usize {
        self.by_reading.len()
    }
}

pub struct SymbolRewriter {
    table: CandidateTable,
}

impl SymbolRewriter {
    /// 同梱の `symbols.tsv`（`読み<TAB>表記<TAB>書式ID`）から作る。
    pub fn builtin() -> Self {
        // 同梱ファイルの正しさは単体テスト（bundled_table_is_valid）で確かめている
        Self::from_tsv(BUNDLED_SYMBOLS).expect("bundled symbols.tsv must be valid")
    }

    /// 任意の TSV から作る（ユーザーが差し替える用途とテスト用）。
    pub fn from_tsv(tsv: &str) -> Result<Self, TableParseError> {
        Ok(Self {
            table: CandidateTable::parse(tsv)?,
        })
    }
}

impl Rewriter for SymbolRewriter {
    fn name(&self) -> &'static str {
        "symbol"
    }

    fn rewrite(&self, input: &RewriteInput<'_>, out: &mut Vec<RewriteCandidate>) {
        self.table.lookup(input.reading, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use crate::rewriter::LocalDateTime;

    fn run(rw: &SymbolRewriter, reading: &str) -> Vec<(u16, String)> {
        let ctx = Context::new();
        let input = RewriteInput {
            reading,
            context: &ctx,
            now: LocalDateTime {
                year: 2026,
                month: 10,
                day: 11,
                hour: 0,
                minute: 0,
                weekday: 6,
            },
        };
        let mut out = Vec::new();
        rw.rewrite(&input, &mut out);
        out.into_iter().map(|c| (c.format, c.surface)).collect()
    }

    #[test]
    fn bundled_table_is_valid() {
        let t = CandidateTable::parse(BUNDLED_SYMBOLS).expect("valid");
        assert!(t.reading_count() >= 100);
    }

    #[test]
    fn spec_examples() {
        let rw = SymbolRewriter::builtin();
        let arrows: Vec<String> = run(&rw, "やじるし").into_iter().map(|(_, s)| s).collect();
        assert_eq!(&arrows[..4], ["←", "→", "↑", "↓"]);
        assert_eq!(run(&rw, "ほし")[0].1, "★");
        assert_eq!(run(&rw, "ほし")[1].1, "☆");
        assert_eq!(run(&rw, "おなじ")[0].1, "々");
        for r in ["まる", "しかく", "さんかく", "こめ", "ゆうびん"] {
            assert!(!run(&rw, r).is_empty(), "{r}");
        }
        assert_eq!(run(&rw, "ゆうびん")[0].1, "〒");
    }

    #[test]
    fn exact_match_only() {
        let rw = SymbolRewriter::builtin();
        for r in ["", "やじる", "やじるしの", "ヤジルシ", "矢印", "ほ"] {
            assert!(run(&rw, r).is_empty(), "{r}");
        }
    }

    #[test]
    fn format_ids_are_unique_in_bundled_file() {
        let rw = SymbolRewriter::builtin();
        let mut ids = HashSet::new();
        for rows in rw.table.by_reading.values() {
            for (_, f) in rows {
                assert!(ids.insert(*f), "duplicate {f}");
            }
        }
    }

    #[test]
    fn rows_keep_file_order_and_format_ids() {
        let rw = SymbolRewriter::from_tsv("# c\nあ\tA\t7\n\nあ\tB\t3\nい\tC\t9\r\n").unwrap();
        assert_eq!(
            run(&rw, "あ"),
            vec![(7, "A".to_owned()), (3, "B".to_owned())]
        );
        assert_eq!(run(&rw, "い"), vec![(9, "C".to_owned())]);
    }

    #[test]
    fn rejects_bad_tables() {
        let cases = [
            ("あ\tA\n", 1),
            ("あ\tA\t1\textra\n", 1),
            ("あ\t\t1\n", 1),
            ("\tA\t1\n", 1),
            ("ア\tA\t1\n", 1),
            ("あ\tA\tx\n", 1),
            ("あ\tA\t-1\n", 1),
            ("あ\tA\t70000\n", 1),
            ("あ\tA\t1\nい\tB\t1\n", 2),
            ("あ\tA\t1\nあ\tA\t2\n", 2),
        ];
        for (tsv, line) in cases {
            let e = SymbolRewriter::from_tsv(tsv)
                .err()
                .unwrap_or_else(|| panic!("{tsv:?}"));
            assert_eq!(e.line, line, "{tsv:?}: {e}");
        }
    }

    #[test]
    fn empty_table_is_valid() {
        let rw = SymbolRewriter::from_tsv("").unwrap();
        assert!(run(&rw, "ほし").is_empty());
    }
}
