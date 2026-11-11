//! ABR（アドレス・ベース・レジストリ）の町字マスターから、辞書に足す地名の行を作る（ADR 0030）。
//!
//! 入力は `mt_town_all.csv`（UTF-8、BOM は付いていても外す。ヘッダの列名で列を引く）。大字・町名の表記と読みの組を取り、
//! Sudachi の lex.csv と同じ 19 列の行（品詞 `名詞,固有名詞,地名,一般`）にして書く。書いた行は
//! `--notcore-lex` として辞書のビルドに渡す（notcore と同じ扱い。品詞の統計と活用規則には数えず、固有名詞の d2s の絞り込みを受ける）。
//!
//! 国の未加工データではない（加工して使う）。出典と加工の表記は `THIRD_PARTY_NOTICES.md`（設定アプリの「このソフトについて」に出る）にある。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use ikari_dict_format::postal::{
    POSTAL_TSV_COLUMNS_COMMENT, POSTAL_TSV_HEADER, format_line, parse_code,
};
use ikari_dict_format::reading::{fold_fullwidth_digits, normalize_key, strip_reading_separators};

use crate::lex::unescape_unicode;
use crate::numeral::expand_digit_town;
use crate::{BuildError, open_reader, read_line_at};

/// (読み, 表記)。Sudachi との照合では読みを `match_key` にしたもの
type Key = (String, String);
/// (左 ID, 右 ID)
type Ids = (u16, u16);

/// 辞書に入れる行の品詞（Sudachi の地名。ADR 0030 §3）
const TOWN_POS: &str = "名詞,固有名詞,地名,一般,*,*";
/// 地名のコストの既定。Sudachi の `名詞,固有名詞,地名,一般` の最頻値（ユーザー決定 2026-10-22。ADR 0030 §4。
/// 7000〜17000 は評価セットの悪化が 0。安全側の中ほど）
pub const DEFAULT_TOWN_COST: i16 = 11000;
/// 住所の連なり（都道府県 + 市区町村 + 町名）のコストの既定。掃引の結果からの暫定値（ユーザー確認待ち。ADR 0030 §12.3）
pub const DEFAULT_CHAIN_COST: i16 = 7000;
/// 「都道府県 + 市区町村」だけの連なりのコストの既定（#202 第 2 段階。ADR 0030 §13。掃引で決める）
pub const DEFAULT_MUNICIPALITY_CHAIN_COST: i16 = 7000;
/// 数字を含む町名の連なりのうち、全角の数字の表記（`北１条西`）の行のコストに足す差。半角の表記（`北1条西`。公用文の既定）を先に出すための
/// 同点の解消で、順位の差を作る大きさではない（ADR 0030 §3.1。ユーザー決定 2026-11-10: 半角を先）
pub const FULLWIDTH_SURFACE_COST_DELTA: i16 = 1;

// ABR の CSV の列名
const H_TYPE: &str = "machiaza_type";
const H_STATUS: &str = "status_flg";
const H_OAZA: &str = "oaza_cho";
const H_OAZA_KANA: &str = "oaza_cho_kana";
// 住所の連なり（第 1 段階。ADR 0030 §12）に使う、都道府県・郡・市区町村・区の列
const H_PREF: &str = "pref";
const H_PREF_KANA: &str = "pref_kana";
const H_COUNTY: &str = "county";
const H_COUNTY_KANA: &str = "county_kana";
const H_CITY: &str = "city";
const H_CITY_KANA: &str = "city_kana";
const H_WARD: &str = "ward";
const H_WARD_KANA: &str = "ward_kana";
// 町字マスターの結合用の列（郵便番号の変換表と (lg_code, machiaza_id) で結ぶ。ADR 0032 §3）
const H_LG_CODE: &str = "lg_code";
const H_MACHIAZA_ID: &str = "machiaza_id";
// 郵便番号の変換表（ABR「町字・郵便番号変換表」）の列
const H_POST_CODE: &str = "post_code";
const H_DELETE_DATE: &str = "dlt_date";
const BOM: char = '\u{feff}';
const CSV_SEPARATOR: char = ',';
const CSV_QUOTE: char = '"';

/// 町字区分（machiaza_type）。大字・町名と丁目の行だけを使う（小字、京都の通り名などは使わない）
const TYPE_OAZA_CHO: &str = "1";
const TYPE_CHOME: &str = "2";
/// 状態フラグ。3 の意味は未確認なので使わない
const STATUS_UNKNOWN: &str = "3";

const PREFIX_OAZA: &str = "大字";
const PREFIX_OAZA_KANA: &str = "オオアザ";
const PREFIX_AZA: &str = "字";
const PREFIX_AZA_KANA: &str = "アザ";
const AZA_KANA: &str = "アザ";
const CHO: char = '町';
const MURA: char = '村';
/// 「町」「村」で終わる表記の読みの終わり
const CHO_READINGS: [&str; 2] = ["チョウ", "マチ"];
const MURA_READINGS: [&str; 2] = ["ムラ", "ソン"];
const CHOME: &str = "丁目";
/// 丁目の読み（小書きにしていない「チヨウメ」も）
const CHOME_KANAS: [&str; 2] = ["チョウメ", "チヨウメ"];
/// ABR の読みで小書きにしていない拗音の直し方（ADR 0030 §12.2。Sudachi の読みと突き合わせた実測で決めた）。
/// 「イ段の字 + ヨ + ウ/ー」は小書きの「ョ」に直す（`チヨウ` は `チョウ`）。ABR の大書きの `ヨ` のうち、Sudachi が小書きにしているものの
/// 99% がこの形（イ段の字は FIX_YO_AFTER）。`チヨダ`（千代田）のように、`ヨ` のあとが `ウ`・`ー` でないものは直さない。
const FIX_YO_AFTER: &str = "キシジチヒビミリ";
/// 「チ・リ・キ + ユ + ウ」は小書きの「ュ」に直す（`チユウ` は `チュウ`。実測で 27/29）。`ジユウ`（自由ヶ丘）は大書きが正しい例が
/// 半分あるので直さない
const FIX_YU_AFTER: &str = "キチリ";
const FIX_FOLLOW: &str = "ウー";
const SMALL_YO: char = 'ョ';
const SMALL_YU: char = 'ュ';
const LARGE_YO: char = 'ヨ';
const LARGE_YU: char = 'ユ';
/// 表記に最低限必要な文字数（1 文字の地名は普通の語と同音になりやすいので入れない）
const MIN_SURFACE_CHARS: usize = 2;
const FULLWIDTH_ZERO: u32 = '０' as u32;

// Sudachi の lex.csv の列
const SUDACHI_COLUMNS_MIN: usize = 12;
const SUDACHI_COL_LEFT: usize = 1;
const SUDACHI_COL_RIGHT: usize = 2;
const SUDACHI_COL_SURFACE: usize = 4;
const SUDACHI_COL_POS_FIRST: usize = 5;
const SUDACHI_COL_POS_LAST: usize = 10;
const SUDACHI_COL_READING: usize = 11;
const SPLIT_ONLY_ID: &str = "-1";

/// スキップの理由（レポートの名前）
pub const TOWN_SKIP_TYPE: &str = "machiaza_type_not_used";
pub const TOWN_SKIP_STATUS: &str = "status_flg_3";
pub const TOWN_SKIP_EMPTY_READING: &str = "empty_reading";
pub const TOWN_SKIP_EMPTY_SURFACE: &str = "empty_surface";
pub const TOWN_SKIP_SURFACE_CHARS: &str = "surface_has_unsupported_char";
pub const TOWN_SKIP_READING_CHARS: &str = "reading_has_unsupported_char";
pub const TOWN_SKIP_PREFIX: &str = "aza_prefix_without_matching_reading";
pub const TOWN_SKIP_COMPOUND_AZA: &str = "compound_with_aza";
pub const TOWN_SKIP_CHOME_IN_READING: &str = "chome_only_in_reading";
pub const TOWN_SKIP_SUFFIX_NOT_IN_READING: &str = "cho_mura_suffix_not_in_reading";
pub const TOWN_SKIP_SHORT: &str = "single_char_surface";
pub const TOWN_SKIP_NO_HAN: &str = "no_han_char";
/// 表記に数字がある町名で、数字を読みにした形を作れなかった（数字の連なりが読みと合わない、助数詞が分からない、読みに数詞が見つからない）
pub const TOWN_SKIP_DIGITS_UNALIGNED: &str = "digits_not_expandable";
/// 数字を含む町名は、単語の行にはしない（住所の連なりと郵便番号の表だけに入れる。ADR 0030 §3.1）
pub const TOWN_SKIP_DIGITS_NOT_WORD: &str = "digits_chain_only";
pub const TOWN_SKIP_DUPLICATE: &str = "duplicate_pair";
pub const TOWN_SKIP_IN_SUDACHI: &str = "already_in_sudachi";
/// 住所の連なりを作れなかった行（都道府県・郡・市・区に使えない字がある）
pub const TOWN_SKIP_CHAIN_PART: &str = "chain_part_unusable";

pub struct TownConfig {
    pub csv: PathBuf,
    /// Sudachi の lex.csv（small、core、notcore）。同じ表記と読みの行を足さないために読む
    pub sudachi_lex: Vec<PathBuf>,
    pub out: PathBuf,
    pub cost: i16,
    /// 住所の連なり（ADR 0030 §12）。`None` なら作らない
    pub chain: Option<ChainConfig>,
    /// 住所と郵便番号の表 `postal.tsv`（ADR 0032 §3）。`None` なら作らない
    pub postal: Option<PostalConfig>,
}

/// `postal.tsv` の入出力。入力は ABR の「町字・郵便番号変換表」
pub struct PostalConfig {
    pub csv: PathBuf,
    pub out: PathBuf,
}

/// 住所の連なりの出力。単語の `out` とは別のファイルに書き、辞書のビルドには `--chain-lex` で渡す
/// （Sudachi の Core にある同じ行より優先させるため）
pub struct ChainConfig {
    pub out: PathBuf,
    pub cost: i16,
    /// 町名がない「都道府県 + 市区町村」の連なりのコスト
    pub municipality_cost: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TownReport {
    pub rows_read: usize,
    pub skipped: BTreeMap<&'static str, usize>,
    /// 町の読みの「チヨウ」を「チョウ」に直した数（重複を除く前の行数）
    pub readings_fixed: usize,
    pub written: usize,
    /// 住所の連なりの行数（`out` とは別のファイル）
    pub chains_written: usize,
    pub left_id: u16,
    pub right_id: u16,
    pub cost: i16,
    /// `postal.tsv` を作ったときの数（作らなければ `None`）
    pub postal: Option<PostalReport>,
}

/// `postal.tsv` を作った結果（結べなかった行・畳めなかった行の数を含む。ADR 0032 §3）
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostalReport {
    /// 変換表のデータ行の数
    pub rows_read: usize,
    /// `dlt_date` がある行（廃止された町字。使わない）
    pub deleted: usize,
    /// 郵便番号が 7 桁の半角でない行
    pub bad_code: usize,
    /// (lg_code, machiaza_id) が町字マスターにない行
    pub unjoined: usize,
    /// 町字マスターにはあるが、大字・町名に畳めなかった行（大字・町名の読み・表記が使えない、市区町村の読みが使えない、状態が不明など）
    pub not_folded: usize,
    /// 市区町村だけの行（machiaza_id が空）を、市区町村の連なりの郵便番号に使った数
    pub municipality_rows_used: usize,
    /// 市区町村だけの行のうち、市区町村の連なりを作れなかった（町字マスターに町がない）数
    pub municipality_rows_unused: usize,
    /// 町（市区町村だけの連なりを含む）の数 = `f` の行数
    pub towns: usize,
    /// そのうち、辞書に連なりがない町（小字だけの大字など。郵便番号 → 住所のために入れる）
    pub towns_outside_chain: usize,
    /// `s` の行数
    pub short_lines: usize,
    /// 同じ (読み, 表記) が別の町にもあって、郵便番号をまとめた `s` の行の数
    pub short_merged: usize,
    /// 別の行の `f` と同じ (読み, 表記) になるので書かなかった `s` の数
    pub short_dropped_as_full: usize,
    /// 郵便番号の種類（出力に出る異なる番号）
    pub distinct_codes: usize,
    /// 2 つ以上の `f`（町）に出る郵便番号の数
    pub codes_with_many_towns: usize,
}

impl std::fmt::Display for PostalReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "postal rows read:  {}", self.rows_read)?;
        writeln!(f, "  skipped (dlt_date): {}", self.deleted)?;
        writeln!(f, "  skipped (bad post_code): {}", self.bad_code)?;
        writeln!(f, "  skipped (not in town master): {}", self.unjoined)?;
        writeln!(f, "  skipped (town not folded): {}", self.not_folded)?;
        writeln!(
            f,
            "  municipality-only rows: used {} / unused {}",
            self.municipality_rows_used, self.municipality_rows_unused
        )?;
        writeln!(
            f,
            "postal lines:      f {} (outside chain {}) / s {} (s merged {}, s dropped as f {})",
            self.towns,
            self.towns_outside_chain,
            self.short_lines,
            self.short_merged,
            self.short_dropped_as_full
        )?;
        write!(
            f,
            "postal codes:      {} distinct, {} with several towns",
            self.distinct_codes, self.codes_with_many_towns
        )
    }
}

impl std::fmt::Display for TownReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "abr rows read:     {}", self.rows_read)?;
        for (reason, count) in &self.skipped {
            writeln!(f, "  skipped ({reason}): {count}")?;
        }
        writeln!(f, "readings fixed:    {}", self.readings_fixed)?;
        writeln!(
            f,
            "ids / cost:        left {} right {} cost {}",
            self.left_id, self.right_id, self.cost
        )?;
        writeln!(f, "town lex rows:     {}", self.written)?;
        write!(f, "chain lex rows:    {}", self.chains_written)?;
        if let Some(postal) = &self.postal {
            write!(f, "\n{postal}")?;
        }
        Ok(())
    }
}

fn input_error(path: &Path, line: usize, message: impl Into<String>) -> BuildError {
    BuildError::Input {
        file: path.to_path_buf(),
        line,
        message: message.into(),
    }
}

fn is_han(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
}

/// CJK 互換漢字のうち、ABR の大字・町名に出る NFKC で統合漢字になる字（塚、渚、隆、琢）を直す。
/// NFKC の全体は使わない（依存を足さない。ABR のそれ以外の NFKC 対象は全角の数字と英字で、どちらも辞書の読みにできないので入れない）。
/// 「﨑」（U+FA11）は NFKC でも変わらない別の字なので、そのまま入れない。
fn unify_compat_ideograph(c: char) -> char {
    match c {
        '\u{fa10}' => '\u{585a}',
        '\u{fa46}' => '\u{6e1a}',
        '\u{f9dc}' => '\u{9686}',
        '\u{fa4a}' => '\u{7422}',
        _ => c,
    }
}

/// 表記に使える文字。CJK 統合漢字（基本ブロック）、々〆〇、ひらがな、カタカナ（ヶ ヵ を含む）、長音。
/// 英字、括弧、私用領域・拡張漢字・異体字選択子は含まない。数字はここに含まない（`convert` が半角にそろえて別に通す。ADR 0030 §3.1）。
fn is_surface_char(c: char) -> bool {
    is_han(c)
        || matches!(c, '々' | '〆' | '〇' | 'ー')
        || ('\u{3041}'..='\u{3096}').contains(&c)
        || ('\u{30a1}'..='\u{30fa}').contains(&c)
}

/// 読みに使える文字（全角カタカナと長音）
fn is_reading_char(c: char) -> bool {
    ('\u{30a1}'..='\u{30f6}').contains(&c) || c == 'ー'
}

/// 町字マスターの 1 組（表記、読み）を辞書に入れる組にする。入れないときは理由を返す。
/// 戻り値の `bool` は、読みを直したか。
fn convert(oaza_raw: &str, kana: &str) -> Result<(String, String, bool), &'static str> {
    if kana.is_empty() {
        return Err(TOWN_SKIP_EMPTY_READING);
    }
    // 全角の数字は半角にそろえる（ABR の表記と読みは `１０` のように全角。ADR 0030 §3.1）
    let oaza_unified: String = fold_fullwidth_digits(oaza_raw)
        .chars()
        .map(unify_compat_ideograph)
        .collect();
    let kana = fold_fullwidth_digits(kana);
    let kana = kana.as_ref();
    let oaza = oaza_unified.as_str();
    if oaza.is_empty() {
        return Err(TOWN_SKIP_EMPTY_SURFACE);
    }
    if !oaza
        .chars()
        .all(|c| is_surface_char(c) || c.is_ascii_digit())
    {
        return Err(TOWN_SKIP_SURFACE_CHARS);
    }
    if !kana
        .chars()
        .all(|c| is_reading_char(c) || c.is_ascii_digit())
    {
        return Err(TOWN_SKIP_READING_CHARS);
    }
    // 「大字」「字」の接頭辞は、入力しないので外す（読みの「オオアザ」「アザ」も外す）
    let (surface, reading) = if let Some(rest) = oaza.strip_prefix(PREFIX_OAZA) {
        let kana_rest = kana
            .strip_prefix(PREFIX_OAZA_KANA)
            .ok_or(TOWN_SKIP_PREFIX)?;
        (rest, kana_rest)
    } else if let Some(rest) = oaza.strip_prefix(PREFIX_AZA) {
        let kana_rest = kana.strip_prefix(PREFIX_AZA_KANA).ok_or(TOWN_SKIP_PREFIX)?;
        (rest, kana_rest)
    } else {
        (oaza, kana)
    };
    if surface.is_empty() || reading.is_empty() {
        return Err(TOWN_SKIP_EMPTY_SURFACE);
    }
    // 「大字〇〇字△△」のように字が残る複合は、住所の一部であって町名ではない
    if surface.contains(PREFIX_AZA) && reading.contains(AZA_KANA) {
        return Err(TOWN_SKIP_COMPOUND_AZA);
    }
    if surface.chars().count() < MIN_SURFACE_CHARS {
        return Err(TOWN_SKIP_SHORT);
    }
    if !surface.chars().any(is_han) {
        return Err(TOWN_SKIP_NO_HAN);
    }
    // 丁目の数が読みだけにある行（表記は丁目を含まない）は、入力に丁目を足さないと変換できない読みになる
    if CHOME_KANAS.iter().any(|k| reading.contains(k)) && !surface.contains(CHOME) {
        return Err(TOWN_SKIP_CHOME_IN_READING);
    }
    // 小書きにしていない拗音（`チヨウ`、`キヨウ`）を直す。直す形は限る（`fix_small_kana`）
    let fixed_reading = fix_small_kana(reading);
    let fixed = fixed_reading != reading;
    let reading = fixed_reading;
    // 表記が町・村で終わるのに、読みがその読み（チョウ、マチ、ムラ、ソン）で終わらない行は、読みにない「町」「村」を足す変換になる
    if !suffix_matches_reading(surface, &reading) {
        return Err(TOWN_SKIP_SUFFIX_NOT_IN_READING);
    }
    Ok((surface.to_string(), reading, fixed))
}

/// 町名 1 つの表記と読み。数字を含む町名（`北1条西`）は、読みが複数になり、表記は全角の数字の形も持つ（ADR 0030 §3.1）
#[derive(Debug, Clone, PartialEq, Eq)]
struct TownNames {
    /// 表記（数字は半角）
    surface: String,
    /// 全角の数字の表記（数字を含む町名だけ）
    surface_fullwidth: Option<String>,
    /// 読み（カタカナ）。先頭が標準の読み。数字を含む町名は、数字を読みにした形（ゆれの全部）、数字を含む読み（`キタ1ジョウニシ`）の順
    readings: Vec<String>,
}

impl TownNames {
    fn has_digits(&self) -> bool {
        self.surface_fullwidth.is_some()
    }
}

/// 半角の数字を全角にする
fn fullwidth_digits(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_digit() {
                char::from_u32(FULLWIDTH_ZERO + u32::from(c) - u32::from('0')).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

/// `convert` に、数字を含む町名の読みの展開（`expand_digit_town`）を足したもの。
/// 戻り値の `bool` は、読みを直したか。数字のない町名は、`convert` と同じ 1 組。
fn convert_names(oaza_raw: &str, kana: &str) -> Result<(TownNames, bool), &'static str> {
    let (surface, reading, fixed) = convert(oaza_raw, kana)?;
    let surface_has_digits = surface.chars().any(|c| c.is_ascii_digit());
    let reading_has_digits = reading.chars().any(|c| c.is_ascii_digit());
    if !surface_has_digits {
        // 表記に数字がなく読みにだけある町名は、これまでどおり使わない
        if reading_has_digits {
            return Err(TOWN_SKIP_READING_CHARS);
        }
        return Ok((
            TownNames {
                surface,
                surface_fullwidth: None,
                readings: vec![reading],
            },
            fixed,
        ));
    }
    let readings = expand_digit_town(&surface, &reading).ok_or(TOWN_SKIP_DIGITS_UNALIGNED)?;
    Ok((
        TownNames {
            surface_fullwidth: Some(fullwidth_digits(&surface)),
            surface,
            readings,
        },
        fixed,
    ))
}

/// 表記が町・村で終わるとき、読みがその読みで終わるか。それ以外の表記は常に true。
fn suffix_matches_reading(surface: &str, reading: &str) -> bool {
    let readings: &[&str] = if surface.ends_with(CHO) {
        &CHO_READINGS
    } else if surface.ends_with(MURA) {
        &MURA_READINGS
    } else {
        return true;
    };
    readings.iter().any(|r| reading.ends_with(r))
}

/// 小書きにしていない拗音を直す（ABR の読みは小書きにしていないものがある。利用者が打つ「きょう」と一致させるため）。
/// 直すのは `FIX_YO_AFTER` の字 + `ヨ` + `ウ`/`ー`（`チヨウ` → `チョウ`）と、`FIX_YU_AFTER` の字 + `ユ` + `ウ`/`ー`（`チユウ` → `チュウ`）だけ。
/// `チヨダ`（千代田）、`ジユウガオカ`（自由ヶ丘）のように大書きが正しい読みは壊さない（理由と実測は ADR 0030 §12.2）。
fn fix_small_kana(reading: &str) -> String {
    let chars: Vec<char> = reading.chars().collect();
    let mut out = String::with_capacity(reading.len());
    for (i, &c) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).map(|j| chars[j]);
        let next = chars.get(i + 1).copied();
        let follows = next.is_some_and(|n| FIX_FOLLOW.contains(n));
        let small = match (c, prev) {
            (LARGE_YO, Some(p)) if follows && FIX_YO_AFTER.contains(p) => SMALL_YO,
            (LARGE_YU, Some(p)) if follows && FIX_YU_AFTER.contains(p) => SMALL_YU,
            _ => c,
        };
        out.push(small);
    }
    out
}

/// 住所の連なりの 1 部分（郡、市、区など）の（表記, 読み）。空の部分は（空, 空）。
/// 表記に使えない字がある、片方だけが空、読みに使えない字がある、のときは `None`。
fn chain_part(surface: &str, kana: &str) -> Option<(String, String)> {
    if surface.is_empty() && kana.is_empty() {
        return Some((String::new(), String::new()));
    }
    let surface: String = surface.chars().map(unify_compat_ideograph).collect();
    if surface.is_empty()
        || kana.is_empty()
        || !surface.chars().all(is_surface_char)
        || !kana.chars().all(is_reading_char)
    {
        return None;
    }
    Some((surface, fix_small_kana(kana)))
}

/// 拗音・促音の小書きを大書きに揃える（Sudachi の読みとの照合用。ABR は小書きにしていない読みがある）
fn fold_small_kana(c: char) -> char {
    match c {
        'ぁ' => 'あ',
        'ぃ' => 'い',
        'ぅ' => 'う',
        'ぇ' => 'え',
        'ぉ' => 'お',
        'っ' => 'つ',
        'ゃ' => 'や',
        'ゅ' => 'ゆ',
        'ょ' => 'よ',
        'ゎ' => 'わ',
        _ => c,
    }
}

/// 照合用の読み（ひらがなにして、小書きを揃える）
pub(crate) fn match_key(reading: &str) -> String {
    normalize_key(reading)
        .chars()
        .map(fold_small_kana)
        .collect()
}

/// CSV の 1 行を列に分ける。引用符は使われていない前提で、出てきたら入力の不正にする
/// （形式が変わったことに気づけるように）。
fn split_row<'a>(line: &'a str, path: &Path, lineno: usize) -> Result<Vec<&'a str>, BuildError> {
    if line.contains(CSV_QUOTE) {
        return Err(input_error(
            path,
            lineno,
            "quoted CSV fields are not supported",
        ));
    }
    Ok(line.split(CSV_SEPARATOR).collect())
}

fn column_index(header: &[&str], name: &str, path: &Path) -> Result<usize, BuildError> {
    header
        .iter()
        .position(|h| *h == name)
        .ok_or_else(|| input_error(path, 1, format!("column `{name}` not found in the header")))
}

/// `collect_pairs` の結果
struct Collected {
    /// (読み, 表記)
    pairs: BTreeSet<Key>,
    /// 住所の連なり（都道府県 + 郡 + 市 + 区 + 町名と、都道府県を省いた形）の (読み, 表記)
    chains: BTreeSet<Key>,
    /// 数字を含む町名の連なりの、全角の数字の表記の行（`北１条西`。コストを `FULLWIDTH_SURFACE_COST_DELTA` だけ高くするため、`chains` とは分ける）
    fullwidth_chains: BTreeSet<Key>,
    /// 町名がない「都道府県 + 郡 + 市 + 区」の連なり（コストを別に決めるため、`chains` とは分ける）
    municipality_chains: BTreeSet<Key>,
    rows_read: usize,
    skipped: BTreeMap<&'static str, usize>,
    fixed: usize,
    /// 連なりまで作れた町字の行（`postal.tsv` で変換表と結ぶため。`want_ids` のときだけ集める）
    towns: Vec<TownRow>,
    /// 町字マスターの全行の (lg_code, machiaza_id)（`want_ids` のときだけ。変換表の行が「マスターにない」か「畳めなかった」かを分ける）
    master_ids: HashSet<(String, String)>,
}

/// 町字マスターの 1 行を町名に畳んだもの（町名・市区町村の読みと表記が使えたものだけ）
struct TownRow {
    lg_code: String,
    machiaza_id: String,
    pref: (String, String),
    /// [郡, 市, 区]
    municipality: Vec<(String, String)>,
    surface: String,
    reading: String,
    /// `reading` のほかの読み（数字を含む町名の、ゆれの読みと数字を含む読み。`postal.tsv` の `s` の行にする）
    alt_readings: Vec<String>,
    /// 大字・町名・丁目の行（区分 1・2）か。連なり（`chain_lex.csv`）はこの行からだけ作る
    in_chain: bool,
}

/// 住所の連なりの形（(読み, 表記), いちばん省略のない形か）。都道府県を省いた形と、郡を省いた形
/// （`南幌町…`。ユーザー決定 2026-10-23）も作る。いちばん省略のない形（都道府県つき・郡つき）が先頭。
/// 市区町村がない行は作らない（都道府県名だけで町名が来る連なりは作らない）。
/// `town_surface` が空のときは「都道府県 + 市区町村」だけの連なりを作る（#202 第 2 段階。町名の前で止まった入力や、
/// 市区町村のあとに助詞や語が続く入力で、都道府県が `ギフ県` のように崩れるのを防ぐ）。このとき都道府県を省いた形
/// （市区町村名だけ）は作らない（Sudachi が普通の地名として持っている語を置き換えないため）。
fn chain_forms(
    pref: &(String, String),
    municipality: &[(String, String)],
    town_surface: &str,
    town_reading: &str,
) -> Vec<(Key, bool)> {
    let mut forms = Vec::new();
    let municipality_only = town_surface.is_empty();
    // municipality は [郡, 市, 区]。郡があるときは、郡を除いた形も作る
    let without_county = municipality
        .get(1..)
        .filter(|_| !municipality[0].0.is_empty());
    for (i, part) in std::iter::once(municipality)
        .chain(without_county)
        .enumerate()
    {
        let mut surface = String::new();
        let mut reading = String::new();
        for (s, r) in part {
            surface.push_str(s);
            reading.push_str(r);
        }
        if surface.is_empty() {
            continue;
        }
        surface.push_str(town_surface);
        reading.push_str(town_reading);
        forms.push((
            (
                format!("{}{}", pref.1, reading),
                format!("{}{}", pref.0, surface),
            ),
            i == 0,
        ));
        if !municipality_only {
            forms.push(((reading, surface), false));
        }
    }
    forms
}

/// 町名（数字を含む町名は、読みの全部と全角の数字の表記も）の住所の連なりを足す
fn add_town_chains(
    chains: &mut BTreeSet<Key>,
    fullwidth_chains: &mut BTreeSet<Key>,
    pref: &(String, String),
    municipality: &[(String, String)],
    names: &TownNames,
) {
    for reading in &names.readings {
        add_chains(chains, pref, municipality, &names.surface, reading);
        if let Some(fullwidth) = &names.surface_fullwidth {
            add_chains(fullwidth_chains, pref, municipality, fullwidth, reading);
        }
    }
}

/// 住所の連なりを足す（形は [`chain_forms`]）
fn add_chains(
    chains: &mut BTreeSet<Key>,
    pref: &(String, String),
    municipality: &[(String, String)],
    town_surface: &str,
    town_reading: &str,
) {
    chains.extend(
        chain_forms(pref, municipality, town_surface, town_reading)
            .into_iter()
            .map(|(key, _)| key),
    );
}

/// 町字マスターを読み、辞書に入れる (読み, 表記) の組を集める。Sudachi との照合の前まで。
fn collect_pairs(path: &Path, want_ids: bool) -> Result<Collected, BuildError> {
    let mut reader = open_reader(path)?;
    let mut line = String::new();
    let mut lineno = 1usize;
    if read_line_at(&mut reader, &mut line, path, lineno)? == 0 {
        return Err(input_error(path, 1, "empty file"));
    }
    let header_text = line.strip_prefix(BOM).unwrap_or(&line).to_string();
    let header: Vec<&str> = header_text.split(CSV_SEPARATOR).collect();
    let i_type = column_index(&header, H_TYPE, path)?;
    let i_status = column_index(&header, H_STATUS, path)?;
    let i_oaza = column_index(&header, H_OAZA, path)?;
    let i_kana = column_index(&header, H_OAZA_KANA, path)?;
    let (i_lg, i_id) = if want_ids {
        (
            column_index(&header, H_LG_CODE, path)?,
            column_index(&header, H_MACHIAZA_ID, path)?,
        )
    } else {
        (0, 0)
    };
    let i_pref = (
        column_index(&header, H_PREF, path)?,
        column_index(&header, H_PREF_KANA, path)?,
    );
    // 郡、市、区の順（表記の列, 読みの列）
    let mut municipality_cols = Vec::new();
    for (surface, kana) in [
        (H_COUNTY, H_COUNTY_KANA),
        (H_CITY, H_CITY_KANA),
        (H_WARD, H_WARD_KANA),
    ] {
        municipality_cols.push((
            column_index(&header, surface, path)?,
            column_index(&header, kana, path)?,
        ));
    }

    let mut pairs: BTreeSet<(String, String)> = BTreeSet::new();
    let mut chains: BTreeSet<(String, String)> = BTreeSet::new();
    let mut fullwidth_chains: BTreeSet<(String, String)> = BTreeSet::new();
    let mut municipality_chains: BTreeSet<(String, String)> = BTreeSet::new();
    let mut skipped: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut rows_read = 0usize;
    let mut fixed = 0usize;
    let mut towns: Vec<TownRow> = Vec::new();
    let mut master_ids: HashSet<(String, String)> = HashSet::new();
    loop {
        lineno += 1;
        if read_line_at(&mut reader, &mut line, path, lineno)? == 0 {
            break;
        }
        if line.is_empty() {
            continue;
        }
        rows_read += 1;
        let cols = split_row(&line, path, lineno)?;
        if cols.len() != header.len() {
            return Err(input_error(
                path,
                lineno,
                format!("expected {} columns, found {}", header.len(), cols.len()),
            ));
        }
        if want_ids {
            master_ids.insert((cols[i_lg].to_string(), cols[i_id].to_string()));
        }
        let mut skip = |reason: &'static str| *skipped.entry(reason).or_insert(0) += 1;
        if cols[i_type] != TYPE_OAZA_CHO && cols[i_type] != TYPE_CHOME {
            skip(TOWN_SKIP_TYPE);
            // 小字などの行も、郵便番号の変換表では大字・町名に畳んで使う（`in_chain: false`。ADR 0032 §3.1）。
            // 連なり（辞書）には入れない
            if want_ids
                && cols[i_status] != STATUS_UNKNOWN
                && let Ok((names, _)) = convert_names(cols[i_oaza], cols[i_kana])
                && let Some(pref) = chain_part(cols[i_pref.0], cols[i_pref.1])
                && let Some(municipality) = municipality_cols
                    .iter()
                    .map(|&(s, k)| chain_part(cols[s], cols[k]))
                    .collect::<Option<Vec<_>>>()
            {
                towns.push(TownRow {
                    lg_code: cols[i_lg].to_string(),
                    machiaza_id: cols[i_id].to_string(),
                    pref,
                    municipality,
                    surface: names.surface,
                    reading: names.readings[0].clone(),
                    alt_readings: names.readings[1..].to_vec(),
                    in_chain: false,
                });
            }
            continue;
        }
        if cols[i_status] == STATUS_UNKNOWN {
            skip(TOWN_SKIP_STATUS);
            continue;
        }
        match convert_names(cols[i_oaza], cols[i_kana]) {
            Ok((names, was_fixed)) => {
                fixed += usize::from(was_fixed);
                let parts = (
                    chain_part(cols[i_pref.0], cols[i_pref.1]),
                    municipality_cols
                        .iter()
                        .map(|&(s, k)| chain_part(cols[s], cols[k]))
                        .collect::<Option<Vec<_>>>(),
                );
                match parts {
                    (Some(pref), Some(municipality)) => {
                        add_town_chains(
                            &mut chains,
                            &mut fullwidth_chains,
                            &pref,
                            &municipality,
                            &names,
                        );
                        add_chains(&mut municipality_chains, &pref, &municipality, "", "");
                        if want_ids {
                            towns.push(TownRow {
                                lg_code: cols[i_lg].to_string(),
                                machiaza_id: cols[i_id].to_string(),
                                pref,
                                municipality,
                                surface: names.surface.clone(),
                                reading: names.readings[0].clone(),
                                alt_readings: names.readings[1..].to_vec(),
                                in_chain: true,
                            });
                        }
                    }
                    _ => skip(TOWN_SKIP_CHAIN_PART),
                }
                // 数字を含む町名は、単語の行にしない（連なりと郵便番号の表だけ）
                if names.has_digits() {
                    skip(TOWN_SKIP_DIGITS_NOT_WORD);
                } else if !pairs.insert((names.readings[0].clone(), names.surface)) {
                    skip(TOWN_SKIP_DUPLICATE);
                }
            }
            Err(reason) => skip(reason),
        }
    }
    Ok(Collected {
        pairs,
        chains,
        fullwidth_chains,
        municipality_chains,
        rows_read,
        skipped,
        fixed,
        towns,
        master_ids,
    })
}

/// Sudachi の lex.csv を読み、(1) 候補と同じ表記・読みの行がある組を `existing` に集め、
/// (2) `名詞,固有名詞,地名,一般` の行で最も多い左右 ID を返す。辞書に入らない行（左 ID が -1）は数えない。
fn scan_sudachi(
    paths: &[PathBuf],
    candidates: &HashSet<Key>,
) -> Result<(HashSet<Key>, Ids), BuildError> {
    let mut existing = HashSet::new();
    let mut id_counts: HashMap<(u16, u16), u64> = HashMap::new();
    let town_pos: Vec<&str> = TOWN_POS.split(',').collect();
    for path in paths {
        let mut reader = open_reader(path)?;
        let mut line = String::new();
        let mut lineno = 0usize;
        loop {
            lineno += 1;
            if read_line_at(&mut reader, &mut line, path, lineno)? == 0 {
                break;
            }
            let cols: Vec<&str> = line.split(CSV_SEPARATOR).collect();
            if cols.len() < SUDACHI_COLUMNS_MIN || cols[SUDACHI_COL_LEFT] == SPLIT_ONLY_ID {
                continue;
            }
            let surface = unescape_unicode(cols[SUDACHI_COL_SURFACE]).into_owned();
            let reading = match_key(&strip_reading_separators(&unescape_unicode(
                cols[SUDACHI_COL_READING],
            )));
            let key = (reading, surface);
            if candidates.contains(&key) {
                existing.insert(key);
            }
            if cols[SUDACHI_COL_POS_FIRST..=SUDACHI_COL_POS_LAST] == town_pos[..] {
                let (Ok(left), Ok(right)) = (
                    cols[SUDACHI_COL_LEFT].parse::<u16>(),
                    cols[SUDACHI_COL_RIGHT].parse::<u16>(),
                ) else {
                    return Err(input_error(path, lineno, "left/right id is not a number"));
                };
                *id_counts.entry((left, right)).or_insert(0) += 1;
            }
        }
    }
    // 最頻の組（同数なら小さい ID の組）
    let best = id_counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(ids, _)| ids)
        .ok_or_else(|| {
            BuildError::Config(format!("no `{TOWN_POS}` rows in the Sudachi lex files"))
        })?;
    Ok((existing, best))
}

fn lex_line(surface: &str, reading: &str, ids: Ids, cost: i16) -> String {
    let (left_id, right_id) = ids;
    format!(
        "{surface},{left_id},{right_id},{cost},{surface},{TOWN_POS},{reading},{surface},*,A,*,*,*,*\n"
    )
}

/// 一時ファイルに書いて rename する
fn write_atomically(out: &Path, text: String) -> std::io::Result<()> {
    let mut tmp = out.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, text).and_then(|()| fs::rename(&tmp, out))
}

/// 町字マスターから `cfg.out` に地名の lex.csv を書く（一時ファイルに書いて rename）。同じ入力からは同じバイト列ができる。
pub fn build_town_lex(cfg: &TownConfig) -> Result<TownReport, BuildError> {
    if cfg.sudachi_lex.is_empty() {
        return Err(BuildError::Config(
            "at least one --lex file is required to skip rows Sudachi already has".into(),
        ));
    }
    let Collected {
        pairs,
        chains,
        fullwidth_chains,
        municipality_chains,
        rows_read,
        mut skipped,
        fixed: readings_fixed,
        towns,
        master_ids,
    } = collect_pairs(&cfg.csv, cfg.postal.is_some())?;
    // 照合用の鍵: 小書きを揃えた読みと表記
    let candidates: HashSet<(String, String)> = pairs
        .iter()
        .map(|(reading, surface)| (match_key(reading), surface.clone()))
        .collect();
    let (existing, (left_id, right_id)) = scan_sudachi(&cfg.sudachi_lex, &candidates)?;

    let mut lines = String::new();
    let mut written = 0usize;
    for (reading, surface) in &pairs {
        if existing.contains(&(match_key(reading), surface.clone())) {
            *skipped.entry(TOWN_SKIP_IN_SUDACHI).or_insert(0) += 1;
            continue;
        }
        lines.push_str(&lex_line(surface, reading, (left_id, right_id), cfg.cost));
        written += 1;
    }
    write_atomically(&cfg.out, lines)?;
    // 連なりは、Sudachi に同じ行があっても書く（Sudachi の Core の行はコストが高いので、辞書のビルドで、同じ行ならこちらを優先する）
    let mut chains_written = 0usize;
    if let Some(chain) = &cfg.chain {
        let ids = (left_id, right_id);
        let mut lines: String = chains
            .iter()
            .map(|(reading, surface)| lex_line(surface, reading, ids, chain.cost))
            .collect();
        lines.extend(fullwidth_chains.iter().map(|(reading, surface)| {
            lex_line(
                surface,
                reading,
                ids,
                chain.cost.saturating_add(FULLWIDTH_SURFACE_COST_DELTA),
            )
        }));
        lines.extend(
            municipality_chains
                .iter()
                .map(|(reading, surface)| lex_line(surface, reading, ids, chain.municipality_cost)),
        );
        chains_written = chains.len() + fullwidth_chains.len() + municipality_chains.len();
        write_atomically(&chain.out, lines)?;
    }
    let postal = match &cfg.postal {
        Some(postal) => Some(build_postal_table(postal, &towns, &master_ids)?),
        None => None,
    };
    Ok(TownReport {
        rows_read,
        skipped,
        readings_fixed,
        written,
        chains_written,
        left_id,
        right_id,
        cost: cfg.cost,
        postal,
    })
}

/// 変換表の 1 つの町（`f` の 1 行と、その `s` の行）
struct PostalGroup {
    /// 並びの鍵（lg_code, machiaza_id）。市区町村だけの行は machiaza_id が空
    order: (String, String),
    /// 連なりの形（先頭が `f`）
    forms: Vec<(Key, bool)>,
    codes: BTreeSet<u32>,
    /// 辞書に連なりがある町か（区分 1・2 の行から作った町）
    in_chain: bool,
}

/// 変換表の CSV を読み、町字マスターの町（`towns`）に結んで `postal.tsv` を書く（ADR 0032 §3）。
///
/// - `dlt_date` がある行は使わない（廃止された町字）。
/// - machiaza_id が空の行（市区町村だけ。例: 札幌市中央区 = 0600000）は、同じ lg_code の町から作る
///   「都道府県 + 市区町村」の連なり（ADR 0030 §13）の郵便番号に使う。
/// - 同じ連なり（丁目だけが違う町字など）は 1 つの町にまとめ、郵便番号を昇順の和集合にする。
/// - `s` の行が別の町の `s` と同じ (読み, 表記) のときは、最初の町の位置に 1 行だけ書き、郵便番号は和集合にする。
fn build_postal_table(
    cfg: &PostalConfig,
    towns: &[TownRow],
    master_ids: &HashSet<(String, String)>,
) -> Result<PostalReport, BuildError> {
    let mut report = PostalReport::default();
    let path = &cfg.csv;
    let mut reader = open_reader(path)?;
    let mut line = String::new();
    let mut lineno = 1usize;
    if read_line_at(&mut reader, &mut line, path, lineno)? == 0 {
        return Err(input_error(path, 1, "empty file"));
    }
    let header_text = line.strip_prefix(BOM).unwrap_or(&line).to_string();
    let header: Vec<&str> = header_text.split(CSV_SEPARATOR).collect();
    let i_lg = column_index(&header, H_LG_CODE, path)?;
    let i_id = column_index(&header, H_MACHIAZA_ID, path)?;
    let i_code = column_index(&header, H_POST_CODE, path)?;
    let i_dlt = column_index(&header, H_DELETE_DATE, path)?;

    let by_id: HashMap<(&str, &str), &TownRow> = towns
        .iter()
        .map(|t| ((t.lg_code.as_str(), t.machiaza_id.as_str()), t))
        .collect();
    // 市区町村の連なりの元（lg_code ごとに最初の町）
    let mut by_lg: BTreeMap<&str, &TownRow> = BTreeMap::new();
    for t in towns {
        by_lg.entry(t.lg_code.as_str()).or_insert(t);
    }

    // 町の連なり（いちばん省略のない形）をキーにまとめる
    let mut groups: HashMap<Key, PostalGroup> = HashMap::new();
    let mut municipality_codes: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    loop {
        lineno += 1;
        if read_line_at(&mut reader, &mut line, path, lineno)? == 0 {
            break;
        }
        if line.is_empty() {
            continue;
        }
        report.rows_read += 1;
        let cols = split_row(&line, path, lineno)?;
        if cols.len() != header.len() {
            return Err(input_error(
                path,
                lineno,
                format!("expected {} columns, found {}", header.len(), cols.len()),
            ));
        }
        if !cols[i_dlt].is_empty() {
            report.deleted += 1;
            continue;
        }
        let Some(code) = parse_code(cols[i_code]) else {
            report.bad_code += 1;
            continue;
        };
        let (lg, id) = (cols[i_lg], cols[i_id]);
        if id.is_empty() {
            if by_lg.contains_key(lg) {
                municipality_codes
                    .entry(lg.to_string())
                    .or_default()
                    .insert(code);
                report.municipality_rows_used += 1;
            } else {
                report.municipality_rows_unused += 1;
            }
            continue;
        }
        let Some(town) = by_id.get(&(lg, id)) else {
            if master_ids.contains(&(lg.to_string(), id.to_string())) {
                report.not_folded += 1;
            } else {
                report.unjoined += 1;
            }
            continue;
        };
        let mut forms = chain_forms(&town.pref, &town.municipality, &town.surface, &town.reading);
        // 数字を含む町名のほかの読みは、読み → 郵便番号にだけ使う `s` の行にする（郵便番号 → 住所は `f` の 1 行だけ）
        for alt in &town.alt_readings {
            forms.extend(
                chain_forms(&town.pref, &town.municipality, &town.surface, alt)
                    .into_iter()
                    .map(|(key, _)| (key, false)),
            );
        }
        let Some(((full_key, _), _)) = forms.split_first() else {
            report.not_folded += 1;
            continue;
        };
        let group = groups
            .entry(full_key.clone())
            .or_insert_with(|| PostalGroup {
                order: (lg.to_string(), id.to_string()),
                forms,
                codes: BTreeSet::new(),
                in_chain: false,
            });
        // 丁目などで同じ連なりになる町字は、ID がいちばん小さいものの位置に置く
        let order = (lg.to_string(), id.to_string());
        if order < group.order {
            group.order = order;
        }
        group.in_chain |= town.in_chain;
        group.codes.insert(code);
    }
    for (lg, codes) in municipality_codes {
        let town = by_lg[lg.as_str()];
        let forms = chain_forms(&town.pref, &town.municipality, "", "");
        let Some(((full_key, _), _)) = forms.split_first() else {
            continue;
        };
        let full_key = full_key.clone();
        groups
            .entry(full_key)
            .or_insert_with(|| PostalGroup {
                order: (lg.clone(), String::new()),
                forms,
                codes: BTreeSet::new(),
                in_chain: true,
            })
            .codes
            .extend(codes);
    }
    let mut groups: Vec<PostalGroup> = groups.into_values().collect();
    groups.sort_by(|a, b| a.order.cmp(&b.order));

    // `s` の行の郵便番号は、同じ (読み, 表記) を持つ町すべての和集合
    let full_keys: HashSet<&Key> = groups.iter().map(|g| &g.forms[0].0).collect();
    let mut short_codes: HashMap<&Key, BTreeSet<u32>> = HashMap::new();
    for g in &groups {
        for (key, _) in g.forms.iter().filter(|(_, is_full)| !is_full) {
            short_codes
                .entry(key)
                .or_default()
                .extend(g.codes.iter().copied());
        }
    }

    let mut text = format!("{POSTAL_TSV_HEADER}\n{POSTAL_TSV_COLUMNS_COMMENT}\n");
    let mut written_short: HashSet<&Key> = HashSet::new();
    let mut code_towns: HashMap<u32, usize> = HashMap::new();
    let mut push = |key: &Key, codes: &BTreeSet<u32>, is_full: bool| -> Result<(), BuildError> {
        let codes: Vec<u32> = codes.iter().copied().collect();
        let reading = normalize_key(&key.0);
        let row = format_line(&reading, &key.1, &codes, is_full).map_err(|e| {
            BuildError::Config(format!("postal row `{}` is not writable: {e}", key.1))
        })?;
        text.push_str(&row);
        text.push('\n');
        Ok(())
    };
    for g in &groups {
        push(&g.forms[0].0, &g.codes, true)?;
        report.towns += 1;
        report.towns_outside_chain += usize::from(!g.in_chain);
        for &code in &g.codes {
            *code_towns.entry(code).or_insert(0) += 1;
        }
        for (key, _) in g.forms.iter().filter(|(_, is_full)| !is_full) {
            if full_keys.contains(key) {
                report.short_dropped_as_full += 1;
                continue;
            }
            if !written_short.insert(key) {
                continue;
            }
            let codes = &short_codes[key];
            if codes.len() > g.codes.len() {
                report.short_merged += 1;
            }
            push(key, codes, false)?;
            report.short_lines += 1;
        }
    }
    report.distinct_codes = code_towns.len();
    report.codes_with_many_towns = code_towns.values().filter(|&&n| n > 1).count();
    write_atomically(&cfg.out, text)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn conv(oaza: &str, kana: &str) -> Result<(String, String), &'static str> {
        convert(oaza, kana).map(|(s, r, _)| (s, r))
    }

    #[test]
    fn plain_names_pass_through() {
        assert_eq!(
            conv("旭ケ丘", "アサヒガオカ"),
            Ok(("旭ケ丘".into(), "アサヒガオカ".into()))
        );
        assert_eq!(
            conv("二十四軒", "ニジュウヨンケン").map(|p| p.0),
            Ok("二十四軒".into())
        );
    }

    #[test]
    fn aza_prefixes_are_stripped_with_their_readings() {
        assert_eq!(
            conv("大字八幡町", "オオアザハチマンチョウ"),
            Ok(("八幡町".into(), "ハチマンチョウ".into()))
        );
        assert_eq!(
            conv("字北浜", "アザキタハマ"),
            Ok(("北浜".into(), "キタハマ".into()))
        );
        assert_eq!(conv("字北浜", "キタハマ"), Err(TOWN_SKIP_PREFIX));
        assert_eq!(
            conv("大字市川町字谷地", "オオアザイチカワマチアザヤチ"),
            Err(TOWN_SKIP_COMPOUND_AZA)
        );
    }

    #[test]
    fn unusable_rows_are_rejected_with_a_reason() {
        assert_eq!(conv("青葉町", ""), Err(TOWN_SKIP_EMPTY_READING));
        assert_eq!(conv("", "アオバ"), Err(TOWN_SKIP_EMPTY_SURFACE));
        // 全角の数字は半角にそろえて通る（ADR 0030 §3.1）。英字と全角の数字の混じり（`Ｂ`）は使わない
        assert_eq!(
            conv("北１条西", "キタ１ジョウニシ"),
            Ok(("北1条西".into(), "キタ1ジョウニシ".into()))
        );
        assert_eq!(
            conv("北１２３Ｂ", "キタ１２３ビー"),
            Err(TOWN_SKIP_SURFACE_CHARS)
        );
        assert_eq!(
            conv("上金剛寺（末野田）", "カミコンゴウジ"),
            Err(TOWN_SKIP_SURFACE_CHARS)
        );
        assert_eq!(
            conv("字𠏹沢", "アザホトケザワ"),
            Err(TOWN_SKIP_SURFACE_CHARS)
        );
        assert_eq!(
            conv("流通センター", "リュウツウセンタ－"),
            Err(TOWN_SKIP_READING_CHARS)
        );
        assert_eq!(conv("乙", "オツ"), Err(TOWN_SKIP_SHORT));
        assert_eq!(conv("あいのさと", "アイノサト"), Err(TOWN_SKIP_NO_HAN));
    }

    fn names(
        oaza: &str,
        kana: &str,
    ) -> Result<(String, Option<String>, Vec<String>), &'static str> {
        convert_names(oaza, kana).map(|(n, _)| (n.surface, n.surface_fullwidth, n.readings))
    }

    #[test]
    fn digit_towns_get_both_readings_and_both_digit_widths() {
        // ABR の表記・読みは全角の数字。読みは数字を読みにした形（先頭）と数字を含む読み（末尾）、表記は半角と全角
        let (surface, fullwidth, readings) = names("北１条西", "キタ１ジョウニシ").unwrap();
        assert_eq!(surface, "北1条西");
        assert_eq!(fullwidth.as_deref(), Some("北１条西"));
        assert_eq!(readings, ["キタイチジョウニシ", "キタ1ジョウニシ"]);
        // 10 以上、複数の数字
        let (_, _, readings) = names("北１０条西", "キタ１０ジョウニシ").unwrap();
        assert_eq!(readings, ["キタジュウジョウニシ", "キタ10ジョウニシ"]);
        // 読みが数詞のもの（ABR の一部）も、数字を含む読みと両方にする
        let (surface, _, readings) = names("新野９線", "ニイノキュウセン").unwrap();
        assert_eq!(surface, "新野9線");
        assert_eq!(
            readings,
            ["ニイノキュウセン", "ニイノクセン", "ニイノ9セン"]
        );
        // 数字のない町名はこれまでと同じ 1 組
        assert_eq!(
            names("千代田", "チヨダ"),
            Ok(("千代田".into(), None, vec!["チヨダ".into()]))
        );
    }

    #[test]
    fn digit_towns_that_cannot_be_expanded_are_rejected() {
        // 助数詞が分からない、読みに数詞が見つからない
        assert_eq!(
            names("字布部１", "アザヌノベイチ"),
            Err(TOWN_SKIP_DIGITS_UNALIGNED)
        );
        // 表記に数字がなく読みにだけある町名はこれまでどおり使わない
        assert_eq!(names("北条", "キタ１ジョウ"), Err(TOWN_SKIP_READING_CHARS));
        // 読みに丁目があって表記にない行は、数字の町でも使わない
        assert_eq!(
            names("春光町１区", "シュンコウチヨウイックイッチョウメ"),
            Err(TOWN_SKIP_CHOME_IN_READING)
        );
    }

    #[test]
    fn cho_readings_get_small_kana_but_other_names_are_kept() {
        assert_eq!(
            convert("大幸町", "ダイコウチヨウ"),
            Ok(("大幸町".into(), "ダイコウチョウ".into(), true))
        );
        assert_eq!(
            convert("千代田", "チヨダ"),
            Ok(("千代田".into(), "チヨダ".into(), false))
        );
    }

    #[test]
    fn readings_that_do_not_cover_the_surface_are_rejected() {
        // 読みにない「町」を足す変換になる
        assert_eq!(
            conv("大幸町", "ダイコウ"),
            Err(TOWN_SKIP_SUFFIX_NOT_IN_READING)
        );
        assert_eq!(
            conv("森末町", "モリスエ"),
            Err(TOWN_SKIP_SUFFIX_NOT_IN_READING)
        );
        assert_eq!(conv("日出村", "ヒデ"), Err(TOWN_SKIP_SUFFIX_NOT_IN_READING));
        assert_eq!(
            conv("日出村", "ヒデムラ").map(|p| p.1),
            Ok("ヒデムラ".into())
        );
        assert_eq!(
            conv("日出村", "ヒデソン").map(|p| p.1),
            Ok("ヒデソン".into())
        );
        assert_eq!(conv("元町", "モトマチ").map(|p| p.1), Ok("モトマチ".into()));
        // 表記にない丁目を読みが持つ行
        assert_eq!(
            conv("荏田東", "エダヒガシイッチョウメ"),
            Err(TOWN_SKIP_CHOME_IN_READING)
        );
        assert_eq!(
            conv("猪熊一丁目", "イノクマイッチョウメ").map(|p| p.0),
            Ok("猪熊一丁目".into())
        );
    }

    #[test]
    fn compat_ideographs_are_unified_but_other_variants_are_not_added() {
        assert_eq!(
            conv("貝\u{fa10}", "カイヅカ"),
            Ok(("貝\u{585a}".into(), "カイヅカ".into()))
        );
        assert_eq!(conv("川\u{fa11}", "カワサキ"), Err(TOWN_SKIP_SURFACE_CHARS));
    }

    #[test]
    fn match_key_folds_case_of_small_kana() {
        assert_eq!(match_key("ダイコウチヨウ"), match_key("ダイコウチョウ"));
        assert_ne!(match_key("チヨダ"), match_key("チダ"));
    }

    #[test]
    fn small_kana_is_fixed_only_in_known_forms() {
        assert_eq!(fix_small_kana("ダイコウチヨウ"), "ダイコウチョウ");
        assert_eq!(fix_small_kana("キヨウト"), "キョウト");
        assert_eq!(fix_small_kana("チユウオウ"), "チュウオウ");
        assert_eq!(fix_small_kana("シヨウナイ"), "ショウナイ");
        assert_eq!(fix_small_kana("ニシヨーカ"), "ニショーカ");
        // 大書きが正しい読みは壊さない
        assert_eq!(fix_small_kana("チヨダ"), "チヨダ");
        assert_eq!(fix_small_kana("キヨミズ"), "キヨミズ");
        assert_eq!(fix_small_kana("ジユウガオカ"), "ジユウガオカ");
        assert_eq!(fix_small_kana("ヨウカイチ"), "ヨウカイチ");
        assert_eq!(fix_small_kana("カヨウ"), "カヨウ");
    }

    #[test]
    fn chains_without_the_county_are_added_too() {
        let p = |s: &str, r: &str| (s.to_string(), r.to_string());
        let mut chains = BTreeSet::new();
        add_chains(
            &mut chains,
            &p("北海道", "ホッカイドウ"),
            &[
                p("空知郡", "ソラチグン"),
                p("南幌町", "ナンポロチョウ"),
                p("", ""),
            ],
            "栄町",
            "サカエマチ",
        );
        let surfaces: Vec<&str> = chains.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(chains.len(), 4);
        for want in [
            "北海道空知郡南幌町栄町",
            "空知郡南幌町栄町",
            "北海道南幌町栄町",
            "南幌町栄町",
        ] {
            assert!(surfaces.contains(&want), "missing {want}");
        }
    }

    #[test]
    fn chain_parts_must_be_complete() {
        assert_eq!(chain_part("", ""), Some((String::new(), String::new())));
        assert_eq!(
            chain_part("千代田区", "チヨダク"),
            Some(("千代田区".into(), "チヨダク".into()))
        );
        assert_eq!(chain_part("千代田区", ""), None);
        assert_eq!(chain_part("", "チヨダク"), None);
        assert_eq!(chain_part("千代田区（旧）", "チヨダク"), None);
    }

    #[test]
    fn builds_a_lex_file_without_rows_sudachi_has() {
        let dir = tempfile::tempdir().expect("tempdir");
        let csv = dir.path().join("mt_town_all.csv");
        let lex = dir.path().join("small_lex.csv");
        let out = dir.path().join("town_lex.csv");
        let header = "lg_code,machiaza_type,oaza_cho,oaza_cho_kana,status_flg,pref,pref_kana,county,county_kana,city,city_kana,ward,ward_kana";
        let mut f = fs::File::create(&csv).expect("create");
        writeln!(f, "\u{feff}{header}").expect("write");
        for row in [
            "1,1,旭ケ丘,アサヒガオカ,1,北海道,ホッカイドウ,,,札幌市,サッポロシ,中央区,チュウオウク",
            "1,2,旭ケ丘,アサヒガオカ,1,北海道,ホッカイドウ,,,札幌市,サッポロシ,中央区,チュウオウク", // 重複
            "1,1,大字八幡町,オオアザハチマンチヨウ,1,東京都,トウキョウト,,,,,千代田区,チヨダク",
            "1,1,梅田,ウメダ,1,大阪府,オオサカフ,,,大阪市,オオサカシ,北区,キタク", // Sudachi にある（連なりは Sudachi にない）
            "1,3,小字だけ,コアザダケ,1,,,,,,,,",
            "1,1,不明,フメイ,3,,,,,,,,",
            // 数字を含む町名（連なりだけ。単語の行にはしない）
            "1,1,北１条西,キタ１ジョウニシ,1,北海道,ホッカイドウ,,,札幌市,サッポロシ,中央区,チュウオウク",
        ] {
            writeln!(f, "{row}").expect("write");
        }
        drop(f);
        let mut g = fs::File::create(&lex).expect("create");
        writeln!(
            g,
            "梅田,4792,4791,9000,梅田,名詞,固有名詞,地名,一般,*,*,ウメダ,梅田,*,A,*,*,*,*"
        )
        .expect("write");
        writeln!(
            g,
            "分割専用,-1,-1,0,分割専用,名詞,固有名詞,地名,一般,*,*,ブンカツ,分割専用,*,A,*,*,*,*"
        )
        .expect("write");
        drop(g);
        let chain_out = dir.path().join("chain_lex.csv");
        let report = build_town_lex(&TownConfig {
            csv,
            sudachi_lex: vec![lex],
            out: out.clone(),
            cost: DEFAULT_TOWN_COST,
            chain: Some(ChainConfig {
                out: chain_out.clone(),
                cost: DEFAULT_CHAIN_COST,
                municipality_cost: DEFAULT_MUNICIPALITY_CHAIN_COST,
            }),
            postal: None,
        })
        .expect("build");
        assert_eq!(report.written, 2);
        // 旭ケ丘 2 + 八幡町 2 + 梅田 2 + 町名のない 3 + 北1条西（読み 2 × 表記 2 × 形 2 = 8）
        assert_eq!(report.chains_written, 17);
        assert_eq!(report.skipped[TOWN_SKIP_DIGITS_NOT_WORD], 1);
        assert_eq!((report.left_id, report.right_id), (4792, 4791));
        assert_eq!(report.skipped[TOWN_SKIP_DUPLICATE], 1);
        assert_eq!(report.skipped[TOWN_SKIP_IN_SUDACHI], 1);
        assert_eq!(report.skipped[TOWN_SKIP_TYPE], 1);
        assert_eq!(report.skipped[TOWN_SKIP_STATUS], 1);
        assert_eq!(report.readings_fixed, 1);
        let text = fs::read_to_string(out).expect("read");
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            [
                "旭ケ丘,4792,4791,11000,旭ケ丘,名詞,固有名詞,地名,一般,*,*,アサヒガオカ,旭ケ丘,*,A,*,*,*,*",
                "八幡町,4792,4791,11000,八幡町,名詞,固有名詞,地名,一般,*,*,ハチマンチョウ,八幡町,*,A,*,*,*,*",
            ]
        );
        // 連なりは別のファイル。都道府県つきと、省いたもの（読みは小書きに直す）。Sudachi にある梅田の連なりも書く
        let chain_text = fs::read_to_string(chain_out).expect("read");
        let lines: Vec<&str> = chain_text.lines().collect();
        assert_eq!(lines.len(), 17);
        for (reading, surface) in [
            (
                "ホッカイドウサッポロシチュウオウクアサヒガオカ",
                "北海道札幌市中央区旭ケ丘",
            ),
            ("サッポロシチュウオウクアサヒガオカ", "札幌市中央区旭ケ丘"),
            ("トウキョウトチヨダクハチマンチョウ", "東京都千代田区八幡町"),
            ("チヨダクハチマンチョウ", "千代田区八幡町"),
            ("オオサカフオオサカシキタクウメダ", "大阪府大阪市北区梅田"),
            ("オオサカシキタクウメダ", "大阪市北区梅田"),
            // 町名の前で止まる「都道府県 + 市区町村」（市区町村だけの連なりは、都道府県を省いた形を作らない）
            ("ホッカイドウサッポロシチュウオウク", "北海道札幌市中央区"),
            ("トウキョウトチヨダク", "東京都千代田区"),
            ("オオサカフオオサカシキタク", "大阪府大阪市北区"),
            // 数字を含む町名: 数字を読みにした形と数字を含む読み × 半角の表記
            (
                "ホッカイドウサッポロシチュウオウクキタイチジョウニシ",
                "北海道札幌市中央区北1条西",
            ),
            (
                "ホッカイドウサッポロシチュウオウクキタ1ジョウニシ",
                "北海道札幌市中央区北1条西",
            ),
            (
                "サッポロシチュウオウクキタイチジョウニシ",
                "札幌市中央区北1条西",
            ),
            (
                "サッポロシチュウオウクキタ1ジョウニシ",
                "札幌市中央区北1条西",
            ),
        ] {
            let row = format!(
                "{surface},4792,4791,{DEFAULT_CHAIN_COST},{surface},名詞,固有名詞,地名,一般,*,*,{reading},{surface},*,A,*,*,*,*"
            );
            assert!(lines.contains(&row.as_str()), "missing {row}");
        }
        // 全角の数字の表記は、コストを `FULLWIDTH_SURFACE_COST_DELTA` だけ高くして、同じ読みで書く（半角が先）
        let fullwidth_cost = DEFAULT_CHAIN_COST + FULLWIDTH_SURFACE_COST_DELTA;
        for reading in [
            "ホッカイドウサッポロシチュウオウクキタイチジョウニシ",
            "ホッカイドウサッポロシチュウオウクキタ1ジョウニシ",
            "サッポロシチュウオウクキタイチジョウニシ",
            "サッポロシチュウオウクキタ1ジョウニシ",
        ] {
            let surface = if reading.starts_with("ホッカイドウ") {
                "北海道札幌市中央区北１条西"
            } else {
                "札幌市中央区北１条西"
            };
            let row = format!(
                "{surface},4792,4791,{fullwidth_cost},{surface},名詞,固有名詞,地名,一般,*,*,{reading},{surface},*,A,*,*,*,*"
            );
            assert!(lines.contains(&row.as_str()), "missing {row}");
        }
    }

    #[test]
    fn builds_a_postal_table_from_the_conversion_table() {
        use ikari_dict_format::postal::{is_skipped_line, parse_line};
        let dir = tempfile::tempdir().expect("tempdir");
        let master = dir.path().join("mt_town_all.csv");
        let lex = dir.path().join("small_lex.csv");
        let header = "lg_code,machiaza_id,machiaza_type,oaza_cho,oaza_cho_kana,status_flg,pref,pref_kana,county,county_kana,city,city_kana,ward,ward_kana";
        let mut rows = vec![header.to_string()];
        rows.extend(
            [
                // 千代田区千代田（1 つの郵便番号）
                "131016,0003000,1,千代田,チヨダ,1,東京都,トウキョウト,,,,,千代田区,チヨダク",
                // 内幸町は丁目で 2 行（同じ町 1 つにまとめる）
                "131016,0001001,2,内幸町,ウチサイワイチョウ,1,東京都,トウキョウト,,,,,千代田区,チヨダク",
                "131016,0001002,2,内幸町,ウチサイワイチョウ,1,東京都,トウキョウト,,,,,千代田区,チヨダク",
                // 小字だけの大字（連なりには入らないが、郵便番号 → 住所のために表に入れる）
                "131016,0009001,3,神田須田,カンダスダ,1,東京都,トウキョウト,,,,,千代田区,チヨダク",
                // 郡つき（郡を省いた形が s に出る）
                "014001,0001000,1,栄町,サカエマチ,1,北海道,ホッカイドウ,空知郡,ソラチグン,南幌町,ナンポロチョウ,,",
                // 表記に数字がある町は、数字を読みにした形と数字を含む読みの両方で入る（ADR 0030 §3.1）
                "131016,0005000,1,北１条,キタ１ジョウ,1,東京都,トウキョウト,,,,,千代田区,チヨダク",
                // 表記に括弧がある町は畳めない
                "131016,0006000,1,北（東）,キタヒガシ,1,東京都,トウキョウト,,,,,千代田区,チヨダク",
            ]
            .map(String::from),
        );
        fs::write(&master, rows.join("\n") + "\n").expect("write");
        fs::write(
            &lex,
            "梅田,4792,4791,9000,梅田,名詞,固有名詞,地名,一般,*,*,ウメダ,梅田,*,A,*,*,*,*\n",
        )
        .expect("write");
        let postal_csv = dir.path().join("abr_post_code.csv");
        let postal_header = "lg_code,machiaza_id,pref,county,city,ward,kyoto_st,oaza_cho,chome,koaza,machiaza_dist,post_code,add_date,dlt_date";
        let mut rows = vec![postal_header.to_string()];
        rows.extend(
            [
                "131016,,東京都,,,千代田区,,,,,,1000000,2026-04-23,", // 市区町村だけ
                "131016,0003000,東京都,,,千代田区,,千代田,,,,1000001,2026-04-23,",
                "131016,0001001,東京都,,,千代田区,,内幸町,一丁目,,,1000011,2026-04-23,",
                "131016,0001002,東京都,,,千代田区,,内幸町,二丁目,,,1000011,2026-04-23,",
                "131016,0009001,東京都,,,千代田区,,神田須田,,,,1010041,2026-04-23,",
                "131016,0003000,東京都,,,千代田区,,千代田,,,,9999999,2020-01-01,2021-01-01", // 廃止
                "131016,0005000,東京都,,,千代田区,,北１条,,,,1000005,2026-04-23,",
                "131016,0006000,東京都,,,千代田区,,北（東）,,,,1000006,2026-04-23,", // 畳めない
                "014001,0001000,北海道,空知郡,南幌町,,,栄町,,,,0691234,2026-04-23,",
                "014001,0001000,北海道,空知郡,南幌町,,,栄町,,,,0691235,2026-04-23,",
                "999999,0000001,不明,,,,,,,,,1234567,2026-04-23,", // マスターにない
                "999999,,不明,,,,,,,,,1234567,2026-04-23,",        // 市区町村だけ（マスターにない）
            ]
            .map(String::from),
        );
        fs::write(&postal_csv, rows.join("\r\n") + "\r\n").expect("write");
        let postal_out = dir.path().join("postal.tsv");
        let report = build_town_lex(&TownConfig {
            csv: master,
            sudachi_lex: vec![lex],
            out: dir.path().join("town_lex.csv"),
            cost: DEFAULT_TOWN_COST,
            chain: None,
            postal: Some(PostalConfig {
                csv: postal_csv,
                out: postal_out.clone(),
            }),
        })
        .expect("build");
        let postal = report.postal.expect("postal report");
        assert_eq!(postal.rows_read, 12);
        assert_eq!(postal.deleted, 1);
        assert_eq!(postal.not_folded, 1);
        assert_eq!(postal.unjoined, 1);
        assert_eq!(postal.municipality_rows_used, 1);
        assert_eq!(postal.municipality_rows_unused, 1);
        // 千代田区（市区町村だけ）、千代田、内幸町、神田須田、栄町、北1条
        assert_eq!(postal.towns, 6);
        assert_eq!(postal.towns_outside_chain, 1);
        // 郵便番号の種類: 1000000、1000001、1000011、1010041、0691234、0691235、1000005
        assert_eq!(postal.distinct_codes, 7);
        assert_eq!(postal.codes_with_many_towns, 0);

        let text = fs::read_to_string(postal_out).expect("read");
        let mut codes = Vec::new();
        let mut got = Vec::new();
        for line in text.lines().filter(|l| !is_skipped_line(l)) {
            let row = parse_line(line, &mut codes).expect(line);
            got.push((
                row.reading.to_string(),
                row.surface.to_string(),
                codes.clone(),
                row.is_full,
            ));
        }
        let want = |reading: &str, surface: &str, codes: &[u32], full: bool| {
            (
                reading.to_string(),
                surface.to_string(),
                codes.to_vec(),
                full,
            )
        };
        // 町は (lg_code, machiaza_id) の順。市区町村だけの行が、同じ lg_code の町より先
        let fulls: Vec<&str> = got.iter().filter(|r| r.3).map(|r| r.1.as_str()).collect();
        assert_eq!(
            fulls,
            [
                "北海道空知郡南幌町栄町",
                "東京都千代田区",
                "東京都千代田区内幸町",
                "東京都千代田区千代田",
                "東京都千代田区北1条",
                "東京都千代田区神田須田",
            ]
        );
        assert!(got.contains(&want(
            "とうきょうとちよだく",
            "東京都千代田区",
            &[1_000_000],
            true
        )));
        let uchi = got
            .iter()
            .find(|r| r.1 == "東京都千代田区内幸町")
            .expect("内幸町");
        assert_eq!(uchi.2, vec![1_000_011]); // 丁目の 2 行は 1 つの町
        assert!(got.contains(&want(
            "とうきょうとちよだくちよだ",
            "東京都千代田区千代田",
            &[1_000_001],
            true
        )));
        // 内幸町の読みは小書きに直る
        assert!(got.contains(&want(
            "ちよだくうちさいわいちょう",
            "千代田区内幸町",
            &[1_000_011],
            false
        )));
        // 郡つきの町: f は郡つき。s は都道府県を省いた形、郡を省いた形、両方を省いた形
        assert!(got.contains(&want(
            "ほっかいどうそらちぐんなんぽろちょうさかえまち",
            "北海道空知郡南幌町栄町",
            &[691_234, 691_235],
            true
        )));
        for surface in ["空知郡南幌町栄町", "北海道南幌町栄町", "南幌町栄町"] {
            assert!(
                got.iter()
                    .any(|r| r.1 == surface && !r.3 && r.2 == vec![691_234, 691_235]),
                "missing {surface}"
            );
        }
        // 小字だけの大字（神田須田）は表に入る。廃止の行と括弧の町は入らない
        assert!(got.iter().any(|r| r.1 == "東京都千代田区神田須田" && r.3));
        assert!(!got.iter().any(|r| r.1.contains("北（東）")));
        // 数字を含む町: f は数字を読みにした形（半角の表記の 1 行）。ほかの読み（数字を含む読み）と、省いた形は s
        assert!(got.contains(&want(
            "とうきょうとちよだくきたいちじょう",
            "東京都千代田区北1条",
            &[1_000_005],
            true
        )));
        for (reading, surface) in [
            ("とうきょうとちよだくきた1じょう", "東京都千代田区北1条"),
            ("ちよだくきたいちじょう", "千代田区北1条"),
            ("ちよだくきた1じょう", "千代田区北1条"),
        ] {
            assert!(
                got.contains(&want(reading, surface, &[1_000_005], false)),
                "missing {reading} {surface}"
            );
        }
        // 全角の数字の表記は、郵便番号の表には入れない（郵便番号 → 住所は半角の 1 行）
        assert!(!got.iter().any(|r| r.1.contains('１')));
        // 同じ (読み, 表記) の行は 1 つだけ
        let mut keys: Vec<(&str, &str)> =
            got.iter().map(|r| (r.0.as_str(), r.1.as_str())).collect();
        let n = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), n);
        // 先頭は f（町ごとに f が先、そのあとに s）
        assert!(got[0].3);
    }

    #[test]
    fn quoted_fields_and_missing_columns_are_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lex = dir.path().join("small_lex.csv");
        fs::write(
            &lex,
            "梅田,4792,4791,9000,梅田,名詞,固有名詞,地名,一般,*,*,ウメダ,梅田,*,A,*,*,*,*\n",
        )
        .expect("write");
        let csv = dir.path().join("a.csv");
        fs::write(&csv, "lg_code,machiaza_type\n1,1\n").expect("write");
        let cfg = |csv: PathBuf| TownConfig {
            csv,
            sudachi_lex: vec![lex.clone()],
            out: dir.path().join("out.csv"),
            cost: DEFAULT_TOWN_COST,
            chain: None,
            postal: None,
        };
        assert!(build_town_lex(&cfg(csv)).is_err());
        let quoted = dir.path().join("b.csv");
        fs::write(
            &quoted,
            "machiaza_type,status_flg,oaza_cho,oaza_cho_kana\n1,1,\"梅,田\",ウメダ\n",
        )
        .expect("write");
        assert!(build_town_lex(&cfg(quoted)).is_err());
    }
}
