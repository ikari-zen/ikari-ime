//! 自前の辞書と Viterbi で左文脈を解析する（ADR 0002 §5.3）。
//!
//! 解析は「文」を単位にする。左文脈の末尾 `CONTEXT_WINDOW_CHARS` 文字を、文末記号
//! （`。！？!?` と改行。連続する記号は同じ文に含める）の直後で区切り、各文を BOS から
//! 独立に Viterbi で解析する。文の結果は文の文字列をキーにして保持するので、こまめに確定する入力では
//! 末尾の1文だけを解析し直せば済み、しかも結果は全体を解析し直したときと必ず一致する。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ikari_dict_format::MAX_READING_CHARS;

use super::{AnalyzedContext, CONTEXT_WINDOW_CHARS, ContextAnalyzer, ContextToken};
use crate::context::Commit;
use crate::dict::{Dictionary, Grammar, WordId};
use crate::pos::is_content_word;

/// 文の区切りになる文字。左文脈がこれで終わっていれば、次は文頭（BOS）になる。
const SENTENCE_TERMINATORS: [char; 6] = ['。', '！', '？', '!', '?', '\n'];
/// 未知語ノードの最長（文字）。辞書の語長の上限と同じにして、ラティスの大きさを抑える。
const MAX_UNKNOWN_RUN_CHARS: usize = MAX_READING_CHARS;
/// 「前のノードがない」ことを表す添字
const NO_PREV: u32 = u32::MAX;

/// 未知語をまとめるときの文字種。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CharClass {
    Hiragana,
    Katakana,
    Kanji,
    Alpha,
    Digit,
    Other,
}

impl CharClass {
    fn of(c: char) -> Self {
        match c {
            '\u{3041}'..='\u{309F}' => Self::Hiragana,
            '\u{30A0}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' | '\u{FF66}'..='\u{FF9F}' => {
                Self::Katakana
            }
            '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{2FFFF}'
            | '々'
            | '〆' => Self::Kanji,
            'a'..='z' | 'A'..='Z' | '\u{FF21}'..='\u{FF3A}' | '\u{FF41}'..='\u{FF5A}' => {
                Self::Alpha
            }
            '0'..='9' | '\u{FF10}'..='\u{FF19}' => Self::Digit,
            _ => Self::Other,
        }
    }

    /// 未知語のとき、品詞だけでなく文字種からも内容語とみなせるか。
    /// ひらがなの未知語（助詞の崩れなど）、数字、記号は内容語に数えない。
    fn can_be_content(self) -> bool {
        matches!(self, Self::Kanji | Self::Katakana | Self::Alpha)
    }
}

/// 未知語の表記が、文字種から内容語とみなせるか（先頭の文字の字種で決める）。
/// 文節の候補の左文脈（`AnalyzedContext::extended`）が、解析器と同じ規則で内容語を決めるために使う。
pub(super) fn unknown_text_can_be_content(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|c| CharClass::of(c).can_be_content())
}

/// Viterbi のノード。位置は文内の文字単位。
#[derive(Clone, Copy)]
struct Node {
    start: u32,
    end: u32,
    word: Option<WordId>,
    left_id: u16,
    right_id: u16,
    pos_id: u16,
    cost: i32,
    /// 未知語ノードなら、その文字種
    unknown: Option<CharClass>,
    best: i32,
    prev: u32,
}

/// 1つの文の解析結果。`char_end` は文頭からの文字数（組み立てるときに末尾からの距離へ直す）。
type SentenceTokens = Arc<SentenceAnalysis>;

struct SentenceAnalysis {
    tokens: Vec<ContextToken>,
    /// 文が終わっていない（末尾が文末記号でない）ときの、末尾の語の右ID の候補と追加コスト（昇順）。
    /// 最良のものが先頭（追加コスト 0）。文が終わっていれば空
    open_ends: Vec<(u16, i32)>,
}

/// 文が終わっていないとき、最良の終わり方からこのコスト以内の終わり方を、BOS の候補として残す
const OPEN_END_MARGIN: i32 = 1000;
/// BOS の候補の数の上限
const MAX_OPEN_ENDS: usize = 6;

/// 直前に解析した結果。
struct Cache {
    /// 解析窓の文字列
    window: String,
    result: Arc<AnalyzedContext>,
    /// 文の文字列 -> その文の解析結果
    sentences: HashMap<String, SentenceTokens>,
}

pub struct LatticeContextAnalyzer {
    dict: Arc<dyn Dictionary>,
    grammar: Arc<dyn Grammar>,
    cache: Mutex<Option<Cache>>,
}

impl LatticeContextAnalyzer {
    pub fn new(dict: Arc<dyn Dictionary>, grammar: Arc<dyn Grammar>) -> Self {
        Self {
            dict,
            grammar,
            cache: Mutex::new(None),
        }
    }

    /// 1つの文を BOS から EOS まで解析し、最良パスのトークンを返す。
    ///
    /// 末尾が文末記号でない文（左文脈の最後の文）は、文が続くので EOS への連接を使わない。
    /// EOS を使うと、「若い」「困った」の末尾が、連体形ではなく文末の形（終止形）に偏る。
    fn analyze_sentence(&self, sentence: &str) -> SentenceAnalysis {
        let open = sentence
            .chars()
            .next_back()
            .is_some_and(|c| !SENTENCE_TERMINATORS.contains(&c));
        let byte_at: Vec<usize> = sentence
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(sentence.len()))
            .collect();
        let n = byte_at.len() - 1;
        if n == 0 {
            return SentenceAnalysis {
                tokens: Vec::new(),
                open_ends: Vec::new(),
            };
        }
        let classes: Vec<CharClass> = sentence.chars().map(CharClass::of).collect();
        let bos_eos = self.grammar.bos_eos_id();
        let unk = self.grammar.unknown();

        let mut nodes: Vec<Node> = Vec::with_capacity(n * 4);
        let mut ends_at: Vec<Vec<u32>> = vec![Vec::new(); n + 1];

        for i in 0..n {
            let first = nodes.len();
            let tail = &sentence[byte_at[i]..];
            // 辞書の語。解析にはコストだけを使い、penalty は足さない（ADR 0002 §5.3-2）
            self.dict.common_prefix_search_surface(tail, &mut |len, e| {
                let Some(matched) = tail.get(..len) else {
                    return;
                };
                let chars = matched.chars().count();
                if chars == 0 {
                    return;
                }
                nodes.push(Node {
                    start: i as u32,
                    end: (i + chars) as u32,
                    word: Some(e.id),
                    left_id: e.left_id,
                    right_id: e.right_id,
                    pos_id: e.pos_id,
                    cost: i32::from(e.cost),
                    unknown: None,
                    best: 0,
                    prev: NO_PREV,
                });
            });
            // 未知語: 同じ文字種の最長の連続を1ノードにし、1文字のノードも置く
            let class = classes[i];
            let mut run = 1;
            while run < MAX_UNKNOWN_RUN_CHARS && i + run < n && classes[i + run] == class {
                run += 1;
            }
            for len in [run, 1] {
                nodes.push(Node {
                    start: i as u32,
                    end: (i + len) as u32,
                    word: None,
                    left_id: unk.left_id,
                    right_id: unk.right_id,
                    pos_id: unk.pos_id,
                    // 文字数に比例させる。長さによらない1回分だと、漢字だけの複合語（関数名、飛行機、効果的）の
                    // 未知語1つが辞書の語より安くなり、語に分けられない（ADR 0006 §16.6）
                    cost: i32::from(unk.cost_per_char) * len as i32,
                    unknown: Some(class),
                    best: 0,
                    prev: NO_PREV,
                });
                if run == 1 {
                    break;
                }
            }

            // 前側（終了位置 == i）のノードは、開始位置が i より小さいので確定済み
            for id in first..nodes.len() {
                let left = nodes[id].left_id;
                let (from_prev, prev) = if i == 0 {
                    (
                        i32::from(self.grammar.connection_cost(bos_eos, left)),
                        NO_PREV,
                    )
                } else {
                    ends_at[i]
                        .iter()
                        .map(|&p| {
                            let prev = &nodes[p as usize];
                            let cost = prev.best
                                + i32::from(self.grammar.connection_cost(prev.right_id, left));
                            (cost, p)
                        })
                        .min()
                        .unwrap_or((i32::MAX / 2, NO_PREV))
                };
                nodes[id].best = from_prev + nodes[id].cost;
                nodes[id].prev = prev;
                ends_at[nodes[id].end as usize].push(id as u32);
            }
        }

        // 最良の終端を選び、さかのぼる。閉じた文は EOS への接続まで含める
        let end_cost = |id: u32| {
            let node = &nodes[id as usize];
            if open {
                node.best
            } else {
                node.best + i32::from(self.grammar.connection_cost(node.right_id, bos_eos))
            }
        };
        let Some(mut at) = ends_at[n]
            .iter()
            .copied()
            .min_by_key(|&id| (end_cost(id), id))
        else {
            return SentenceAnalysis {
                tokens: Vec::new(),
                open_ends: Vec::new(),
            };
        };
        let open_ends = if open {
            let best = end_cost(at);
            let mut ends: Vec<(u16, i32)> = Vec::new();
            for &id in &ends_at[n] {
                let delta = end_cost(id) - best;
                if delta > OPEN_END_MARGIN {
                    continue;
                }
                let right = nodes[id as usize].right_id;
                match ends.iter_mut().find(|(r, _)| *r == right) {
                    Some(e) => e.1 = e.1.min(delta),
                    None => ends.push((right, delta)),
                }
            }
            ends.sort_by_key(|&(right, delta)| (delta, right));
            ends.truncate(MAX_OPEN_ENDS);
            ends
        } else {
            Vec::new()
        };
        let mut path = Vec::new();
        loop {
            path.push(at);
            let prev = nodes[at as usize].prev;
            if prev == NO_PREV {
                break;
            }
            at = prev;
        }
        path.reverse();
        SentenceAnalysis {
            tokens: path
                .into_iter()
                .map(|id| self.token_of(&nodes[id as usize], sentence, &byte_at))
                .collect(),
            open_ends,
        }
    }

    fn token_of(&self, node: &Node, sentence: &str, byte_at: &[usize]) -> ContextToken {
        let surface = &sentence[byte_at[node.start as usize]..byte_at[node.end as usize]];
        let pos_is_content = self
            .grammar
            .pos_name(node.pos_id)
            .is_some_and(is_content_word);
        match node.word {
            Some(word) => {
                let lemma = self.dict.lemma(word);
                ContextToken {
                    surface: surface.to_owned(),
                    lemma: lemma.map_or_else(|| surface.to_owned(), |l| l.text.to_owned()),
                    lemma_id: lemma.and_then(|l| l.id),
                    word: Some(word),
                    left_id: node.left_id,
                    right_id: node.right_id,
                    pos_id: node.pos_id,
                    is_content: pos_is_content,
                    char_end: node.end,
                }
            }
            None => ContextToken {
                surface: surface.to_owned(),
                lemma: surface.to_owned(),
                lemma_id: None,
                word: None,
                left_id: node.left_id,
                right_id: node.right_id,
                pos_id: node.pos_id,
                is_content: pos_is_content && node.unknown.is_some_and(CharClass::can_be_content),
                char_end: node.end,
            },
        }
    }
}

/// 末尾 `CONTEXT_WINDOW_CHARS` 文字。
fn tail_window(text: &str) -> &str {
    match text.char_indices().rev().nth(CONTEXT_WINDOW_CHARS - 1) {
        Some((byte, _)) => &text[byte..],
        None => text,
    }
}

/// 文末記号（連続していれば全部）の直後で区切る。空の文は返さない。
fn split_sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev_terminator = false;
    for (i, c) in text.char_indices() {
        let is_terminator = SENTENCE_TERMINATORS.contains(&c);
        if prev_terminator && !is_terminator {
            out.push(&text[start..i]);
            start = i;
        }
        prev_terminator = is_terminator;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

impl ContextAnalyzer for LatticeContextAnalyzer {
    /// 文ごとに解析する（`analyze` と同じ規則）が、キャッシュは読み書きしない。
    /// 再変換が選択した表記を解析しても、次の変換の左文脈の解析を追い出さない。
    fn analyze_text(&self, text: &str) -> Vec<ContextToken> {
        let window = tail_window(text);
        let sentences = split_sentences(window);
        let total_chars = window.chars().count();
        let mut tokens = Vec::new();
        let mut offset = 0;
        for sentence in sentences {
            for mut token in self.analyze_sentence(sentence).tokens {
                token.char_end = (total_chars - offset - token.char_end as usize) as u32;
                tokens.push(token);
            }
            offset += sentence.chars().count();
        }
        tokens
    }

    /// `history` は `left_text` に含まれている前提なので、ここでは使わない
    /// （確定済みの語としての利用は DocumentCacheRanker が `Context.history` から行う）。
    fn analyze(&self, left_text: &str, _history: &[Commit]) -> Arc<AnalyzedContext> {
        let window = tail_window(left_text);
        let mut guard = self
            .cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(cache) = guard.as_ref()
            && cache.window == window
        {
            return Arc::clone(&cache.result);
        }

        let old = guard.take().map(|c| c.sentences).unwrap_or_default();
        let mut sentences: HashMap<String, SentenceTokens> = HashMap::new();
        let mut parts: Vec<SentenceTokens> = Vec::new();
        let mut sentence_chars: Vec<usize> = Vec::new();
        for sentence in split_sentences(window) {
            let tokens = match sentences.get(sentence).or_else(|| old.get(sentence)) {
                Some(t) => Arc::clone(t),
                None => Arc::new(self.analyze_sentence(sentence)),
            };
            sentences.insert(sentence.to_owned(), Arc::clone(&tokens));
            parts.push(tokens);
            sentence_chars.push(sentence.chars().count());
        }

        let total_chars: usize = sentence_chars.iter().sum();
        let mut tokens = Vec::with_capacity(parts.iter().map(|p| p.tokens.len()).sum());
        let mut offset = 0;
        for (part, chars) in parts.iter().zip(&sentence_chars) {
            for t in part.tokens.iter() {
                let mut t = t.clone();
                t.char_end = (total_chars - offset - t.char_end as usize) as u32;
                tokens.push(t);
            }
            offset += chars;
        }

        let bos_eos = self.grammar.bos_eos_id();
        let ends_sentence = window
            .chars()
            .next_back()
            .is_none_or(|c| SENTENCE_TERMINATORS.contains(&c));
        // 文が終わっていなければ、最後の文の終わり方の候補（先頭が最良）を BOS にする
        let open_ends: &[(u16, i32)] = match parts.last() {
            Some(last) if !ends_sentence => &last.open_ends,
            _ => &[],
        };
        let (bos_right_id, bos_alternatives) = match (open_ends.first(), tokens.last()) {
            (Some(&(best, _)), _) => (best, open_ends[1..].to_vec()),
            (None, Some(last)) if !ends_sentence => (last.right_id, Vec::new()),
            _ => (bos_eos, Vec::new()),
        };
        let content_count = tokens.iter().filter(|t| t.is_content).count();
        let result = Arc::new(AnalyzedContext {
            tokens,
            bos_right_id,
            bos_alternatives,
            content_count,
        });
        *guard = Some(Cache {
            window: window.to_owned(),
            result: Arc::clone(&result),
            sentences,
        });
        result
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::dict::{DenseGrammar, UnknownParams, WordEntry};
    use crate::lattice::testutil::TestDict;

    const ID_BOS: u16 = 0;
    const ID_NOUN: u16 = 1;
    const ID_ADVERB: u16 = 2;
    const ID_PARTICLE: u16 = 3;
    const ID_UNK: u16 = 4;
    const ID_VERB: u16 = 5;
    const ID_PUNCT: u16 = 6;
    const ID_COUNT: u16 = 7;
    const UNK_COST: i16 = 6000;

    const POS_NOUN: u16 = 0;
    const POS_ADVERB: u16 = 1;
    const POS_PARTICLE: u16 = 2;
    const POS_UNK: u16 = 3;
    const POS_PUNCT: u16 = 4;

    fn grammar() -> DenseGrammar {
        let mut g = DenseGrammar::new(
            ID_COUNT,
            ID_COUNT,
            500,
            UnknownParams {
                left_id: ID_UNK,
                right_id: ID_UNK,
                pos_id: POS_UNK,
                cost_per_char: UNK_COST,
            },
        );
        g.add_pos("名詞,普通名詞,一般,*,*,*");
        g.add_pos("副詞,*,*,*,*,*");
        g.add_pos("助詞,格助詞,*,*,*,*");
        g.add_pos("名詞,固有名詞,一般,*,*,*");
        g.add_pos("補助記号,句点,*,*,*,*");
        g
    }

    /// 「やっと春に。」と少しの語が引ける辞書。
    fn dict(g: &DenseGrammar) -> TestDict {
        use crate::dict::Grammar;
        let mut d = TestDict::new(g.grammar_id());
        d.add("やっと", "やっと", ID_ADVERB, ID_ADVERB, 3000, POS_ADVERB);
        d.add("はる", "春", ID_NOUN, ID_NOUN, 3000, POS_NOUN);
        d.add("に", "に", ID_PARTICLE, ID_PARTICLE, 1000, POS_PARTICLE);
        d.add("を", "を", ID_PARTICLE, ID_PARTICLE, 1000, POS_PARTICLE);
        d.add("。", "。", ID_PUNCT, ID_PUNCT, 500, POS_PUNCT);
        d.add("みる", "見る", ID_VERB, ID_VERB, 3000, POS_NOUN);
        d
    }

    fn analyzer() -> LatticeContextAnalyzer {
        let g = grammar();
        let d = dict(&g);
        LatticeContextAnalyzer::new(Arc::new(d), Arc::new(g))
    }

    type TokenKey = (String, String, Option<WordId>, u16, u16, u16, bool, u32);

    fn keys(a: &AnalyzedContext) -> Vec<TokenKey> {
        a.tokens
            .iter()
            .map(|t| {
                (
                    t.surface.clone(),
                    t.lemma.clone(),
                    t.word,
                    t.left_id,
                    t.right_id,
                    t.pos_id,
                    t.is_content,
                    t.char_end,
                )
            })
            .collect()
    }

    fn surfaces(a: &AnalyzedContext) -> Vec<&str> {
        a.tokens.iter().map(|t| t.surface.as_str()).collect()
    }

    #[test]
    fn bos_is_right_id_of_the_last_token() {
        let a = analyzer().analyze("やっと春に", &[]);
        assert_eq!(surfaces(&a), ["やっと", "春", "に"]);
        // 「やっと春に」の次の語は、格助詞「に」の右IDに接続する
        assert_eq!(a.bos_right_id, ID_PARTICLE);
    }

    #[test]
    fn bos_returns_to_bos_eos_at_sentence_end() {
        let an = analyzer();
        for text in ["", "やっと春に。", "春を\n", "春に！？", "春に!", "春に?"] {
            let a = an.analyze(text, &[]);
            assert_eq!(a.bos_right_id, ID_BOS, "{text:?}");
        }
        // 文の途中は直前の語
        assert_eq!(an.analyze("春を", &[]).bos_right_id, ID_PARTICLE);
    }

    /// 終止形と連体形のように、同じ表記で右IDだけが違う語。EOS への連接は終止形に有利
    fn open_end_analyzer() -> LatticeContextAnalyzer {
        const ID_END_FORM: u16 = 1;
        const ID_ATTR_FORM: u16 = 2;
        let mut g = DenseGrammar::new(
            3,
            3,
            0,
            UnknownParams {
                left_id: 0,
                right_id: 0,
                pos_id: POS_UNK,
                cost_per_char: 6000,
            },
        );
        g.set(ID_END_FORM, ID_BOS, -500);
        g.set(ID_ATTR_FORM, ID_BOS, 500);
        g.add_pos("形容詞,一般,*,*,形容詞,終止形-一般");
        let mut d = TestDict::new(g.grammar_id());
        d.add("わかい", "若い", ID_END_FORM, ID_END_FORM, 1100, 0);
        d.add("わかい", "若い", ID_ATTR_FORM, ID_ATTR_FORM, 1000, 0);
        LatticeContextAnalyzer::new(Arc::new(d), Arc::new(g))
    }

    #[test]
    fn open_sentence_does_not_use_eos_and_keeps_the_other_forms() {
        let an = open_end_analyzer();
        // 文が終わっていない: EOS を使わないので、単語コストの安い連体形が最良。終止形も追加コスト付きで残る
        let a = an.analyze("若い", &[]);
        assert_eq!(a.bos_right_id, 2);
        assert_eq!(a.bos_alternatives, vec![(1, 100)]);
        // 次の語への連接は、候補の最小
        let g = &*an.grammar;
        assert_eq!(
            a.connection_to(g, 0),
            i32::from(g.connection_cost(2, 0)).min(100 + i32::from(g.connection_cost(1, 0)))
        );
        // 文が終わっていれば、EOS まで含めて選び、BOS は文頭に戻る
        let closed = an.analyze("若い。", &[]);
        assert_eq!(closed.bos_right_id, ID_BOS);
        assert!(closed.bos_alternatives.is_empty());
    }

    #[test]
    fn empty_text_has_no_tokens() {
        let a = analyzer().analyze("", &[]);
        assert!(a.tokens.is_empty());
        assert_eq!(a.content_count, 0);
    }

    #[test]
    fn content_flags_counts_and_distances() {
        let a = analyzer().analyze("やっと春に", &[]);
        let flags: Vec<bool> = a.tokens.iter().map(|t| t.is_content).collect();
        assert_eq!(flags, [true, true, false]);
        assert_eq!(a.content_count, 2);
        // 末尾からの距離（文字）。最後のトークンは 0
        let ends: Vec<u32> = a.tokens.iter().map(|t| t.char_end).collect();
        assert_eq!(ends, [2, 1, 0]);
        assert!(a.tokens.iter().all(|t| t.word.is_some()));
    }

    #[test]
    fn unknown_run_is_a_single_token_with_a_single_cost() {
        let a = analyzer().analyze("ユーザー辞書", &[]);
        // カタカナの連続は1ノード（4文字分の未知語コストを払わない）
        assert_eq!(a.tokens[0].surface, "ユーザー");
        assert_eq!(a.tokens[0].lemma, "ユーザー");
        assert!(a.tokens[0].word.is_none());
        assert!(a.tokens[0].lemma_id.is_none());
        assert_eq!(a.tokens[0].pos_id, POS_UNK);
        assert_eq!(surfaces(&a).concat(), "ユーザー辞書");
        // 未知語の品詞は名詞だが、カタカナなので内容語
        assert!(a.tokens[0].is_content);
    }

    #[test]
    fn hiragana_and_symbol_unknowns_are_not_content() {
        let a = analyzer().analyze("ぬぬぬ", &[]);
        assert!(a.tokens.iter().all(|t| !t.is_content));
        assert_eq!(a.content_count, 0);
        let a = analyzer().analyze("123", &[]);
        assert!(a.tokens.iter().all(|t| !t.is_content));
    }

    #[test]
    fn viterbi_uses_connection_costs() {
        let mut g = grammar();
        // 「見る」は 動詞 より 名詞 の方が単語コストは安いが、「を」からの接続は動詞の方が安い
        g.set(ID_PARTICLE, ID_VERB, -1000);
        g.set(ID_PARTICLE, ID_NOUN, 500);
        let mut d = dict(&g);
        d.add("みる", "見る", ID_NOUN, ID_NOUN, 2500, POS_NOUN);
        let an = LatticeContextAnalyzer::new(Arc::new(d), Arc::new(g));
        let a = an.analyze("春を見る", &[]);
        assert_eq!(surfaces(&a), ["春", "を", "見る"]);
        assert_eq!(a.tokens[2].right_id, ID_VERB);
        assert_eq!(a.bos_right_id, ID_VERB);
    }

    #[test]
    fn penalty_is_not_applied_in_analysis() {
        use crate::dict::{Grammar, IME_EXCLUDED};
        let g = grammar();
        let mut d = TestDict::new(g.grammar_id());
        let id = d.add("はる", "春", ID_NOUN, ID_NOUN, 100, POS_NOUN);
        d.set_penalty(id, IME_EXCLUDED);
        let an = LatticeContextAnalyzer::new(Arc::new(d), Arc::new(g));
        let a = an.analyze("春", &[]);
        // 変換では使わない語でも、解析では語として引ける
        assert_eq!(surfaces(&a), ["春"]);
        assert!(a.tokens[0].word.is_some());
    }

    #[test]
    fn lemma_and_lemma_id_come_from_the_dictionary() {
        use crate::dict::Grammar;
        let g = grammar();
        let mut d = TestDict::new(g.grammar_id());
        let id = d.add("みる", "見る", ID_VERB, ID_VERB, 100, POS_NOUN);
        d.set_lemma(id, "見る", Some(42));
        let id = d.add("みた", "観た", ID_VERB, ID_VERB, 100, POS_NOUN);
        d.set_lemma(id, "観る", Some(43));
        let an = LatticeContextAnalyzer::new(Arc::new(d), Arc::new(g));
        let a = an.analyze("観た", &[]);
        assert_eq!(a.tokens[0].surface, "観た");
        assert_eq!(a.tokens[0].lemma, "観る");
        assert_eq!(a.tokens[0].lemma_id, Some(43));
    }

    #[test]
    fn same_text_returns_the_same_arc() {
        let an = analyzer();
        let a = an.analyze("やっと春に", &[]);
        let b = an.analyze("やっと春に", &[]);
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn splits_sentences_after_terminator_runs() {
        assert_eq!(
            split_sentences("春に。春を！？やっと"),
            ["春に。", "春を！？", "やっと"]
        );
        assert_eq!(split_sentences("。春"), ["。", "春"]);
        assert_eq!(split_sentences("春\n\n"), ["春\n\n"]);
        assert!(split_sentences("").is_empty());
    }

    #[test]
    fn window_keeps_only_the_tail() {
        let text: String = "春に".repeat((CONTEXT_WINDOW_CHARS + 500) / 2);
        assert_eq!(tail_window(&text).chars().count(), CONTEXT_WINDOW_CHARS);
        assert_eq!(tail_window("春に"), "春に");
        let a = analyzer().analyze(&text, &[]);
        assert_eq!(a.tokens.len(), CONTEXT_WINDOW_CHARS);
        assert_eq!(a.tokens[0].char_end as usize, CONTEXT_WINDOW_CHARS - 1);
    }

    fn story() -> String {
        "やっと春に。春を見る！ユーザー辞書に春を\nやっと春に春を見る。ぬぬ春に".repeat(3)
    }

    #[test]
    fn incremental_cache_matches_full_analysis() {
        let text = story();
        let chars: Vec<char> = text.chars().collect();
        let incremental = analyzer();
        for end in 0..=chars.len() {
            let prefix: String = chars[..end].iter().collect();
            let a = incremental.analyze(&prefix, &[]);
            let fresh = analyzer().analyze(&prefix, &[]);
            assert_eq!(keys(&a), keys(&fresh), "end={end}");
            assert_eq!(a.bos_right_id, fresh.bos_right_id, "end={end}");
            assert_eq!(a.content_count, fresh.content_count, "end={end}");
        }
    }

    #[test]
    fn incremental_cache_matches_full_analysis_when_the_window_slides() {
        let unit = "やっと春に。春を見る！ユーザー辞書に春を\n";
        let text = unit.repeat(CONTEXT_WINDOW_CHARS / unit.chars().count() + 4);
        let chars: Vec<char> = text.chars().collect();
        assert!(chars.len() > CONTEXT_WINDOW_CHARS + 50);
        let incremental = analyzer();
        let from = CONTEXT_WINDOW_CHARS - 30;
        for end in (from..=chars.len()).step_by(7).chain([chars.len()]) {
            let prefix: String = chars[..end].iter().collect();
            let a = incremental.analyze(&prefix, &[]);
            let fresh = analyzer().analyze(&prefix, &[]);
            assert_eq!(keys(&a), keys(&fresh), "end={end}");
            assert_eq!(a.bos_right_id, fresh.bos_right_id, "end={end}");
        }
    }

    /// 検索の回数を数える辞書（再解析が末尾の文だけで済むことの確認用）。
    struct Counting {
        inner: TestDict,
        searches: Arc<AtomicUsize>,
    }

    impl Dictionary for Counting {
        fn common_prefix_search(&self, key: &str, f: &mut dyn FnMut(usize, WordEntry<'_>)) {
            self.inner.common_prefix_search(key, f);
        }
        fn lookup_exact(&self, reading: &str, f: &mut dyn FnMut(WordEntry<'_>)) {
            self.inner.lookup_exact(reading, f);
        }
        fn entry(&self, id: WordId) -> Option<WordEntry<'_>> {
            self.inner.entry(id)
        }
        fn grammar_id(&self) -> u64 {
            self.inner.grammar_id()
        }
        fn common_prefix_search_surface(
            &self,
            text: &str,
            f: &mut dyn FnMut(usize, WordEntry<'_>),
        ) {
            self.searches.fetch_add(1, Ordering::Relaxed);
            self.inner.common_prefix_search_surface(text, f);
        }
        fn lemma(&self, id: WordId) -> Option<crate::dict::Lemma<'_>> {
            self.inner.lemma(id)
        }
    }

    #[test]
    fn appending_to_the_last_sentence_reanalyzes_only_that_sentence() {
        let g = grammar();
        let searches = Arc::new(AtomicUsize::new(0));
        let d = Counting {
            inner: dict(&g),
            searches: Arc::clone(&searches),
        };
        let an = LatticeContextAnalyzer::new(Arc::new(d), Arc::new(g));
        // 文ごとに文字列が違うようにする（同じ文は結果を共有するため）
        let base: String = (1..=20)
            .map(|i| format!("{}春に。", "ぬ".repeat(i)))
            .collect();
        an.analyze(&base, &[]);
        let full = searches.swap(0, Ordering::Relaxed);
        assert_eq!(full, base.chars().count());

        an.analyze(&format!("{base}やっと"), &[]);
        let tail_only = searches.swap(0, Ordering::Relaxed);
        assert_eq!(tail_only, "やっと".chars().count());

        an.analyze(&format!("{base}やっと春"), &[]);
        assert_eq!(searches.load(Ordering::Relaxed), "やっと春".chars().count());
    }

    #[test]
    fn history_is_not_required() {
        let history = vec![Commit {
            reading: "はる".into(),
            surface: "春".into(),
        }];
        let a = analyzer().analyze("やっと春", &history);
        assert_eq!(surfaces(&a), ["やっと", "春"]);
    }

    /// 2000文字の解析の時間を測る。`cargo test -p ikari-core --release -- --ignored --nocapture perf`
    #[test]
    #[ignore = "perf: run in release with --ignored --nocapture"]
    fn perf_two_thousand_chars() {
        use std::time::Instant;
        // 文が重複しない（結果の共有が効かない）ように、疑似乱数で語を並べる
        const WORDS: [&str; 12] = [
            "やっと",
            "春",
            "に",
            "を",
            "見る",
            "ユーザー",
            "辞書",
            "ぬ",
            "ABC",
            "123",
            "。",
            "\n",
        ];
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut text = String::new();
        while text.chars().count() < CONTEXT_WINDOW_CHARS {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            text.push_str(WORDS[(state >> 33) as usize % WORDS.len()]);
        }
        let text: String = text.chars().take(CONTEXT_WINDOW_CHARS).collect();
        assert_eq!(text.chars().count(), CONTEXT_WINDOW_CHARS);

        // 全部を解析し直す（キャッシュなし）。warm = 何度か回したあとの中央値
        let mut full = Vec::new();
        for _ in 0..30 {
            let an = analyzer();
            let t = Instant::now();
            std::hint::black_box(an.analyze(std::hint::black_box(&text), &[]));
            full.push(t.elapsed());
        }
        full.sort();
        // 同じ文字列の再解析（キャッシュの命中）
        let an = analyzer();
        an.analyze(&text, &[]);
        let t = Instant::now();
        for _ in 0..100 {
            std::hint::black_box(an.analyze(std::hint::black_box(&text), &[]));
        }
        let hit = t.elapsed() / 100;
        // 1文字ずつ確定していく入力（窓が満杯のまま滑る）
        let mut typed = text.clone();
        let t = Instant::now();
        let steps = 100;
        for i in 0..steps {
            typed.push(['春', 'に', '。', 'を'][i % 4]);
            std::hint::black_box(an.analyze(std::hint::black_box(&typed), &[]));
        }
        let sliding = t.elapsed() / steps as u32;
        println!(
            "full(no cache) median={:?} min={:?} max={:?}; cache hit={:?}; sliding append={:?}",
            full[full.len() / 2],
            full[0],
            full[full.len() - 1],
            hit,
            sliding
        );
        assert!(
            full[full.len() / 2].as_micros() < 3000,
            "{:?}",
            full[full.len() / 2]
        );
    }

    fn segments_of(a: &AnalyzedContext) -> Vec<crate::candidate::Segment> {
        a.tokens
            .iter()
            .map(|t| crate::candidate::Segment {
                reading: String::new(),
                surface: t.surface.clone(),
                word: t.word,
                left_id: t.left_id,
                right_id: t.right_id,
                pos_id: t.pos_id,
            })
            .collect()
    }

    #[test]
    fn extended_equals_analyzing_the_concatenated_text() {
        let g = grammar();
        let d = dict(&g);
        let a = LatticeContextAnalyzer::new(Arc::new(dict(&g)), Arc::new(grammar()));
        let head = a.analyze("やっと", &[]);
        let tail = a.analyze("春に", &[]);
        let whole = a.analyze("やっと春に", &[]);
        let extended = head.extended(&segments_of(&tail), &g, &d);
        assert_eq!(keys(&extended), keys(&whole));
        assert_eq!(extended.bos_right_id, whole.bos_right_id);
        assert_eq!(extended.content_count, whole.content_count);
        assert!(extended.bos_alternatives.is_empty());
    }

    #[test]
    fn extended_keeps_unknown_word_content_rule() {
        let g = grammar();
        let d = dict(&g);
        let a = LatticeContextAnalyzer::new(Arc::new(dict(&g)), Arc::new(grammar()));
        // 辞書にない漢字、ひらがな、カタカナ
        let tail = a.analyze("鰯ぬイワシ", &[]);
        let whole = a.analyze("やっと鰯ぬイワシ", &[]);
        let extended = a
            .analyze("やっと", &[])
            .extended(&segments_of(&tail), &g, &d);
        assert_eq!(keys(&extended), keys(&whole));
    }

    #[test]
    fn extended_with_nothing_is_a_copy() {
        let g = grammar();
        let d = dict(&g);
        let a = LatticeContextAnalyzer::new(Arc::new(dict(&g)), Arc::new(grammar()));
        let base = a.analyze("やっと春に", &[]);
        let same = base.extended(&[], &g, &d);
        assert_eq!(keys(&same), keys(&base));
        assert_eq!(same.bos_right_id, base.bos_right_id);
        assert_eq!(same.bos_alternatives, base.bos_alternatives);
    }

    #[test]
    fn extended_from_an_empty_context_starts_at_the_first_word() {
        let g = grammar();
        let d = dict(&g);
        let a = LatticeContextAnalyzer::new(Arc::new(dict(&g)), Arc::new(grammar()));
        let tail = a.analyze("春に", &[]);
        let extended = AnalyzedContext::default().extended(&segments_of(&tail), &g, &d);
        assert_eq!(surfaces(&extended), ["春", "に"]);
        assert_eq!(extended.tokens.last().unwrap().char_end, 0);
        assert_eq!(extended.tokens[0].char_end, 1);
        assert_eq!(extended.bos_right_id, ID_PARTICLE);
    }

    /// 「関数名」を 関数 + 名 に分けられる辞書。未知語（漢字3文字を1ノード）の固定のコスト 6000 より、
    /// 辞書の語 2 つ（3000 + 3000 + 連接 500）のほうが高い
    fn compound_analyzer() -> LatticeContextAnalyzer {
        let g = grammar();
        let mut d = dict(&g);
        d.add("かんすう", "関数", ID_NOUN, ID_NOUN, 3000, POS_NOUN);
        d.add("めい", "名", ID_NOUN, ID_NOUN, 3000, POS_NOUN);
        LatticeContextAnalyzer::new(Arc::new(d), Arc::new(g))
    }

    #[test]
    fn analyze_text_splits_a_kanji_compound_into_dictionary_words() {
        let a = compound_analyzer();
        let tokens = a.analyze_text("関数名");
        let got: Vec<_> = tokens
            .iter()
            .map(|t| (t.surface.as_str(), t.word.is_some()))
            .collect();
        assert_eq!(got, [("関数", true), ("名", true)]);
        // 本当に辞書にない漢字の連続は、未知語のまま
        let unknown = a.analyze_text("鰯鯖");
        assert_eq!(unknown.len(), 1, "{unknown:?}");
        assert!(unknown[0].word.is_none());
    }

    #[test]
    fn the_left_context_analysis_also_splits_kanji_compounds_into_words() {
        // 左文脈の解析も、再変換と同じく未知語のコストを文字数に比例させる（ADR 0006 §16.6。ext の正解 950 → 973）
        let a = compound_analyzer().analyze("関数名", &[]);
        assert_eq!(surfaces(&a), ["関数", "名"]);
        assert!(a.tokens.iter().all(|t| t.word.is_some()));
    }

    #[test]
    fn analyze_text_matches_analyze_without_touching_the_cache() {
        let a = analyzer();
        let left = a.analyze("やっと春に", &[]);
        for text in ["春に。やっと", "やっと春に", "", "鰯を春に。"] {
            let tokens = a.analyze_text(text);
            let reference = analyzer().analyze(text, &[]);
            let got: Vec<_> = tokens
                .iter()
                .map(|t| (t.surface.clone(), t.word, t.char_end))
                .collect();
            let want: Vec<_> = reference
                .tokens
                .iter()
                .map(|t| (t.surface.clone(), t.word, t.char_end))
                .collect();
            assert_eq!(got, want, "{text}");
        }
        // 左文脈のキャッシュはそのまま
        assert!(Arc::ptr_eq(&left, &a.analyze("やっと春に", &[])));
    }
}
