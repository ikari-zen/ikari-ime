//! Sudachi の lex.csv の読み込みと行フィルタ（ADR 0001 §3.7）。
//!
//! 列: `0:索引用見出し 1:左ID 2:右ID 3:コスト 4:表記 5-10:品詞1..4,活用型,活用形 11:読み(カタカナ)
//! 12:正規化表記(lemma) 13:辞書形ID ...`（全19列）
//!
//! 辞書形ID（13列目）は、入力した lex.csv を**指定した順に連結した全体での行番号**（0始まり）。
//! SudachiDict では small_lex.csv、core_lex.csv の順に連結する。
//!
//! Full の `notcore_lex.csv` は、その後ろに連結する入力として別に渡す（`LexAccumulator::add_notcore_file`。ADR 0012 §11.3）。
//! notcore の行は辞書の行にはなるが、品詞表、品詞の代表左右ID、`user_cost` の元になるコスト、活用規則には**数えない**
//! （grammar.bin が Core だけのときとバイト単位で一致する。U1）。固有名詞の行には d2s の絞り込みを行う。

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use ikari_dict_format::MAX_READING_CHARS;
use ikari_dict_format::reading::{
    is_indexable, is_indexable_with_digits, normalize_key, strip_reading_separators,
};

use crate::{
    BuildError, SKIP_LEFT_ID_MINUS1, SKIP_NOTCORE_PROPER_HOMOPHONE, SKIP_NOTCORE_PROPER_SHORT,
    SKIP_NOTCORE_UNKNOWN_POS, SKIP_READING_NOT_INDEXABLE, SKIP_READING_TOO_LONG, SKIP_SYMBOL_POS,
    open_reader, read_line_at,
};

/// Sudachi CSV の列数
const COLUMNS: usize = 19;
const COL_LEFT: usize = 1;
const COL_RIGHT: usize = 2;
const COL_COST: usize = 3;
const COL_SURFACE: usize = 4;
const COL_POS_FIRST: usize = 5;
const COL_POS_LAST: usize = 10;
const COL_CONJ_TYPE: usize = 9;
const COL_READING: usize = 11;
const COL_LEMMA: usize = 12;
const COL_DICT_FORM: usize = 13;
/// 品詞2が固有名詞
const POS2_PROPER_NOUN: &str = "固有名詞";
const POS_SEPARATOR: char = ',';
/// d2s: notcore の固有名詞のうち、読みがこの文字数以下のものは入れない（ADR 0012 §3.5）
const NOTCORE_SHORT_READING_CHARS: usize = 2;
/// d2s: 表記が ASCII の印字文字だけ（英字の名前）の固有名詞は、読みがこの文字数以下なら入れない
const NOTCORE_SHORT_ASCII_READING_CHARS: usize = 3;
/// 値がないことを表す CSV の記号（lemma、辞書形ID、活用型など）
const NO_VALUE: &str = "*";

/// 左ID/右ID がこの値の行は分割要素専用（Sudachi も索引に入れていない）
const SPLIT_ONLY_ID: i32 = -1;
/// 品詞1がこれらの行は除外する（§3.7-3）
const EXCLUDED_POS1: [&str; 2] = ["空白", "補助記号"];

const UNICODE_ESCAPE_PREFIX: &str = "\\u";
const HEX_RADIX: u32 = 16;
const UNICODE_ESCAPE_DIGITS: usize = 4;

/// フィルタを通った1行。品詞は読み込み順の仮 ID（`LexAccumulator::finish` で品詞名順の ID に直す）。
#[derive(Debug)]
pub struct RawRow {
    pub reading: String,
    pub surface: String,
    pub left_id: u16,
    pub right_id: u16,
    pub cost: i16,
    pub pos_tmp: u32,
    /// 正規化表記（`*` や空なら表記）
    pub lemma: String,
    /// 辞書形の行番号（入力全体での行番号。`InflectRow::id`）。活用しない語や、`*` のときは `None`
    pub dict_form: Option<u32>,
    /// Full の notcore の行か（品詞の統計と活用規則に数えない。`LexAccumulator::add_notcore_file`）
    pub notcore: bool,
    /// 重複除去で Core の行にも勝つ行か（住所の連なり。`LexAccumulator::add_priority_notcore_file`）。notcore の一種
    pub priority: bool,
    /// 優先の行が置き換えた Core の行のコスト。品詞の統計（`user_cost`、代表の左右ID）は、置き換えられた Core の行を数える
    /// （Core だけのときと同じ grammar.bin にするため。ADR 0012 §13 の U1）
    pub replaced_core_cost: Option<i16>,
}

/// 活用する行（活用型が `*` でない行）。活用規則の導出に使う。除外した行も含む。
#[derive(Debug, Clone)]
pub struct InflectRow {
    /// 入力全体での行番号（0始まり）
    pub id: u32,
    pub surface: String,
    /// 正規化した読み（索引に入れられない読みも、そのまま）
    pub reading: String,
    pub pos_tmp: u32,
    /// 辞書形の行番号
    pub dict_form: Option<u32>,
    /// notcore の行か（活用規則の導出には使わない。異字同訓の表の辞書形の参照には使う）
    pub notcore: bool,
}

/// 品詞表の1件（品詞名の昇順で並べたもの）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PosEntry {
    pub name: String,
    pub rep_left_id: u16,
    pub rep_right_id: u16,
}

/// `LexAccumulator::finish` の結果。
#[derive(Debug)]
pub struct Lexicon {
    pub rows_read: usize,
    pub skipped: BTreeMap<&'static str, usize>,
    /// 品詞名の昇順。添字が pos_id
    pub pos: Vec<PosEntry>,
    /// 重複除去・pos_id 付け替え済みの行
    pub rows: Vec<(RawRow, u16)>,
    /// 活用する行（`id` の昇順）
    pub inflect: Vec<InflectRow>,
    /// 読み込み順の仮の品詞 ID -> 品詞表の pos_id
    pub pos_final: Vec<u16>,
}

/// 複数の lex.csv を読み、フィルタを通した行と品詞の統計を溜める。
pub struct LexAccumulator {
    prev_right_size: u16,
    next_left_size: u16,
    rows_read: usize,
    skipped: BTreeMap<&'static str, usize>,
    pos_index: HashMap<String, u32>,
    pos_names: Vec<String>,
    /// 品詞ごとの (左ID, 右ID) の出現回数
    pos_pairs: Vec<HashMap<(u16, u16), u32>>,
    rows: Vec<RawRow>,
    inflect: Vec<InflectRow>,
    /// いま読んでいるファイルが notcore か
    adding_notcore: bool,
    /// いま読んでいるファイルの行が、重複除去で Core にも勝つか（notcore でもある）
    adding_priority: bool,
}

fn input_error(path: &Path, line: usize, message: impl Into<String>) -> BuildError {
    BuildError::Input {
        file: path.to_path_buf(),
        line,
        message: message.into(),
    }
}

impl LexAccumulator {
    pub fn new(prev_right_size: u16, next_left_size: u16) -> Self {
        Self {
            prev_right_size,
            next_left_size,
            rows_read: 0,
            skipped: BTreeMap::new(),
            pos_index: HashMap::new(),
            pos_names: Vec::new(),
            pos_pairs: Vec::new(),
            rows: Vec::new(),
            inflect: Vec::new(),
            adding_notcore: false,
            adding_priority: false,
        }
    }

    fn skip(&mut self, reason: &'static str) {
        *self.skipped.entry(reason).or_insert(0) += 1;
    }

    /// 登録済みの品詞の仮 ID。なければ `None`（登録はしない）。
    fn find_pos(&self, cols: &[&str]) -> Option<u32> {
        self.pos_index
            .get(&cols[COL_POS_FIRST..=COL_POS_LAST].join(","))
            .copied()
    }

    fn intern_pos(&mut self, cols: &[&str]) -> u32 {
        let name = cols[COL_POS_FIRST..=COL_POS_LAST].join(",");
        if let Some(&id) = self.pos_index.get(&name) {
            return id;
        }
        let id = self.pos_names.len() as u32;
        self.pos_index.insert(name.clone(), id);
        self.pos_names.push(name);
        self.pos_pairs.push(HashMap::new());
        id
    }

    /// Core（small、core）の lex.csv を読む。
    pub fn add_file(&mut self, path: &Path) -> Result<(), BuildError> {
        self.adding_notcore = false;
        self.add_file_rows(path)
    }

    /// Full の notcore_lex.csv を読む。Core のファイルをすべて読んだあとに呼ぶこと
    /// （品詞表は Core の行だけで作るので、Core にない品詞の行は捨てる）。
    pub fn add_notcore_file(&mut self, path: &Path) -> Result<(), BuildError> {
        self.adding_notcore = true;
        let result = self.add_file_rows(path);
        self.adding_notcore = false;
        result
    }

    /// notcore と同じに読むが、**同じ（読み、表記、左右 ID、品詞）の Core の行より優先する**行のファイル。
    /// 住所の連なり（ADR 0030 §12）用。Sudachi は住所を Core に入れているが、コストが高く（中央値 2 万）、1 位に出ないため、
    /// 低いコストの同じ行で置き換える。Core の統計と活用規則には数えない。notcore の後に呼ぶこと。
    pub fn add_priority_notcore_file(&mut self, path: &Path) -> Result<(), BuildError> {
        self.adding_notcore = true;
        self.adding_priority = true;
        let result = self.add_file_rows(path);
        self.adding_notcore = false;
        self.adding_priority = false;
        result
    }

    fn add_file_rows(&mut self, path: &Path) -> Result<(), BuildError> {
        let mut reader = open_reader(path)?;
        let mut line = String::new();
        let mut lineno = 0usize;
        loop {
            lineno += 1;
            if read_line_at(&mut reader, &mut line, path, lineno)? == 0 {
                return Ok(());
            }
            self.add_line(&line, path, lineno)?;
        }
    }

    /// 1行を処理する（`line` の末尾の改行は取り除いてあること）。
    pub fn add_line(&mut self, line: &str, path: &Path, lineno: usize) -> Result<(), BuildError> {
        // 辞書形ID が指す行番号。除外する行も数える。
        let row_id = u32::try_from(self.rows_read)
            .map_err(|_| input_error(path, lineno, "too many rows (more than u32::MAX)"))?;
        self.rows_read += 1;

        // 規則1: 列数と数値列
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() != COLUMNS {
            return Err(input_error(
                path,
                lineno,
                format!("expected {COLUMNS} columns, found {}", cols.len()),
            ));
        }
        let parse_num = |col: usize, what: &str| -> Result<i32, BuildError> {
            cols[col].parse::<i32>().map_err(|_| {
                input_error(
                    path,
                    lineno,
                    format!("{what} `{}` is not a number", cols[col]),
                )
            })
        };
        let left = parse_num(COL_LEFT, "left id")?;
        let right = parse_num(COL_RIGHT, "right id")?;
        let cost = i16::try_from(parse_num(COL_COST, "cost")?).map_err(|_| {
            input_error(
                path,
                lineno,
                format!("cost `{}` does not fit in i16", cols[COL_COST]),
            )
        })?;

        // 品詞の集合は、除外する行も含む Core の全入力行から作る。notcore の行は品詞表を増やさない
        let pos_tmp = if self.adding_notcore {
            let Some(id) = self.find_pos(&cols) else {
                self.skip(SKIP_NOTCORE_UNKNOWN_POS);
                return Ok(());
            };
            id
        } else {
            self.intern_pos(&cols)
        };
        let in_range = |id: i32, size: u16| (0..i32::from(size)).contains(&id);
        if !self.adding_notcore
            && in_range(left, self.next_left_size)
            && in_range(right, self.prev_right_size)
        {
            *self.pos_pairs[pos_tmp as usize]
                .entry((left as u16, right as u16))
                .or_insert(0) += 1;
        }

        // 活用規則の導出用: 活用する行は、除外する行も含めて覚える（辞書形に使われうる）
        if cols[COL_CONJ_TYPE] != NO_VALUE {
            self.inflect.push(InflectRow {
                id: row_id,
                surface: unescape_unicode(cols[COL_SURFACE]).into_owned(),
                reading: normalize_key(&strip_reading_separators(&unescape_unicode(
                    cols[COL_READING],
                ))),
                pos_tmp,
                dict_form: cols[COL_DICT_FORM].parse().ok(),
                notcore: self.adding_notcore,
            });
        }

        // 規則2: 分割要素専用の行
        if left == SPLIT_ONLY_ID {
            self.skip(SKIP_LEFT_ID_MINUS1);
            return Ok(());
        }
        // 規則3: 空白と補助記号
        if EXCLUDED_POS1.contains(&cols[COL_POS_FIRST]) {
            self.skip(SKIP_SYMBOL_POS);
            return Ok(());
        }
        // 規則4: \uXXXX を戻し、読みを正規化する
        let surface = unescape_unicode(cols[COL_SURFACE]);
        let reading = normalize_key(&strip_reading_separators(&unescape_unicode(
            cols[COL_READING],
        )));
        // 規則5: 索引に入れられない読み
        // 住所の連なり（`adding_priority`）だけは、数字を含む町名の読み（`きた1じょうにし`）を入れる
        let indexable = if self.adding_priority {
            is_indexable_with_digits(&reading)
        } else {
            is_indexable(&reading)
        };
        if !indexable {
            let too_long = reading.chars().count() > MAX_READING_CHARS;
            self.skip(if too_long {
                SKIP_READING_TOO_LONG
            } else {
                SKIP_READING_NOT_INDEXABLE
            });
            return Ok(());
        }
        // 規則6: 左右 ID が grammar の範囲外ならエラー
        if !in_range(left, self.next_left_size) {
            return Err(input_error(
                path,
                lineno,
                format!(
                    "left id {left} is out of range (matrix has {})",
                    self.next_left_size
                ),
            ));
        }
        if !in_range(right, self.prev_right_size) {
            return Err(input_error(
                path,
                lineno,
                format!(
                    "right id {right} is out of range (matrix has {})",
                    self.prev_right_size
                ),
            ));
        }
        let lemma = match cols[COL_LEMMA] {
            "" | NO_VALUE => surface.to_string(),
            lemma => unescape_unicode(lemma).into_owned(),
        };
        self.rows.push(RawRow {
            reading,
            surface: surface.into_owned(),
            left_id: left as u16,
            right_id: right as u16,
            cost,
            pos_tmp,
            lemma,
            dict_form: cols[COL_DICT_FORM].parse().ok(),
            notcore: self.adding_notcore,
            priority: self.adding_priority,
            replaced_core_cost: None,
        });
        Ok(())
    }

    /// notcore の固有名詞の行のうち、d2s で入れない行を取り除く（ADR 0012 §3.5。行は消すだけで、行番号は変えない）。
    /// - 同じ読みに、固有名詞以外の行（活用形を含む。Core でも notcore でも）がある
    /// - 読みが短い（`NOTCORE_SHORT_READING_CHARS` 文字以下。表記が ASCII の印字文字だけなら `NOTCORE_SHORT_ASCII_READING_CHARS` 文字以下）
    ///
    /// 前者を先に数え、後者は前者に当たらなかった行だけを数える。
    fn drop_notcore_proper_nouns(&mut self) {
        if !self.rows.iter().any(|r| r.notcore) {
            return;
        }
        let is_proper: Vec<bool> = self
            .pos_names
            .iter()
            .map(|name| name.split(POS_SEPARATOR).nth(1) == Some(POS2_PROPER_NOUN))
            .collect();
        let nonproper_readings: HashSet<&str> = self
            .rows
            .iter()
            .filter(|r| !is_proper[r.pos_tmp as usize])
            .map(|r| r.reading.as_str())
            .collect();
        let verdicts: Vec<Option<&'static str>> = self
            .rows
            .iter()
            .map(|r| {
                if !r.notcore || !is_proper[r.pos_tmp as usize] {
                    None
                } else if nonproper_readings.contains(r.reading.as_str()) {
                    Some(SKIP_NOTCORE_PROPER_HOMOPHONE)
                } else if is_short_proper_noun(&r.surface, &r.reading) {
                    Some(SKIP_NOTCORE_PROPER_SHORT)
                } else {
                    None
                }
            })
            .collect();
        drop(nonproper_readings);
        let mut verdicts = verdicts.into_iter();
        let mut dropped: BTreeMap<&'static str, usize> = BTreeMap::new();
        self.rows.retain(|_| match verdicts.next().flatten() {
            Some(reason) => {
                *dropped.entry(reason).or_insert(0) += 1;
                false
            }
            None => true,
        });
        for (reason, count) in dropped {
            *self.skipped.entry(reason).or_insert(0) += count;
        }
    }

    /// 規則7（重複除去）を行い、品詞表を品詞名の昇順に確定する。
    pub fn finish(mut self) -> Lexicon {
        self.drop_notcore_proper_nouns();
        // 同じ (読み, 表記, left, right, pos) の中で、優先の行（住所の連なり）があればそれの最小コストの行、なければ
        // 最小コストの行だけを残す。コストが同じなら Core の行を残す（notcore は後ろ）。
        self.rows.sort_by(|a, b| {
            (
                &a.reading,
                &a.surface,
                a.left_id,
                a.right_id,
                a.pos_tmp,
                !a.priority,
                a.notcore,
                a.cost,
            )
                .cmp(&(
                    &b.reading,
                    &b.surface,
                    b.left_id,
                    b.right_id,
                    b.pos_tmp,
                    !b.priority,
                    b.notcore,
                    b.cost,
                ))
        });
        let before = self.rows.len();
        self.rows.dedup_by(|b, a| {
            let same = a.reading == b.reading
                && a.surface == b.surface
                && a.left_id == b.left_id
                && a.right_id == b.right_id
                && a.pos_tmp == b.pos_tmp;
            // 優先の行が Core の行を置き換えるとき、最初の（最安の）Core の行のコストを覚える
            if same && a.priority && !b.notcore && a.replaced_core_cost.is_none() {
                a.replaced_core_cost = Some(b.cost);
            }
            same
        });
        let removed = before - self.rows.len();
        if removed > 0 {
            *self.skipped.entry(crate::SKIP_DUPLICATE).or_insert(0) += removed;
        }

        // 品詞名の昇順で pos_id を振る
        let mut order: Vec<u32> = (0..self.pos_names.len() as u32).collect();
        order.sort_by(|&a, &b| self.pos_names[a as usize].cmp(&self.pos_names[b as usize]));
        let mut final_id = vec![0u16; order.len()];
        let mut pos = Vec::with_capacity(order.len());
        for (new_id, &tmp) in order.iter().enumerate() {
            final_id[tmp as usize] = new_id as u16; // 品詞数が 65536 を超えると encode_grammar が拒否する
            let (rep_left_id, rep_right_id) = most_frequent_pair(&self.pos_pairs[tmp as usize]);
            pos.push(PosEntry {
                name: std::mem::take(&mut self.pos_names[tmp as usize]),
                rep_left_id,
                rep_right_id,
            });
        }
        let rows = self
            .rows
            .into_iter()
            .map(|r| {
                let id = final_id[r.pos_tmp as usize];
                (r, id)
            })
            .collect();
        Lexicon {
            rows_read: self.rows_read,
            skipped: self.skipped,
            pos,
            rows,
            inflect: self.inflect,
            pos_final: final_id,
        }
    }
}

/// d2s の「読みの短い固有名詞」か。
fn is_short_proper_noun(surface: &str, reading: &str) -> bool {
    let reading_chars = reading.chars().count();
    reading_chars <= NOTCORE_SHORT_READING_CHARS
        || (reading_chars <= NOTCORE_SHORT_ASCII_READING_CHARS
            && !surface.is_empty()
            && surface.chars().all(|c| c.is_ascii_graphic() || c == ' '))
}

/// 最も多い (左, 右)。同数なら小さい方。1件もなければ (0, 0)（BOS/EOS の ID。そのような品詞は
/// 分割要素専用の行にしか現れず、辞書には入らない）。
fn most_frequent_pair(pairs: &HashMap<(u16, u16), u32>) -> (u16, u16) {
    pairs
        .iter()
        .max_by(|(ka, ca), (kb, cb)| ca.cmp(cb).then_with(|| kb.cmp(ka)))
        .map_or((0, 0), |(&k, _)| k)
}

/// `\uXXXX`（4桁の16進。サロゲートペアは2つ続けて1文字）を元の文字に戻す。
/// 形式が合わないものと、対のないサロゲートは、そのまま残す。
pub fn unescape_unicode(s: &str) -> Cow<'_, str> {
    if !s.contains(UNICODE_ESCAPE_PREFIX) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find(UNICODE_ESCAPE_PREFIX) {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        match decode_escape(after) {
            Some((c, used)) => {
                out.push(c);
                rest = &after[used..];
            }
            None => {
                out.push_str(UNICODE_ESCAPE_PREFIX);
                rest = &after[UNICODE_ESCAPE_PREFIX.len()..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// 先頭の `\uXXXX`（と、続く下位サロゲート）を読む。(文字, 消費バイト数)。
fn decode_escape(s: &str) -> Option<(char, usize)> {
    let hex4 = |s: &str| -> Option<u32> {
        let digits = s
            .strip_prefix(UNICODE_ESCAPE_PREFIX)?
            .get(..UNICODE_ESCAPE_DIGITS)?;
        if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(digits, HEX_RADIX).ok()
    };
    let escape_len = UNICODE_ESCAPE_PREFIX.len() + UNICODE_ESCAPE_DIGITS;
    let first = hex4(s)?;
    match first {
        0xD800..=0xDBFF => {
            let second = hex4(s.get(escape_len..)?)?;
            if !(0xDC00..=0xDFFF).contains(&second) {
                return None;
            }
            let code = 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00);
            Some((char::from_u32(code)?, escape_len * 2))
        }
        0xDC00..=0xDFFF => None,
        _ => Some((char::from_u32(first)?, escape_len)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn unescape_basic() {
        assert_eq!(unescape_unicode("abc"), "abc");
        assert_eq!(unescape_unicode("1\\u002c2"), "1,2");
        assert_eq!(unescape_unicode("1\\u002C2"), "1,2");
        assert_eq!(unescape_unicode("\\u0028x\\u0029"), "(x)");
        assert_eq!(unescape_unicode("\\u3042\\u3044"), "あい");
    }

    #[test]
    fn unescape_leaves_malformed_alone() {
        assert_eq!(unescape_unicode("a\\u12"), "a\\u12");
        assert_eq!(unescape_unicode("a\\uZZZZ"), "a\\uZZZZ");
        assert_eq!(unescape_unicode("a\\u+123"), "a\\u+123");
        assert_eq!(unescape_unicode("\\u"), "\\u");
        assert_eq!(unescape_unicode("\\\\u0041"), "\\A"); // バックスラッシュ自体は触らない
        assert_eq!(unescape_unicode("\\u00あ1"), "\\u00あ1"); // 多バイト文字をまたぐスライスでもパニックしない
        assert_eq!(unescape_unicode("\\,"), "\\,");
    }

    #[test]
    fn unescape_surrogates() {
        assert_eq!(unescape_unicode("\\ud83d\\ude00"), "\u{1F600}");
        assert_eq!(unescape_unicode("\\ud83dx"), "\\ud83dx"); // 対がない
        assert_eq!(unescape_unicode("\\ude00"), "\\ude00");
        assert_eq!(unescape_unicode("\\ud83d\\u0041"), "\\ud83dA");
    }

    fn acc() -> LexAccumulator {
        LexAccumulator::new(10, 10)
    }

    fn add(acc: &mut LexAccumulator, line: &str) -> Result<(), BuildError> {
        acc.add_line(line, &PathBuf::from("test.csv"), 7)
    }

    /// 19列の行を作る。
    fn row(surface: &str, left: i32, right: i32, cost: i32, pos: &str, reading: &str) -> String {
        format!("{surface},{left},{right},{cost},{surface},{pos},*,{reading},{surface},*,A,*,*,*,*")
    }

    #[test]
    fn rejects_wrong_column_count_and_non_numbers() {
        let mut a = acc();
        assert!(add(&mut a, "a,1,2,3").is_err());
        let mut a = acc();
        let too_many = format!("{},x", row("a", 1, 1, 1, "名詞,普通名詞,一般,*,*", "ア"));
        assert!(add(&mut a, &too_many).is_err());
        let mut a = acc();
        assert!(
            add(
                &mut a,
                &row("a", 1, 1, 1, "名詞,普通名詞,一般,*,*", "ア").replace(",1,1,1,", ",x,1,1,")
            )
            .is_err()
        );
        let mut a = acc();
        let err = add(
            &mut a,
            &row("a", 1, 1, 40000, "名詞,普通名詞,一般,*,*", "ア"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("test.csv:7"), "{err}");
    }

    #[test]
    fn left_id_minus_one_is_skipped_but_pos_is_kept() {
        let mut a = acc();
        add(
            &mut a,
            &row("x", -1, -1, 0, "名詞,固有名詞,人名,*,*", "エックス"),
        )
        .unwrap();
        let lex = a.finish();
        assert_eq!(lex.skipped.get(SKIP_LEFT_ID_MINUS1), Some(&1));
        assert!(lex.rows.is_empty());
        assert_eq!(lex.pos.len(), 1);
        assert_eq!(lex.pos[0].name, "名詞,固有名詞,人名,*,*,*");
    }

    #[test]
    fn symbol_and_whitespace_pos_are_skipped_but_symbol_kept() {
        let mut a = acc();
        add(
            &mut a,
            &row("、", 1, 1, -2049, "補助記号,読点,*,*,*", "テン"),
        )
        .unwrap();
        add(&mut a, &row(" ", 2, 2, 10, "空白,*,*,*,*", "キゴウ")).unwrap();
        add(&mut a, &row("☆", 3, 3, 10, "記号,一般,*,*,*", "ホシ")).unwrap();
        let lex = a.finish();
        assert_eq!(lex.skipped.get(SKIP_SYMBOL_POS), Some(&2));
        assert_eq!(lex.rows.len(), 1);
        assert_eq!(lex.rows[0].0.surface, "☆");
        // 除外した行の品詞も品詞表に入る
        assert_eq!(lex.pos.len(), 3);
    }

    #[test]
    fn reading_is_unescaped_stripped_and_normalized() {
        let mut a = acc();
        add(
            &mut a,
            &row(
                "北斗",
                1,
                1,
                5,
                "名詞,普通名詞,一般,*,*",
                "ホ\\u30fbク・ト＝ ゠\u{3000}ト",
            ),
        )
        .unwrap();
        add(
            &mut a,
            &row("ヴ", 1, 1, 5, "名詞,普通名詞,一般,*,*", "ヴァー"),
        )
        .unwrap();
        let lex = a.finish();
        let readings: Vec<&str> = lex.rows.iter().map(|(r, _)| r.reading.as_str()).collect();
        assert_eq!(readings, vec!["ほくとと", "ゔぁー"]);
    }

    #[test]
    fn surface_is_unescaped() {
        let mut a = acc();
        add(
            &mut a,
            &row(
                "1\\u002c2塁",
                1,
                1,
                5,
                "名詞,普通名詞,一般,*,*",
                "イチニルイ",
            ),
        )
        .unwrap();
        let lex = a.finish();
        assert_eq!(lex.rows[0].0.surface, "1,2塁");
    }

    #[test]
    fn unindexable_readings_are_skipped() {
        let mut a = acc();
        add(
            &mut a,
            &row("profile", 1, 1, 5, "名詞,普通名詞,一般,*,*", "profile"),
        )
        .unwrap();
        add(
            &mut a,
            &row("ＡＢ", 1, 1, 5, "名詞,普通名詞,一般,*,*", "エービー1"),
        )
        .unwrap();
        let long = "ア".repeat(MAX_READING_CHARS + 1);
        add(
            &mut a,
            &row("long", 1, 1, 5, "名詞,普通名詞,一般,*,*", &long),
        )
        .unwrap();
        let max = "ア".repeat(MAX_READING_CHARS);
        add(&mut a, &row("max", 1, 1, 5, "名詞,普通名詞,一般,*,*", &max)).unwrap();
        add(&mut a, &row("empty", 1, 1, 5, "名詞,普通名詞,一般,*,*", "")).unwrap();
        add(&mut a, &row("sep", 1, 1, 5, "名詞,普通名詞,一般,*,*", "・")).unwrap(); // 区切りだけ -> 空
        let lex = a.finish();
        assert_eq!(lex.skipped.get(SKIP_READING_NOT_INDEXABLE), Some(&4));
        assert_eq!(lex.skipped.get(SKIP_READING_TOO_LONG), Some(&1));
        assert_eq!(lex.rows.len(), 1);
        assert_eq!(lex.rows[0].0.surface, "max");
    }

    #[test]
    fn ids_out_of_range_are_errors_only_for_kept_rows() {
        let mut a = acc();
        let err = add(&mut a, &row("a", 10, 1, 5, "名詞,普通名詞,一般,*,*", "ア")).unwrap_err();
        assert!(err.to_string().contains("left id 10"), "{err}");
        let mut a = acc();
        let err = add(&mut a, &row("a", 1, 10, 5, "名詞,普通名詞,一般,*,*", "ア")).unwrap_err();
        assert!(err.to_string().contains("right id 10"), "{err}");
        // 除外される行なら範囲外でもエラーにしない
        let mut a = acc();
        add(
            &mut a,
            &row("a", 99, 99, 5, "名詞,普通名詞,一般,*,*", "abc"),
        )
        .unwrap();
        add(&mut a, &row("b", 99, 99, 5, "補助記号,一般,*,*,*", "ビー")).unwrap();
        let lex = a.finish();
        assert!(lex.rows.is_empty());
        // 範囲外の ID は代表 ID の統計にも入らない
        assert_eq!((lex.pos[0].rep_left_id, lex.pos[0].rep_right_id), (0, 0));
    }

    #[test]
    fn duplicates_keep_minimum_cost() {
        let mut a = acc();
        let pos = "名詞,普通名詞,一般,*,*";
        add(&mut a, &row("品", 1, 1, 300, pos, "ヒン")).unwrap();
        add(&mut a, &row("品", 1, 1, 100, pos, "ヒン")).unwrap();
        add(&mut a, &row("品", 1, 1, 200, pos, "ヒン")).unwrap();
        add(&mut a, &row("品", 2, 1, 400, pos, "ヒン")).unwrap(); // left が違えば別
        add(
            &mut a,
            &row("品", 1, 1, 50, "名詞,普通名詞,サ変可能,*,*", "ヒン"),
        )
        .unwrap(); // 品詞が違えば別
        add(&mut a, &row("品", 1, 1, 70, pos, "ヒ")).unwrap(); // 読みが違えば別
        let lex = a.finish();
        assert_eq!(lex.skipped.get(crate::SKIP_DUPLICATE), Some(&2));
        assert_eq!(lex.rows.len(), 4);
        let costs: Vec<(String, u16, i16)> = lex
            .rows
            .iter()
            .map(|(r, p)| (r.reading.clone(), *p, r.cost))
            .collect();
        assert!(costs.contains(&("ひん".to_string(), 1, 100)));
        assert!(!costs.iter().any(|c| c.2 == 300 || c.2 == 200));
    }

    fn add_notcore(acc: &mut LexAccumulator, line: &str) -> Result<(), BuildError> {
        acc.adding_notcore = true;
        let result = add(acc, line);
        acc.adding_notcore = false;
        result
    }

    #[test]
    fn notcore_rows_do_not_touch_the_pos_table_or_statistics() {
        let noun = "名詞,普通名詞,一般,*,*";
        let mut a = acc();
        add(&mut a, &row("a", 1, 2, 1, noun, "ア")).unwrap();
        // notcore の安い行や別の左右ID は代表IDに影響しない。Core にない品詞の行は捨てる
        add_notcore(&mut a, &row("b", 3, 4, 1, noun, "イ")).unwrap();
        add_notcore(&mut a, &row("c", 3, 4, 1, noun, "ウ")).unwrap();
        add_notcore(&mut a, &row("d", 1, 1, 1, "感動詞,一般,*,*,*", "エ")).unwrap();
        let lex = a.finish();
        assert_eq!(lex.pos.len(), 1);
        assert_eq!((lex.pos[0].rep_left_id, lex.pos[0].rep_right_id), (1, 2));
        assert_eq!(lex.skipped.get(SKIP_NOTCORE_UNKNOWN_POS), Some(&1));
        assert_eq!(lex.rows.len(), 3);
        assert_eq!(lex.rows.iter().filter(|(r, _)| r.notcore).count(), 2);
        assert_eq!(lex.rows_read, 4);
    }

    #[test]
    fn only_chain_rows_may_have_digits_in_the_reading() {
        let noun = "名詞,普通名詞,一般,*,*";
        let mut a = acc();
        add(&mut a, &row("a", 1, 1, 1, noun, "ア")).unwrap();
        // notcore（Sudachi の行、ウェブ語彙）の数字を含む読みは、これまでどおり入れない
        add_notcore(&mut a, &row("北1条", 1, 1, 1, noun, "キタ1ジョウ")).unwrap();
        a.adding_notcore = true;
        a.adding_priority = true;
        add(&mut a, &row("北1条", 1, 1, 1, noun, "キタ1ジョウ")).unwrap();
        add(&mut a, &row("北１条", 1, 1, 1, noun, "キタ１ジョウ")).unwrap(); // 全角の数字は連なりでも入れない
        a.adding_notcore = false;
        a.adding_priority = false;
        let lex = a.finish();
        assert_eq!(lex.skipped.get(SKIP_READING_NOT_INDEXABLE), Some(&2));
        let kept: Vec<&str> = lex.rows.iter().map(|(r, _)| r.reading.as_str()).collect();
        assert_eq!(kept, ["あ", "きた1じょう"]);
    }

    #[test]
    fn notcore_duplicates_lose_to_core_even_when_cheaper() {
        let noun = "名詞,普通名詞,一般,*,*";
        let mut a = acc();
        add(&mut a, &row("品", 1, 1, 500, noun, "ヒン")).unwrap();
        add_notcore(&mut a, &row("品", 1, 1, 100, noun, "ヒン")).unwrap();
        let lex = a.finish();
        assert_eq!(lex.rows.len(), 1);
        assert_eq!((lex.rows[0].0.cost, lex.rows[0].0.notcore), (500, false));
    }

    #[test]
    fn priority_rows_beat_core_duplicates_only_when_the_ids_and_pos_match() {
        let noun = "名詞,普通名詞,一般,*,*";
        let mut a = acc();
        add(&mut a, &row("品", 1, 1, 500, noun, "ヒン")).unwrap();
        add(&mut a, &row("品", 2, 2, 700, noun, "ヒン")).unwrap();
        a.adding_notcore = true;
        a.adding_priority = true;
        add(&mut a, &row("品", 1, 1, 900, noun, "ヒン")).unwrap(); // Core より高くても、同じ行は置き換える
        add(&mut a, &row("品", 1, 2, 100, noun, "ヒン")).unwrap(); // 別の ID は別の行
        a.adding_notcore = false;
        a.adding_priority = false;
        let lex = a.finish();
        let mut rows: Vec<_> = lex
            .rows
            .iter()
            .map(|(r, _)| {
                (
                    r.left_id,
                    r.right_id,
                    r.cost,
                    r.priority,
                    r.replaced_core_cost,
                )
            })
            .collect();
        rows.sort_unstable();
        assert_eq!(
            rows,
            [
                (1, 1, 900, true, Some(500)),
                (1, 2, 100, true, None),
                (2, 2, 700, false, None)
            ]
        );
    }

    #[test]
    fn dropped_proper_nouns_keep_the_row_numbers_of_later_rows() {
        let noun = "名詞,普通名詞,一般,*,*";
        let proper = "名詞,固有名詞,一般,*,*";
        let verb = "動詞,一般,*,*,五段-ラ行";
        let mut a = acc();
        add(&mut a, &row("花", 1, 1, 100, noun, "ハナ")).unwrap(); // 0
        add(&mut a, &row("花子", 1, 1, 100, proper, "ハナ")).unwrap(); // 1: Core の固有名詞は d2s の対象外
        // 2: Core の動詞（notcore の動詞の品詞を Core に作っておく）
        add(
            &mut a,
            &format!("取る,1,1,100,取る,{verb},*,トル,取る,*,A,*,*,*,*"),
        )
        .unwrap();
        add_notcore(&mut a, &row("華", 1, 1, 100, proper, "ハナ")).unwrap(); // 3: 外す
        let verb_line = format!("沼る,1,1,100,沼る,{verb},*,ヌマル,沼る,4,A,*,*,*,*");
        add_notcore(&mut a, &verb_line).unwrap(); // 4
        let past = format!("沼っ,1,1,100,沼っ,{verb},*,ヌマッ,沼る,4,A,*,*,*,*");
        add_notcore(&mut a, &past).unwrap(); // 5
        let lex = a.finish();
        assert_eq!(lex.skipped.get(SKIP_NOTCORE_PROPER_HOMOPHONE), Some(&1));
        assert!(!lex.rows.iter().any(|(r, _)| r.surface == "華"));
        assert!(lex.rows.iter().any(|(r, _)| r.surface == "花子"));
        // 活用行の id は入力全体での行番号のまま（外した行を詰めない）
        let ids: Vec<(u32, bool)> = lex.inflect.iter().map(|r| (r.id, r.notcore)).collect();
        assert_eq!(ids, vec![(2, false), (4, true), (5, true)]);
        assert_eq!(lex.inflect[2].dict_form, Some(4));
    }

    #[test]
    fn short_proper_noun_rule() {
        assert!(is_short_proper_noun("桃", "もも"));
        assert!(!is_short_proper_noun("桃太", "ももた"));
        assert!(is_short_proper_noun("KAZ", "かずき"));
        assert!(!is_short_proper_noun("KAZE", "かぜのき"));
        // 英字を含んでも ASCII だけでなければ 3 文字の読みは残す
        assert!(!is_short_proper_noun("Ka桃", "かもも"));
        assert!(!is_short_proper_noun("", "かもも"));
    }

    #[test]
    fn pos_table_is_sorted_with_representative_ids() {
        let mut a = acc();
        let noun = "名詞,普通名詞,一般,*,*";
        let verb = "動詞,一般,*,*,五段-ラ行";
        // 名詞: (3,4) が2回、(1,2) が1回 -> (3,4)
        add(&mut a, &row("a", 1, 2, 1, noun, "ア")).unwrap();
        add(&mut a, &row("b", 3, 4, 1, noun, "イ")).unwrap();
        add(&mut a, &row("c", 3, 4, 1, noun, "ウ")).unwrap();
        // 動詞: (5,6) と (2,2) が1回ずつ -> 同数なら小さい方 (2,2)
        add(&mut a, &row("d", 5, 6, 1, verb, "エ")).unwrap();
        add(&mut a, &row("e", 2, 2, 1, verb, "オ")).unwrap();
        let lex = a.finish();
        let names: Vec<&str> = lex.pos.iter().map(|p| p.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        let find = |prefix: &str| {
            lex.pos
                .iter()
                .position(|p| p.name.starts_with(prefix))
                .unwrap()
        };
        let n = &lex.pos[find("名詞")];
        assert_eq!((n.rep_left_id, n.rep_right_id), (3, 4));
        let v = &lex.pos[find("動詞")];
        assert_eq!((v.rep_left_id, v.rep_right_id), (2, 2));
        // 行の pos_id が品詞表を指す
        for (r, id) in &lex.rows {
            let expected = if r.surface == "d" || r.surface == "e" {
                "動詞"
            } else {
                "名詞"
            };
            assert!(lex.pos[usize::from(*id)].name.starts_with(expected));
        }
    }
}
