//! 左文脈の解析（ADR 0002 §5）。

use std::sync::Arc;

use crate::candidate::Segment;
use crate::context::Commit;
use crate::dict::{Dictionary, Grammar, WordId};

mod lattice_analyzer;

pub use lattice_analyzer::LatticeContextAnalyzer;

/// 解析窓の長さ。左文脈の末尾からこの文字数だけを解析する。
pub const CONTEXT_WINDOW_CHARS: usize = 2000;

pub trait ContextAnalyzer: Send + Sync {
    /// 左文脈を解析する。結果は Arc で共有する（Converter と Ranker が同じものを見る）。
    fn analyze(&self, left_text: &str, history: &[Commit]) -> Arc<AnalyzedContext>;

    /// 文脈とは別に、文字列だけを解析してトークンの列にする（再変換で、選択した表記を語に分けるために使う。
    /// ADR 0006 §9.3）。`char_end` は文字列の末尾からの距離。左文脈の解析のキャッシュを使わず、書き換えない
    /// 実装が望ましい（既定は `analyze` を使う）。
    fn analyze_text(&self, text: &str) -> Vec<ContextToken> {
        self.analyze(text, &[]).tokens.clone()
    }
}

#[derive(Clone, Debug, Default)]
pub struct AnalyzedContext {
    /// 解析窓の中のトークン（古い順）。窓は末尾の CONTEXT_WINDOW_CHARS 文字
    pub tokens: Vec<ContextToken>,
    /// 変換ラティスの BOS に使う右ID（最も確からしい終わり方）
    pub bos_right_id: u16,
    /// `bos_right_id` のほかに、末尾の語の右IDになりうるものと、その追加コスト（昇順）。
    /// 文が終わっていない左文脈では、終止形と連体形（「若い」「困った」）を区別できない。
    /// 次の語への連接は、これらも含めた最小値で評価する。空なら `bos_right_id` だけ
    pub bos_alternatives: Vec<(u16, i32)>,
    /// tokens のうち内容語の数（プロファイルの減衰に使う）
    pub content_count: usize,
}

impl AnalyzedContext {
    /// BOS の右IDの候補（`bos_right_id` が先頭で追加コスト 0）と、その追加コスト
    pub fn bos_options(&self) -> Vec<(u16, i32)> {
        let mut v = Vec::with_capacity(1 + self.bos_alternatives.len());
        v.push((self.bos_right_id, 0));
        v.extend(self.bos_alternatives.iter().copied());
        v
    }

    /// 文節 0〜i-1 のいま選ばれている候補の語（`segments`）を、トークンとして後ろに足した左文脈（ADR 0006 §3.1-1）。
    /// 解析器は通さない。足した語の `char_end` ぶん既存のトークンをずらし、`bos_right_id` は最後の語の右IDにする
    /// （`bos_alternatives` は空）。`segments` が空なら `self` の複製。
    pub fn extended(
        &self,
        segments: &[Segment],
        grammar: &dyn Grammar,
        dict: &dyn Dictionary,
    ) -> AnalyzedContext {
        let Some(last) = segments.last() else {
            return self.clone();
        };
        let added_chars: usize = segments.iter().map(|s| s.surface.chars().count()).sum();
        let shift = u32::try_from(added_chars).unwrap_or(u32::MAX);
        let mut tokens = Vec::with_capacity(self.tokens.len() + segments.len());
        for token in &self.tokens {
            let mut token = token.clone();
            token.char_end = token.char_end.saturating_add(shift);
            tokens.push(token);
        }
        // 足した語の末尾からの距離。最後の語が 0
        let mut distance = 0u32;
        let mut added: Vec<ContextToken> = Vec::with_capacity(segments.len());
        for segment in segments.iter().rev() {
            added.push(token_of_segment(segment, distance, grammar, dict));
            let chars = u32::try_from(segment.surface.chars().count()).unwrap_or(u32::MAX);
            distance = distance.saturating_add(chars);
        }
        added.reverse();
        tokens.extend(added);
        let content_count = tokens.iter().filter(|t| t.is_content).count();
        AnalyzedContext {
            tokens,
            bos_right_id: last.right_id,
            bos_alternatives: Vec::new(),
            content_count,
        }
    }

    /// 左ID `left_id` の語への連接コスト（BOS の右IDの候補のうち最小）
    pub fn connection_to(&self, grammar: &dyn crate::dict::Grammar, left_id: u16) -> i32 {
        self.bos_options()
            .into_iter()
            .map(|(right, extra)| {
                extra.saturating_add(i32::from(grammar.connection_cost(right, left_id)))
            })
            .min()
            .unwrap_or(i32::MAX)
    }
}

/// 変換候補の語を、左文脈のトークンにする。内容語の判定と lemma は解析器と同じ規則
fn token_of_segment(
    segment: &Segment,
    char_end: u32,
    grammar: &dyn Grammar,
    dict: &dyn Dictionary,
) -> ContextToken {
    let pos_is_content = grammar
        .pos_name(segment.pos_id)
        .is_some_and(crate::pos::is_content_word);
    match segment.word {
        Some(word) => {
            let lemma = dict.lemma(word);
            ContextToken {
                surface: segment.surface.clone(),
                lemma: lemma.map_or_else(|| segment.surface.clone(), |l| l.text.to_owned()),
                lemma_id: lemma.and_then(|l| l.id),
                word: Some(word),
                left_id: segment.left_id,
                right_id: segment.right_id,
                pos_id: segment.pos_id,
                is_content: pos_is_content,
                char_end,
            }
        }
        None => ContextToken {
            surface: segment.surface.clone(),
            lemma: segment.surface.clone(),
            lemma_id: None,
            word: None,
            left_id: segment.left_id,
            right_id: segment.right_id,
            pos_id: segment.pos_id,
            is_content: pos_is_content
                && lattice_analyzer::unknown_text_can_be_content(&segment.surface),
            char_end,
        },
    }
}

#[derive(Clone, Debug)]
pub struct ContextToken {
    pub surface: String,
    /// 未知語なら表記
    pub lemma: String,
    /// システム辞書の lemma 番号（ベクトルを引く用）。それ以外は None
    pub lemma_id: Option<u32>,
    pub word: Option<WordId>,
    pub left_id: u16,
    pub right_id: u16,
    pub pos_id: u16,
    /// `crate::pos::is_content_word`
    pub is_content: bool,
    /// 左文脈の末尾からの距離（文字）。キャッシュの減衰に使う
    pub char_end: u32,
}

/// BOS を固定し、トークンを返さない（フェーズ0と同じ挙動。テストと比較実験用）。
///
/// `bos_right_id` は 0（Sudachi の `bos_eos_id`）。`bos_eos_id` が 0 でない文法を使うときは、
/// Converter がトークンのない解析結果の BOS を `grammar.bos_eos_id()` に置き換えること。
#[derive(Clone, Copy, Debug, Default)]
pub struct NullAnalyzer;

impl ContextAnalyzer for NullAnalyzer {
    fn analyze(&self, _left_text: &str, _history: &[Commit]) -> Arc<AnalyzedContext> {
        Arc::new(AnalyzedContext::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_analyzer_returns_empty_context() {
        let history = vec![Commit {
            reading: "ひんし".to_string(),
            surface: "品詞".to_string(),
        }];
        let a = NullAnalyzer.analyze("ユーザー辞書に単語を登録するときは", &history);
        assert!(a.tokens.is_empty());
        assert_eq!(a.bos_right_id, 0);
        assert_eq!(a.content_count, 0);
    }

    #[test]
    fn null_analyzer_is_usable_as_trait_object() {
        let a: Arc<dyn ContextAnalyzer> = Arc::new(NullAnalyzer);
        assert!(a.analyze("", &[]).tokens.is_empty());
    }
}
