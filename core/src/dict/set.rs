//! システム辞書とユーザー辞書などの合成（ADR 0001 §4.1）。
//!
//! push 時に grammar_id を照合し、検索結果を全辞書分合わせて返し、`WordId.dict` を付け替える。

use std::borrow::Cow;
use std::sync::Arc;

use super::{DictError, Dictionary, Lemma, WordEntry, WordId};

/// `WordId.dict` は u8 なので、束ねられる辞書は最大 256 個
const MAX_DICTS: usize = u8::MAX as usize + 1;
/// 束ねられる辞書が返す `WordId.dict`（`DictionarySet` の入れ子は想定しない）
const MEMBER_DICT_ID: u8 = 0;
/// システム辞書を入れる slot（lemma 番号とベクトルの対応が有効なのはここだけ）
pub(crate) const SYSTEM_SLOT: u8 = 0;

pub struct DictionarySet {
    dicts: Vec<Arc<dyn Dictionary>>,
}

impl DictionarySet {
    pub fn new() -> Self {
        Self { dicts: Vec::new() }
    }

    /// 辞書を末尾に加え、その `WordId.dict` を返す。
    ///
    /// 最初の辞書の grammar_id と異なれば `GrammarMismatch`（`dict` は加えようとした辞書、
    /// `grammar` は既に入っている辞書の値）。辞書が 256 個を超えるときは I/O エラー（InvalidInput）。
    pub fn push(&mut self, d: Arc<dyn Dictionary>) -> Result<u8, DictError> {
        if let Some(first) = self.dicts.first() {
            let expected = first.grammar_id();
            let actual = d.grammar_id();
            if actual != expected {
                return Err(DictError::GrammarMismatch {
                    dict: actual,
                    grammar: expected,
                });
            }
        }
        let id = u8::try_from(self.dicts.len()).map_err(|_| {
            DictError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("too many dictionaries (at most {MAX_DICTS})"),
            ))
        })?;
        self.dicts.push(d);
        Ok(id)
    }

    /// 束ねている辞書の数。
    pub fn len(&self) -> usize {
        self.dicts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dicts.is_empty()
    }
}

impl Default for DictionarySet {
    fn default() -> Self {
        Self::new()
    }
}

fn relabel(entry: WordEntry<'_>, dict: u8) -> WordEntry<'_> {
    WordEntry {
        id: WordId {
            dict,
            index: entry.id.index,
        },
        ..entry
    }
}

impl Dictionary for DictionarySet {
    /// 各辞書の結果を辞書の順に連結して返す（辞書をまたいだコスト順の整列はしない）。
    fn common_prefix_search(&self, key: &str, f: &mut dyn FnMut(usize, WordEntry<'_>)) {
        for (i, d) in self.dicts.iter().enumerate() {
            // push で 256 個までに制限しているので、i は u8 に収まる
            let dict = i as u8;
            d.common_prefix_search(key, &mut |len, entry| f(len, relabel(entry, dict)));
        }
    }

    fn lookup_exact(&self, reading: &str, f: &mut dyn FnMut(WordEntry<'_>)) {
        for (i, d) in self.dicts.iter().enumerate() {
            let dict = i as u8;
            d.lookup_exact(reading, &mut |entry| f(relabel(entry, dict)));
        }
    }

    fn entry(&self, id: WordId) -> Option<WordEntry<'_>> {
        let d = self.dicts.get(usize::from(id.dict))?;
        let entry = d.entry(WordId {
            dict: MEMBER_DICT_ID,
            index: id.index,
        })?;
        Some(relabel(entry, id.dict))
    }

    /// 最初の辞書の grammar_id。空なら 0（push で全辞書が同じ値であることを保証している）。
    fn grammar_id(&self) -> u64 {
        self.dicts.first().map_or(0, |d| d.grammar_id())
    }

    /// 各辞書の表記検索を、辞書の順に連結して返す（`common_prefix_search` と同じ規則）。
    fn common_prefix_search_surface(&self, text: &str, f: &mut dyn FnMut(usize, WordEntry<'_>)) {
        for (i, d) in self.dicts.iter().enumerate() {
            let dict = i as u8;
            d.common_prefix_search_surface(text, &mut |len, entry| f(len, relabel(entry, dict)));
        }
    }

    /// lemma 番号（`Lemma::id`）は slot 0（システム辞書）のものだけ返す。ほかの辞書では None。
    fn lemma(&self, id: WordId) -> Option<Lemma<'_>> {
        let d = self.dicts.get(usize::from(id.dict))?;
        let lemma = d.lemma(WordId {
            dict: MEMBER_DICT_ID,
            index: id.index,
        })?;
        Some(Lemma {
            text: lemma.text,
            id: lemma.id.filter(|_| id.dict == SYSTEM_SLOT),
        })
    }

    /// 語IDの `dict` が指す辞書がユーザー辞書か。
    fn is_user_authored(&self, id: WordId) -> bool {
        self.dicts.get(usize::from(id.dict)).is_some_and(|d| {
            d.is_user_authored(WordId {
                dict: MEMBER_DICT_ID,
                index: id.index,
            })
        })
    }

    /// 最初の辞書（システム辞書）の dict_id。
    fn dict_id(&self) -> Option<u64> {
        self.dicts.first().and_then(|d| d.dict_id())
    }

    /// 語IDの `dict` が指す辞書の読み。
    fn reading(&self, id: WordId) -> Option<Cow<'_, str>> {
        let d = self.dicts.get(usize::from(id.dict))?;
        d.reading(WordId {
            dict: MEMBER_DICT_ID,
            index: id.index,
        })
    }

    fn warm_reading_index(&self) {
        for d in &self.dicts {
            d.warm_reading_index();
        }
    }

    fn prefault(&self) {
        for d in &self.dicts {
            d.prefault();
        }
    }
}
