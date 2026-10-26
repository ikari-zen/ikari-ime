//! 期待値が候補に出るかの機械的な検証（ADR 0002 §15.2-1）。
//!
//! 出ないケース（辞書で期待値の読みが再現できない、など）に `unreachable` の印を付けて評価から除く。
//! 印は評価セットの `tags` 列に書く。

use std::collections::HashSet;

use crate::{CaseResult, TAG_UNREACHABLE};

const COMMENT_PREFIX: char = '#';
const COL_ID: &str = "id";
const COL_TAGS: &str = "tags";
const TAG_SEPARATOR: &str = ",";

/// 順位が見つからなかった（`max_rank` 以内に候補として出ない）ケースの id
pub fn unreachable_ids(results: &[CaseResult]) -> Vec<String> {
    results
        .iter()
        .filter(|r| r.rank.is_none())
        .map(|r| r.case.id.clone())
        .collect()
}

/// 評価セットの TSV の、指定した id のケースの `tags` に `unreachable` を足した文字列を返す。
/// コメント行、空行、他のケースの行は元のまま残す。`tags` 列がなければエラー。
/// すでに印があるケースは変えない。
pub fn mark_unreachable(tsv: &str, ids: &HashSet<String>) -> Result<String, String> {
    let mut out = String::with_capacity(tsv.len());
    let mut cols: Option<(usize, usize)> = None; // (id 列, tags 列)
    for raw in tsv.split_inclusive('\n') {
        let (body, eol) = match raw.strip_suffix("\r\n") {
            Some(b) => (b, "\r\n"),
            None => match raw.strip_suffix('\n') {
                Some(b) => (b, "\n"),
                None => (raw, ""),
            },
        };
        if body.trim().is_empty() || body.starts_with(COMMENT_PREFIX) {
            out.push_str(raw);
            continue;
        }
        let mut cells: Vec<String> = body.split('\t').map(str::to_owned).collect();
        let Some((id_idx, tags_idx)) = cols else {
            let find = |name: &str| cells.iter().position(|c| c.trim() == name);
            let id_idx = find(COL_ID).ok_or("missing column `id`")?;
            let tags_idx =
                find(COL_TAGS).ok_or("missing column `tags` (add it to the header first)")?;
            cols = Some((id_idx, tags_idx));
            out.push_str(raw);
            continue;
        };
        if cells
            .get(id_idx)
            .is_some_and(|id| ids.contains(id.as_str()))
        {
            if cells.len() <= tags_idx {
                cells.resize(tags_idx + 1, String::new());
            }
            let mut tags: Vec<String> = cells[tags_idx]
                .split(TAG_SEPARATOR)
                .map(|t| t.trim().to_owned())
                .filter(|t| !t.is_empty())
                .collect();
            if !tags.iter().any(|t| t == TAG_UNREACHABLE) {
                tags.push(TAG_UNREACHABLE.to_owned());
                cells[tags_idx] = tags.join(TAG_SEPARATOR);
            }
            out.push_str(&cells.join("\t"));
            out.push_str(eol);
        } else {
            out.push_str(raw);
        }
    }
    if cols.is_none() {
        return Err("missing header line".to_owned());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_cases;
    use crate::testutil::result;

    fn set(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| (*s).to_owned()).collect()
    }

    const TSV: &str = "\
# comment
id\tkind\ttags\tcontext\treading\texpected
a\tk\tllm\tc\tr\te
b\tk\t\tc\tr\te

c\tk\tllm,unreachable\tc\tr\te
d\tk
";

    #[test]
    fn unreachable_ids_are_those_without_a_rank() {
        let results = vec![
            result("a", "k", &[], Some(1)),
            result("b", "k", &[], None),
            result("c", "k", &[], Some(40)),
            result("d", "k", &[], None),
        ];
        assert_eq!(unreachable_ids(&results), ["b", "d"]);
    }

    #[test]
    fn marks_only_the_given_ids_and_keeps_everything_else() {
        let out = mark_unreachable(TSV, &set(&["a", "b", "d"])).unwrap();
        // d は元の行が短い（セルが足りない）ので、tags 列まで広げて印を付ける
        let expected = "\
# comment
id\tkind\ttags\tcontext\treading\texpected
a\tk\tllm,unreachable\tc\tr\te
b\tk\tunreachable\tc\tr\te

c\tk\tllm,unreachable\tc\tr\te
d\tk\tunreachable
";
        assert_eq!(out, expected);
    }

    #[test]
    fn marking_is_idempotent_and_loadable() {
        let tsv =
            "id\tkind\ttags\tcontext\treading\texpected\na\tk\tllm\tc\tr\te\nb\tk\t\tc\tr\te\n";
        let once = mark_unreachable(tsv, &set(&["a"])).unwrap();
        let twice = mark_unreachable(&once, &set(&["a"])).unwrap();
        assert_eq!(once, twice);
        let cases = load_cases(&once).unwrap();
        assert!(cases[0].is_unreachable());
        assert_eq!(cases[0].tags, ["llm", "unreachable"]);
        assert!(!cases[1].is_unreachable());
    }

    #[test]
    fn preserves_crlf_and_missing_final_newline() {
        let tsv = "id\ttags\r\na\tx\r\nb\t";
        let out = mark_unreachable(tsv, &set(&["a", "b"])).unwrap();
        assert_eq!(out, "id\ttags\r\na\tx,unreachable\r\nb\tunreachable");
    }

    #[test]
    fn requires_id_and_tags_columns() {
        assert!(mark_unreachable("id\tkind\na\tk\n", &set(&["a"])).is_err());
        assert!(mark_unreachable("kind\ttags\nk\tt\n", &set(&["a"])).is_err());
        assert!(mark_unreachable("", &set(&[])).is_err());
        assert!(mark_unreachable("# only comments\n", &set(&[])).is_err());
    }

    #[test]
    fn no_ids_means_no_changes() {
        assert_eq!(mark_unreachable(TSV, &set(&[])).unwrap(), TSV);
    }
}
