//! 異字同訓の表（ADR 0009）と、辞書の行の分類。
//!
//! 表（`policy/kun.tsv`）は、動詞と形容詞の lemma ごとに、公用文の書き表し方で使う漢字を持つ。
//! 辞書の行を、辞書形の表記で次のどれかに分ける:
//! - 対象外: 表にない lemma、辞書形が漢字で始まらない（かなだけ、カタカナまじり）、読みが標準の形と違う（別の語）
//! - 表の中（[`KunNotation::Listed`]）: 漢字が表にあり、送り仮名が lemma に合う（可能動詞の形も許す）
//! - 表外（[`KunNotation::Outside`]）: 漢字が表にない、または送り仮名が合わない（奨める、現わす、帰える）
//!
//! 結果は変換用のペナルティ（`ime_penalty`）にだけ使い、解析用のコストと候補の集合は変えない。

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::lex::Lexicon;
use crate::policy::is_han;

/// 同梱の表
pub const KUN_TABLE_TEXT: &str = include_str!("../policy/kun.tsv");

const POS_SEPARATOR: char = ',';
/// 表の対象の品詞1
const KUN_POS1: [&str; 2] = ["動詞", "形容詞"];
/// 表の列: lemma、漢字、lemma の中の既定、読みの組の既定の読み（それ以降の列は備考）
const COL_LEMMA: usize = 0;
const COL_KANJI: usize = 1;
const COL_DEFAULT: usize = 2;
const COL_GROUP: usize = 3;
const MIN_COLUMNS: usize = 3;
/// 可能動詞の語尾にする文字（五段活用の終わりの文字の母音を「え」段に替えて「る」を付ける）
const POTENTIAL_SUFFIX: char = 'る';
const POTENTIAL_ENDINGS: [(char, char); 9] = [
    ('う', 'え'),
    ('く', 'け'),
    ('ぐ', 'げ'),
    ('す', 'せ'),
    ('つ', 'て'),
    ('ぬ', 'ね'),
    ('ぶ', 'べ'),
    ('む', 'め'),
    ('る', 'れ'),
];

/// 表の1行
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KunEntry {
    /// 公用文で使う漢字
    pub kanji: Vec<char>,
    /// lemma の中の既定の漢字
    pub default: Option<char>,
    /// 読みの組の既定の読み（辞書形の読み。例: すすめる）
    pub reading_group: Option<String>,
    /// lemma の送り仮名（漢字1字のあとの部分）
    tail: String,
}

impl KunEntry {
    /// 読みの組の既定として照合する、読みの語幹（辞書形の読みの最後の1文字を除いたもの）
    fn group_stem(&self) -> Option<&str> {
        let group = self.reading_group.as_deref()?;
        let last = group.char_indices().next_back()?.0;
        Some(&group[..last])
    }
}

/// 異字同訓の表。lemma から引く。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KunTable {
    entries: BTreeMap<String, KunEntry>,
}

/// 辞書の行の分類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KunNotation {
    /// 表外の表記（`kun_outside`）
    Outside,
    /// 表の中の表記
    Listed {
        /// lemma の中の既定の漢字か
        is_default: bool,
        /// 送り仮名が lemma と同じか（false は可能動詞の形）
        standard_tail: bool,
    },
}

/// 1行ぶんの、ポリシーに渡す結果（[`crate::PolicyRow`]）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KunInfo {
    /// 表外の表記（`kun_outside`）
    pub outside: bool,
    /// lemma の中の既定の行で、同じ (lemma, 読み, 品詞) のほかの表の表記の最小のコストより、いくら高いか
    /// （`kun_lemma_default`。負なら、すでに安い）。比べる表記がなければ `None`
    pub lemma_default_excess: Option<i32>,
    /// 読みの組の既定の行（`kun_reading_default`）
    pub reading_default: bool,
}

/// lemma ごとの、標準の形（表の漢字 + lemma の送り仮名）の辞書形の読み。表記のゆれを同じ語の別の表記と見分けるのに使う。
/// 読みが標準の形（とその可能動詞の形）と違う行は別の語（乗っかる、書くぅ、早え）なので、分類しない。
#[derive(Debug, Default)]
pub struct StandardReadings<'a> {
    by_lemma: HashMap<&'a str, HashSet<String>>,
}

impl StandardReadings<'_> {
    fn accepts(&self, lemma: &str, reading: &str) -> bool {
        self.by_lemma.get(lemma).is_some_and(|readings| {
            readings
                .iter()
                .any(|r| r == reading || potential_form(r).as_deref() == Some(reading))
        })
    }
}

/// 辞書形の可能動詞の形（書く → 書ける、取る → 取れる、見る → 見れる）。語尾が合わなければ `None`
fn potential_form(base: &str) -> Option<String> {
    let last = base.chars().next_back()?;
    let (_, e) = POTENTIAL_ENDINGS.iter().find(|(u, _)| *u == last)?;
    let mut out = base[..base.len() - last.len_utf8()].to_string();
    out.push(*e);
    out.push(POTENTIAL_SUFFIX);
    Some(out)
}

/// 表記を、先頭の漢字の並びと、残りに分ける。
fn split_leading_han(surface: &str) -> (&str, &str) {
    let end = surface
        .char_indices()
        .find(|(_, c)| !is_han(*c))
        .map_or(surface.len(), |(i, _)| i);
    surface.split_at(end)
}

fn is_kun_pos(pos: &str) -> bool {
    KUN_POS1.contains(&pos.split(POS_SEPARATOR).next().unwrap_or(""))
}

impl KunTable {
    /// 同梱の表。単体テストで検証している
    pub fn builtin() -> Self {
        Self::parse(KUN_TABLE_TEXT).unwrap_or(Self {
            entries: BTreeMap::new(),
        })
    }

    /// タブ区切りの表を読む。`#` で始まる行と空行は無視する。
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut entries = BTreeMap::new();
        for (i, line) in text.lines().enumerate() {
            let lineno = i + 1;
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() < MIN_COLUMNS {
                return Err(format!(
                    "line {lineno}: expected `lemma<TAB>kanji<TAB>default[<TAB>group]`"
                ));
            }
            let lemma = cols[COL_LEMMA];
            let (lemma_kanji, tail) = split_leading_han(lemma);
            if lemma_kanji.chars().count() != 1 || tail.is_empty() {
                return Err(format!(
                    "line {lineno}: lemma `{lemma}` is not one kanji plus okurigana"
                ));
            }
            let kanji: Vec<char> = cols[COL_KANJI].chars().collect();
            if kanji.is_empty() || !kanji.iter().all(|c| is_han(*c)) {
                return Err(format!(
                    "line {lineno}: kanji `{}` must be one or more kanji",
                    cols[COL_KANJI]
                ));
            }
            let default = match cols[COL_DEFAULT].chars().collect::<Vec<_>>().as_slice() {
                [] => None,
                [c] if kanji.contains(c) => Some(*c),
                _ => {
                    return Err(format!(
                        "line {lineno}: default `{}` must be empty or one of the kanji",
                        cols[COL_DEFAULT]
                    ));
                }
            };
            let reading_group = cols
                .get(COL_GROUP)
                .filter(|g| !g.is_empty())
                .map(|g| (*g).to_string());
            if reading_group.is_some() && default.is_none() {
                return Err(format!(
                    "line {lineno}: a reading group needs a default kanji"
                ));
            }
            let entry = KunEntry {
                kanji,
                default,
                reading_group,
                tail: tail.to_string(),
            };
            if entries.insert(lemma.to_string(), entry).is_some() {
                return Err(format!("line {lineno}: lemma `{lemma}` is defined twice"));
            }
        }
        Ok(Self { entries })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, lemma: &str) -> Option<&KunEntry> {
        self.entries.get(lemma)
    }

    /// 内容を固定した文字列（ポリシーのハッシュに入れる。コメントや備考の違いでは変わらない）。
    pub fn canonical_text(&self) -> String {
        let mut out = String::new();
        for (lemma, e) in &self.entries {
            let kanji: String = e.kanji.iter().collect();
            let default = e.default.map(String::from).unwrap_or_default();
            let group = e.reading_group.as_deref().unwrap_or("");
            out.push_str(&format!("{lemma}\t{kanji}\t{default}\t{group}\n"));
        }
        out
    }

    /// 標準の形の辞書形の読みを集める。`dict_forms` は (lemma, 品詞, 辞書形の表記, 辞書形の読み)。
    pub fn standard_readings<'a>(
        &self,
        dict_forms: impl Iterator<Item = (&'a str, &'a str, &'a str, &'a str)>,
    ) -> StandardReadings<'a> {
        let mut by_lemma: HashMap<&str, HashSet<String>> = HashMap::new();
        for (lemma, pos, surface, reading) in dict_forms {
            let Some(entry) = self.get(lemma).filter(|_| is_kun_pos(pos)) else {
                continue;
            };
            let (kanji, tail) = split_leading_han(surface);
            let mut chars = kanji.chars();
            if let (Some(c), None) = (chars.next(), chars.next())
                && entry.kanji.contains(&c)
                && tail == entry.tail
            {
                by_lemma
                    .entry(lemma)
                    .or_default()
                    .insert(reading.to_string());
            }
        }
        StandardReadings { by_lemma }
    }

    /// 行を分類する。対象外なら `None`。引数は行の lemma、品詞、辞書形の表記と読み。
    pub fn classify(
        &self,
        standard: &StandardReadings<'_>,
        lemma: &str,
        pos: &str,
        dict_surface: &str,
        dict_reading: &str,
    ) -> Option<KunNotation> {
        let entry = self.get(lemma).filter(|_| is_kun_pos(pos))?;
        let (kanji, tail) = split_leading_han(dict_surface);
        if kanji.is_empty() || !standard.accepts(lemma, dict_reading) {
            return None;
        }
        let mut chars = kanji.chars();
        let single = match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        };
        let listed = single.filter(|c| entry.kanji.contains(c));
        let standard_tail = tail == entry.tail;
        let potential_tail = potential_form(&entry.tail).is_some_and(|p| p == tail);
        match listed {
            Some(c) if standard_tail || potential_tail => Some(KunNotation::Listed {
                is_default: entry.default == Some(c),
                standard_tail,
            }),
            _ => Some(KunNotation::Outside),
        }
    }
}

/// 辞書の全行の [`KunInfo`]（`lexicon.rows` と同じ並び）。
/// 辞書形の表記と読みは、行の辞書形 ID から引く（`lexicon.inflect`）。引けなければ行自身のものを使う。
pub fn kun_infos(table: &KunTable, lexicon: &Lexicon) -> Vec<KunInfo> {
    let mut infos = vec![KunInfo::default(); lexicon.rows.len()];
    if table.is_empty() {
        return infos;
    }
    // 表の lemma の動詞・形容詞の行だけ、辞書形を引く
    let candidates: Vec<(usize, &str, &str, &str)> = lexicon
        .rows
        .iter()
        .enumerate()
        .filter_map(|(i, (row, pos_id))| {
            let pos = lexicon.pos[usize::from(*pos_id)].name.as_str();
            if !is_kun_pos(pos) || table.get(&row.lemma).is_none() {
                return None;
            }
            let (surface, reading) = row
                .dict_form
                .and_then(|id| {
                    lexicon
                        .inflect
                        .binary_search_by_key(&id, |r| r.id)
                        .ok()
                        .map(|k| &lexicon.inflect[k])
                })
                .map_or((row.surface.as_str(), row.reading.as_str()), |r| {
                    (r.surface.as_str(), r.reading.as_str())
                });
            Some((i, pos, surface, reading))
        })
        .collect();
    let standard = table.standard_readings(candidates.iter().map(|&(i, pos, surface, reading)| {
        (lexicon.rows[i].0.lemma.as_str(), pos, surface, reading)
    }));
    let notations: Vec<(usize, KunNotation)> = candidates
        .iter()
        .filter_map(|&(i, pos, surface, reading)| {
            table
                .classify(&standard, &lexicon.rows[i].0.lemma, pos, surface, reading)
                .map(|n| (i, n))
        })
        .collect();

    // lemma の中の既定: 同じ (lemma, 読み, 品詞) の、既定でない表の表記の最小のコスト
    let mut min_other: HashMap<(&str, &str, u16), i32> = HashMap::new();
    for &(i, notation) in &notations {
        if let KunNotation::Listed {
            is_default: false, ..
        } = notation
        {
            let (row, pos_id) = &lexicon.rows[i];
            let key = (row.lemma.as_str(), row.reading.as_str(), *pos_id);
            let cost = i32::from(row.cost);
            min_other
                .entry(key)
                .and_modify(|m| *m = (*m).min(cost))
                .or_insert(cost);
        }
    }
    for &(i, notation) in &notations {
        let (row, pos_id) = &lexicon.rows[i];
        match notation {
            KunNotation::Outside => infos[i].outside = true,
            KunNotation::Listed {
                is_default: true,
                standard_tail,
            } => {
                let key = (row.lemma.as_str(), row.reading.as_str(), *pos_id);
                infos[i].lemma_default_excess =
                    min_other.get(&key).map(|m| i32::from(row.cost) - m);
                let stem = table.get(&row.lemma).and_then(KunEntry::group_stem);
                infos[i].reading_default =
                    standard_tail && stem.is_some_and(|s| row.reading.starts_with(s));
            }
            KunNotation::Listed { .. } => {}
        }
    }
    infos
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::lex::LexAccumulator;

    const TABLE: &str = "\
# 例
勧める\t勧薦\t勧\t
進める\t進\t進\tすすめる
取る\t取採執捕\t取\tとる
書く\t書\t書\tかく
見る\t見診\t見\tみる
測る\t測計量\t\t
暖かい\t暖\t暖\t
";

    fn table() -> KunTable {
        KunTable::parse(TABLE).unwrap()
    }

    /// 辞書形の読みを標準の形にして分類する（`classify` の単体テスト用）
    fn classify(lemma: &str, pos: &str, surface: &str, reading: &str) -> Option<KunNotation> {
        let t = table();
        let std = t.standard_readings(
            [
                ("勧める", pos, "勧める", "すすめる"),
                ("取る", pos, "取る", "とる"),
                ("書く", pos, "書く", "かく"),
                ("見る", pos, "見る", "みる"),
                (
                    "暖かい",
                    "形容詞,一般,*,*,形容詞,終止形-一般",
                    "暖かい",
                    "あたたかい",
                ),
            ]
            .into_iter(),
        );
        t.classify(&std, lemma, pos, surface, reading)
    }

    const VERB: &str = "動詞,一般,*,*,五段-ラ行,終止形-一般";
    const LISTED_DEFAULT: Option<KunNotation> = Some(KunNotation::Listed {
        is_default: true,
        standard_tail: true,
    });
    const LISTED_OTHER: Option<KunNotation> = Some(KunNotation::Listed {
        is_default: false,
        standard_tail: true,
    });

    #[test]
    fn builtin_table_parses_and_has_the_adr_entries() {
        let t = KunTable::parse(KUN_TABLE_TEXT).unwrap();
        assert!(t.len() > 150, "{}", t.len());
        let susumeru = t.get("勧める").unwrap();
        assert_eq!(susumeru.default, Some('勧'));
        assert!(susumeru.kanji.contains(&'薦'));
        // 読みの組の既定（ADR 0009 §10 Q2）
        for (lemma, group) in [
            ("進める", "すすめる"),
            ("直る", "なおる"),
            ("直す", "なおす"),
            ("書く", "かく"),
            ("取る", "とる"),
            ("聞く", "きく"),
            ("変える", "かえる"),
            ("帰る", "かえる"),
            ("開ける", "あける"),
            ("上る", "のぼる"),
            ("送る", "おくる"),
            ("打つ", "うつ"),
            ("表す", "あらわす"),
            ("見る", "みる"),
        ] {
            assert_eq!(
                t.get(lemma).unwrap().reading_group.as_deref(),
                Some(group),
                "{lemma}"
            );
        }
        // 既定を置かないもの
        for lemma in [
            "会う",
            "合う",
            "つとめる",
            "勤める",
            "務める",
            "測る",
            "写る",
            "映る",
            "移る",
        ] {
            if let Some(e) = t.get(lemma) {
                assert_eq!(e.reading_group, None, "{lemma}");
            }
        }
        assert_eq!(t.get("会う").unwrap().default, Some('会'));
        assert_eq!(t.get("測る").unwrap().default, None);
        // 形容詞（早い・速い、暑い・熱い）
        assert!(t.get("早い").is_some() && t.get("暑い").is_some());
        // 表外の漢字を表に入れていない
        assert!(!t.get("勧める").unwrap().kanji.contains(&'奨'));
        assert!(!t.get("現す").unwrap().kanji.contains(&'顕'));
        // 観る・視る・看る、逢う・遇う は表外（ADR 0009 §10 Q3）
        let miru = &t.get("見る").unwrap().kanji;
        assert!(miru.contains(&'見') && miru.contains(&'診'));
        assert!(!['観', '視', '看'].iter().any(|c| miru.contains(c)));
        assert!(
            !['逢', '遇']
                .iter()
                .any(|c| t.get("会う").unwrap().kanji.contains(c))
        );
    }

    #[test]
    fn parse_rejects_bad_tables() {
        for bad in [
            "勧める\t勧",                                 // 列が足りない
            "勧める\t勧薦\t推\t",                         // 既定が漢字の中にない
            "勧める\t\t\t",                               // 漢字が空
            "勧める\t勧\t勧\tすすめる\n勧める\t勧\t勧\t", // 重複
            "すすめる\t勧\t勧\t",                         // lemma が漢字で始まらない
            "還帰る\t還\t還\t",                           // 漢字2字
            "勧\t勧\t勧\t",                               // 送り仮名がない
            "勧める\t勧薦\t\tすすめる",                   // 読みの組には既定が要る
            "勧める\tかん\t\t",                           // 漢字でない
        ] {
            assert!(KunTable::parse(bad).is_err(), "{bad:?}");
        }
        assert!(KunTable::parse("# c\n\n勧める\t勧薦\t勧\t\t備考\n").is_ok());
    }

    #[test]
    fn canonical_text_ignores_comments_and_notes() {
        let a = KunTable::parse("勧める\t勧薦\t勧\t\t備考\n").unwrap();
        let b = KunTable::parse("# x\n\n勧める\t勧薦\t勧\t\n").unwrap();
        let c = KunTable::parse("勧める\t勧薦\t薦\t\n").unwrap();
        assert_eq!(a.canonical_text(), b.canonical_text());
        assert_ne!(a.canonical_text(), c.canonical_text());
    }

    #[test]
    fn outside_when_the_kanji_is_not_in_the_table() {
        assert_eq!(
            classify("勧める", VERB, "勧める", "すすめる"),
            LISTED_DEFAULT
        );
        assert_eq!(classify("勧める", VERB, "薦める", "すすめる"), LISTED_OTHER);
        for outside in ["奨める", "奬める", "推める"] {
            assert_eq!(
                classify("勧める", VERB, outside, "すすめる"),
                Some(KunNotation::Outside),
                "{outside}"
            );
        }
        // 取る: 摂る・盗る・獲る は表外（ADR 0009 §2.1）
        for outside in ["摂る", "盗る", "獲る", "録る"] {
            assert_eq!(
                classify("取る", VERB, outside, "とる"),
                Some(KunNotation::Outside)
            );
        }
        // 漢字2字は表外
        assert_eq!(
            classify("帰る", VERB, "還帰る", "かえる"),
            None,
            "表にない lemma は対象外"
        );
        assert_eq!(
            classify("取る", VERB, "取取る", "とる"),
            Some(KunNotation::Outside)
        );
    }

    #[test]
    fn outside_when_okurigana_does_not_fit() {
        assert_eq!(classify("見る", VERB, "見る", "みる"), LISTED_DEFAULT);
        assert_eq!(
            classify("見る", VERB, "見ル", "みる"),
            Some(KunNotation::Outside)
        );
        assert_eq!(
            classify("取る", VERB, "取", "とる"),
            Some(KunNotation::Outside)
        );
        assert_eq!(
            classify(
                "暖かい",
                "形容詞,一般,*,*,形容詞,終止形-一般",
                "暖い",
                "あたたかい"
            ),
            Some(KunNotation::Outside)
        );
    }

    #[test]
    fn potential_verbs_are_not_outside() {
        // 書ける、取れる、見れる（ら抜き）は元の動詞と同じに扱う
        let potential_other = |is_default| {
            Some(KunNotation::Listed {
                is_default,
                standard_tail: false,
            })
        };
        assert_eq!(
            classify("書く", VERB, "書ける", "かける"),
            potential_other(true)
        );
        assert_eq!(
            classify("取る", VERB, "取れる", "とれる"),
            potential_other(true)
        );
        assert_eq!(
            classify("取る", VERB, "捕れる", "とれる"),
            potential_other(false)
        );
        assert_eq!(
            classify("見る", VERB, "見れる", "みれる"),
            potential_other(true)
        );
        // 表外の漢字の可能動詞は表外のまま
        assert_eq!(
            classify("取る", VERB, "摂れる", "とれる"),
            Some(KunNotation::Outside)
        );
        assert_eq!(
            classify("勧める", VERB, "奨めれる", "すすめれる"),
            Some(KunNotation::Outside)
        );
    }

    #[test]
    fn kana_only_and_different_readings_are_not_classified() {
        // かなだけ・カタカナまじり・漢字で始まらない表記
        for surface in ["すすめる", "ススメる", "ススめる", "お勧める"] {
            assert_eq!(
                classify("勧める", VERB, surface, "すすめる"),
                None,
                "{surface}"
            );
        }
        // 読みが標準の形と違う行は別の語（取る/どる、書くぅ）
        assert_eq!(classify("取る", VERB, "取る", "どる"), None);
        assert_eq!(classify("書く", VERB, "書くぅ", "かくぅ"), None);
        // 動詞・形容詞以外は対象外
        assert_eq!(
            classify("取る", "名詞,普通名詞,一般,*,*,*", "取る", "とる"),
            None
        );
        // 表にない lemma
        assert_eq!(classify("行く", VERB, "行く", "いく"), None);
    }

    #[test]
    fn group_stem_drops_the_last_char() {
        let t = table();
        assert_eq!(t.get("進める").unwrap().group_stem(), Some("すすめ"));
        assert_eq!(t.get("書く").unwrap().group_stem(), Some("か"));
        assert_eq!(t.get("勧める").unwrap().group_stem(), None);
    }

    /// 19列の Sudachi 形式の行（辞書形の ID は行番号）
    fn line(
        surface: &str,
        cost: i32,
        pos: &str,
        reading: &str,
        lemma: &str,
        dict_form: usize,
    ) -> String {
        format!("{surface},1,1,{cost},{surface},{pos},{reading},{lemma},{dict_form},A,*,*,*,*")
    }

    fn lexicon(lines: &[String]) -> Lexicon {
        let mut acc = LexAccumulator::new(10, 10);
        for (i, l) in lines.iter().enumerate() {
            acc.add_line(l, &PathBuf::from("t.csv"), i + 1).unwrap();
        }
        acc.finish()
    }

    /// 品詞は「動詞,一般,*,*,五段-ラ行,<活用形>」の形（`line` の引数に活用形も含める）
    fn verb(form: &str) -> String {
        format!("動詞,一般,*,*,下一段-マ行,{form}")
    }

    #[test]
    fn kun_infos_applies_the_three_rules() {
        // 0 勧める(終止) 1 薦める(終止) 2 奨める(終止)   終止形の行がそれぞれの辞書形
        // 3 勧め(未然) 4 薦め(未然) 5 進める(終止)   すすめ: 勧 11385 薦 11242
        let end = verb("終止形-一般");
        let neg = verb("未然形-一般");
        let lines = vec![
            line("勧める", 6000, &end, "ススメル", "勧める", 0),
            line("薦める", 6100, &end, "ススメル", "勧める", 1),
            line("奨める", 6200, &end, "ススメル", "勧める", 2),
            line("勧め", 11385, &neg, "ススメ", "勧める", 0),
            line("薦め", 11242, &neg, "ススメ", "勧める", 1),
            line("奨め", 11636, &neg, "ススメ", "勧める", 2),
            line("進める", 5000, &end, "ススメル", "進める", 6),
            line("進め", 11727, &neg, "ススメ", "進める", 6),
        ];
        let lex = lexicon(&lines);
        let t = KunTable::parse("勧める\t勧薦\t勧\t\n進める\t進\t進\tすすめる\n").unwrap();
        let infos = kun_infos(&t, &lex);
        let by_surface = |surface: &str, reading: &str| {
            let i = lex
                .rows
                .iter()
                .position(|(r, _)| r.surface == surface && r.reading == reading)
                .unwrap();
            infos[i]
        };
        // 表外
        assert!(by_surface("奨める", "すすめる").outside);
        assert!(by_surface("奨め", "すすめ").outside);
        assert!(!by_surface("勧める", "すすめる").outside);
        // lemma の中の既定: 勧め(11385) は 薦め(11242) より 143 高い。勧める(6000) は 薦める(6100) より安い
        assert_eq!(by_surface("勧め", "すすめ").lemma_default_excess, Some(143));
        assert_eq!(
            by_surface("勧める", "すすめる").lemma_default_excess,
            Some(-100)
        );
        assert_eq!(by_surface("薦め", "すすめ").lemma_default_excess, None);
        // lemma をまたいだ比較はしない: 進める は 勧める の中の比較に入らない
        assert_eq!(by_surface("進める", "すすめる").lemma_default_excess, None);
        // 読みの組の既定: 進める の既定の行（読みが すすめ で始まる）だけ
        assert!(by_surface("進める", "すすめる").reading_default);
        assert!(by_surface("進め", "すすめ").reading_default);
        assert!(!by_surface("勧める", "すすめる").reading_default);
    }

    #[test]
    fn reading_default_needs_the_group_reading_and_the_standard_tail() {
        let end = "動詞,一般,*,*,五段-カ行,終止形-一般";
        let potential = "動詞,一般,*,*,下一段-カ行,終止形-一般";
        let lines = vec![
            line("書く", 8875, end, "カク", "書く", 0),
            line("書ける", 10722, potential, "カケル", "書く", 1),
            line("描く", 9000, end, "カク", "書く", 2),
        ];
        let lex = lexicon(&lines);
        let t = KunTable::parse("書く\t書描\t書\tかく\n").unwrap();
        let infos = kun_infos(&t, &lex);
        let at = |surface: &str| {
            let i = lex
                .rows
                .iter()
                .position(|(r, _)| r.surface == surface)
                .unwrap();
            infos[i]
        };
        assert!(at("書く").reading_default);
        // 可能動詞は読みの組の既定を受けない。罰せられもしない
        assert!(!at("書ける").reading_default);
        assert!(!at("書ける").outside);
        // 描く（かく）は表の中の別の表記。書く(8875) は 描く(9000) より安いので変えない
        assert!(!at("描く").outside);
        assert_eq!(at("書く").lemma_default_excess, Some(8875 - 9000));
    }

    #[test]
    fn rows_without_the_table_are_untouched() {
        let end = "動詞,一般,*,*,五段-カ行,終止形-一般";
        let lex = lexicon(&[line("行く", 7000, end, "イク", "行く", 0)]);
        let t = KunTable::parse("書く\t書\t書\tかく\n").unwrap();
        assert_eq!(kun_infos(&t, &lex), vec![KunInfo::default()]);
        assert_eq!(
            kun_infos(&KunTable::parse("").unwrap(), &lex),
            vec![KunInfo::default()]
        );
    }
}
