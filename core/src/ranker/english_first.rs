//! 英字の語を読みのカタカナに置き換えた候補を作る（ADR 0020。B32）。
//!
//! 1位の候補が「英字の語」を含むとき、その語を置き換えた候補を1位にする規則の、置き換え側の部品。
//! 並びの規則は [`super::CompositeRanker`] の最後の段、置き換えた候補の追加は `Conversion::build_first_pool`。
//!
//! 英字の語（規則の対象）は、辞書の語で、表記に ASCII の英字を含み、読みに英字を含まないもの。
//! 例外は、ユーザーが英字だけで登録した語（E3）だけ。

use ikari_input::hiragana_to_katakana;

use crate::candidate::{Candidate, Segment};
use crate::dict::{Dictionary, IME_EXCLUDED};

/// 表記に ASCII の英字を含むか
fn has_ascii_letter(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_alphabetic())
}

/// 同じ読みで、表記が `surface` の辞書のエントリのうち、変換のコストが最も安いもの
fn cheapest_entry(
    dict: &dyn Dictionary,
    reading: &str,
    surface: &str,
) -> Option<(crate::dict::WordId, u16, u16, u16)> {
    let mut best: Option<(i32, crate::dict::WordId, u16, u16, u16)> = None;
    dict.lookup_exact(reading, &mut |e| {
        if e.surface != surface || e.penalty == IME_EXCLUDED {
            return;
        }
        let cost = i32::from(e.cost) + i32::from(e.penalty);
        if best.is_none_or(|(c, ..)| cost < c) {
            best = Some((cost, e.id, e.left_id, e.right_id, e.pos_id));
        }
    });
    best.map(|(_, id, l, r, p)| (id, l, r, p))
}

/// 混ざった表記（X線）の置き換え先。同じ読みで、正規化表記が同じ（同じ語）、
/// 表記に英字を含まない辞書の語のうち、コストが最も安いもの。なければ None。
fn mixed_replacement(seg: &Segment, dict: &dyn Dictionary) -> Option<String> {
    let lemma = dict.lemma(seg.word?)?.text.to_owned();
    let mut best: Option<(i32, String)> = None;
    dict.lookup_exact(&seg.reading, &mut |e| {
        if has_ascii_letter(e.surface) || e.penalty == IME_EXCLUDED {
            return;
        }
        if dict.lemma(e.id).is_none_or(|l| l.text != lemma) {
            return;
        }
        let cost = i32::from(e.cost) + i32::from(e.penalty);
        if best.as_ref().is_none_or(|(c, _)| cost < *c) {
            best = Some((cost, e.surface.to_owned()));
        }
    });
    best.map(|(_, s)| s)
}

/// 語の置き換え先の表記。`seg` が英字の語でなければ（または例外なら）None。
fn replacement(seg: &Segment, dict: &dyn Dictionary) -> Option<String> {
    let word = seg.word?;
    if !has_ascii_letter(&seg.surface) || has_ascii_letter(&seg.reading) {
        return None;
    }
    let katakana = hiragana_to_katakana(&seg.reading);
    // E3: ユーザー辞書に英字だけで登録した語（同じ辞書に同じ読みのカタカナの項目がない）は英字のまま
    if dict.is_user_authored(word) {
        let mut has_katakana = false;
        dict.lookup_exact(&seg.reading, &mut |e| {
            if e.id.dict == word.dict && e.surface == katakana {
                has_katakana = true;
            }
        });
        if !has_katakana {
            return None;
        }
    }
    let target = if seg.surface.is_ascii() {
        katakana
    } else {
        mixed_replacement(seg, dict)?
    };
    (target != seg.surface).then_some(target)
}

/// `c` の英字の語を置き換えた候補。置き換える語がなければ None。
/// 置き換え先と同じ読み・表記の辞書の語があればその語として、なければ語IDなし（辞書にない語）で、
/// 品詞と連接IDは元の英字の語のまま持つ。スコアは元の候補と同じ。
pub(crate) fn variant(c: &Candidate, dict: &dyn Dictionary) -> Option<Candidate> {
    let replaced: Vec<Option<String>> = c.segments.iter().map(|s| replacement(s, dict)).collect();
    if replaced.iter().all(Option::is_none) {
        return None;
    }
    let mut out = c.clone();
    out.surface.clear();
    for (seg, target) in out.segments.iter_mut().zip(replaced) {
        if let Some(surface) = target {
            match cheapest_entry(dict, &seg.reading, &surface) {
                Some((id, left_id, right_id, pos_id)) => {
                    seg.word = Some(id);
                    seg.left_id = left_id;
                    seg.right_id = right_id;
                    seg.pos_id = pos_id;
                }
                None => seg.word = None,
            }
            seg.surface = surface;
        }
        out.surface.push_str(&seg.surface);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::candidate::CandidateOrigin;
    use crate::dict::{DictionarySet, InMemoryDictionary, WordId};

    const POS: u16 = 7;

    fn word(dict: &InMemoryDictionary, reading: &str, surface: &str) -> WordId {
        let mut found = None;
        dict.lookup_exact(reading, &mut |e| {
            if e.surface == surface {
                found = Some(e.id);
            }
        });
        found.unwrap_or_else(|| panic!("{reading} {surface}"))
    }

    fn candidate(dict: &dyn Dictionary, parts: &[(&str, &str)]) -> Candidate {
        let segments: Vec<Segment> = parts
            .iter()
            .map(|(reading, surface)| {
                let mut id = None;
                dict.lookup_exact(reading, &mut |e| {
                    if e.surface == *surface && id.is_none() {
                        id = Some(e.id);
                    }
                });
                Segment {
                    reading: (*reading).to_owned(),
                    surface: (*surface).to_owned(),
                    word: id,
                    left_id: 1,
                    right_id: 1,
                    pos_id: POS,
                }
            })
            .collect();
        Candidate {
            surface: segments.iter().map(|s| s.surface.as_str()).collect(),
            segments,
            base_cost: 5000,
            score: 5000,
            origin: CandidateOrigin::Dictionary,
        }
    }

    fn dict_of(rows: &[(&str, &str, i16)]) -> InMemoryDictionary {
        let mut d = InMemoryDictionary::new(0);
        for (reading, surface, cost) in rows {
            d.insert(reading, surface, 1, 1, *cost, POS);
        }
        d
    }

    fn surface_after(dict: &dyn Dictionary, parts: &[(&str, &str)]) -> Option<String> {
        variant(&candidate(dict, parts), dict).map(|c| c.surface)
    }

    #[test]
    fn latin_word_becomes_the_katakana_of_the_reading() {
        let d = dict_of(&[
            ("はるしねーしょん", "Hallucination", 5000),
            ("はるしねーしょん", "ハルシネーション", 15000),
            ("たこ", "TAKO", 5000),
            ("たこ", "蛸", 6000),
            ("ようこ", "YOKO", 5000),
        ]);
        let hallucination = candidate(&d, &[("はるしねーしょん", "Hallucination")]);
        let v = variant(&hallucination, &d).unwrap();
        assert_eq!(v.surface, "ハルシネーション");
        // 同じ読み・表記の辞書の語があれば、その語として扱う
        assert_eq!(
            v.segments[0].word,
            Some(word(&d, "はるしねーしょん", "ハルシネーション"))
        );
        assert_eq!(v.score, hallucination.score);
        assert_eq!(
            surface_after(&d, &[("たこ", "TAKO")]).as_deref(),
            Some("タコ")
        );
        assert_eq!(
            surface_after(&d, &[("ようこ", "YOKO")]).as_deref(),
            Some("ヨウコ")
        );
        assert_eq!(surface_after(&d, &[("たこ", "蛸")]), None);
    }

    #[test]
    fn no_exception_for_letter_names_units_or_partial_names() {
        let d = dict_of(&[
            ("ゆーあーるえる", "URL", 5000),
            ("きろめーとるまいじ", "km/h", 5000),
            ("ちゃっとじーぴーてぃー", "ChatGPT", 5000),
            ("こびっどじゅうきゅう", "COVID-19", 5000),
            ("えすでぃーじーず", "SDGs", 5000),
            ("しーでぃーろむ", "CD-ROM", 5000),
        ]);
        for (reading, surface, katakana) in [
            ("ゆーあーるえる", "URL", "ユーアールエル"),
            ("きろめーとるまいじ", "km/h", "キロメートルマイジ"),
            (
                "ちゃっとじーぴーてぃー",
                "ChatGPT",
                "チャットジーピーティー",
            ),
            ("こびっどじゅうきゅう", "COVID-19", "コビッドジュウキュウ"),
            ("えすでぃーじーず", "SDGs", "エスディージーズ"),
            ("しーでぃーろむ", "CD-ROM", "シーディーロム"),
        ] {
            assert_eq!(
                surface_after(&d, &[(reading, surface)]).as_deref(),
                Some(katakana)
            );
        }
    }

    #[test]
    fn a_word_without_a_katakana_entry_becomes_a_word_outside_the_dictionary() {
        let d = dict_of(&[("えすきゅーえる", "SQL", 5000)]);
        let v = variant(&candidate(&d, &[("えすきゅーえる", "SQL")]), &d).unwrap();
        assert_eq!(v.surface, "エスキューエル");
        assert_eq!(v.segments[0].word, None);
        // 品詞と連接IDは元の語のまま
        assert_eq!(v.segments[0].pos_id, POS);
        assert_eq!(v.segments[0].left_id, 1);
    }

    #[test]
    fn only_one_word_of_a_sentence_is_replaced() {
        let d = dict_of(&[
            ("はるしねーしょん", "Hallucination", 5000),
            ("が", "が", 100),
            ("おきる", "起きる", 100),
        ]);
        let v = surface_after(
            &d,
            &[
                ("はるしねーしょん", "Hallucination"),
                ("が", "が"),
                ("おきる", "起きる"),
            ],
        );
        assert_eq!(v.as_deref(), Some("ハルシネーションが起きる"));
    }

    #[test]
    fn mixed_surface_goes_to_the_same_word_without_latin() {
        let mut d = dict_of(&[
            ("えっくすせん", "X線", 5000),
            ("えっくすせん", "エックス線", 6000),
            ("えっくすせん", "エックスセン", 9000),
            ("びーえむあいち", "BMI値", 5000),
        ]);
        for surface in ["X線", "エックス線"] {
            let id = word(&d, "えっくすせん", surface);
            assert!(d.set_lemma(id, "X線"));
        }
        assert_eq!(
            surface_after(&d, &[("えっくすせん", "X線")]).as_deref(),
            Some("エックス線")
        );
        // 同じ語の英字を含まない表記がなければ対象にしない
        assert_eq!(surface_after(&d, &[("びーえむあいち", "BMI値")]), None);
    }

    #[test]
    fn words_whose_reading_has_latin_or_not_in_the_dictionary_are_not_targets() {
        let d = dict_of(&[("あいふぉんx", "iPhoneX", 5000), ("ひらがな", "かな", 5000)]);
        assert_eq!(surface_after(&d, &[("あいふぉんx", "iPhoneX")]), None);
        assert_eq!(surface_after(&d, &[("ひらがな", "かな")]), None);
        // 辞書にない語（未知語、かなの候補）
        let c = candidate(&d, &[("たこ", "TAKO")]);
        assert!(c.segments[0].word.is_none());
        assert!(variant(&c, &d).is_none());
    }

    /// システム辞書（slot 0）にユーザー辞書（slot 1）を重ねた辞書
    fn layered(system: InMemoryDictionary, user_rows: &[(&str, &str)]) -> DictionarySet {
        let rows: Vec<(&str, &str, i16)> = user_rows.iter().map(|(r, s)| (*r, *s, 3000)).collect();
        let mut user = dict_of(&rows);
        user.mark_user_authored();
        let mut set = DictionarySet::new();
        set.push(Arc::new(system)).unwrap();
        set.push(Arc::new(user)).unwrap();
        set
    }

    #[test]
    fn user_dictionary_word_registered_only_in_latin_stays_latin() {
        let system = dict_of(&[("ぷろじぇくと", "プロジェクト", 5000)]);
        let d = layered(
            system,
            &[
                ("ぷろじぇくとえっくす", "ProjectX"),
                ("まいぷろ", "MyPro"),
                ("まいぷろ", "マイプロ"),
            ],
        );
        // 英字だけで登録した語は、ユーザーの登録なので英字のまま（E3）
        assert_eq!(
            surface_after(&d, &[("ぷろじぇくとえっくす", "ProjectX")]),
            None
        );
        // 同じ読みのカタカナも登録していれば対象
        assert_eq!(
            surface_after(&d, &[("まいぷろ", "MyPro")]).as_deref(),
            Some("マイプロ")
        );
    }

    #[test]
    fn a_dictionary_that_is_not_user_authored_is_a_target_even_without_katakana() {
        // 語彙集の形 latin（ADR 0020 §12。2026-10-16 ユーザー決定）
        let system = dict_of(&[("ぷろじぇくと", "プロジェクト", 5000)]);
        let pack = dict_of(&[("まいえすきゅーえる", "MySQL", 3000)]);
        let mut set = DictionarySet::new();
        set.push(Arc::new(system)).unwrap();
        set.push(Arc::new(pack)).unwrap();
        assert_eq!(
            surface_after(&set, &[("まいえすきゅーえる", "MySQL")]).as_deref(),
            Some("マイエスキューエル")
        );
    }
}
