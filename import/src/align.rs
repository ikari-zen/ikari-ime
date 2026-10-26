//! 強制アライメント（ADR 0002 §11.3）。辞書で「表記の読み」を再現して、文を語に分ける。
//!
//! 状態は (表記の位置 i, 読みの位置 j) と、直前の語の右ID。辺は 3 種類。
//! - 辞書の辺: `common_prefix_search(読み[j..])` のうち、表記が `表記[i..]` の接頭辞と一致するもの
//! - 文字そのままの辺: 表記の 1 文字が、かな、記号、英数字として読みの 1 文字と一致する
//! - 未知語の辺: 表記の漢字・カタカナ・英数字の連続を、1 文字以上の読みに当てる（1 文につき 1 回まで）
//!
//! 最小コストのパスを採用する。パスがなければ `None`（その文は捨てる）。

use std::collections::{BTreeMap, HashMap};

use ikari_core::{Dictionary, Grammar, WordId};
use ikari_dict_format::reading::normalize_key;

use crate::schema::{is_cjk_ideograph, is_kana};

/// 文字そのままの辺のコスト。辞書の語（数千）より十分高くして、辞書で説明できる所では使わない。
const LITERAL_COST: i64 = 8_000;
/// 未知語の辺の基本コスト。文字そのままの辺を何文字か使うより高く、辞書の語より遥かに高い。
const UNKNOWN_BASE_COST: i64 = 20_000;
const UNKNOWN_COST_PER_SURFACE_CHAR: i64 = 500;
const UNKNOWN_COST_PER_READING_CHAR: i64 = 300;
/// 未知語にする表記の最大文字数
pub const MAX_UNKNOWN_SURFACE_CHARS: usize = 12;
/// 表記 1 文字あたりに当てる読みの最大文字数（に定数を足す）
const MAX_READING_PER_SURFACE_CHAR: usize = 3;
const MAX_READING_EXTRA: usize = 2;
/// 状態の数の上限（異常に複雑な入力で時間とメモリを使い切らないため）
const MAX_STATES: usize = 200_000;
const PROLONGED_SOUND_MARK: char = '\u{30FC}';

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    /// 辞書の語
    Dictionary,
    /// 文字そのまま（かな、記号、英数字）
    Literal,
    /// 辞書にない語（推定した読み）
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlignedToken {
    pub surface: String,
    pub reading: String,
    pub word: Option<WordId>,
    pub left_id: u16,
    pub right_id: u16,
    pub pos_id: u16,
    pub kind: TokenKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alignment {
    pub tokens: Vec<AlignedToken>,
    pub cost: i64,
}

impl Alignment {
    pub fn uses_unknown(&self) -> bool {
        self.tokens.iter().any(|t| t.kind == TokenKind::Unknown)
    }
}

struct State {
    right_id: u16,
    used_unknown: bool,
    cost: i64,
    prev: Option<usize>,
    token: Option<AlignedToken>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Kanji,
    Katakana,
    Alnum,
    Other,
}

fn class_of(c: char) -> CharClass {
    if is_cjk_ideograph(c) {
        CharClass::Kanji
    } else if matches!(c, '\u{30A1}'..='\u{30FA}' | '\u{30FC}') {
        CharClass::Katakana
    } else if c.is_alphanumeric() && !is_kana(c) {
        CharClass::Alnum
    } else {
        CharClass::Other
    }
}

fn byte_offsets(chars: &[char]) -> Vec<usize> {
    let mut off = Vec::with_capacity(chars.len() + 1);
    let mut b = 0;
    for c in chars {
        off.push(b);
        b += c.len_utf8();
    }
    off.push(b);
    off
}

/// 辞書の辺 1 本（読みの終わりの位置でまとめ直す前の形）
struct DictEdge {
    reading_chars: usize,
    surface: String,
    surface_chars: usize,
    word: WordId,
    left_id: u16,
    right_id: u16,
    pos_id: u16,
    cost: i64,
}

/// 文を辞書で語に分ける。`reading` は `normalize_key` 済みであること。
pub fn align(
    dict: &dyn Dictionary,
    grammar: &dyn Grammar,
    surface: &str,
    reading: &str,
) -> Option<Alignment> {
    let s: Vec<char> = surface.chars().collect();
    let r: Vec<char> = reading.chars().collect();
    if s.is_empty() || r.is_empty() {
        return None;
    }
    let (n, m) = (s.len(), r.len());
    let s_off = byte_offsets(&s);
    let r_off = byte_offsets(&r);
    let unk = grammar.unknown();
    let bos = grammar.bos_eos_id();
    let conn = |prev: u16, next: u16| i64::from(grammar.connection_cost(prev, next));

    let mut states = vec![State {
        right_id: bos,
        used_unknown: false,
        cost: 0,
        prev: None,
        token: None,
    }];
    let mut cells: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    cells.insert((0, 0), vec![0]);
    let mut finals: Vec<usize> = Vec::new();
    let mut known_surface: HashMap<(usize, usize), bool> = HashMap::new();

    // 辺は i と j をどちらも増やすので、(i, j) の辞書順が位相順になる
    while let Some(((i, j), ids)) = cells.pop_first() {
        if (i, j) == (n, m) {
            finals = ids;
            continue;
        }
        if i >= n || j >= m {
            continue;
        }

        // 辞書の辺は状態によらないので、セルごとに 1 回だけ集める
        let mut dict_edges: Vec<DictEdge> = Vec::new();
        let rest_surface = &surface[s_off[i]..];
        dict.common_prefix_search(&reading[r_off[j]..], &mut |len, e| {
            if e.surface.is_empty() || !rest_surface.starts_with(e.surface) {
                return;
            }
            let end_byte = r_off[j] + len;
            let Ok(end) = r_off.binary_search(&end_byte) else {
                return;
            };
            dict_edges.push(DictEdge {
                reading_chars: end - j,
                surface: e.surface.to_string(),
                surface_chars: e.surface.chars().count(),
                word: e.id,
                left_id: e.left_id,
                right_id: e.right_id,
                pos_id: e.pos_id,
                cost: i64::from(e.cost),
            });
        });

        // 漢字とカタカナ（長音記号を除く）は辞書の語か未知語でなければならない
        let literal_ok = match class_of(s[i]) {
            CharClass::Kanji => false,
            CharClass::Katakana => s[i] == PROLONGED_SOUND_MARK,
            CharClass::Alnum | CharClass::Other => true,
        };
        let literal = (literal_ok && normalize_key(&s[i].to_string()).starts_with(r[j]))
            .then_some((s[i], r[j]));
        let run_class = class_of(s[i]);
        let run_len = if matches!(
            run_class,
            CharClass::Kanji | CharClass::Katakana | CharClass::Alnum
        ) {
            s[i..]
                .iter()
                .take_while(|c| class_of(**c) == run_class)
                .count()
        } else {
            0
        };

        for &id in &ids {
            let (prev_right, used_unknown, prev_cost) = {
                let st = &states[id];
                (st.right_id, st.used_unknown, st.cost)
            };
            let push = |states: &mut Vec<State>,
                        cells: &mut BTreeMap<(usize, usize), Vec<usize>>,
                        to: (usize, usize),
                        cost: i64,
                        used_unknown: bool,
                        token: AlignedToken| {
                let right_id = token.right_id;
                let existing = cells.get(&to).and_then(|v| {
                    v.iter().copied().find(|&k| {
                        states[k].right_id == right_id && states[k].used_unknown == used_unknown
                    })
                });
                let new_state = State {
                    right_id,
                    used_unknown,
                    cost,
                    prev: Some(id),
                    token: Some(token),
                };
                match existing {
                    Some(k) if states[k].cost <= cost => {}
                    Some(k) => states[k] = new_state,
                    None => {
                        states.push(new_state);
                        let k = states.len() - 1;
                        cells.entry(to).or_default().push(k);
                    }
                }
            };

            for e in &dict_edges {
                let cost = prev_cost + e.cost + conn(prev_right, e.left_id);
                push(
                    &mut states,
                    &mut cells,
                    (i + e.surface_chars, j + e.reading_chars),
                    cost,
                    used_unknown,
                    AlignedToken {
                        surface: e.surface.clone(),
                        reading: reading[r_off[j]..r_off[j + e.reading_chars]].to_string(),
                        word: Some(e.word),
                        left_id: e.left_id,
                        right_id: e.right_id,
                        pos_id: e.pos_id,
                        kind: TokenKind::Dictionary,
                    },
                );
            }

            if let Some((sc, rc)) = literal {
                let cost = prev_cost + LITERAL_COST + conn(prev_right, unk.left_id);
                push(
                    &mut states,
                    &mut cells,
                    (i + 1, j + 1),
                    cost,
                    used_unknown,
                    AlignedToken {
                        surface: sc.to_string(),
                        reading: rc.to_string(),
                        word: None,
                        left_id: unk.left_id,
                        right_id: unk.right_id,
                        pos_id: unk.pos_id,
                        kind: TokenKind::Literal,
                    },
                );
            }

            if !used_unknown && run_len > 0 {
                for l in 1..=run_len.min(MAX_UNKNOWN_SURFACE_CHARS) {
                    // 辞書にある表記を、別の読みの未知語にはしない（LLM の読み違いは捨てる）
                    let known = *known_surface
                        .entry((i, l))
                        .or_insert_with(|| has_surface(dict, &surface[s_off[i]..s_off[i + l]]));
                    if known {
                        continue;
                    }
                    let max_k = (MAX_READING_PER_SURFACE_CHAR * l + MAX_READING_EXTRA).min(m - j);
                    for k in 1..=max_k {
                        if i + l > n {
                            break;
                        }
                        // 表記と同じ文字がそのまま読みにある英数字は、文字そのままの辺で説明できる
                        if run_class == CharClass::Alnum && s[i..i + l] == r[j..j + k] {
                            continue;
                        }
                        let cost = prev_cost
                            + UNKNOWN_BASE_COST
                            + UNKNOWN_COST_PER_SURFACE_CHAR * l as i64
                            + UNKNOWN_COST_PER_READING_CHAR * k as i64
                            + conn(prev_right, unk.left_id);
                        push(
                            &mut states,
                            &mut cells,
                            (i + l, j + k),
                            cost,
                            true,
                            AlignedToken {
                                surface: surface[s_off[i]..s_off[i + l]].to_string(),
                                reading: reading[r_off[j]..r_off[j + k]].to_string(),
                                word: None,
                                left_id: unk.left_id,
                                right_id: unk.right_id,
                                pos_id: unk.pos_id,
                                kind: TokenKind::Unknown,
                            },
                        );
                    }
                }
            }
        }
        if states.len() > MAX_STATES {
            return None;
        }
    }

    let best = finals
        .into_iter()
        .map(|id| (states[id].cost + conn(states[id].right_id, bos), id))
        .min_by_key(|(cost, id)| (*cost, *id))?;
    let mut tokens = Vec::new();
    let mut cur = Some(best.1);
    while let Some(id) = cur {
        if let Some(t) = &states[id].token {
            tokens.push(t.clone());
        }
        cur = states[id].prev;
    }
    tokens.reverse();
    Some(Alignment {
        tokens,
        cost: best.0,
    })
}

/// 辞書にこの表記そのもののエントリがあるか。
fn has_surface(dict: &dyn Dictionary, text: &str) -> bool {
    let mut found = false;
    dict.common_prefix_search_surface(text, &mut |len, _| found |= len == text.len());
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use ikari_dict_format::reading::normalize_key;

    fn run(m: &Mini, surface: &str, reading: &str) -> Option<Alignment> {
        align(&m.dict, &m.grammar, surface, &normalize_key(reading))
    }

    fn pairs(a: &Alignment) -> Vec<(&str, &str)> {
        a.tokens
            .iter()
            .map(|t| (t.surface.as_str(), t.reading.as_str()))
            .collect()
    }

    #[test]
    fn splits_a_sentence_with_the_dictionary() {
        let m = mini();
        let a = run(
            &m,
            "ユーザー辞書に単語を登録する",
            "ゆーざーじしょにたんごをとうろくする",
        )
        .unwrap();
        assert_eq!(
            pairs(&a),
            [
                ("ユーザー", "ゆーざー"),
                ("辞書", "じしょ"),
                ("に", "に"),
                ("単語", "たんご"),
                ("を", "を"),
                ("登録", "とうろく"),
                ("する", "する"),
            ]
        );
        assert!(!a.uses_unknown());
        assert!(a.tokens.iter().all(|t| t.kind == TokenKind::Dictionary));
        assert!(a.tokens.iter().all(|t| t.word.is_some()));
    }

    #[test]
    fn homophone_is_chosen_by_surface_not_cost() {
        let m = mini();
        let a = run(&m, "瀕死", "ひんし").unwrap();
        assert_eq!(pairs(&a), [("瀕死", "ひんし")]);
        let a = run(&m, "品詞", "ひんし").unwrap();
        assert_eq!(pairs(&a), [("品詞", "ひんし")]);
    }

    #[test]
    fn punctuation_and_ascii_are_literal_tokens() {
        let m = mini();
        let a = run(&m, "辞書に単語、API3つ。", "じしょにたんご、API3つ。").unwrap();
        let lits: String = a
            .tokens
            .iter()
            .filter(|t| t.kind == TokenKind::Literal)
            .map(|t| t.surface.as_str())
            .collect();
        assert_eq!(lits, "、API3つ。");
        assert!(!a.uses_unknown());
        assert!(
            a.tokens
                .iter()
                .all(|t| t.surface == t.reading || t.kind == TokenKind::Dictionary)
        );
    }

    #[test]
    fn katakana_surface_with_hiragana_reading_aligns_literally() {
        let m = mini();
        // 辞書にないカタカナ語は、文字そのままの辺ではなく未知語になる（登録候補になる）
        let a = run(&m, "イカリに", "いかりに").unwrap();
        assert!(a.uses_unknown());
        let unk: Vec<_> = a
            .tokens
            .iter()
            .filter(|t| t.kind == TokenKind::Unknown)
            .map(|t| (t.surface.as_str(), t.reading.as_str()))
            .collect();
        assert_eq!(unk, [("イカリ", "いかり")]);
    }

    #[test]
    fn unknown_kanji_word_gets_an_estimated_reading() {
        let m = mini();
        let a = run(&m, "怒里を登録する", "いかりをとうろくする").unwrap();
        assert_eq!(a.tokens[0].kind, TokenKind::Unknown);
        assert_eq!(
            (a.tokens[0].surface.as_str(), a.tokens[0].reading.as_str()),
            ("怒里", "いかり")
        );
        assert_eq!(a.tokens[1].surface, "を");
        assert!(a.tokens[1].word.is_some());
    }

    #[test]
    fn at_most_one_unknown_word_per_sentence() {
        let m = mini();
        assert!(run(&m, "怒里と鬼才", "いかりときさい").is_none());
    }

    #[test]
    fn mismatched_readings_cannot_be_aligned() {
        let m = mini();
        assert!(run(&m, "辞書に", "じしょを").is_none());
        assert!(run(&m, "辞書", "たんご").is_none());
        assert!(run(&m, "", "あ").is_none());
        assert!(run(&m, "あ", "").is_none());
        // 読みが余る
        assert!(run(&m, "辞書", "じしょに").is_none());
    }

    #[test]
    fn alignment_is_deterministic() {
        let m = mini();
        let (s, r) = ("その話を初めて知った", "そのはなしをはじめてしった");
        let a = run(&m, s, r).unwrap();
        for _ in 0..5 {
            assert_eq!(run(&m, s, r).unwrap(), a);
        }
    }

    #[test]
    fn very_long_sentences_finish() {
        let m = mini();
        let s = "辞書".repeat(100);
        let r = "じしょ".repeat(100);
        let a = run(&m, &s, &r).unwrap();
        assert_eq!(a.tokens.len(), 100);
    }

    #[test]
    fn long_unalignable_input_returns_none_quickly() {
        let m = mini();
        let s = "怒".repeat(200);
        let r = "あ".repeat(200);
        let _ = run(&m, &s, &r); // パニックせず戻ればよい（結果は問わない）
    }
}
