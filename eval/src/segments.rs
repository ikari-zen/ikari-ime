//! 文節の評価セット（`eval/sets/segments.tsv`）の読み込み（ADR 0006 §7.1）。
//!
//! 共通の列（id、kind、context、reading、expected、tags、note、split）は `EvalCase` として読み、
//! この評価だけの 2 列を足す。
//!
//! - `expected_segments`: `expected` の区切りの正解。読みを `|` で区切る（連結すると `reading` に戻る）
//! - `target`: 一部だけを変えた目標の表記。空なら `expected` が目標

use std::fmt;

use crate::{EvalCase, EvalSetError, load_cases};

/// `expected_segments` の文節の区切り。`expected` の別解の区切り（`|`）とは別の列なので、同じ文字を使う
pub const SEGMENT_SEPARATOR: char = '|';
const COL_SEGMENTS: &str = "expected_segments";
const COL_TARGET: &str = "target";
const COMMENT_PREFIX: char = '#';
const CELL_SEPARATOR: char = '\t';

/// 種別: 最初の区切りの正解率を見るケース
pub const KIND_BREAK: &str = "区切り";
/// 種別: 区切りはそのままで、一部の文節を選び直すケース
pub const KIND_PART: &str = "部分";
/// 種別: 複合名詞の一部だけを選び直すケース（区切り直しが要る）
pub const KIND_STRETCH: &str = "伸縮";

/// 文節の評価セットの 1 ケース
#[derive(Clone, Debug)]
pub struct SegmentCase {
    pub case: EvalCase,
    /// `expected` の区切りの正解（各文節の読み）
    pub expected_segments: Vec<String>,
    /// 目標の表記。空なら `expected`
    pub target: String,
}

impl SegmentCase {
    /// 打鍵数の目標の表記
    pub fn goal(&self) -> &str {
        if self.target.is_empty() {
            &self.case.expected
        } else {
            &self.target
        }
    }

    /// 区切りの正解の境界（読みの文字位置。先頭と末尾は含まない）
    pub fn gold_boundaries(&self) -> Vec<usize> {
        boundaries_of(self.expected_segments.iter().map(String::as_str))
    }
}

/// 文節の読みの並びから、境界（読みの文字位置の累計。末尾は含まない）を作る
pub fn boundaries_of<'a>(readings: impl IntoIterator<Item = &'a str>) -> Vec<usize> {
    let mut out = Vec::new();
    let mut pos = 0;
    for r in readings {
        pos += r.chars().count();
        out.push(pos);
    }
    out.pop();
    out
}

/// 文節の評価セットを読む。`expected_segments` 列が必須で、連結すると `reading` に一致しなければならない
pub fn load_segment_cases(tsv: &str) -> Result<Vec<SegmentCase>, EvalSetError> {
    let cases = load_cases(tsv)?;
    let extras = read_extra_columns(tsv)?;
    debug_assert_eq!(cases.len(), extras.len());
    cases
        .into_iter()
        .zip(extras)
        .map(|(case, (line, segments, target))| {
            let expected_segments: Vec<String> = segments
                .split(SEGMENT_SEPARATOR)
                .map(str::to_owned)
                .collect();
            if expected_segments.iter().any(String::is_empty) {
                return Err(line_error(line, "empty segment in expected_segments"));
            }
            if expected_segments.concat() != case.reading {
                return Err(line_error(
                    line,
                    format!(
                        "expected_segments `{segments}` does not join to reading `{}`",
                        case.reading
                    ),
                ));
            }
            Ok(SegmentCase {
                case,
                expected_segments,
                target,
            })
        })
        .collect()
}

fn line_error(line: usize, message: impl fmt::Display) -> EvalSetError {
    EvalSetError {
        line,
        message: message.to_string(),
    }
}

/// (行番号, expected_segments, target) をデータ行の順に返す。コメント行と空行は `load_cases` と同じく除く
fn read_extra_columns(tsv: &str) -> Result<Vec<(usize, String, String)>, EvalSetError> {
    let mut header: Option<(usize, Option<usize>)> = None;
    let mut out = Vec::new();
    for (i, raw) in tsv.lines().enumerate() {
        let line_no = i + 1;
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with(COMMENT_PREFIX) {
            continue;
        }
        let cells: Vec<&str> = line.split(CELL_SEPARATOR).collect();
        let Some((seg_col, target_col)) = header else {
            let find = |name: &str| cells.iter().position(|c| c.trim() == name);
            let seg_col = find(COL_SEGMENTS).ok_or_else(|| {
                line_error(line_no, format!("missing required column `{COL_SEGMENTS}`"))
            })?;
            header = Some((seg_col, find(COL_TARGET)));
            continue;
        };
        let get = |col: usize| cells.get(col).copied().unwrap_or("").to_owned();
        let segments = get(seg_col);
        if segments.is_empty() {
            return Err(line_error(
                line_no,
                format!("empty required value `{COL_SEGMENTS}`"),
            ));
        }
        out.push((line_no, segments, target_col.map(get).unwrap_or_default()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Split, split_of_id};

    const HEAD: &str = "id\tkind\tcontext\treading\texpected\texpected_segments\ttarget\n";

    #[test]
    fn loads_segments_and_target() {
        let tsv = format!(
            "# c\n{HEAD}a\t区切り\t\tはじめてしった\t初めて知った\tはじめて|しった\t\nb\t伸縮\t\tりぽじとりめい\tリポジトリ名\tりぽじとりめい\tリポジトリ銘\n"
        );
        let cases = load_segment_cases(&tsv).unwrap();
        assert_eq!(cases.len(), 2);
        assert_eq!(cases[0].expected_segments, ["はじめて", "しった"]);
        assert_eq!(cases[0].goal(), "初めて知った", "empty target -> expected");
        assert_eq!(cases[0].gold_boundaries(), [4]);
        assert_eq!(cases[1].goal(), "リポジトリ銘");
        assert!(cases[1].gold_boundaries().is_empty());
    }

    #[test]
    fn rejects_segments_that_do_not_join_to_the_reading() {
        let tsv = format!("{HEAD}a\tk\t\tはじめて\t初めて\tはじ|めまて\t\n");
        let e = load_segment_cases(&tsv).unwrap_err();
        assert_eq!(e.line, 2);
        assert!(e.message.contains("join"), "{e}");

        let tsv = format!("{HEAD}a\tk\t\tはじめて\t初めて\tはじめて|\t\n");
        assert!(
            load_segment_cases(&tsv)
                .unwrap_err()
                .message
                .contains("empty")
        );

        let tsv = "id\tkind\tcontext\treading\texpected\na\tk\t\tr\te\n";
        let e = load_segment_cases(tsv).unwrap_err();
        assert!(e.message.contains(COL_SEGMENTS), "{e}");
    }

    #[test]
    fn boundaries_count_characters_not_bytes() {
        assert_eq!(boundaries_of(["あ", "いう", "えお"]), [1, 3]);
        assert!(boundaries_of(["あいう"]).is_empty());
        assert!(boundaries_of([]).is_empty());
    }

    #[test]
    fn bundled_segments_set_is_consistent() {
        let cases = load_segment_cases(include_str!("../sets/segments.tsv")).unwrap();
        for kind in [KIND_BREAK, KIND_PART, KIND_STRETCH] {
            let n = cases.iter().filter(|c| c.case.kind == kind).count();
            assert!(n >= 40, "{kind}: {n}");
        }
        assert!(
            cases
                .iter()
                .all(|c| c.case.tags.contains(&"llm".to_owned())),
            "every case is tagged llm"
        );
        let mut seen = std::collections::HashSet::new();
        for c in &cases {
            let id = &c.case.id;
            assert!(seen.insert(id.clone()), "duplicate id {id}");
            // split 列は id のハッシュと一致する（調整は dev だけで行う）
            assert_eq!(c.case.split, Some(split_of_id(id)), "{id}");
            // 部分と伸縮は、期待と違う目標を持つ
            if c.case.kind == KIND_BREAK {
                assert!(c.target.is_empty(), "{id}");
            } else {
                assert!(!c.target.is_empty() && c.target != c.case.expected, "{id}");
            }
            // 伸縮の最初の区切りは、目標のために区切り直しが要る形（複合名詞の 1 文節を含む）
            if c.case.kind == KIND_STRETCH {
                assert!(c.expected_segments.len() <= 2, "{id}");
            }
        }
        let test = cases
            .iter()
            .filter(|c| c.case.split == Some(Split::Test))
            .count();
        assert!((cases.len() / 8..cases.len() / 3).contains(&test), "{test}");
    }
}
