//! zenz v3 のプロンプト（ADR 0002 §0.4, §13.2）。
//!
//! `[EE03 プロフィール][EE04 話題] EE02 左文脈 EE00 読み(カタカナ) EE01` のあとに出力の表記が続く。
//! 条件（プロフィールと話題）は、空ならタグごと省く。
//! 右文脈（EE07）は左文脈の後・読みの前に置き、空ならタグごと省く（azooKey の zenz-v3.2 の形式。
//! 学習されているかは未確認で、測って決める。#270）。kanakanji-edge には付けない。

use std::collections::HashMap;

use ikari_core::ContextToken;

use crate::config::ModelKind;

pub const TAG_INPUT: char = '\u{EE00}';
pub const TAG_OUTPUT: char = '\u{EE01}';
pub const TAG_CONTEXT: char = '\u{EE02}';
pub const TAG_PROFILE: char = '\u{EE03}';
pub const TAG_TOPIC: char = '\u{EE04}';
pub const TAG_RIGHT: char = '\u{EE07}';

const TOPIC_SEPARATOR: &str = "、";

const HIRAGANA_FIRST: char = '\u{3041}';
const HIRAGANA_LAST: char = '\u{3096}';
const HIRAGANA_TO_KATAKANA_OFFSET: u32 = 0x60;

#[derive(Clone, Copy, Debug, Default)]
pub struct PromptInput<'a> {
    pub profile: Option<&'a str>,
    pub topic: Option<&'a str>,
    /// 左文脈の末尾（呼び出し側で `tail_chars` を通しておく）
    pub left_context: &'a str,
    /// 右文脈の先頭（呼び出し側で先頭 N 字に切っておく）。空ならタグごと省く
    pub right_context: &'a str,
    /// ひらがなの読み
    pub reading: &'a str,
}

pub fn build_prompt(p: &PromptInput<'_>) -> String {
    let mut s = String::new();
    if let Some(profile) = p.profile.filter(|t| !t.is_empty()) {
        s.push(TAG_PROFILE);
        s.push_str(profile);
    }
    if let Some(topic) = p.topic.filter(|t| !t.is_empty()) {
        s.push(TAG_TOPIC);
        s.push_str(topic);
    }
    if !p.left_context.is_empty() {
        s.push(TAG_CONTEXT);
        s.push_str(p.left_context);
    }
    if !p.right_context.is_empty() {
        s.push(TAG_RIGHT);
        s.push_str(p.right_context);
    }
    s.push(TAG_INPUT);
    s.push_str(&to_katakana(p.reading));
    s.push(TAG_OUTPUT);
    s
}

/// モデルの種類に合わせたプロンプト。
///
/// kanakanji-edge は `EE02 左文脈 EE00 読み EE01` で、**EE02 は左文脈が空でも置く**（学習時の形式）。
/// プロフィール（EE03）と話題（EE04）は学習していないので付けない。
pub fn build_prompt_for(kind: ModelKind, p: &PromptInput<'_>) -> String {
    match kind {
        ModelKind::ZenzV3 => build_prompt(p),
        ModelKind::KanakanjiEdge => {
            let mut s = String::new();
            s.push(TAG_CONTEXT);
            s.push_str(p.left_context);
            s.push(TAG_INPUT);
            s.push_str(&to_katakana(p.reading));
            s.push(TAG_OUTPUT);
            s
        }
    }
}

/// 末尾の `n` 文字（文字単位。バイトで切らない）
pub fn tail_chars(s: &str, n: usize) -> &str {
    let count = s.chars().count();
    if count <= n {
        return s;
    }
    let skip = count - n;
    let idx = s.char_indices().nth(skip).map_or(s.len(), |(i, _)| i);
    &s[idx..]
}

/// 先頭の `n` 文字（文字単位。バイトで切らない）
pub fn head_chars(s: &str, n: usize) -> &str {
    let idx = s.char_indices().nth(n).map_or(s.len(), |(i, _)| i);
    &s[..idx]
}

/// ひらがなをカタカナにする（それ以外はそのまま。「ー」もそのまま）。
///
/// `ikari-input` や core の同名の変換とは別に持つ。**踊り字（ゝ、ゞ）を変えない**ので、
/// 置き換えると kanakanji-edge のプロンプトが変わりうる。
pub(crate) fn to_katakana(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            HIRAGANA_FIRST..=HIRAGANA_LAST => {
                char::from_u32(c as u32 + HIRAGANA_TO_KATAKANA_OFFSET).unwrap_or(c)
            }
            _ => c,
        })
        .collect()
}

/// 解析窓の内容語から、話題として使う語を `n` 個まで選ぶ。
/// 出現回数の多い lemma を先にし、同数なら新しい（窓の末尾に近い）方を先にする。
/// 1文字の語は話題にならないので除く。
pub(crate) fn topic_words(tokens: &[ContextToken], n: usize) -> Vec<&str> {
    if n == 0 {
        return Vec::new();
    }
    // lemma -> (出現回数, 最後に現れた位置)
    let mut stats: HashMap<&str, (usize, usize)> = HashMap::new();
    for (i, t) in tokens.iter().enumerate() {
        if !t.is_content || t.lemma.chars().count() < 2 {
            continue;
        }
        let e = stats.entry(t.lemma.as_str()).or_insert((0, i));
        e.0 += 1;
        e.1 = i;
    }
    let mut v: Vec<(&str, (usize, usize))> = stats.into_iter().collect();
    v.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(b.1.1.cmp(&a.1.1)).then(a.0.cmp(b.0)));
    v.into_iter().take(n).map(|(w, _)| w).collect()
}

pub fn join_topic(words: &[&str]) -> String {
    words.join(TOPIC_SEPARATOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(lemma: &str, content: bool) -> ContextToken {
        ContextToken {
            surface: lemma.to_owned(),
            lemma: lemma.to_owned(),
            lemma_id: None,
            word: None,
            left_id: 0,
            right_id: 0,
            pos_id: 0,
            is_content: content,
            char_end: 0,
        }
    }

    #[test]
    fn prompt_with_all_parts() {
        let p = build_prompt(&PromptInput {
            profile: Some("IT"),
            topic: Some("辞書、登録"),
            left_context: "ユーザー辞書に単語を登録するときは",
            right_context: "",
            reading: "ひんし",
        });
        assert_eq!(
            p,
            "\u{EE03}IT\u{EE04}辞書、登録\u{EE02}ユーザー辞書に単語を登録するときは\u{EE00}ヒンシ\u{EE01}"
        );
    }

    #[test]
    fn prompt_omits_empty_conditions() {
        let p = build_prompt(&PromptInput {
            profile: None,
            topic: Some(""),
            right_context: "",
            left_context: "",
            reading: "らーめん",
        });
        assert_eq!(p, "\u{EE00}ラーメン\u{EE01}");
        let p = build_prompt(&PromptInput {
            left_context: "システムの",
            reading: "こうせい",
            ..Default::default()
        });
        assert_eq!(p, "\u{EE02}システムの\u{EE00}コウセイ\u{EE01}");
    }

    #[test]
    fn prompt_places_right_context_after_left_and_before_reading() {
        let base = PromptInput {
            reading: "はし",
            ..Default::default()
        };
        let right_only = build_prompt(&PromptInput {
            right_context: "を渡る",
            ..base
        });
        assert_eq!(right_only, "\u{EE07}を渡る\u{EE00}ハシ\u{EE01}");
        let left_only = build_prompt(&PromptInput {
            left_context: "川に架かる",
            ..base
        });
        assert_eq!(left_only, "\u{EE02}川に架かる\u{EE00}ハシ\u{EE01}");
        let both = build_prompt(&PromptInput {
            left_context: "川に架かる",
            right_context: "を渡る",
            ..base
        });
        assert_eq!(both, "\u{EE02}川に架かる\u{EE07}を渡る\u{EE00}ハシ\u{EE01}");
        // 空なら従来と同一
        assert_eq!(build_prompt(&base), "\u{EE00}ハシ\u{EE01}");
    }

    #[test]
    fn edge_prompt_always_has_the_context_tag_and_never_conditions() {
        let with = PromptInput {
            profile: Some("IT"),
            topic: Some("辞書、登録"),
            left_context: "システムの",
            right_context: "です",
            reading: "こうせい",
        };
        assert_eq!(
            build_prompt_for(ModelKind::KanakanjiEdge, &with),
            "\u{EE02}システムの\u{EE00}コウセイ\u{EE01}"
        );
        // 左文脈が空でも EE02 を置く
        let empty = PromptInput {
            reading: "らーめん",
            ..Default::default()
        };
        assert_eq!(
            build_prompt_for(ModelKind::KanakanjiEdge, &empty),
            "\u{EE02}\u{EE00}ラーメン\u{EE01}"
        );
        // zenz の形式は変わらない（空なら EE02 を省く）
        assert_eq!(
            build_prompt_for(ModelKind::ZenzV3, &empty),
            "\u{EE00}ラーメン\u{EE01}"
        );
        assert_eq!(
            build_prompt_for(ModelKind::ZenzV3, &with),
            build_prompt(&with)
        );
    }

    #[test]
    fn katakana_conversion_keeps_other_chars() {
        assert_eq!(to_katakana("ひんし"), "ヒンシ");
        assert_eq!(to_katakana("らーめん"), "ラーメン");
        assert_eq!(to_katakana("ゔぁ"), "ヴァ");
        assert_eq!(to_katakana("abc1漢字ア"), "abc1漢字ア");
        assert_eq!(to_katakana(""), "");
    }

    #[test]
    fn tail_chars_cuts_on_char_boundaries() {
        assert_eq!(tail_chars("あいうえお", 3), "うえお");
        assert_eq!(tail_chars("あいうえお", 5), "あいうえお");
        assert_eq!(tail_chars("あいうえお", 10), "あいうえお");
        assert_eq!(tail_chars("あいうえお", 0), "");
        assert_eq!(tail_chars("", 3), "");
        assert_eq!(tail_chars("a😀b", 2), "😀b");
    }

    #[test]
    fn head_chars_cuts_on_char_boundaries() {
        assert_eq!(head_chars("あいうえお", 3), "あいう");
        assert_eq!(head_chars("あいうえお", 10), "あいうえお");
        assert_eq!(head_chars("あいうえお", 0), "");
        assert_eq!(head_chars("a😀b", 2), "a😀");
    }

    #[test]
    fn topic_words_by_frequency_then_recency() {
        let tokens = vec![
            tok("辞書", true),
            tok("登録", true),
            tok("を", false),
            tok("辞書", true),
            tok("単語", true),
            tok("品詞", true),
            tok("の", false),
            tok("品", true), // 1文字は除く
        ];
        assert_eq!(topic_words(&tokens, 3), vec!["辞書", "品詞", "単語"]);
        assert_eq!(topic_words(&tokens, 1), vec!["辞書"]);
        assert!(topic_words(&tokens, 0).is_empty());
        assert!(topic_words(&[], 3).is_empty());
        assert_eq!(join_topic(&["辞書", "品詞"]), "辞書、品詞");
    }
}
