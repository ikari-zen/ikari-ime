//! `--ingest-holdout <user_dir>`: 取り込んだ文章の評価用 2 割から評価ケースを作る（ADR 0002 §11.4, §15.1）。
//!
//! 評価用の文は取り込み元のファイルから決定的に取り出す（複製しない。NFR-19、NFR-26）。
//! アライメント済みのトークン列を「内容語 + 後続の機能語」の単位にまとめ、
//! (左文脈, 読み, 期待) のケースにする。
//!
//! 取り出しとアライメント（`ikari-import`、レーンU）はこのモジュールの外にある。
//! ここは、その結果（[`HoldoutToken`] の列）からケースを作る部分と、取り出しの口 [`read_holdout`] を持つ。

use std::path::Path;

use ikari_core::pos::is_content_word;
use ikari_core::user::UserDir;
use ikari_core::{Dictionary, Grammar};
use ikari_import::{Env, load_holdout};

use crate::EvalCase;

pub const KIND_HOLDOUT: &str = "取り込み";
pub const TAG_HOLDOUT: &str = "holdout";
const ID_PREFIX: &str = "holdout";
/// 左文脈として渡す文字数の上限（解析窓より小さく取る。評価の時間を抑える）
pub const DEFAULT_CONTEXT_CHARS: usize = 200;

/// アライメントしたトークン（表記、読み、内容語か）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoldoutToken {
    pub surface: String,
    /// ひらがな
    pub reading: String,
    pub is_content: bool,
}

/// 評価用の 1 文。`left_text` は同じ文書の、この文より前の文（取り込みファイルから読み直したもの）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoldoutSentence {
    pub left_text: String,
    pub tokens: Vec<HoldoutToken>,
}

/// 評価用の文の取り出し結果
pub type HoldoutSentences = Vec<HoldoutSentence>;

/// `user_dir` の取り込み台帳（`ingest_sources.tsv`）から、評価用の文を取り出してアライメントする。
///
/// 取り出しとアライメントは `ikari-import` が持つ。参照先がなくなった取り込み元と、取り込み後に
/// 変わったファイルは、警告を出して続ける（NFR-27）。
pub fn read_holdout(
    user_dir: &Path,
    dict: &dyn Dictionary,
    grammar: &dyn Grammar,
) -> Result<HoldoutSentences, String> {
    let env = Env {
        dict,
        grammar,
        vectors: None,
    };
    let set = load_holdout(&UserDir::new(user_dir), &env)
        .map_err(|e| format!("{}: {e}", user_dir.display()))?;
    for path in &set.missing {
        eprintln!("warning: 取り込み元が見つかりません: {path}");
    }
    for path in &set.changed {
        eprintln!("warning: 取り込み後にファイルが変わっています: {path}");
    }
    Ok(set
        .sentences
        .into_iter()
        .map(|s| HoldoutSentence {
            left_text: s.left_text,
            tokens: s
                .tokens
                .iter()
                .map(|t| HoldoutToken {
                    surface: t.surface.clone(),
                    reading: t.reading.clone(),
                    is_content: grammar.pos_name(t.pos_id).is_some_and(is_content_word),
                })
                .collect(),
        })
        .collect())
}

/// 末尾の `n` 文字
fn tail(s: &str, n: usize) -> &str {
    let count = s.chars().count();
    if count <= n {
        return s;
    }
    let idx = s.char_indices().nth(count - n).map_or(s.len(), |(i, _)| i);
    &s[idx..]
}

/// 文ごとのトークン列から、評価ケースを作る。
///
/// - 単位: 内容語から始まり、次の内容語の手前まで（後続の機能語を含む）。文頭の機能語だけの並びも1単位。
/// - 左文脈: 同じ文書の前の文（`left_text`）と、同じ文のその単位より前の表記を連結したもの（末尾 `context_chars` 文字）。
/// - 読みと表記が同じ単位（かな書きの語など）は、変換の評価にならないので除く。
/// - id は `holdout-<文番号>-<単位番号>`（0始まり）。
pub fn holdout_cases(sentences: &[HoldoutSentence], context_chars: usize) -> Vec<EvalCase> {
    let mut cases = Vec::new();
    for (si, sentence) in sentences.iter().enumerate() {
        let tokens = &sentence.tokens;
        let mut context = sentence.left_text.clone();
        let mut unit_index = 0;
        let mut start = 0;
        while start < tokens.len() {
            // この単位の終わり: 先頭を除いて、次の内容語の手前まで
            let mut end = start + 1;
            while end < tokens.len() && !tokens[end].is_content {
                end += 1;
            }
            let unit = &tokens[start..end];
            let surface: String = unit.iter().map(|t| t.surface.as_str()).collect();
            let reading: String = unit.iter().map(|t| t.reading.as_str()).collect();
            if !surface.is_empty() && !reading.is_empty() && surface != reading {
                cases.push(EvalCase {
                    id: format!("{ID_PREFIX}-{si}-{unit_index}"),
                    kind: KIND_HOLDOUT.to_owned(),
                    tags: vec![TAG_HOLDOUT.to_owned()],
                    profile: None,
                    context: tail(&context, context_chars).to_owned(),
                    right: String::new(),
                    reading,
                    expected: surface.clone(),
                    note: String::new(),
                    split: None,
                });
            }
            context.push_str(&surface);
            unit_index += 1;
            start = end;
        }
    }
    cases
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(surface: &str, reading: &str, content: bool) -> HoldoutToken {
        HoldoutToken {
            surface: surface.to_owned(),
            reading: reading.to_owned(),
            is_content: content,
        }
    }

    fn sent(left_text: &str, tokens: Vec<HoldoutToken>) -> HoldoutSentence {
        HoldoutSentence {
            left_text: left_text.to_owned(),
            tokens,
        }
    }

    fn sentence() -> HoldoutSentence {
        // 「辞書に単語を登録する。」
        sent(
            "",
            vec![
                tok("辞書", "じしょ", true),
                tok("に", "に", false),
                tok("単語", "たんご", true),
                tok("を", "を", false),
                tok("登録", "とうろく", true),
                tok("する", "する", false),
                tok("。", "。", false),
            ],
        )
    }

    #[test]
    fn groups_content_word_with_following_function_words() {
        let cases = holdout_cases(&[sentence()], DEFAULT_CONTEXT_CHARS);
        let got: Vec<(&str, &str, &str, &str)> = cases
            .iter()
            .map(|c| {
                (
                    c.id.as_str(),
                    c.context.as_str(),
                    c.reading.as_str(),
                    c.expected.as_str(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("holdout-0-0", "", "じしょに", "辞書に"),
                ("holdout-0-1", "辞書に", "たんごを", "単語を"),
                (
                    "holdout-0-2",
                    "辞書に単語を",
                    "とうろくする。",
                    "登録する。"
                ),
            ]
        );
        assert!(
            cases
                .iter()
                .all(|c| c.kind == KIND_HOLDOUT && c.tags == [TAG_HOLDOUT])
        );
        assert!(
            cases
                .iter()
                .all(|c| c.split.is_none() && c.profile.is_none())
        );
    }

    #[test]
    fn leading_function_words_form_their_own_unit() {
        let s = sent(
            "",
            vec![
                tok("そして", "そして", false),
                tok("、", "、", false),
                tok("猫", "ねこ", true),
                tok("が", "が", false),
            ],
        );
        let cases = holdout_cases(&[s], DEFAULT_CONTEXT_CHARS);
        // 文頭の「そして、」は表記と読みが同じなので除かれ、猫が から始まる
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].expected, "猫が");
        assert_eq!(cases[0].context, "そして、");
        assert_eq!(cases[0].id, "holdout-0-1");
    }

    #[test]
    fn trivial_kana_units_are_dropped() {
        let s = sent(
            "",
            vec![
                tok("ここ", "ここ", true),
                tok("に", "に", false),
                tok("来る", "くる", true),
            ],
        );
        let cases = holdout_cases(&[s], DEFAULT_CONTEXT_CHARS);
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].expected, "来る");
        assert_eq!(cases[0].context, "ここに");
    }

    #[test]
    fn context_is_limited_to_the_tail_and_cut_on_char_boundaries() {
        let cases = holdout_cases(&[sentence()], 3);
        assert_eq!(cases[1].context, "辞書に");
        assert_eq!(cases[2].context, "単語を");
        assert_eq!(tail("あいうえお", 2), "えお");
        assert_eq!(tail("あ", 5), "あ");
        assert_eq!(tail("あいう", 0), "");
    }

    #[test]
    fn each_sentence_has_its_own_context_and_numbering() {
        let cases = holdout_cases(&[sentence(), sentence()], DEFAULT_CONTEXT_CHARS);
        assert_eq!(cases.len(), 6);
        assert_eq!(cases[3].id, "holdout-1-0");
        assert_eq!(
            cases[3].context, "",
            "context does not leak across sentences"
        );
    }

    #[test]
    fn empty_input_gives_no_cases() {
        assert!(holdout_cases(&[], 10).is_empty());
        assert!(holdout_cases(&[sent("", vec![])], 10).is_empty());
    }

    #[test]
    fn left_text_of_the_document_precedes_the_sentence() {
        let mut s = sentence();
        s.left_text = "前の文です。".to_owned();
        let cases = holdout_cases(&[s], DEFAULT_CONTEXT_CHARS);
        assert_eq!(cases[0].context, "前の文です。");
        assert_eq!(cases[1].context, "前の文です。辞書に");
    }
}
