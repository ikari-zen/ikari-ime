//! ローマ字表と RomajiComposer（ADR 0001 §4.2）。

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fmt;

#[cfg(test)]
mod equivalence;

const BUILTIN_TSV: &str = include_str!("../assets/romaji.tsv");
const COMMENT_PREFIX: char = '#';
const COLUMN_SEPARATOR: char = '\t';
const MIN_COLUMNS: usize = 2;
const MAX_COLUMNS: usize = 3;

/// `#` で始まる行はコメント。ただし `#<TAB>` で始まる行は、記号 `#` の行として読む
fn is_comment(line: &str) -> bool {
    line.starts_with(COMMENT_PREFIX)
        && !line[COMMENT_PREFIX.len_utf8()..].starts_with(COLUMN_SEPARATOR)
}

/// 表の1行の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    output: String,
    /// 確定後に未確定バッファへ戻すローマ字（"kk" → っ + "k"）
    next_pending: String,
}

/// Mozc 互換の TSV。「入力<TAB>出力[<TAB>次の未確定]」。# で始まる行と空行は無視する。
///
/// 検索は Trie（文字をキーにした木）で行う。1 回の `lookup` で「完全一致する規則」と「より長い入力があるか」が
/// 同時に分かる（以前は `BTreeMap` を 2 回引いていた。その実装は `equivalence` のテストに参照として残している）。
#[derive(Debug, Clone)]
pub struct RomajiTable {
    /// 先頭（添字 0）が根。根は空の入力に当たり、規則を持たない
    nodes: Vec<Node>,
}

#[derive(Debug, Clone, Default)]
struct Node {
    /// この節までの文字列が入力に完全一致するときの結果
    rule: Option<Rule>,
    /// 次の 1 文字 -> 子の添字（文字の昇順。二分探索で引く）
    children: Vec<(char, usize)>,
}

/// `RomajiTable::lookup` の結果。
struct Lookup<'a> {
    /// 完全一致する規則
    exact: Option<&'a Rule>,
    /// 検索した文字列より長く、それで始まる入力が存在するか
    has_longer: bool,
}

#[derive(Debug)]
pub struct TableError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "romaji table line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for TableError {}

/// TSV を読み、入力 -> 規則の表にする。列数の不正、空の入力、入力の重複はエラー（行番号付き、1始まり）。
fn parse_rules(tsv: &str) -> Result<BTreeMap<String, Rule>, TableError> {
    let mut rules = BTreeMap::new();
    for (idx, raw) in tsv.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim_end_matches('\r');
        if line.is_empty() || is_comment(line) {
            continue;
        }
        let cols: Vec<&str> = line.split(COLUMN_SEPARATOR).collect();
        if !(MIN_COLUMNS..=MAX_COLUMNS).contains(&cols.len()) {
            return Err(TableError {
                line: line_no,
                message: format!(
                    "expected {MIN_COLUMNS} or {MAX_COLUMNS} tab-separated columns, found {}",
                    cols.len()
                ),
            });
        }
        let input = cols[0];
        if input.is_empty() {
            return Err(TableError {
                line: line_no,
                message: "input must not be empty".to_string(),
            });
        }
        let rule = Rule {
            output: cols[1].to_string(),
            next_pending: cols.get(2).copied().unwrap_or("").to_string(),
        };
        match rules.entry(input.to_string()) {
            Entry::Occupied(_) => {
                return Err(TableError {
                    line: line_no,
                    message: format!("duplicate input {input:?}"),
                });
            }
            Entry::Vacant(v) => {
                v.insert(rule);
            }
        }
    }
    Ok(rules)
}

impl RomajiTable {
    /// 列数の不正や入力の重複はエラー（行番号付き、1始まり）。
    pub fn parse(tsv: &str) -> Result<Self, TableError> {
        Ok(Self::from_rules(&parse_rules(tsv)?))
    }

    fn from_rules(rules: &BTreeMap<String, Rule>) -> Self {
        let mut nodes = vec![Node::default()];
        for (input, rule) in rules {
            let mut at = 0;
            for c in input.chars() {
                at = match nodes[at].children.binary_search_by_key(&c, |(key, _)| *key) {
                    Ok(found) => nodes[at].children[found].1,
                    Err(place) => {
                        nodes.push(Node::default());
                        let child = nodes.len() - 1;
                        nodes[at].children.insert(place, (c, child));
                        child
                    }
                };
            }
            nodes[at].rule = Some(rule.clone());
        }
        Self { nodes }
    }

    /// 同梱の既定表（`input/assets/romaji.tsv`）。
    pub fn builtin() -> Self {
        Self::parse(BUILTIN_TSV).expect("builtin romaji table must be valid")
    }

    /// `s` に完全一致する規則と、`s` より長く `s` で始まる入力があるかを、1 回の探索で返す。
    /// バイト列の前方一致と文字の前方一致は UTF-8 では同じなので、文字ごとの木で足りる。
    fn lookup(&self, s: &str) -> Lookup<'_> {
        let mut at = 0;
        for c in s.chars() {
            let children = &self.nodes[at].children;
            match children.binary_search_by_key(&c, |(key, _)| *key) {
                Ok(found) => at = children[found].1,
                Err(_) => {
                    return Lookup {
                        exact: None,
                        has_longer: false,
                    };
                }
            }
        }
        let node = &self.nodes[at];
        Lookup {
            exact: node.rule.as_ref(),
            has_longer: !node.children.is_empty(),
        }
    }

    fn exact(&self, s: &str) -> Option<&Rule> {
        self.lookup(s).exact
    }
}

pub struct RomajiComposer<'t> {
    table: &'t RomajiTable,
    kana: String,
    pending: String,
    /// 規則3で「処理し直す」文字（`push` の間だけ使う。領域を使い回して、打鍵ごとに確保しない）
    queue: Vec<char>,
}

impl<'t> RomajiComposer<'t> {
    pub fn new(table: &'t RomajiTable) -> Self {
        Self {
            table,
            kana: String::new(),
            pending: String::new(),
            queue: Vec::new(),
        }
    }

    /// 1文字を追加する。変換規則は ADR §4.2 の 1〜3。
    pub fn push(&mut self, c: char) {
        // 規則3で「処理し直す」文字列をキューで扱い、再帰を避ける。
        let table = self.table;
        self.queue.clear();
        self.queue.push(c);
        while let Some(c) = self.queue.pop() {
            self.pending.push(c);

            let found = table.lookup(&self.pending);
            if found.has_longer {
                // 規則1: より長い入力があるので待つ（pending 自体が完全一致でも待つ）
            } else if let Some(rule) = found.exact {
                // 規則2
                self.kana.push_str(&rule.output);
                self.pending.clear();
                self.pending.push_str(&rule.next_pending);
            } else {
                // 規則3: p を確定して c を処理し直す。
                self.pending.pop();
                if self.pending.is_empty() {
                    self.kana.push(c);
                } else if let Some(rule) = table.exact(&self.pending) {
                    self.kana.push_str(&rule.output);
                    self.pending.clear();
                    self.pending.push_str(&rule.next_pending);
                    self.queue.push(c);
                } else {
                    // p の先頭1文字を ASCII のまま確定し、残りと c を処理し直す。
                    let mut chars = self.pending.chars();
                    if let Some(first) = chars.next() {
                        self.kana.push(first);
                    }
                    self.queue.push(c);
                    for rest in chars.as_str().chars().rev() {
                        self.queue.push(rest);
                    }
                    self.pending.clear();
                }
            }
        }
    }

    /// まず pending から1文字削る。空なら kana の末尾を削る。何か削れたら true。
    pub fn backspace(&mut self) -> bool {
        self.pending.pop().is_some() || self.kana.pop().is_some()
    }

    /// 確定したかな
    pub fn kana(&self) -> &str {
        &self.kana
    }

    /// 未確定のローマ字（FR-07: 変換対象にしない）
    pub fn pending(&self) -> &str {
        &self.pending
    }

    /// 変換開始時に呼ぶ。pending が表の入力と完全一致すれば出力に置き換える（例: 末尾の "n" → ん）。
    /// 一致しなければ pending を残す（"へんk" の "k"）。
    pub fn flush(&mut self) {
        if let Some(rule) = self.table.exact(&self.pending) {
            self.kana.push_str(&rule.output);
            self.pending.clear();
            self.pending.push_str(&rule.next_pending);
        }
    }
}

pub struct Composition {
    pub kana: String,
    pub pending: String,
}

/// push をすべて行ってから flush する。
pub fn romaji_to_kana(table: &RomajiTable, input: &str) -> Composition {
    let mut composer = RomajiComposer::new(table);
    for c in input.chars() {
        composer.push(c);
    }
    composer.flush();
    Composition {
        kana: composer.kana,
        pending: composer.pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(input: &str) -> (String, String) {
        let table = RomajiTable::builtin();
        let c = romaji_to_kana(&table, input);
        (c.kana, c.pending)
    }

    fn kana(input: &str) -> String {
        conv(input).0
    }

    #[test]
    fn basic_syllables() {
        assert_eq!(kana("ka"), "か");
        assert_eq!(kana("kanji"), "かんじ");
        assert_eq!(kana("shi"), "し");
        assert_eq!(kana("si"), "し");
        assert_eq!(kana("kya"), "きゃ");
        assert_eq!(kana("cha"), "ちゃ");
        assert_eq!(kana("ja"), "じゃ");
        assert_eq!(kana("fa"), "ふぁ");
        assert_eq!(kana("va"), "ゔぁ");
        assert_eq!(kana("tsu"), "つ");
        assert_eq!(kana("wi"), "うぃ");
        assert_eq!(kana("we"), "うぇ");
        assert_eq!(kana("xa"), "ぁ");
        assert_eq!(kana("la"), "ぁ");
        assert_eq!(kana("xtu"), "っ");
        assert_eq!(kana("xya"), "ゃ");
        assert_eq!(kana("thi"), "てぃ");
        assert_eq!(kana("dhu"), "でゅ");
    }

    #[test]
    fn sokuon() {
        assert_eq!(kana("kka"), "っか");
        assert_eq!(kana("tte"), "って");
        assert_eq!(kana("sshi"), "っし");
        assert_eq!(kana("ccha"), "っちゃ");
    }

    #[test]
    fn n_handling() {
        assert_eq!(kana("nn"), "ん");
        assert_eq!(kana("kan'i"), "かんい");
        assert_eq!(kana("kanji"), "かんじ");
        assert_eq!(conv("nk"), ("ん".to_string(), "k".to_string()));
        assert_eq!(kana("nya"), "にゃ");
        assert_eq!(kana("konnnichiha"), "こんにちは");
    }

    #[test]
    fn trailing_n_is_flushed() {
        // "n" だけは表に完全一致するので flush で ん になる
        assert_eq!(conv("n"), ("ん".to_string(), String::new()));
        assert_eq!(conv("hen"), ("へん".to_string(), String::new()));
    }

    #[test]
    fn unconfirmed_consonant_is_kept_pending() {
        // FR-07
        assert_eq!(conv("henk"), ("へん".to_string(), "k".to_string()));
        assert_eq!(conv("hensh"), ("へん".to_string(), "sh".to_string()));
        assert_eq!(conv("kk"), ("っ".to_string(), "k".to_string()));
    }

    #[test]
    fn non_matching_keys_pass_through() {
        assert_eq!(conv("kx"), ("k".to_string(), "x".to_string()));
        assert_eq!(kana("A"), "A");
        assert_eq!(kana("1"), "1");
        assert_eq!(conv("kyk"), ("ky".to_string(), "k".to_string()));
    }

    #[test]
    fn symbols_and_z_keys() {
        assert_eq!(kana("zh"), "←");
        assert_eq!(kana("zj"), "↓");
        assert_eq!(kana("zk"), "↑");
        assert_eq!(kana("zl"), "→");
        assert_eq!(kana("z."), "…");
        assert_eq!(kana("z-"), "〜");
        assert_eq!(kana("z/"), "・");
        assert_eq!(kana("z["), "『");
        assert_eq!(kana("z]"), "』");
        assert_eq!(kana("-"), "ー");
        assert_eq!(kana(","), "、");
        assert_eq!(kana("."), "。");
        assert_eq!(kana("["), "「");
        assert_eq!(kana("]"), "」");
        assert_eq!(kana("/"), "・");
        assert_eq!(kana("~"), "〜");
        assert_eq!(kana("ra-menn"), "らーめん");
    }

    #[test]
    fn z_prefix_waits_then_falls_back() {
        // "z" 単独は "za" などの先頭なので待つ
        assert_eq!(conv("z"), (String::new(), "z".to_string()));
        assert_eq!(kana("zu"), "ず");
    }

    #[test]
    fn backspace_behaviour() {
        let table = RomajiTable::builtin();
        let mut c = RomajiComposer::new(&table);
        for ch in "kak".chars() {
            c.push(ch);
        }
        assert_eq!((c.kana(), c.pending()), ("か", "k"));
        assert!(c.backspace());
        assert_eq!((c.kana(), c.pending()), ("か", ""));
        assert!(c.backspace());
        assert_eq!((c.kana(), c.pending()), ("", ""));
        assert!(!c.backspace());
    }

    #[test]
    fn flush_keeps_non_matching_pending() {
        let table = RomajiTable::builtin();
        let mut c = RomajiComposer::new(&table);
        for ch in "hek".chars() {
            c.push(ch);
        }
        c.flush();
        assert_eq!((c.kana(), c.pending()), ("へ", "k"));
    }

    #[test]
    fn parse_skips_comments_and_blank_lines() {
        let t = RomajiTable::parse("# c\n\nka\tか\nkk\tっ\tk\n").unwrap();
        let hash = RomajiTable::parse("# comment\n#\t＃\n").unwrap();
        assert_eq!(romaji_to_kana(&hash, "#").kana, "＃");
        // 同梱の表は、日本語モードの ASCII 記号を全角にする（Mozc と同じ）。「n'」は従来どおり「ん」
        let builtin = RomajiTable::builtin();
        assert_eq!(romaji_to_kana(&builtin, "nani?").kana, "なに？");
        assert_eq!(romaji_to_kana(&builtin, "#!(a)").kana, "＃！（あ）");
        assert_eq!(romaji_to_kana(&builtin, "kan'i").kana, "かんい");
        // 同梱の表は、日本語モードの ASCII 記号を全角にする（Mozc と同じ）。「n'」は従来どおり「ん」
        let builtin = RomajiTable::builtin();
        assert_eq!(romaji_to_kana(&builtin, "nani?").kana, "なに？");
        assert_eq!(romaji_to_kana(&builtin, "#!(a)").kana, "＃！（あ）");
        assert_eq!(romaji_to_kana(&builtin, "kan'i").kana, "かんい");
        let c = romaji_to_kana(&t, "kka");
        assert_eq!(c.kana, "っか");
    }

    #[test]
    fn parse_accepts_crlf() {
        let t = RomajiTable::parse("ka\tか\r\n").unwrap();
        assert_eq!(romaji_to_kana(&t, "ka").kana, "か");
    }

    #[test]
    fn parse_error_wrong_column_count() {
        let err = RomajiTable::parse("ka\tか\nkx\n").unwrap_err();
        assert_eq!(err.line, 2);
        let err = RomajiTable::parse("a\tb\tc\td\n").unwrap_err();
        assert_eq!(err.line, 1);
    }

    #[test]
    fn parse_error_duplicate_input() {
        let err = RomajiTable::parse("# head\nka\tか\n\nka\tこ\n").unwrap_err();
        assert_eq!(err.line, 4);
        assert!(err.to_string().contains("line 4"));
    }

    #[test]
    fn parse_error_empty_input() {
        let err = RomajiTable::parse("\tか\n").unwrap_err();
        assert_eq!(err.line, 1);
    }

    #[test]
    fn custom_table_replaces_builtin() {
        let t = RomajiTable::parse("a\tあ\n").unwrap();
        // 表にない文字は ASCII のまま通る
        assert_eq!(romaji_to_kana(&t, "ka").kana, "kあ");
    }
}
