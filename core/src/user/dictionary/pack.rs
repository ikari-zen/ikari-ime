//! 語彙集（分野別の語彙パック。ADR 0012 §4）。
//!
//! ```text
//! #ikari-pack<TAB>id=it<TAB>schema=1<TAB>name=IT の語彙
//! くばねてぃす<TAB>Kubernetes<TAB>固有名詞<TAB>kana<TAB>コンテナ<TAB>注記
//! えーだぶりゅーえす<TAB>AWS<TAB>固有名詞<TAB>latin<TAB>クラウド<TAB>
//! ```
//! 列は 読み、英字の表記、品詞（簡略名）、形、分野、注記。形は
//! - `kana`: 読みをそのままカタカナにした表記を1位、英字を2位（コスト +[`PACK_LATIN_OFFSET`]）に置く
//! - `latin`: 英字だけを出す（略語と、文字の名前を読みに含む語）
//!
//! 辞書はユーザー辞書と同じ作り方（品詞の代表左右IDと `user_cost`。`UserDictionary::build`）で作る。

use std::path::Path;

use ikari_dict_format::reading::normalize_key;

use super::{UserDataError, UserDictEntry, UserDictionary};
use crate::dict::{Dictionary, Grammar, InMemoryDictionary};
use crate::user::util::{FIELD_SEPARATOR, check_schema, parse_error, parse_header};

pub const PACK_SCHEMA: u32 = 1;
/// 同梱の IT の語彙集の id
pub const IT_PACK_ID: &str = "it";
/// 形 `kana` の英字を、カタカナより後ろに置く差。
/// 0 より大きく（カタカナが1位）、1回英字を選んだときの学習の加点（文脈が一致しないとき 300。ADR 0012 §4.3）より小さい。
pub const PACK_LATIN_OFFSET: i16 = 100;

const HEADER_NAME: &str = "#ikari-pack";
const COMMENT_PREFIX: char = '#';
const KEY_ID: &str = "id";
const KEY_NAME: &str = "name";
const FORM_KANA: &str = "kana";
const FORM_LATIN: &str = "latin";
const MIN_COLUMNS: usize = 4;
const MAX_COLUMNS: usize = 6;
const IT_PACK_TEXT: &str = include_str!("../../../assets/packs/it.tsv");
const IT_PACK_FILE: &str = "packs/it.tsv";
const HIRAGANA_FIRST: u32 = 0x3041;
const HIRAGANA_LAST: u32 = 0x3096;
const HIRAGANA_TO_KATAKANA: u32 = 0x60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackForm {
    /// カタカナ（読みのとおり）を1位、英字を2位
    Kana,
    /// 英字だけ
    Latin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackEntry {
    /// 正規化したひらがなの読み
    pub reading: String,
    pub latin: String,
    /// 簡略品詞（`user_pos.tsv`）
    pub pos: String,
    pub form: PackForm,
    pub field: String,
    pub note: String,
}

#[derive(Clone, Debug)]
pub struct VocabularyPack {
    id: String,
    name: String,
    entries: Vec<PackEntry>,
}

/// 同梱の IT の語彙集。同梱の内容が不正ならテストで落ちる。
pub fn builtin_it_pack() -> Result<VocabularyPack, UserDataError> {
    VocabularyPack::parse_in(IT_PACK_TEXT, Path::new(IT_PACK_FILE))
}

impl VocabularyPack {
    pub fn parse(text: &str) -> Result<Self, UserDataError> {
        Self::parse_in(text, Path::new("pack"))
    }

    fn parse_in(text: &str, file: &Path) -> Result<Self, UserDataError> {
        let mut lines = text.lines();
        let header = lines
            .next()
            .ok_or_else(|| parse_error(file, 1, "空のファイルです"))?;
        let kv = parse_header(header, HEADER_NAME).ok_or_else(|| {
            parse_error(file, 1, format!("ヘッダが `{HEADER_NAME}` ではありません"))
        })?;
        check_schema(file, &kv, PACK_SCHEMA)?;
        let find = |key: &str| kv.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
        let id = find(KEY_ID)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| parse_error(file, 1, "ヘッダに id=<名前> がありません"))?
            .to_string();
        let name = find(KEY_NAME).unwrap_or(&id).to_string();
        let mut pack = VocabularyPack {
            id,
            name,
            entries: Vec::new(),
        };
        for (i, line) in lines.enumerate() {
            let n = i + 2;
            if line.is_empty() || line.starts_with(COMMENT_PREFIX) {
                continue;
            }
            let f: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
            if !(MIN_COLUMNS..=MAX_COLUMNS).contains(&f.len()) {
                return Err(parse_error(
                    file,
                    n,
                    "列は 読み, 英字, 品詞, 形, 分野, 注記 です（分野と注記は省略できます）",
                ));
            }
            let form = match f[3] {
                FORM_KANA => PackForm::Kana,
                FORM_LATIN => PackForm::Latin,
                other => {
                    return Err(parse_error(
                        file,
                        n,
                        format!("形は {FORM_KANA} か {FORM_LATIN} です: {other}"),
                    ));
                }
            };
            let entry = PackEntry {
                reading: normalize_key(f[0]),
                latin: f[1].to_string(),
                pos: f[2].to_string(),
                form,
                field: f.get(4).copied().unwrap_or("").to_string(),
                note: f.get(5).copied().unwrap_or("").to_string(),
            };
            if pack
                .entries
                .iter()
                .any(|e| e.reading == entry.reading && e.latin == entry.latin)
            {
                return Err(parse_error(file, n, "同じ (読み, 英字) が重複しています"));
            }
            pack.entries.push(entry);
        }
        // 品詞、読み、表記の検査はユーザー辞書と同じもの（不正は行番号の代わりに語で知らせる）
        pack.to_user_dictionary()
            .map(|_| ())
            .map_err(|e| parse_error(file, 1, e.to_string()))?;
        Ok(pack)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn entries(&self) -> &[PackEntry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 辞書の項目の並び。形 `kana` の行は カタカナ、英字 の2項目、形 `latin` の行は英字の1項目。
    /// 返す `Vec<Option<String>>` は項目ごとの印で、形 `kana` の英字には並べる相手のカタカナの表記が入る。
    fn expanded(&self) -> (Vec<UserDictEntry>, Vec<Option<String>>) {
        let mut items = Vec::with_capacity(self.entries.len() * 2);
        let mut after_kana = Vec::with_capacity(self.entries.len() * 2);
        for e in &self.entries {
            let item = |surface: String| UserDictEntry {
                reading: e.reading.clone(),
                surface,
                pos: e.pos.clone(),
                comment: e.note.clone(),
            };
            if e.form == PackForm::Kana {
                let kana = to_katakana(&e.reading);
                items.push(item(kana.clone()));
                after_kana.push(None);
                items.push(item(e.latin.clone()));
                after_kana.push(Some(kana));
            } else {
                items.push(item(e.latin.clone()));
                after_kana.push(None);
            }
        }
        (items, after_kana)
    }

    fn to_user_dictionary(&self) -> Result<(UserDictionary, Vec<Option<String>>), UserDataError> {
        let (items, after_kana) = self.expanded();
        let n = items.len();
        let mut dict = UserDictionary::default();
        for item in items {
            dict.add(item)?;
        }
        // `add` は同じ (読み, 表記) を上書きするので、項目の並びと添字が崩れる語は弾く
        if dict.len() != n {
            return Err(UserDataError::Invalid(format!(
                "語彙集 {} に、読みと表記が同じ項目があります",
                self.id
            )));
        }
        Ok((dict, after_kana))
    }

    /// ユーザー辞書と同じ作り方で辞書にする。形 `kana` の英字は、カタカナの表記の「文頭から文末までの
    /// コスト」に [`PACK_LATIN_OFFSET`] を足したコストにする。
    ///
    /// カタカナの側は、語彙集のもの（`user_cost`）と、`reference`（システム辞書）に同じ (読み, カタカナ) の
    /// 語があればそれ（コスト＋ペナルティ）の、安い方。パール のように普通の語として安く入っている語（2262）でも、
    /// 英字（Perl）がすぐ後ろ（+100）に並び、1回選べば英字が1位になる。
    /// 文頭と文末の連接コストは、単語1つの変換の合計に入るので、両方に足して比べる。
    pub fn build(
        &self,
        grammar: &dyn Grammar,
        reference: &dyn Dictionary,
    ) -> Result<InMemoryDictionary, UserDataError> {
        let (dict, after_kana) = self.to_user_dictionary()?;
        let reading_of: Vec<&str> = self
            .entries
            .iter()
            .flat_map(|e| {
                let n = if e.form == PackForm::Kana { 2 } else { 1 };
                std::iter::repeat_n(e.reading.as_str(), n)
            })
            .collect();
        let bos = grammar.bos_eos_id();
        let edge_cost = |left_id: u16, right_id: u16| {
            i32::from(grammar.connection_cost(bos, left_id))
                + i32::from(grammar.connection_cost(right_id, bos))
        };
        dict.build_with_costs(grammar, &|i, info| match &after_kana[i] {
            None => info.user_cost,
            Some(kana) => {
                let edge = edge_cost(info.rep_left_id, info.rep_right_id);
                let mut kana_total = i32::from(info.user_cost) + edge;
                reference.lookup_exact(reading_of[i], &mut |e| {
                    if e.surface == kana.as_str() {
                        let total = i32::from(e.cost)
                            + i32::from(e.penalty)
                            + edge_cost(e.left_id, e.right_id);
                        kana_total = kana_total.min(total);
                    }
                });
                let cost = kana_total + i32::from(PACK_LATIN_OFFSET) - edge;
                cost.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
            }
        })
    }
}

/// ひらがなをカタカナにする（長音符などはそのまま）。
fn to_katakana(reading: &str) -> String {
    reading
        .chars()
        .map(|c| {
            let code = c as u32;
            if (HIRAGANA_FIRST..=HIRAGANA_LAST).contains(&code) {
                char::from_u32(code + HIRAGANA_TO_KATAKANA).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::tests::MockGrammar;
    use super::*;

    #[test]
    fn katakana_follows_the_reading() {
        assert_eq!(to_katakana("くーばねてぃす"), "クーバネティス");
        assert_eq!(to_katakana("ぐーぐる"), "グーグル");
    }

    #[test]
    fn rejects_bad_rows() {
        let h = "#ikari-pack\tid=t\tschema=1\n";
        assert!(VocabularyPack::parse(&format!("{h}あ\tA\t名詞\tfoo\n")).is_err());
        assert!(VocabularyPack::parse(&format!("{h}あ\tA\t名詞\n")).is_err());
        assert!(VocabularyPack::parse(&format!("{h}あ\tA\t謎\tkana\n")).is_err());
        assert!(
            VocabularyPack::parse(&format!("{h}あ\tA\t名詞\tkana\nあ\tA\t名詞\tlatin\n")).is_err()
        );
        assert!(VocabularyPack::parse("#ikari-pack\tid=t\tschema=2\n").is_err());
        assert!(VocabularyPack::parse("#ikari-user-dict\tschema=1\n").is_err());
    }

    #[test]
    fn comments_and_optional_columns() {
        let p = VocabularyPack::parse(
            "#ikari-pack\tid=t\tschema=1\tname=テスト\n# c\n#あ\tA\t名詞\tkana\n\nい\tB\t名詞\tlatin\n",
        )
        .unwrap();
        assert_eq!(p.name(), "テスト");
        assert_eq!(p.entries().len(), 1);
    }

    fn total(g: &dyn Grammar, e: &crate::dict::WordEntry<'_>) -> i32 {
        let bos = g.bos_eos_id();
        i32::from(e.cost)
            + i32::from(e.penalty)
            + i32::from(g.connection_cost(bos, e.left_id))
            + i32::from(g.connection_cost(e.right_id, bos))
    }

    /// (表記, 合計コスト) を安い順に
    fn entries_of(dict: &dyn Dictionary, g: &dyn Grammar, reading: &str) -> Vec<(String, i32)> {
        let mut out = Vec::new();
        dict.lookup_exact(reading, &mut |e| {
            out.push((e.surface.to_string(), total(g, &e)))
        });
        out.sort_by_key(|(_, c)| *c);
        out
    }

    #[test]
    fn every_bundled_row_puts_katakana_before_latin() {
        let g = MockGrammar::new();
        let empty = InMemoryDictionary::new(g.grammar_id());
        let pack = builtin_it_pack().unwrap();
        let dict = pack.build(&g, &empty).unwrap();
        for e in pack.entries() {
            let found = entries_of(&dict, &g, &e.reading);
            match e.form {
                PackForm::Kana => {
                    let kana = to_katakana(&e.reading);
                    let k = found
                        .iter()
                        .find(|(s, _)| *s == kana)
                        .unwrap_or_else(|| panic!("{}: no katakana in {found:?}", e.reading));
                    let l = found
                        .iter()
                        .find(|(s, _)| *s == e.latin)
                        .unwrap_or_else(|| panic!("{}: no latin in {found:?}", e.reading));
                    assert_eq!(
                        l.1 - k.1,
                        i32::from(PACK_LATIN_OFFSET),
                        "{}: {found:?}",
                        e.reading
                    );
                }
                PackForm::Latin => {
                    assert!(
                        found.iter().any(|(s, _)| *s == e.latin),
                        "{}: {found:?}",
                        e.reading
                    );
                    assert!(
                        !found.iter().any(|(s, _)| *s == to_katakana(&e.reading)),
                        "{}: a latin row must not make katakana: {found:?}",
                        e.reading
                    );
                }
            }
        }
    }

    #[test]
    fn latin_follows_a_cheaper_katakana_in_the_reference_dictionary() {
        let g = MockGrammar::new();
        let pack =
            VocabularyPack::parse("#ikari-pack\tid=t\tschema=1\nぱーる\tPerl\t固有名詞\tkana\n")
                .unwrap();
        let pos = g.pos_id("名詞,普通名詞,一般,*,*,*").unwrap();
        let mut reference = InMemoryDictionary::new(g.grammar_id());
        let id = reference.insert("ぱーる", "パール", 5, 5, 100, pos);
        reference.set_penalty(id, 50);
        let dict = pack.build(&g, &reference).unwrap();
        let mut reference_total = 0;
        reference.lookup_exact("ぱーる", &mut |e| reference_total = total(&g, &e));
        let found = entries_of(&dict, &g, "ぱーる");
        let latin = found.iter().find(|(s, _)| s == "Perl").unwrap();
        assert_eq!(latin.1, reference_total + i32::from(PACK_LATIN_OFFSET));
    }

    #[test]
    fn bundled_it_pack_parses() {
        let p = builtin_it_pack().unwrap();
        assert_eq!(p.id(), IT_PACK_ID);
        assert!(p.entries().len() > 300);
    }
}
