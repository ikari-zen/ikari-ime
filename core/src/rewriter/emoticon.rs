//! 顔文字と絵文字（`core/assets/emoticons.tsv`。ADR 0002 §12.2）。

use super::symbol::{CandidateTable, TableParseError};
use super::{RewriteCandidate, RewriteInput, Rewriter};

const BUNDLED_EMOTICONS: &str = include_str!("../../assets/emoticons.tsv");

pub struct EmoticonRewriter {
    table: CandidateTable,
}

impl EmoticonRewriter {
    /// 同梱の `emoticons.tsv` から作る。
    pub fn builtin() -> Self {
        // 同梱ファイルの正しさは単体テスト（bundled_table_is_valid）で確かめている
        Self::from_tsv(BUNDLED_EMOTICONS).expect("bundled emoticons.tsv must be valid")
    }

    /// 任意の TSV（`読み<TAB>表記<TAB>書式ID`）から作る。
    pub fn from_tsv(tsv: &str) -> Result<Self, TableParseError> {
        Ok(Self {
            table: CandidateTable::parse(tsv)?,
        })
    }
}

impl Rewriter for EmoticonRewriter {
    fn name(&self) -> &'static str {
        "emoticon"
    }

    fn rewrite(&self, input: &RewriteInput<'_>, out: &mut Vec<RewriteCandidate>) {
        self.table.lookup(input.reading, out);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::context::Context;
    use crate::rewriter::LocalDateTime;

    fn run(rw: &EmoticonRewriter, reading: &str) -> Vec<(u16, String)> {
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
        let rw = EmoticonRewriter::builtin();
        assert!(rw.table.reading_count() >= 20);
    }

    #[test]
    fn spec_readings_have_candidates() {
        let rw = EmoticonRewriter::builtin();
        for r in ["かお", "かおもじ", "えもじ", "わらい"] {
            assert!(!run(&rw, r).is_empty(), "{r}");
        }
        assert!(run(&rw, "かおもじ").iter().any(|(_, s)| s == "(^_^)"));
        assert!(run(&rw, "えもじ").iter().any(|(_, s)| s == "😀"));
    }

    #[test]
    fn exact_match_only() {
        let rw = EmoticonRewriter::builtin();
        for r in ["", "か", "かおも", "かおもじの", "顔文字"] {
            assert!(run(&rw, r).is_empty(), "{r}");
        }
    }

    #[test]
    fn format_ids_are_unique_in_bundled_file() {
        let tsv = BUNDLED_EMOTICONS;
        let mut ids = HashSet::new();
        for line in tsv.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
            let id: u16 = line.split('\t').nth(2).unwrap().parse().unwrap();
            assert!(ids.insert(id), "duplicate {id}");
        }
        assert!(ids.len() > 100);
    }

    #[test]
    fn bundled_readings_do_not_use_katakana() {
        // 読みは normalize_key 済みで渡されるので、ひらがなで書く
        for line in BUNDLED_EMOTICONS.lines().filter(|l| !l.starts_with('#')) {
            let reading = line.split('\t').next().unwrap();
            assert!(
                reading
                    .chars()
                    .all(|c| ('\u{3041}'..='\u{3096}').contains(&c) || c == 'ー'),
                "{reading}"
            );
        }
    }
}
