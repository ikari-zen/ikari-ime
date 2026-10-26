//! `system.dict`（索引、エントリ、文字列）のエンコードと解析（ADR 0001 §3.4）。

use std::collections::HashMap;
use std::ops::Range;

use crate::reading::is_indexable;
use crate::util::{
    Fnv1a64, HeaderReader, check_preamble, check_section, get_u16, get_u32, pad_to_align,
};
use crate::{
    DICT_HEADER_LEN, DICT_KIND_EXTENSION, DICT_KIND_SYSTEM, DICT_MAGIC, ENTRY_SIZE, FORMAT_VERSION,
    FormatError, MAX_READING_CHARS,
};

const WHAT: &str = "system.dict";

// ヘッダのフィールド位置（ADR 0002 §3.2）
const OFF_GRAMMAR_ID: usize = 16;
const OFF_ENTRY_COUNT: usize = 24;
const OFF_KIND: usize = 28;
const OFF_DICT_ID: usize = 32;
const OFF_LEMMA_COUNT: usize = 40;
// 44: 予約
const OFF_INDEX_OFFSET: usize = 48;
const OFF_INDEX_LEN: usize = 56;
const OFF_ENTRIES_OFFSET: usize = 64;
const OFF_ENTRIES_LEN: usize = 72;
const OFF_STRINGS_OFFSET: usize = 80;
const OFF_STRINGS_LEN: usize = 88;
const OFF_META_OFFSET: usize = 96;
const OFF_META_LEN: usize = 104;
const OFF_SURFACE_INDEX_OFFSET: usize = 112;
const OFF_SURFACE_INDEX_LEN: usize = 120;
const OFF_POSTINGS_OFFSET: usize = 128;
const OFF_POSTINGS_LEN: usize = 136;
const OFF_LEMMA_IDS_OFFSET: usize = 144;
const OFF_LEMMA_IDS_LEN: usize = 152;
const OFF_LEMMA_TABLE_OFFSET: usize = 160;
const OFF_LEMMA_TABLE_LEN: usize = 168;

// エントリ内のフィールド位置（§3.4）
const ENT_SURFACE_OFFSET: usize = 0;
const ENT_SURFACE_LEN: usize = 4;
const ENT_LEFT_ID: usize = 6;
const ENT_RIGHT_ID: usize = 8;
const ENT_COST: usize = 10;
const ENT_POS_ID: usize = 12;
const ENT_IME_PENALTY: usize = 14;

/// 索引の値で first_entry_index を持つ上位ビットのシフト量
const INDEX_VALUE_SHIFT: u32 = 32;
/// postings と lemma_ids の要素（u32）のバイト数
const U32_SIZE: usize = 4;

/// `encode_dict` の入力1件。順序は問わない（encode が (読み, cost, 表記, left, right, pos) で整列する）。
/// reading は `reading::is_indexable` を満たす正規化済みの読みであること。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictEntryInput<'a> {
    pub reading: &'a str,
    pub surface: &'a str,
    pub left_id: u16,
    pub right_id: u16,
    pub cost: i16,
    pub pos_id: u16,
    /// 変換時に cost へ足す値。`IME_EXCLUDED` なら変換では使わない
    pub ime_penalty: i16,
    /// 正規化表記（Sudachi の12列目）。ベクトルの対応付けと文脈解析に使う
    pub lemma: &'a str,
}

/// `encode_dict` の入力。
#[derive(Debug, Clone)]
pub struct DictSpec<'a> {
    pub grammar_id: u64,
    /// `DICT_KIND_SYSTEM` | `DICT_KIND_EXTENSION`
    pub kind: u32,
    pub entries: &'a [DictEntryInput<'a>],
    /// `key=value\n` の UTF-8。日時は入れない
    pub meta: &'a str,
}

/// `parse_dict_layout` の結果。範囲は入力 `&[u8]` 内のバイト位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictLayout {
    pub grammar_id: u64,
    pub entry_count: u32,
    pub kind: u32,
    /// fst::Map のバイト列
    pub index: Range<usize>,
    /// `entry_count * ENTRY_SIZE` バイト
    pub entries: Range<usize>,
    pub strings: Range<usize>,
    pub meta: Range<usize>,
    /// index、entries、strings、lemma_ids、lemma_table の FNV-1a 64（vectors.bin の照合用）
    pub dict_id: u64,
    pub lemma_count: u32,
    /// 表記の fst（値は postings の範囲 `start << 32 | count`。start は u32 の要素番号）
    pub surface_index: Range<usize>,
    /// u32 のエントリ番号。表記ごとに (cost, index) の昇順
    pub postings: Range<usize>,
    /// u32 × entry_count。エントリ -> lemma 番号
    pub lemma_ids: Range<usize>,
    /// `u32 offsets[lemma_count + 1]` + UTF-8 の連結（lemma は辞書順、番号は添字）
    pub lemma_table: Range<usize>,
}

/// エントリ（16バイト）をデコードした生の値。surface は strings セクション内の位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawEntry {
    pub surface_offset: u32,
    pub surface_len: u16,
    pub left_id: u16,
    pub right_id: u16,
    pub cost: i16,
    pub pos_id: u16,
    /// 変換時に cost へ足す。`IME_EXCLUDED` なら変換では使わない
    pub ime_penalty: i16,
}

/// 索引の値 `(first_entry_index << 32) | count` を作る。
pub fn pack_index_value(first_entry: u32, count: u32) -> u64 {
    (u64::from(first_entry) << INDEX_VALUE_SHIFT) | u64::from(count)
}

/// 索引の値を `(first_entry_index, count)` に分解する。
pub fn unpack_index_value(value: u64) -> (u32, u32) {
    ((value >> INDEX_VALUE_SHIFT) as u32, value as u32)
}

fn invalid(message: impl Into<String>) -> FormatError {
    FormatError::InvalidInput(message.into())
}

fn bad_section(name: &'static str, message: impl Into<String>) -> FormatError {
    FormatError::BadSection {
        name,
        message: message.into(),
    }
}

type SortKey<'a> = (&'a str, i16, &'a str, u16, u16, u16, i16, &'a str);

fn sort_key<'a>(e: &DictEntryInput<'a>) -> SortKey<'a> {
    (
        e.reading,
        e.cost,
        e.surface,
        e.left_id,
        e.right_id,
        e.pos_id,
        e.ime_penalty,
        e.lemma,
    )
}

/// 表記の索引に入れる表記か。1..=`MAX_READING_CHARS` 文字（解析ラティスの最大語長）。
fn is_surface_indexable(surface: &str) -> bool {
    let mut chars = surface.chars();
    chars.next().is_some() && chars.count() < MAX_READING_CHARS
}

/// dict_id = FNV-1a 64 を index、entries、strings、lemma_ids、lemma_table の順にかけた値。
pub fn compute_dict_id(
    index: &[u8],
    entries: &[u8],
    strings: &[u8],
    lemma_ids: &[u8],
    lemma_table: &[u8],
) -> u64 {
    let mut h = Fnv1a64::new();
    for section in [index, entries, strings, lemma_ids, lemma_table] {
        h.update(section);
    }
    h.finish()
}

/// ヘッダの dict_id が、実際のセクションの内容から計算した値と一致するか確かめる。
/// 全体を読むので O(ファイルサイズ)。builder の `--verify` 用。
pub fn verify_dict_id(bytes: &[u8], layout: &DictLayout) -> Result<(), FormatError> {
    let get = |r: &Range<usize>| bytes.get(r.clone());
    let (Some(index), Some(entries), Some(strings), Some(lemma_ids), Some(lemma_table)) = (
        get(&layout.index),
        get(&layout.entries),
        get(&layout.strings),
        get(&layout.lemma_ids),
        get(&layout.lemma_table),
    ) else {
        return Err(bad_section("dict", "layout does not match the bytes"));
    };
    let actual = compute_dict_id(index, entries, strings, lemma_ids, lemma_table);
    if actual != layout.dict_id {
        return Err(bad_section(
            "dict_id",
            format!(
                "header says {:#018x} but contents hash to {actual:#018x}",
                layout.dict_id
            ),
        ));
    }
    Ok(())
}

/// system.dict のバイト列を作る。fst の構築、表記の重複排除、整列もここで行う。
pub fn encode_dict(spec: &DictSpec<'_>) -> Result<Vec<u8>, FormatError> {
    if spec.kind != DICT_KIND_SYSTEM && spec.kind != DICT_KIND_EXTENSION {
        return Err(invalid(format!("unknown dictionary kind {}", spec.kind)));
    }
    let entry_count = u32::try_from(spec.entries.len())
        .map_err(|_| invalid("too many entries (more than u32::MAX)"))?;
    for e in spec.entries {
        if !is_indexable(e.reading) {
            return Err(invalid(format!("reading {:?} is not indexable", e.reading)));
        }
        if u16::try_from(e.surface.len()).is_err() {
            return Err(invalid(format!(
                "surface of {:?} is too long ({} bytes)",
                e.reading,
                e.surface.len()
            )));
        }
    }

    // 入力の順序に依存しないよう、参照を整列する。
    let mut sorted: Vec<&DictEntryInput<'_>> = spec.entries.iter().collect();
    sorted.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));

    // lemma 表（辞書順。番号は添字）
    let mut lemma_texts: Vec<&str> = sorted.iter().map(|e| e.lemma).collect();
    lemma_texts.sort_unstable();
    lemma_texts.dedup();
    let lemma_count = u32::try_from(lemma_texts.len())
        .map_err(|_| invalid("too many lemmas (more than u32::MAX)"))?;
    let mut lemma_offsets: Vec<u8> = Vec::with_capacity((lemma_texts.len() + 1) * U32_SIZE);
    let mut lemma_bytes: Vec<u8> = Vec::new();
    for text in &lemma_texts {
        let offset = u32::try_from(lemma_bytes.len())
            .map_err(|_| invalid("lemma table is too large (more than 4 GiB)"))?;
        lemma_offsets.extend_from_slice(&offset.to_le_bytes());
        lemma_bytes.extend_from_slice(text.as_bytes());
    }
    let end = u32::try_from(lemma_bytes.len())
        .map_err(|_| invalid("lemma table is too large (more than 4 GiB)"))?;
    lemma_offsets.extend_from_slice(&end.to_le_bytes());
    let mut lemma_table = lemma_offsets;
    lemma_table.extend_from_slice(&lemma_bytes);

    // 文字列（同じ表記は最初に出た位置を使う）、エントリ、lemma_ids、fst を同時に作る。
    let mut strings: Vec<u8> = Vec::new();
    let mut surface_pos: HashMap<&str, u32> = HashMap::new();
    let mut entries: Vec<u8> = Vec::with_capacity(sorted.len() * ENTRY_SIZE);
    let mut lemma_ids: Vec<u8> = Vec::with_capacity(sorted.len() * U32_SIZE);
    let mut fst_builder = fst::MapBuilder::memory();

    let mut i = 0usize;
    while i < sorted.len() {
        let reading = sorted[i].reading;
        let mut j = i;
        while j < sorted.len() && sorted[j].reading == reading {
            j += 1;
        }
        let first = i as u32; // entry_count が u32 に収まるので切り捨てはない
        let count = (j - i) as u32;
        fst_builder
            .insert(reading.as_bytes(), pack_index_value(first, count))
            .map_err(|e| FormatError::Fst(e.to_string()))?;
        i = j;
    }
    for e in &sorted {
        let offset = match surface_pos.get(e.surface) {
            Some(&o) => o,
            None => {
                let o = u32::try_from(strings.len())
                    .map_err(|_| invalid("string section is too large (more than 4 GiB)"))?;
                strings.extend_from_slice(e.surface.as_bytes());
                surface_pos.insert(e.surface, o);
                o
            }
        };
        entries.extend_from_slice(&offset.to_le_bytes());
        entries.extend_from_slice(&(e.surface.len() as u16).to_le_bytes()); // 上で検証済み
        entries.extend_from_slice(&e.left_id.to_le_bytes());
        entries.extend_from_slice(&e.right_id.to_le_bytes());
        entries.extend_from_slice(&e.cost.to_le_bytes());
        entries.extend_from_slice(&e.pos_id.to_le_bytes());
        entries.extend_from_slice(&e.ime_penalty.to_le_bytes());
        // lemma_texts は整列・重複排除済みなので、二分探索で必ず見つかる
        let lemma_id = lemma_texts.binary_search(&e.lemma).unwrap_or_default() as u32;
        lemma_ids.extend_from_slice(&lemma_id.to_le_bytes());
    }
    let index = fst_builder
        .into_inner()
        .map_err(|e| FormatError::Fst(e.to_string()))?;

    // 表記の索引: (表記, cost, エントリ番号) の昇順に postings を並べ、表記ごとに範囲を fst に入れる。
    let mut surface_rows: Vec<(&str, i16, u32)> = sorted
        .iter()
        .enumerate()
        .filter(|(_, e)| is_surface_indexable(e.surface))
        .map(|(idx, e)| (e.surface, e.cost, idx as u32))
        .collect();
    surface_rows.sort_unstable();
    let mut postings: Vec<u8> = Vec::with_capacity(surface_rows.len() * U32_SIZE);
    let mut surface_builder = fst::MapBuilder::memory();
    let mut i = 0usize;
    while i < surface_rows.len() {
        let surface = surface_rows[i].0;
        let mut j = i;
        while j < surface_rows.len() && surface_rows[j].0 == surface {
            postings.extend_from_slice(&surface_rows[j].2.to_le_bytes());
            j += 1;
        }
        surface_builder
            .insert(
                surface.as_bytes(),
                pack_index_value(i as u32, (j - i) as u32),
            )
            .map_err(|e| FormatError::Fst(e.to_string()))?;
        i = j;
    }
    let surface_index = surface_builder
        .into_inner()
        .map_err(|e| FormatError::Fst(e.to_string()))?;

    let dict_id = compute_dict_id(&index, &entries, &strings, &lemma_ids, &lemma_table);

    // セクションの並び: index, entries, strings, meta, surface_index, postings, lemma_ids, lemma_table
    let sections: [&[u8]; 8] = [
        &index,
        &entries,
        &strings,
        spec.meta.as_bytes(),
        &surface_index,
        &postings,
        &lemma_ids,
        &lemma_table,
    ];
    let mut ranges = [(0usize, 0usize); 8];
    let mut cursor = DICT_HEADER_LEN;
    for (range, section) in ranges.iter_mut().zip(sections) {
        *range = (cursor, section.len());
        cursor = crate::util::align_up(cursor + section.len());
    }

    let mut out = Vec::with_capacity(cursor);
    out.extend_from_slice(&DICT_MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&(DICT_HEADER_LEN as u32).to_le_bytes());
    out.extend_from_slice(&spec.grammar_id.to_le_bytes());
    out.extend_from_slice(&entry_count.to_le_bytes());
    out.extend_from_slice(&spec.kind.to_le_bytes());
    out.extend_from_slice(&dict_id.to_le_bytes());
    out.extend_from_slice(&lemma_count.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // 予約
    for (offset, len) in ranges {
        out.extend_from_slice(&(offset as u64).to_le_bytes());
        out.extend_from_slice(&(len as u64).to_le_bytes());
    }
    debug_assert_eq!(out.len(), DICT_HEADER_LEN);

    for section in sections {
        out.extend_from_slice(section);
        pad_to_align(&mut out);
    }
    debug_assert_eq!(out.len(), cursor);
    Ok(out)
}

/// system.dict のヘッダとセクション範囲を検証して返す（O(1) の検証のみ。fst の `Map::new` の成否を含む）。
pub fn parse_dict_layout(bytes: &[u8]) -> Result<DictLayout, FormatError> {
    check_preamble(bytes, WHAT, &DICT_MAGIC, FORMAT_VERSION, DICT_HEADER_LEN)?;
    let r = HeaderReader::new(bytes, WHAT);

    let grammar_id = r.u64(OFF_GRAMMAR_ID)?;
    let entry_count = r.u32(OFF_ENTRY_COUNT)?;
    let kind = r.u32(OFF_KIND)?;
    let dict_id = r.u64(OFF_DICT_ID)?;
    let lemma_count = r.u32(OFF_LEMMA_COUNT)?;
    if kind != DICT_KIND_SYSTEM && kind != DICT_KIND_EXTENSION {
        return Err(bad_section(
            "header",
            format!("unknown dictionary kind {kind}"),
        ));
    }

    let index = check_section(
        "index",
        WHAT,
        bytes.len(),
        r.u64(OFF_INDEX_OFFSET)?,
        r.u64(OFF_INDEX_LEN)?,
        DICT_HEADER_LEN,
    )?;
    let entries_len = r.u64(OFF_ENTRIES_LEN)?;
    let expected = u64::from(entry_count) * ENTRY_SIZE as u64;
    if entries_len != expected {
        return Err(bad_section(
            "entries",
            format!("length {entries_len} does not match entry_count {entry_count} x {ENTRY_SIZE}"),
        ));
    }
    let entries = check_section(
        "entries",
        WHAT,
        bytes.len(),
        r.u64(OFF_ENTRIES_OFFSET)?,
        entries_len,
        index.end,
    )?;
    let strings = check_section(
        "strings",
        WHAT,
        bytes.len(),
        r.u64(OFF_STRINGS_OFFSET)?,
        r.u64(OFF_STRINGS_LEN)?,
        entries.end,
    )?;
    let meta = check_section(
        "meta",
        WHAT,
        bytes.len(),
        r.u64(OFF_META_OFFSET)?,
        r.u64(OFF_META_LEN)?,
        strings.end,
    )?;
    let surface_index = check_section(
        "surface_index",
        WHAT,
        bytes.len(),
        r.u64(OFF_SURFACE_INDEX_OFFSET)?,
        r.u64(OFF_SURFACE_INDEX_LEN)?,
        meta.end,
    )?;
    let postings_len = r.u64(OFF_POSTINGS_LEN)?;
    if !postings_len.is_multiple_of(U32_SIZE as u64) {
        return Err(bad_section(
            "postings",
            format!("length {postings_len} is not a multiple of {U32_SIZE}"),
        ));
    }
    let postings = check_section(
        "postings",
        WHAT,
        bytes.len(),
        r.u64(OFF_POSTINGS_OFFSET)?,
        postings_len,
        surface_index.end,
    )?;
    let lemma_ids_len = r.u64(OFF_LEMMA_IDS_LEN)?;
    let expected = u64::from(entry_count) * U32_SIZE as u64;
    if lemma_ids_len != expected {
        return Err(bad_section(
            "lemma_ids",
            format!("length {lemma_ids_len} does not match entry_count {entry_count} x {U32_SIZE}"),
        ));
    }
    let lemma_ids = check_section(
        "lemma_ids",
        WHAT,
        bytes.len(),
        r.u64(OFF_LEMMA_IDS_OFFSET)?,
        lemma_ids_len,
        postings.end,
    )?;
    let lemma_table_len = r.u64(OFF_LEMMA_TABLE_LEN)?;
    let offsets_len = (u64::from(lemma_count) + 1) * U32_SIZE as u64;
    if lemma_table_len < offsets_len {
        return Err(bad_section(
            "lemma_table",
            format!("length {lemma_table_len} is shorter than the offset table ({offsets_len})"),
        ));
    }
    let lemma_table = check_section(
        "lemma_table",
        WHAT,
        bytes.len(),
        r.u64(OFF_LEMMA_TABLE_OFFSET)?,
        lemma_table_len,
        lemma_ids.end,
    )?;
    // surface_offset は u32 なので、それより大きい文字列領域は表現できない
    if strings.len() > u32::MAX as usize + u16::MAX as usize {
        return Err(bad_section("strings", "section is too large"));
    }

    // fst の最低限の整合（バージョン、ルートのアドレスなど）。全体の走査はしない。
    fst::Map::new(&bytes[index.clone()]).map_err(|e| FormatError::Fst(e.to_string()))?;
    fst::Map::new(&bytes[surface_index.clone()]).map_err(|e| FormatError::Fst(e.to_string()))?;

    Ok(DictLayout {
        grammar_id,
        entry_count,
        kind,
        index,
        entries,
        strings,
        meta,
        dict_id,
        lemma_count,
        surface_index,
        postings,
        lemma_ids,
        lemma_table,
    })
}

/// lemma_ids セクションから、エントリ番号 `entry_index` の lemma 番号を読む。範囲外なら None。
pub fn read_lemma_id(lemma_ids_section: &[u8], entry_index: u32) -> Option<u32> {
    let start = (entry_index as usize).checked_mul(U32_SIZE)?;
    get_u32(lemma_ids_section, start)
}

/// lemma_table セクションから lemma 番号の文字列を引く。範囲外や壊れた表（UTF-8 不正、オフセット逆転）なら None。
pub fn read_lemma(lemma_table_section: &[u8], lemma_count: u32, lemma_id: u32) -> Option<&str> {
    if lemma_id >= lemma_count {
        return None;
    }
    let offsets_len = (lemma_count as usize)
        .checked_add(1)?
        .checked_mul(U32_SIZE)?;
    let texts = lemma_table_section.get(offsets_len..)?;
    let at = (lemma_id as usize).checked_mul(U32_SIZE)?;
    let start = get_u32(lemma_table_section, at)? as usize;
    let end = get_u32(lemma_table_section, at + U32_SIZE)? as usize;
    std::str::from_utf8(texts.get(start..end)?).ok()
}

/// postings セクションの `position` 番目（u32 の要素番号）のエントリ番号。範囲外なら None。
pub fn read_posting(postings_section: &[u8], position: u32) -> Option<u32> {
    let start = (position as usize).checked_mul(U32_SIZE)?;
    get_u32(postings_section, start)
}

/// 索引（fst::Map）から完全一致で値を引く。builder の検証やテストで fst に直接依存せずに済むようにする。
pub fn index_get(index_section: &[u8], key: &str) -> Result<Option<u64>, FormatError> {
    let map = fst::Map::new(index_section).map_err(|e| FormatError::Fst(e.to_string()))?;
    Ok(map.get(key))
}

/// 索引の全キーを昇順に走査する（O(索引サイズ)。open 時ではなく `--verify` 用）。
pub fn index_for_each(
    index_section: &[u8],
    f: &mut dyn FnMut(&[u8], u64),
) -> Result<(), FormatError> {
    use fst::{IntoStreamer, Streamer};
    let map = fst::Map::new(index_section).map_err(|e| FormatError::Fst(e.to_string()))?;
    let mut stream = map.into_stream();
    while let Some((key, value)) = stream.next() {
        f(key, value);
    }
    Ok(())
}

/// エントリセクションから添字 `index` のエントリを読む。範囲外なら None。
pub fn read_raw_entry(entries_section: &[u8], index: u32) -> Option<RawEntry> {
    let start = (index as usize).checked_mul(ENTRY_SIZE)?;
    let e = entries_section.get(start..start.checked_add(ENTRY_SIZE)?)?;
    Some(RawEntry {
        surface_offset: get_u32(e, ENT_SURFACE_OFFSET)?,
        surface_len: get_u16(e, ENT_SURFACE_LEN)?,
        left_id: get_u16(e, ENT_LEFT_ID)?,
        right_id: get_u16(e, ENT_RIGHT_ID)?,
        cost: get_u16(e, ENT_COST)? as i16,
        pos_id: get_u16(e, ENT_POS_ID)?,
        ime_penalty: get_u16(e, ENT_IME_PENALTY)? as i16,
    })
}

/// 文字列セクションからエントリの表記を引く。範囲外や UTF-8 不正なら None。
pub fn read_surface<'a>(strings_section: &'a [u8], entry: &RawEntry) -> Option<&'a str> {
    let start = entry.surface_offset as usize;
    let end = start.checked_add(usize::from(entry.surface_len))?;
    std::str::from_utf8(strings_section.get(start..end)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IME_EXCLUDED;

    fn input<'a>(reading: &'a str, surface: &'a str, cost: i16, pos_id: u16) -> DictEntryInput<'a> {
        DictEntryInput {
            reading,
            surface,
            left_id: 1,
            right_id: 2,
            cost,
            pos_id,
            ime_penalty: 0,
            lemma: surface,
        }
    }

    fn sample_entries() -> Vec<DictEntryInput<'static>> {
        vec![
            input("ひんし", "瀕死", 7403, 3),
            input("ひんし", "品詞", 7392, 3),
            input("ひん", "品", -5, 1),
            input("ひ", "日", 100, 2),
            input("ひんし", "瀕死", 7403, 4), // 同じ表記は文字列を共有する
            input("らーめん", "ラーメン", 3000, 1),
            input("の", "の", 200, 5),
        ]
    }

    const META: &str = "source=test\nentries=7\n";

    fn spec<'a>(entries: &'a [DictEntryInput<'a>]) -> DictSpec<'a> {
        DictSpec {
            grammar_id: 0x1122_3344_5566_7788,
            kind: DICT_KIND_SYSTEM,
            entries,
            meta: META,
        }
    }

    fn encoded() -> Vec<u8> {
        encode_dict(&spec(&sample_entries())).unwrap()
    }

    fn lookup<'a>(bytes: &'a [u8], layout: &DictLayout, reading: &str) -> Vec<(&'a str, RawEntry)> {
        let map = fst::Map::new(&bytes[layout.index.clone()]).unwrap();
        let Some(v) = map.get(reading) else {
            return Vec::new();
        };
        let (first, count) = unpack_index_value(v);
        (first..first + count)
            .map(|i| {
                let e = read_raw_entry(&bytes[layout.entries.clone()], i).unwrap();
                let s = read_surface(&bytes[layout.strings.clone()], &e).unwrap();
                (s, e)
            })
            .collect()
    }

    #[test]
    fn index_value_pack_roundtrip() {
        for (first, count) in [(0, 0), (1, 2), (u32::MAX, u32::MAX), (123_456, 7)] {
            assert_eq!(
                unpack_index_value(pack_index_value(first, count)),
                (first, count)
            );
        }
    }

    #[test]
    fn roundtrip_all_fields() {
        let bytes = encoded();
        assert_eq!(bytes.len() % 8, 0);
        let layout = parse_dict_layout(&bytes).unwrap();
        assert_eq!(layout.grammar_id, 0x1122_3344_5566_7788);
        assert_eq!(layout.entry_count, 7);
        assert_eq!(layout.kind, DICT_KIND_SYSTEM);
        for r in [
            &layout.index,
            &layout.entries,
            &layout.strings,
            &layout.meta,
        ] {
            assert_eq!(r.start % 8, 0);
        }
        assert_eq!(layout.entries.len(), 7 * ENTRY_SIZE);
        assert_eq!(&bytes[layout.meta.clone()], META.as_bytes());

        let hits = lookup(&bytes, &layout, "ひんし");
        // コスト昇順、同コストは表記順
        let got: Vec<(&str, i16, u16)> = hits.iter().map(|(s, e)| (*s, e.cost, e.pos_id)).collect();
        assert_eq!(
            got,
            vec![("品詞", 7392, 3), ("瀕死", 7403, 3), ("瀕死", 7403, 4)]
        );
        for (_, e) in &hits {
            assert_eq!((e.left_id, e.right_id, e.ime_penalty), (1, 2, 0));
        }
        assert_eq!(lookup(&bytes, &layout, "ひん")[0].0, "品");
        assert_eq!(lookup(&bytes, &layout, "ひん")[0].1.cost, -5);
        assert_eq!(lookup(&bytes, &layout, "らーめん")[0].0, "ラーメン");
        assert!(lookup(&bytes, &layout, "ひんしx").is_empty());
        assert!(lookup(&bytes, &layout, "ひんし。").is_empty());
    }

    #[test]
    fn identical_surfaces_share_one_string() {
        let bytes = encoded();
        let layout = parse_dict_layout(&bytes).unwrap();
        let hits = lookup(&bytes, &layout, "ひんし");
        assert_eq!(hits[1].1.surface_offset, hits[2].1.surface_offset);
        // 文字列は 品, 品詞, 瀕死, 日, ラーメン, の の重複なし連結（出現順は整列後）
        let total: usize = ["品", "品詞", "瀕死", "日", "ラーメン", "の"]
            .iter()
            .map(|s| s.len())
            .sum();
        assert_eq!(layout.strings.len(), total);
    }

    #[test]
    fn output_is_independent_of_input_order() {
        let mut reversed = sample_entries();
        reversed.reverse();
        assert_eq!(encoded(), encode_dict(&spec(&reversed)).unwrap());
        assert_eq!(encoded(), encoded());
    }

    #[test]
    fn empty_dictionary_roundtrips() {
        let bytes = encode_dict(&spec(&[])).unwrap();
        let layout = parse_dict_layout(&bytes).unwrap();
        assert_eq!(layout.entry_count, 0);
        assert!(lookup(&bytes, &layout, "あ").is_empty());
    }

    #[test]
    fn extension_kind_roundtrips() {
        let entries = sample_entries();
        let mut s = spec(&entries);
        s.kind = DICT_KIND_EXTENSION;
        let layout = parse_dict_layout(&encode_dict(&s).unwrap()).unwrap();
        assert_eq!(layout.kind, DICT_KIND_EXTENSION);
    }

    #[test]
    fn read_raw_entry_out_of_range_is_none() {
        let bytes = encoded();
        let layout = parse_dict_layout(&bytes).unwrap();
        let section = &bytes[layout.entries.clone()];
        assert!(read_raw_entry(section, 6).is_some());
        assert!(read_raw_entry(section, 7).is_none());
        assert!(read_raw_entry(section, u32::MAX).is_none());
        assert!(read_raw_entry(&section[..ENTRY_SIZE - 1], 0).is_none());
        assert!(read_raw_entry(&[], 0).is_none());
    }

    #[test]
    fn read_surface_rejects_bad_ranges() {
        let strings = "品詞".as_bytes();
        let entry = |offset, len| RawEntry {
            surface_offset: offset,
            surface_len: len,
            left_id: 0,
            right_id: 0,
            cost: 0,
            pos_id: 0,
            ime_penalty: 0,
        };
        assert_eq!(read_surface(strings, &entry(0, 6)), Some("品詞"));
        assert_eq!(read_surface(strings, &entry(0, 7)), None);
        assert_eq!(read_surface(strings, &entry(u32::MAX, u16::MAX)), None);
        assert_eq!(read_surface(strings, &entry(1, 2)), None); // 文字の途中
    }

    #[test]
    fn bad_magic_version_header_len() {
        let mut bytes = encoded();
        bytes[3] = b'X';
        assert!(matches!(
            parse_dict_layout(&bytes),
            Err(FormatError::BadMagic { .. })
        ));

        let mut bytes = encoded();
        bytes[8..12].copy_from_slice(&0u32.to_le_bytes());
        assert!(matches!(
            parse_dict_layout(&bytes),
            Err(FormatError::UnsupportedVersion { found: 0, .. })
        ));

        let mut bytes = encoded();
        bytes[12..16].copy_from_slice(&80u32.to_le_bytes());
        assert!(matches!(
            parse_dict_layout(&bytes),
            Err(FormatError::BadHeaderLen { .. })
        ));

        // grammar.bin のマジックを渡してもエラー
        assert!(matches!(
            parse_dict_layout(b"IKARIGRM\x01\0\0\0\x60\0\0\0"),
            Err(FormatError::BadMagic { .. })
        ));
    }

    #[test]
    fn every_truncation_is_an_error_without_panic() {
        let bytes = encoded();
        let layout = parse_dict_layout(&bytes).unwrap();
        let used = layout.lemma_table.end;
        for len in 0..used {
            assert!(
                parse_dict_layout(&bytes[..len]).is_err(),
                "len {len} should fail"
            );
        }
        assert!(parse_dict_layout(&bytes[..used]).is_ok());
    }

    #[test]
    fn out_of_range_sections_are_errors() {
        let cases: [(usize, u64); 10] = [
            (OFF_INDEX_OFFSET, u64::MAX),
            (OFF_INDEX_OFFSET, 100), // 8 バイト境界でない
            (OFF_INDEX_OFFSET, 8),   // ヘッダと重なる
            (OFF_INDEX_LEN, u64::MAX),
            (OFF_ENTRIES_OFFSET, 1 << 40),
            (OFF_ENTRIES_LEN, 0), // entry_count と合わない
            (OFF_STRINGS_OFFSET, u64::MAX - 7),
            (OFF_STRINGS_LEN, 1 << 40),
            (OFF_META_OFFSET, DICT_HEADER_LEN as u64), // 前のセクションと重なる
            (OFF_META_LEN, u64::MAX),
        ];
        for (off, value) in cases {
            let mut bytes = encoded();
            bytes[off..off + 8].copy_from_slice(&value.to_le_bytes());
            assert!(
                parse_dict_layout(&bytes).is_err(),
                "offset {off} value {value:#x}"
            );
        }
        // entry_count が実際と合わない
        let mut bytes = encoded();
        bytes[OFF_ENTRY_COUNT..OFF_ENTRY_COUNT + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_dict_layout(&bytes).is_err());
        // kind が不明
        let mut bytes = encoded();
        bytes[OFF_KIND..OFF_KIND + 4].copy_from_slice(&9u32.to_le_bytes());
        assert!(parse_dict_layout(&bytes).is_err());
    }

    #[test]
    fn corrupt_fst_header_is_an_error() {
        let bytes = encoded();
        let layout = parse_dict_layout(&bytes).unwrap();
        // fst の先頭 8 バイトはバージョン。壊すと Map::new が失敗する
        let mut broken = bytes.clone();
        broken[layout.index.start..layout.index.start + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(matches!(
            parse_dict_layout(&broken),
            Err(FormatError::Fst(_))
        ));
        // 空の索引
        let mut broken = bytes;
        broken[OFF_INDEX_LEN..OFF_INDEX_LEN + 8].copy_from_slice(&0u64.to_le_bytes());
        assert!(parse_dict_layout(&broken).is_err());
    }

    #[test]
    fn single_byte_flips_never_panic() {
        // ファイル全体の1バイト反転。fst の走査はしないが、スライスの読み取りはすべて試す。
        let bytes = encoded();
        for i in 0..bytes.len() {
            for mask in [0x01u8, 0x10, 0x80, 0xff] {
                let mut b = bytes.clone();
                b[i] ^= mask;
                if let Ok(layout) = parse_dict_layout(&b) {
                    for idx in 0..layout.entry_count.min(16) {
                        if let Some(e) = read_raw_entry(&b[layout.entries.clone()], idx) {
                            let _ = read_surface(&b[layout.strings.clone()], &e);
                        }
                        let _ = read_lemma_id(&b[layout.lemma_ids.clone()], idx);
                        let _ = read_posting(&b[layout.postings.clone()], idx);
                    }
                    for id in 0..layout.lemma_count.min(16) {
                        let _ = read_lemma(&b[layout.lemma_table.clone()], layout.lemma_count, id);
                    }
                    let _ = verify_dict_id(&b, &layout);
                }
            }
        }
    }

    #[test]
    fn every_section_is_aligned_and_ordered() {
        let bytes = encoded();
        let l = parse_dict_layout(&bytes).unwrap();
        let ranges = [
            &l.index,
            &l.entries,
            &l.strings,
            &l.meta,
            &l.surface_index,
            &l.postings,
            &l.lemma_ids,
            &l.lemma_table,
        ];
        for r in &ranges {
            assert_eq!(r.start % 8, 0);
        }
        for pair in ranges.windows(2) {
            assert!(pair[0].end <= pair[1].start);
        }
        assert_eq!(l.lemma_ids.len(), 7 * 4);
    }

    fn entry_with<'a>(
        reading: &'a str,
        surface: &'a str,
        cost: i16,
        penalty: i16,
        lemma: &'a str,
    ) -> DictEntryInput<'a> {
        DictEntryInput {
            ime_penalty: penalty,
            lemma,
            ..input(reading, surface, cost, 1)
        }
    }

    #[test]
    fn penalty_and_lemma_roundtrip() {
        let entries = [
            entry_with("こう", "コウ", 2924, 3000, "こう"),
            entry_with("こう", "公", 5000, 0, "公"),
            entry_with("こう", "こう", 5100, IME_EXCLUDED, "こう"),
            entry_with("ゆーざ", "ユーザ", 4000, 0, "ユーザー"),
            entry_with("ゆーざー", "ユーザー", 4100, -5, "ユーザー"),
        ];
        let bytes = encode_dict(&spec(&entries)).unwrap();
        let l = parse_dict_layout(&bytes).unwrap();
        assert_eq!(l.lemma_count, 3);
        let table = &bytes[l.lemma_table.clone()];
        // lemma は辞書順で、番号は添字
        let lemmas: Vec<&str> = (0..3).map(|i| read_lemma(table, 3, i).unwrap()).collect();
        assert_eq!(lemmas, ["こう", "ユーザー", "公"]); // UTF-8 のバイト順
        assert_eq!(read_lemma(table, 3, 3), None);

        let hits = lookup(&bytes, &l, "こう");
        let got: Vec<(&str, i16)> = hits.iter().map(|(s, e)| (*s, e.ime_penalty)).collect();
        assert_eq!(got, [("コウ", 3000), ("公", 0), ("こう", IME_EXCLUDED)]);
        let ids = &bytes[l.lemma_ids.clone()];
        let first = unpack_index_value(
            fst::Map::new(&bytes[l.index.clone()])
                .unwrap()
                .get("こう")
                .unwrap(),
        )
        .0;
        let lemma_of = |i: u32| read_lemma(table, 3, read_lemma_id(ids, i).unwrap()).unwrap();
        assert_eq!(lemma_of(first), "こう");
        assert_eq!(lemma_of(first + 1), "公");
        assert_eq!(read_lemma_id(ids, 5), None);
    }

    /// 表記の検索は、全エントリの総当たりと一致する。
    #[test]
    fn surface_search_matches_brute_force() {
        let texts = [
            "あ",
            "あい",
            "あいう",
            "いう",
            "漢",
            "漢字",
            "漢字仮名",
            "A",
            "AB",
            "ab",
            "ラーメン",
            "ラ",
        ];
        let long = "あ".repeat(MAX_READING_CHARS);
        let too_long = "あ".repeat(MAX_READING_CHARS + 1);
        let mut entries: Vec<DictEntryInput<'_>> = Vec::new();
        let mut seed = 12345u64;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let readings = ["あ", "い", "うえ", "おかき"];
        for _ in 0..120 {
            let surface = match next(texts.len() + 2) {
                i if i < texts.len() => texts[i],
                i if i == texts.len() => long.as_str(),
                _ => too_long.as_str(),
            };
            entries.push(entry_with(
                readings[next(readings.len())],
                surface,
                (next(2000) as i16) - 1000,
                0,
                surface,
            ));
        }
        let bytes = encode_dict(&spec(&entries)).unwrap();
        let l = parse_dict_layout(&bytes).unwrap();
        let entries_section = &bytes[l.entries.clone()];
        let strings = &bytes[l.strings.clone()];
        let all: Vec<(u32, RawEntry, &str)> = (0..l.entry_count)
            .map(|i| {
                let e = read_raw_entry(entries_section, i).unwrap();
                (i, e, read_surface(strings, &e).unwrap())
            })
            .collect();
        let map = fst::Map::new(&bytes[l.surface_index.clone()]).unwrap();
        let postings = &bytes[l.postings.clone()];

        let mut probes: Vec<&str> = texts.to_vec();
        probes.push(&long);
        probes.push(&too_long);
        probes.push("存在しない");
        for probe in probes {
            let mut expected: Vec<(u32, i16)> = all
                .iter()
                .filter(|(_, _, s)| *s == probe && s.chars().count() <= MAX_READING_CHARS)
                .map(|(i, e, _)| (*i, e.cost))
                .collect();
            expected.sort_by_key(|&(i, c)| (c, i));
            let got: Vec<(u32, i16)> = match map.get(probe) {
                None => Vec::new(),
                Some(v) => {
                    let (start, count) = unpack_index_value(v);
                    (start..start + count)
                        .map(|p| {
                            let idx = read_posting(postings, p).unwrap();
                            (idx, all[idx as usize].1.cost)
                        })
                        .collect()
                }
            };
            assert_eq!(got, expected, "{probe:?}");
        }
        // 32文字を超える表記は索引に入らない
        assert!(map.get(too_long.as_str()).is_none());
        // 索引に入ったエントリ数は、32文字以下の表記を持つエントリ数と一致する
        let indexed = all
            .iter()
            .filter(|(_, _, s)| s.chars().count() <= MAX_READING_CHARS)
            .count();
        assert_eq!(l.postings.len(), indexed * 4);
    }

    #[test]
    fn dict_id_detects_changes() {
        let bytes = encoded();
        let l = parse_dict_layout(&bytes).unwrap();
        verify_dict_id(&bytes, &l).unwrap();

        // 内容が違えば dict_id も違う（penalty だけの違いでも変わる）
        let a = [entry_with("あ", "亜", 1, 0, "亜")];
        let b = [entry_with("あ", "亜", 1, 5, "亜")];
        let id = |e: &[DictEntryInput<'_>]| {
            parse_dict_layout(&encode_dict(&spec(e)).unwrap())
                .unwrap()
                .dict_id
        };
        assert_ne!(id(&a), id(&b));
        assert_eq!(id(&a), id(&a));

        // エントリを書き換えると検出できる
        let mut broken = bytes.clone();
        broken[l.entries.start] ^= 1;
        let l2 = parse_dict_layout(&broken).unwrap();
        assert!(verify_dict_id(&broken, &l2).is_err());
        let mut broken = bytes;
        broken[l.lemma_ids.start] ^= 1;
        let l2 = parse_dict_layout(&broken).unwrap();
        assert!(verify_dict_id(&broken, &l2).is_err());
    }

    #[test]
    fn v1_files_are_rejected() {
        let mut bytes = encoded();
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
        assert!(matches!(
            parse_dict_layout(&bytes),
            Err(FormatError::UnsupportedVersion {
                found: 1,
                expected: 2
            })
        ));
    }

    #[test]
    fn encode_rejects_invalid_inputs() {
        for reading in ["", "ヒンシ", "abc", "ひ ん", &"あ".repeat(33)] {
            let entries = [input(reading, "x", 0, 0)];
            assert!(
                matches!(
                    encode_dict(&spec(&entries)),
                    Err(FormatError::InvalidInput(_))
                ),
                "{reading:?}"
            );
        }
        let long_surface = "あ".repeat(30_000); // 90000 バイト > u16::MAX
        let entries = [input("あ", &long_surface, 0, 0)];
        assert!(encode_dict(&spec(&entries)).is_err());

        let entries = sample_entries();
        let mut s = spec(&entries);
        s.kind = 7;
        assert!(encode_dict(&s).is_err());
    }
}
