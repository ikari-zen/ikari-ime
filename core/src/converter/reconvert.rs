//! 確定後の再変換（ADR 0006 §9）。
//!
//! 選択した表記から読みを求め（確定履歴 → 表記の解析）、元の表記を各文節の候補の 0 番にした
//! `Segmented` を作る。読めない部分は `CandidateOrigin::Literal` の候補だけを持つ文節にする。

use std::time::Instant;

use ikari_dict_format::reading::normalize_key;
use ikari_input::hiragana_to_katakana;

use super::segment::{group_words, head_of, layout_of, literal_candidate};
use super::{Converter, SegmentError, Segmented, SegmentedOrigin};
use crate::analysis::ContextToken;
use crate::candidate::{Candidate, CandidateOrigin, Segment};
use crate::context::{Commit, Context};
use crate::lattice;
use crate::nbest::NBest;

/// ひらがな（ぁ..=ゖ、ゝ、ゞ）の範囲。`normalize_key` のあとの読みの文字
const HIRAGANA_FIRST: char = '\u{3041}';
const HIRAGANA_LAST: char = '\u{3096}';
const HIRAGANA_ITERATION: [char; 2] = ['\u{309D}', '\u{309E}'];
const PROLONGED_SOUND_MARK: char = '\u{30FC}';

/// 選択した表記を分けたもの。`Words` の中の語は、必ず元の表記をつないだ形になる
enum Piece {
    /// 読みのある語の並び。文節の境目は `group_words` で決める。`Piece` の境目は必ず文節の境目
    Words(Vec<Segment>),
    /// 読めなかった部分の元の表記
    Literal(String),
}

impl Converter {
    /// 再変換。`surface` の読みを、確定履歴 `hint`（新しい順）、次に表記の解析の順で求め
    /// （§9.3）、元の表記を各文節の候補の 0 番にした `Segmented`（origin = `Reconvert`）を返す。
    /// 読めない部分は `CandidateOrigin::Literal` の候補だけを持つ文節にする。
    /// 全部が読めなければ（secure のときも）`SegmentError::Empty`（殻は何もしない）。
    /// 左文脈は `ctx.left_text` だけを使う（`ctx.history` は使わない。選択が履歴の末尾にあるとは限らないため）。
    /// 返す `Segmented::whole()` は読みが空の変換で、候補はない
    pub fn reconvert(
        &self,
        surface: &str,
        ctx: &Context,
        hint: &[Commit],
        deadline: Option<Instant>,
    ) -> Result<Segmented, SegmentError> {
        if surface.is_empty() || ctx.secure {
            return Err(SegmentError::Empty);
        }
        let mut ctx = ctx.clone();
        ctx.history.clear();
        let pieces = self
            .pieces_from_history(surface, hint)
            .unwrap_or_else(|| self.pieces_from_analysis(surface));
        let heads = self.heads_of_pieces(pieces);
        if heads.iter().all(|h| h.origin == CandidateOrigin::Literal) {
            return Err(SegmentError::Empty);
        }
        let whole = self.convert_until("", &ctx, deadline);
        let layout = layout_of(0, &heads);
        let lists = heads.iter().map(|_| None).collect();
        Ok(Segmented {
            whole,
            layout,
            lists,
            origin: SegmentedOrigin::Reconvert,
            heads,
        })
    }

    /// 文節ごとの 0 番（元の表記）。`Piece` の境目は文節の境目
    fn heads_of_pieces(&self, pieces: Vec<Piece>) -> Vec<Candidate> {
        let mut heads = Vec::new();
        for piece in pieces {
            match piece {
                Piece::Literal(text) => heads.push(literal_candidate(&text, &*self.grammar)),
                Piece::Words(words) => {
                    for group in group_words(&words, &*self.grammar) {
                        let part = &words[group];
                        heads.push(head_of(part, origin_of(part)));
                    }
                }
            }
        }
        heads
    }

    // ---- 確定履歴（§9.3-1）----

    /// 確定履歴（新しい順）の中に、表記をつなぐと `surface` と一致する連続した項目があれば、
    /// その項目ごとの語の並び（古い順）を返す。新しい項目から始まる一致を優先し、途中の項目は飛ばさない
    fn pieces_from_history(&self, surface: &str, hint: &[Commit]) -> Option<Vec<Piece>> {
        for newest in 0..hint.len() {
            let mut joined = String::new();
            for oldest in newest..hint.len() {
                let item = &hint[oldest];
                if item.surface.is_empty() || item.reading.is_empty() {
                    break;
                }
                // 古い項目は前に足す。つないだものは常に選択の末尾と一致していなければならない
                joined.insert_str(0, &item.surface);
                if !surface.ends_with(&joined) {
                    break;
                }
                if joined == surface {
                    let pieces = hint[newest..=oldest]
                        .iter()
                        .rev()
                        .map(|c| Piece::Words(self.words_of_item(&c.reading, &c.surface)))
                        .collect();
                    return Some(pieces);
                }
            }
        }
        None
    }

    /// 項目の読みの N-best（`max_astar_pops` の範囲）から、表記が一致するパスを探して語に分ける。
    /// 見つからなければ、項目全体を未知語 1 つにする（1 文節）
    fn words_of_item(&self, reading: &str, surface: &str) -> Vec<Segment> {
        let key = normalize_key(reading);
        let starts = [(self.grammar.bos_eos_id(), 0)];
        let lattice = lattice::build_with_starts(
            &key,
            &*self.dict,
            &*self.grammar,
            &starts,
            &self.edge_rules,
        );
        let mut nbest = NBest::new(&lattice, &*self.grammar);
        let mut pops = self.opts.max_astar_pops;
        while let Some(path) = nbest.next(&lattice, &*self.grammar, &mut pops) {
            let mut words = Vec::with_capacity(path.nodes.len());
            let mut text = String::new();
            let mut complete = true;
            for &id in &path.nodes {
                let node = lattice.node(id);
                let word_reading = lattice.slice(node);
                let word_surface = match node.word {
                    Some(word) => match self.dict.entry(word) {
                        Some(entry) => entry.surface,
                        None => {
                            complete = false;
                            break;
                        }
                    },
                    None => word_reading,
                };
                text.push_str(word_surface);
                words.push(Segment {
                    reading: word_reading.to_owned(),
                    surface: word_surface.to_owned(),
                    word: node.word,
                    left_id: node.left_id,
                    right_id: node.right_id,
                    pos_id: node.pos_id,
                });
            }
            if complete && text == surface {
                return words;
            }
        }
        vec![self.unknown_word(&key, surface)]
    }

    // ---- 表記の解析（§9.3-2）----

    /// 解析器で表記を語に分け、語ごとに読みを引く。読めない語は `Piece::Literal`（連続は 1 つにまとめる）
    fn pieces_from_analysis(&self, surface: &str) -> Vec<Piece> {
        let mut tokens = self.analyzer.analyze_text(surface);
        let covered: String = tokens.iter().map(|t| t.surface.as_str()).collect();
        if covered != surface {
            // 解析器がない（NullAnalyzer）または窓の外がある。全体を 1 つの語として扱う
            tokens = vec![ContextToken {
                surface: surface.to_owned(),
                lemma: surface.to_owned(),
                lemma_id: None,
                word: None,
                left_id: self.grammar.unknown().left_id,
                right_id: self.grammar.unknown().right_id,
                pos_id: self.grammar.unknown().pos_id,
                is_content: false,
                char_end: 0,
            }];
        }
        let mut pieces: Vec<Piece> = Vec::new();
        let mut words: Vec<Segment> = Vec::new();
        for token in &tokens {
            match self.word_of_token(token) {
                Some(word) => words.push(word),
                None => {
                    if !words.is_empty() {
                        pieces.push(Piece::Words(std::mem::take(&mut words)));
                    }
                    match pieces.last_mut() {
                        Some(Piece::Literal(text)) => text.push_str(&token.surface),
                        _ => pieces.push(Piece::Literal(token.surface.clone())),
                    }
                }
            }
        }
        if !words.is_empty() {
            pieces.push(Piece::Words(words));
        }
        pieces
    }

    /// トークンの読み。辞書の語は辞書の読み（同じ表記で読みが複数あるときは、解析の最良のパスの語の読み）、
    /// 未知語はひらがなとカタカナ（ひらがなにする）と英数字（ASCII のまま）。読めなければ None
    fn word_of_token(&self, token: &ContextToken) -> Option<Segment> {
        let reading = token
            .word
            .and_then(|word| self.dict.reading(word))
            .map(std::borrow::Cow::into_owned)
            .filter(|r| !r.is_empty())
            .or_else(|| readable_unknown_reading(&token.surface))?;
        Some(Segment {
            reading,
            surface: token.surface.clone(),
            word: token.word,
            left_id: token.left_id,
            right_id: token.right_id,
            pos_id: token.pos_id,
        })
    }

    fn unknown_word(&self, reading: &str, surface: &str) -> Segment {
        let unk = self.grammar.unknown();
        Segment {
            reading: reading.to_owned(),
            surface: surface.to_owned(),
            word: None,
            left_id: unk.left_id,
            right_id: unk.right_id,
            pos_id: unk.pos_id,
        }
    }
}

/// 辞書にない語の読み。ひらがな、カタカナ、英数字（ASCII）だけでできていれば、正規化した読みを返す
/// （カタカナはひらがなにする。英数字はそのまま）。それ以外が混じれば None
fn readable_unknown_reading(text: &str) -> Option<String> {
    let reading = normalize_key(text);
    let readable = !reading.is_empty()
        && reading.chars().all(|c| {
            (HIRAGANA_FIRST..=HIRAGANA_LAST).contains(&c)
                || HIRAGANA_ITERATION.contains(&c)
                || c == PROLONGED_SOUND_MARK
                || c.is_ascii_alphanumeric()
        });
    readable.then_some(reading)
}

/// 0 番の候補の種類。未知語だけの文節は、表記が読みのままならひらがな、カタカナにしたものならカタカナ
fn origin_of(words: &[Segment]) -> CandidateOrigin {
    if words.iter().any(|w| w.word.is_some()) {
        return CandidateOrigin::Dictionary;
    }
    let reading: String = words.iter().map(|w| w.reading.as_str()).collect();
    let surface: String = words.iter().map(|w| w.surface.as_str()).collect();
    let is_hiragana = reading
        .chars()
        .all(|c| (HIRAGANA_FIRST..=HIRAGANA_LAST).contains(&c) || c == PROLONGED_SOUND_MARK);
    if surface == reading && is_hiragana {
        CandidateOrigin::Hiragana
    } else if surface != reading && surface == hiragana_to_katakana(&reading) {
        CandidateOrigin::Katakana
    } else {
        CandidateOrigin::Dictionary
    }
}
