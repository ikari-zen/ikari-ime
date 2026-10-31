//! Trie 版の `RomajiComposer` が、以前の `BTreeMap` 版（参照実装）と同じ出力になることの確かめ（issue #161）。
//!
//! 参照実装は、置き換える前の `has_longer`（範囲検索）、`exact`（検索）、`push`（キューと `take`）をそのまま残したもの。
//! 軽い範囲（全キー、全前置、全キーのペア、乱数の列、自作の小さな表）は通常のテストで流す。
//! 重い全列挙は `#[ignore]`（`cargo test -p ikari-input -- --ignored` で流す）。

use std::collections::BTreeMap;
use std::ops::Bound;

use super::{RomajiComposer, RomajiTable, Rule, parse_rules};

// ---------------------------------------------------------------- 参照実装（置き換え前のコード）

struct RefTable {
    rules: BTreeMap<String, Rule>,
}

impl RefTable {
    fn parse(tsv: &str) -> Self {
        Self {
            rules: parse_rules(tsv).expect("valid table"),
        }
    }

    fn exact(&self, s: &str) -> Option<&Rule> {
        self.rules.get(s)
    }

    fn has_longer(&self, s: &str) -> bool {
        self.rules
            .range::<str, _>((Bound::Excluded(s), Bound::Unbounded))
            .take_while(|(k, _)| k.starts_with(s))
            .next()
            .is_some()
    }
}

struct RefComposer<'t> {
    table: &'t RefTable,
    kana: String,
    pending: String,
}

impl<'t> RefComposer<'t> {
    fn new(table: &'t RefTable) -> Self {
        Self {
            table,
            kana: String::new(),
            pending: String::new(),
        }
    }

    fn push(&mut self, c: char) {
        let mut queue: Vec<char> = vec![c];
        queue.reverse();
        while let Some(c) = queue.pop() {
            let mut s = std::mem::take(&mut self.pending);
            s.push(c);

            if self.table.has_longer(&s) {
                self.pending = s;
            } else if let Some(rule) = self.table.exact(&s) {
                self.kana.push_str(&rule.output);
                self.pending = rule.next_pending.clone();
            } else {
                let mut p = s;
                p.pop();
                if p.is_empty() {
                    self.kana.push(c);
                } else if let Some(rule) = self.table.exact(&p) {
                    self.kana.push_str(&rule.output);
                    self.pending = rule.next_pending.clone();
                    queue.push(c);
                } else {
                    let mut chars = p.chars();
                    if let Some(first) = chars.next() {
                        self.kana.push(first);
                    }
                    queue.push(c);
                    for rest in chars.as_str().chars().rev() {
                        queue.push(rest);
                    }
                }
            }
        }
    }

    fn backspace(&mut self) -> bool {
        self.pending.pop().is_some() || self.kana.pop().is_some()
    }

    fn flush(&mut self) {
        if let Some(rule) = self.table.exact(&self.pending) {
            self.kana.push_str(&rule.output);
            self.pending = rule.next_pending.clone();
        }
    }
}

// ---------------------------------------------------------------- 比較の道具

const BUILTIN: &str = include_str!("../../assets/romaji.tsv");

/// 操作の列。文字の push、backspace、flush。
#[derive(Clone, Copy)]
enum Op {
    Push(char),
    Backspace,
    Flush,
}

struct Pair<'t> {
    new: RomajiComposer<'t>,
    old: RefComposer<'t>,
}

impl<'t> Pair<'t> {
    fn new(table: &'t RomajiTable, reference: &'t RefTable) -> Self {
        Self {
            new: RomajiComposer::new(table),
            old: RefComposer::new(reference),
        }
    }

    /// 1 操作を両方に行い、かな・未確定・戻り値が同じであることを確かめる。
    fn apply(&mut self, op: Op, context: &dyn Fn() -> String) {
        match op {
            Op::Push(c) => {
                self.new.push(c);
                self.old.push(c);
            }
            Op::Backspace => {
                assert_eq!(self.new.backspace(), self.old.backspace(), "{}", context());
            }
            Op::Flush => {
                self.new.flush();
                self.old.flush();
            }
        }
        assert_eq!(self.new.kana(), self.old.kana, "kana: {}", context());
        assert_eq!(
            self.new.pending(),
            self.old.pending,
            "pending: {}",
            context()
        );
    }
}

/// 文字列を 1 文字ずつ push し、最後に flush する（途中の状態もすべて比べる）。
fn compare_text(table: &RomajiTable, reference: &RefTable, text: &str) {
    let mut pair = Pair::new(table, reference);
    let mut seen = String::new();
    for c in text.chars() {
        seen.push(c);
        pair.apply(Op::Push(c), &|| format!("after {seen:?}"));
    }
    pair.apply(Op::Flush, &|| format!("flush after {text:?}"));
}

fn compare_ops(table: &RomajiTable, reference: &RefTable, ops: &[Op]) {
    let mut pair = Pair::new(table, reference);
    for (index, op) in ops.iter().enumerate() {
        pair.apply(*op, &|| format!("op #{index}"));
    }
}

fn keys(tsv: &str) -> Vec<String> {
    parse_rules(tsv)
        .expect("valid table")
        .keys()
        .cloned()
        .collect()
}

/// 表のキーに出てくる文字（重複なし、昇順）。
fn alphabet(tsv: &str) -> Vec<char> {
    let mut chars: Vec<char> = keys(tsv).iter().flat_map(|key| key.chars()).collect();
    chars.sort_unstable();
    chars.dedup();
    chars
}

/// 固定シードの疑似乱数（xorshift64*）。外部の依存を足さない。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(bound).unwrap_or(1)).unwrap_or(0)
    }
}

/// `alphabet` 上の長さ `length` の列を、すべて比べる。数えた列の数を返す。
fn compare_all_sequences(
    table: &RomajiTable,
    reference: &RefTable,
    alphabet: &[char],
    length: usize,
) -> usize {
    let mut count = 0;
    let mut indexes = vec![0_usize; length];
    loop {
        let text: String = indexes.iter().map(|index| alphabet[*index]).collect();
        compare_text(table, reference, &text);
        count += 1;
        // 繰り上がり（最後の桁から進める）
        let mut digit = length;
        loop {
            if digit == 0 {
                return count;
            }
            digit -= 1;
            indexes[digit] += 1;
            if indexes[digit] < alphabet.len() {
                break;
            }
            indexes[digit] = 0;
        }
    }
}

// ---------------------------------------------------------------- 自作の小さな表

/// 接頭辞が重なる表、1 文字のキー、促音（次の未確定が複数文字）、拗音、撥音、記号、非 ASCII のキー。
const CUSTOM_TABLES: &[(&str, &str)] = &[
    (
        "overlapping prefixes",
        "a\t1\nab\t2\nabc\t3\nb\t4\nbc\t5\nbcd\t6\n",
    ),
    (
        "sokuon with multi-character next pending",
        "kk\tっ\tk\nka\tか\nkya\tきゃ\nk\tK\nkkk\tっっ\tk\nkky\tっ\tky\n",
    ),
    (
        "nasal and palatal",
        "n\tん\nna\tな\nnn\tん\nn'\tん\nnya\tにゃ\nny\tNY\ncha\tちゃ\nch\tCH\nc\tC\n",
    ),
    (
        "single characters and symbols",
        "-\tー\n,\t、\n.\t。\n/\t・\n[\t「\n]\t」\n#\t＃\n",
    ),
    (
        "non-ASCII keys",
        "あ\tA\nあい\tAI\nあいう\tAIU\nい\tI\nかな\tkana\n",
    ),
    (
        "a key that is a prefix of a key with a next pending",
        "t\tT\ntt\tっ\tt\ntta\tTTA\nta\tた\n",
    ),
    ("empty-output rule", "x\t\nxy\t\tx\n"),
    (
        "z keys fall back",
        "z\tZ\nzh\t←\nzj\t↓\nzk\t↑\nzl\t→\nzz\tzz\n",
    ),
];

// ---------------------------------------------------------------- 通常のテスト（軽い範囲）

#[test]
fn every_builtin_key_and_prefix_gives_the_same_output() {
    let table = RomajiTable::builtin();
    let reference = RefTable::parse(BUILTIN);
    for key in keys(BUILTIN) {
        // 途中の状態（すべての前置）も 1 文字ごとに比べている
        compare_text(&table, &reference, &key);
    }
}

#[test]
fn every_pair_of_builtin_keys_gives_the_same_output() {
    let table = RomajiTable::builtin();
    let reference = RefTable::parse(BUILTIN);
    let all = keys(BUILTIN);
    let mut pairs = 0;
    for first in &all {
        for second in &all {
            compare_text(&table, &reference, &format!("{first}{second}"));
            pairs += 1;
        }
    }
    assert_eq!(pairs, all.len() * all.len());
}

#[test]
fn seeded_random_sequences_with_edits_give_the_same_output() {
    let table = RomajiTable::builtin();
    let reference = RefTable::parse(BUILTIN);
    // 表の文字に、キーにない文字（大文字、数字、かな、改行）を少し混ぜる
    let mut letters = alphabet(BUILTIN);
    letters.extend(['A', 'Z', '0', '9', 'あ', '\n']);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..4000 {
        let length = 1 + rng.below(30);
        let ops: Vec<Op> = (0..length)
            .map(|_| match rng.below(10) {
                0 => Op::Backspace,
                1 => Op::Flush,
                _ => Op::Push(letters[rng.below(letters.len())]),
            })
            .collect();
        compare_ops(&table, &reference, &ops);
    }
}

#[test]
fn custom_tables_give_the_same_output_for_all_short_sequences() {
    for (name, tsv) in CUSTOM_TABLES {
        let table = RomajiTable::parse(tsv).expect("valid table");
        let reference = RefTable::parse(tsv);
        // 表の文字に、キーにない文字を 1 つ足す
        let mut letters = alphabet(tsv);
        letters.push('Q');
        let mut total = 0;
        for length in 1..=5 {
            total += compare_all_sequences(&table, &reference, &letters, length);
        }
        assert!(total > 0, "{name}");
        // 全キーとそのペア
        let all = keys(tsv);
        for first in &all {
            compare_text(&table, &reference, first);
            for second in &all {
                compare_text(&table, &reference, &format!("{first}{second}"));
            }
        }
    }
}

#[test]
fn replacing_the_table_with_a_user_table_gives_the_same_output() {
    // ユーザーの romaji.tsv の差し替え（組み込み表の一部を上書きし、記号を足した表）
    let mut user = String::from(BUILTIN);
    user.push_str("\n;;\t；；\n@@\t＠\t@\n");
    let table = RomajiTable::parse(&user).expect("valid table");
    let reference = RefTable::parse(&user);
    for key in keys(&user) {
        compare_text(&table, &reference, &key);
    }
    let text = "kyoukakkoiinnjanai;;@@";
    compare_text(&table, &reference, text);
}

#[test]
fn trie_lookup_agrees_with_the_map_for_every_key_and_prefix() {
    let table = RomajiTable::builtin();
    let reference = RefTable::parse(BUILTIN);
    let mut strings: Vec<String> = Vec::new();
    for key in keys(BUILTIN) {
        // 全前置と、1 文字足したもの
        for (end, _) in key.char_indices().skip(1) {
            strings.push(key[..end].to_string());
        }
        strings.push(key.clone());
        strings.push(format!("{key}Q"));
    }
    for s in &strings {
        let found = table.lookup(s);
        assert_eq!(found.exact, reference.exact(s), "exact {s:?}");
        assert_eq!(
            found.has_longer,
            reference.has_longer(s),
            "has_longer {s:?}"
        );
    }
}

// ---------------------------------------------------------------- 重い全列挙（#[ignore]）

/// ASCII の表示できる文字（0x21〜0x7e）の、長さ 3 までの全列。
#[test]
#[ignore = "heavy: about 0.84 million sequences"]
fn all_printable_ascii_sequences_up_to_length_three() {
    let table = RomajiTable::builtin();
    let reference = RefTable::parse(BUILTIN);
    let ascii: Vec<char> = (0x21_u8..=0x7e).map(char::from).collect();
    let mut total = 0;
    for length in 1..=3 {
        total += compare_all_sequences(&table, &reference, &ascii, length);
    }
    eprintln!("compared {total} sequences");
    assert_eq!(total, 94 + 94 * 94 + 94 * 94 * 94);
}

/// 英小文字の長さ 4 の全列と、長さ 5 の一部（先頭 3 文字を固定した全列ではなく、母音と子音の代表）。
#[test]
#[ignore = "heavy: about 0.46 million sequences"]
fn all_lowercase_sequences_of_length_four() {
    let table = RomajiTable::builtin();
    let reference = RefTable::parse(BUILTIN);
    let lower: Vec<char> = ('a'..='z').collect();
    let total = compare_all_sequences(&table, &reference, &lower, 4);
    eprintln!("compared {total} sequences");
    assert_eq!(total, 26 * 26 * 26 * 26);
}

/// 小さな表すべてで、長さ 8 までの全列。
#[test]
#[ignore = "heavy: custom tables up to length eight"]
fn custom_tables_up_to_length_eight() {
    let mut total = 0;
    for (_, tsv) in CUSTOM_TABLES {
        let table = RomajiTable::parse(tsv).expect("valid table");
        let reference = RefTable::parse(tsv);
        let mut letters = alphabet(tsv);
        letters.push('Q');
        // 文字が多い表は長さを抑える（組み合わせが 10 万程度に収まるように）
        for length in 6..=8 {
            if letters.len().pow(u32::try_from(length).unwrap_or(0)) <= 200_000 {
                total += compare_all_sequences(&table, &reference, &letters, length);
            }
        }
    }
    eprintln!("compared {total} sequences");
    assert!(total > 0);
}
