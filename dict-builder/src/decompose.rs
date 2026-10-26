//! chiVe にない lemma のベクトルを、Sudachi の分割の部分のベクトルの平均で代わりにする（ADR 0012 §12。「分解」）。
//!
//! lex.csv の分割の列（15 列目 = A 単位、16 列目 = B 単位。0 始まり）は、`/` で区切った行番号の列。行番号は、辞書を作った
//! ときと同じ順に（small、core、notcore）連結した全体での 0 始まりの番号で、`lex.rs` の辞書形ID と同じ数え方
//! （除外する行も数える）。`U` で始まる値（ユーザー辞書の参照）は使わない。
//!
//! 部分の lemma は、参照先の行の正規化表記。部分が chiVe にないときは、その部分の分割を 1 段だけたどる
//! （深さ 2 まで）。**部分が 1 つでも chiVe にない語には付けない**（一部だけの平均は語の意味から外れやすい）。
//! 分割を持たない語（沼る、陰キャ、推し変）にも付かない。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::lex::unescape_unicode;
use crate::{BuildError, open_reader, read_line_at};

const COLUMNS: usize = 19;
const COL_SURFACE: usize = 4;
const COL_LEMMA: usize = 12;
const COL_SPLIT_A: usize = 15;
const COL_SPLIT_B: usize = 16;
const NO_VALUE: &str = "*";
const SPLIT_SEPARATOR: char = '/';
/// 部分が chiVe にないとき、さらにたどる段数
const MAX_EXTRA_DEPTH: usize = 1;
/// 分解として成り立つ部分の数の下限（1 つでは元の語と同じ）
const MIN_PARTS: usize = 2;

/// 全行の lemma と、使える分割（B があれば B、なければ A）。
#[derive(Debug, Default)]
pub(crate) struct SplitTable {
    lemmas: Vec<String>,
    splits: Vec<Option<Box<[u32]>>>,
}

/// `12/34` のような分割の値。`*`、空、数字以外を含むもの（`U1` など）は `None`。
fn parse_split(value: &str) -> Option<Box<[u32]>> {
    if value.is_empty() || value == NO_VALUE {
        return None;
    }
    value
        .split(SPLIT_SEPARATOR)
        .map(|part| part.parse::<u32>().ok())
        .collect::<Option<Vec<u32>>>()
        .map(Vec::into_boxed_slice)
}

impl SplitTable {
    /// lex.csv を、辞書を作ったときと同じ順に読む。
    pub(crate) fn read(files: &[impl AsRef<Path>]) -> Result<Self, BuildError> {
        let mut table = Self::default();
        let mut line = String::new();
        for path in files {
            let path = path.as_ref();
            let mut reader = open_reader(path)?;
            let mut lineno = 0usize;
            loop {
                lineno += 1;
                if read_line_at(&mut reader, &mut line, path, lineno)? == 0 {
                    break;
                }
                table.push_line(&line, path, lineno)?;
            }
        }
        Ok(table)
    }

    pub(crate) fn push_line(
        &mut self,
        line: &str,
        path: &Path,
        lineno: usize,
    ) -> Result<(), BuildError> {
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() != COLUMNS {
            return Err(BuildError::Input {
                file: path.to_path_buf(),
                line: lineno,
                message: format!("expected {COLUMNS} columns, found {}", cols.len()),
            });
        }
        let lemma = match cols[COL_LEMMA] {
            "" | NO_VALUE => unescape_unicode(cols[COL_SURFACE]).into_owned(),
            lemma => unescape_unicode(lemma).into_owned(),
        };
        self.lemmas.push(lemma);
        self.splits
            .push(parse_split(cols[COL_SPLIT_B]).or_else(|| parse_split(cols[COL_SPLIT_A])));
        Ok(())
    }

    fn lemma(&self, row: u32) -> Option<&str> {
        self.lemmas.get(row as usize).map(String::as_str)
    }

    fn split(&self, row: u32) -> Option<&[u32]> {
        self.splits.get(row as usize)?.as_deref()
    }

    /// 分解の対象になりうる行の部分の lemma（深さ 2 まで）を集める。chiVe の読み込みのときに、
    /// これらの語のベクトルも拾うための集合（chiVe にあるかどうかはまだ分からないので、上の集合になる）。
    pub(crate) fn wanted_parts(&self, is_target: impl Fn(&str) -> bool) -> HashSet<String> {
        let mut wanted = HashSet::new();
        for (row, lemma) in self.lemmas.iter().enumerate() {
            let Some(split) = self.split(row as u32) else {
                continue;
            };
            if !is_target(lemma) {
                continue;
            }
            for &part in split {
                if let Some(text) = self.lemma(part) {
                    wanted.insert(text.to_string());
                }
                for &sub in self.split(part).unwrap_or_default() {
                    if let Some(text) = self.lemma(sub) {
                        wanted.insert(text.to_string());
                    }
                }
            }
        }
        wanted
    }

    /// 行 `row`（lemma が `lemma`）の部分の lemma の列。部分が chiVe（`has_vector`）にないときは 1 段だけ分割をたどる。
    /// 1 つでも決まらない部分があるとき、部分が `MIN_PARTS` 未満のときは `None`。
    fn parts_of(&self, row: u32, has_vector: &impl Fn(&str) -> bool) -> Option<Vec<String>> {
        let lemma = self.lemma(row)?;
        let mut parts = Vec::new();
        for &part in self.split(row)? {
            if part == row {
                return None;
            }
            self.expand(part, lemma, MAX_EXTRA_DEPTH, has_vector, &mut parts)?;
        }
        (parts.len() >= MIN_PARTS).then_some(parts)
    }

    /// 部分 `row` を chiVe にある lemma の列に直して `out` に足す。`parent` は元の語の lemma
    /// （同じ lemma の部分を指す分割は成り立たない）。
    fn expand(
        &self,
        row: u32,
        parent: &str,
        depth: usize,
        has_vector: &impl Fn(&str) -> bool,
        out: &mut Vec<String>,
    ) -> Option<()> {
        let lemma = self.lemma(row)?;
        if lemma == parent {
            return None;
        }
        if has_vector(lemma) {
            out.push(lemma.to_string());
            return Some(());
        }
        if depth == 0 {
            return None;
        }
        let split = self.split(row)?;
        if split == [row] {
            return None;
        }
        for &sub in split {
            self.expand(sub, lemma, depth - 1, has_vector, out)?;
        }
        Some(())
    }

    /// 分解の対象の lemma（`is_target`）ごとに、部分の lemma の列を求める。同じ lemma の行が複数あるときは、
    /// 行の順で最初に決まったもの。結果は最初に決まった行の順。
    pub(crate) fn decompose(
        &self,
        is_target: impl Fn(&str) -> bool,
        has_vector: impl Fn(&str) -> bool,
    ) -> Vec<(String, Vec<String>)> {
        let mut done: HashMap<&str, ()> = HashMap::new();
        let mut result = Vec::new();
        for (row, lemma) in self.lemmas.iter().enumerate() {
            if done.contains_key(lemma.as_str()) || self.split(row as u32).is_none() {
                continue;
            }
            if !is_target(lemma) {
                continue;
            }
            if let Some(parts) = self.parts_of(row as u32, &has_vector) {
                done.insert(lemma.as_str(), ());
                result.push((lemma.clone(), parts));
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 19 列の行。分割は A、B の順
    fn line(lemma: &str, split_a: &str, split_b: &str) -> String {
        format!(
            "{lemma},1,1,100,{lemma},名詞,普通名詞,一般,*,*,*,カ,{lemma},*,A,{split_a},{split_b},*,*"
        )
    }

    fn table(rows: &[(&str, &str, &str)]) -> SplitTable {
        let mut t = SplitTable::default();
        for (i, (lemma, a, b)) in rows.iter().enumerate() {
            t.push_line(&line(lemma, a, b), &PathBuf::from("t.csv"), i + 1)
                .unwrap();
        }
        t
    }

    fn vocab(words: &[&'static str]) -> impl Fn(&str) -> bool {
        let set: HashSet<&'static str> = words.iter().copied().collect();
        move |w| set.contains(w)
    }

    #[test]
    fn split_values_are_parsed_strictly() {
        assert_eq!(parse_split("1/22/3").as_deref(), Some(&[1, 22, 3][..]));
        assert_eq!(parse_split("7").as_deref(), Some(&[7][..]));
        assert_eq!(parse_split("*"), None);
        assert_eq!(parse_split(""), None);
        // ユーザー辞書の参照は使わない
        assert_eq!(parse_split("U1/2"), None);
        assert_eq!(parse_split("1/x"), None);
    }

    #[test]
    fn b_split_wins_and_a_split_is_the_fallback() {
        // 行 0 コイン、1 ロッカー、2 コインロッカー（B=0/1）、3 少年、4 マンガ、5 少年マンガ（A=3/4）
        let t = table(&[
            ("コイン", "*", "*"),
            ("ロッカー", "*", "*"),
            ("コインロッカー", "0/1", "0/1"),
            ("少年", "*", "*"),
            ("マンガ", "*", "*"),
            ("少年マンガ", "3/4", "*"),
        ]);
        let all = |_: &str| true;
        let got = t.decompose(all, vocab(&["コイン", "ロッカー", "少年", "マンガ"]));
        let got: Vec<(&str, Vec<&str>)> = got
            .iter()
            .map(|(l, p)| (l.as_str(), p.iter().map(String::as_str).collect()))
            .collect();
        assert!(got.contains(&("コインロッカー", vec!["コイン", "ロッカー"])));
        assert!(got.contains(&("少年マンガ", vec!["少年", "マンガ"])));
    }

    #[test]
    fn a_missing_part_descends_one_level_only() {
        // 0 親 1 ガ 2 チャ 3 ガチャ（A=1/2）4 親ガチャ（A=0/3）
        let t = table(&[
            ("親", "*", "*"),
            ("ガ", "*", "*"),
            ("チャ", "*", "*"),
            ("ガチャ", "1/2", "*"),
            ("親ガチャ", "0/3", "*"),
        ]);
        // ガチャ が chiVe にあれば 親 + ガチャ
        let got = t.decompose(|l| l == "親ガチャ", vocab(&["親", "ガチャ"]));
        assert_eq!(
            got,
            vec![("親ガチャ".to_string(), vec!["親".into(), "ガチャ".into()])]
        );
        // ガチャ がなく、ガ と チャ があれば 1 段たどって 親 + ガ + チャ
        let got = t.decompose(|l| l == "親ガチャ", vocab(&["親", "ガ", "チャ"]));
        assert_eq!(
            got,
            vec![(
                "親ガチャ".to_string(),
                vec!["親".into(), "ガ".into(), "チャ".into()]
            )]
        );
        // 1 つでも欠けると付けない
        assert!(
            t.decompose(|l| l == "親ガチャ", vocab(&["親", "ガ"]))
                .is_empty()
        );
        assert!(
            t.decompose(|l| l == "親ガチャ", vocab(&["ガ", "チャ"]))
                .is_empty()
        );
    }

    #[test]
    fn two_levels_below_are_not_followed() {
        // 0 あ 1 い 2 あい（A=0/1）3 う 4 あいう（A=2/3）5 おあいう（A=7/4）6 お 7 お
        let t = table(&[
            ("あ", "*", "*"),
            ("い", "*", "*"),
            ("あい", "0/1", "*"),
            ("う", "*", "*"),
            ("あいう", "2/3", "*"),
            ("おあいう", "7/4", "*"),
            ("お", "*", "*"),
            ("お", "*", "*"),
        ]);
        // おあいう = お + あいう。あいう が chiVe になく、あい もなければ 2 段目は見ない
        let words = vocab(&["お", "あ", "い", "う"]);
        assert!(t.decompose(|l| l == "おあいう", &words).is_empty());
        // あいう の 1 段下（あい、う）が chiVe にあれば成り立つ
        let words = vocab(&["お", "あい", "う"]);
        assert_eq!(
            t.decompose(|l| l == "おあいう", &words),
            vec![(
                "おあいう".to_string(),
                vec!["お".into(), "あい".into(), "う".into()]
            )]
        );
    }

    #[test]
    fn rows_without_a_usable_split_get_nothing() {
        // 分割なし、ユーザー辞書の参照、自分自身を指す分割、範囲外の行
        let t = table(&[
            ("沼る", "*", "*"),
            ("陰キャ", "*", "*"),
            ("参照", "U3", "*"),
            ("自己", "3/3", "*"),
            ("範囲外", "99/100", "*"),
        ]);
        let all = vocab(&["沼", "る", "陰", "キャ"]);
        assert!(t.decompose(|_| true, all).is_empty());
    }

    #[test]
    fn one_part_is_not_a_decomposition_and_targets_are_respected() {
        let t = table(&[
            ("犬", "*", "*"),
            ("犬の", "0", "*"),
            ("猫", "*", "*"),
            ("猫犬", "2/0", "*"),
        ]);
        let words = vocab(&["犬", "猫"]);
        // 部分が 1 つ（犬の = 犬）は成り立たない。猫犬 は対象でなければ付けない
        let got = t.decompose(|l| l == "犬の", &words);
        assert!(got.is_empty());
        let got = t.decompose(|l| l == "猫犬", &words);
        assert_eq!(got.len(), 1);
        assert!(t.decompose(|l| l == "ない", &words).is_empty());
    }

    #[test]
    fn same_lemma_takes_the_first_row_that_works() {
        // 0 あ 1 い 2 あい（分割が未登録の部分 99 を含む → 失敗）3 あい（0/1 → 成功）
        let t = table(&[
            ("あ", "*", "*"),
            ("い", "*", "*"),
            ("あい", "0/99", "*"),
            ("あい", "0/1", "*"),
        ]);
        let got = t.decompose(|l| l == "あい", vocab(&["あ", "い"]));
        assert_eq!(
            got,
            vec![("あい".to_string(), vec!["あ".into(), "い".into()])]
        );
    }

    #[test]
    fn wanted_parts_covers_two_levels() {
        let t = table(&[
            ("親", "*", "*"),
            ("ガ", "*", "*"),
            ("チャ", "*", "*"),
            ("ガチャ", "1/2", "*"),
            ("親ガチャ", "0/3", "*"),
        ]);
        let wanted = t.wanted_parts(|l| l == "親ガチャ");
        for w in ["親", "ガチャ", "ガ", "チャ"] {
            assert!(wanted.contains(w), "{w}");
        }
        assert!(!wanted.contains("親ガチャ"));
    }

    #[test]
    fn wrong_column_count_is_an_error_with_position() {
        let mut t = SplitTable::default();
        let err = t.push_line("a,b", &PathBuf::from("x.csv"), 5).unwrap_err();
        assert!(err.to_string().contains("x.csv:5"), "{err}");
    }
}
