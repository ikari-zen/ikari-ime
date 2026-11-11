//! ビルド済みの辞書ディレクトリの全数検証（`--verify`、ADR 0001 §8）。
//!
//! core が open 時に行う O(1) の検証とは違い、全エントリを読む（約157万件で数秒）。

use std::fs;
use std::path::Path;

use ikari_dict_format::reading::is_indexable_with_digits;
use ikari_dict_format::{
    DICT_KIND_SYSTEM, FormatError, GRAMMAR_FILE_NAME, MAX_READING_CHARS, SYSTEM_DICT_FILE_NAME,
    VECTORS_FILE_NAME, index_for_each, parse_dict_layout, parse_grammar_layout,
    parse_vectors_layout, read_conj_rules, read_lemma, read_lemma_id, read_lemma_row,
    read_pos_record, read_posting, read_raw_entry, read_surface, unpack_index_value,
    verify_dict_id, verify_grammar_id,
};

use crate::BuildError;

/// `verify` が確認できた内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    pub grammar_id: u64,
    pub entries: usize,
    pub readings: usize,
    pub pos_count: usize,
    pub lemmas: usize,
    /// 表記の索引のキー数
    pub surfaces: usize,
    pub conj_rules: usize,
}

/// `verify_vectors` が確認できた内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorsVerifyReport {
    pub dict_id: u64,
    pub dims: usize,
    pub rows: usize,
    pub clusters: usize,
    /// ベクトルを持つ lemma の数
    pub lemmas_with_vector: usize,
}

fn fail(name: &'static str, message: impl Into<String>) -> BuildError {
    BuildError::Format(FormatError::BadSection {
        name,
        message: message.into(),
    })
}

/// `dir` の grammar.bin と system.dict を全数検証する。
///
/// - 両ファイルのヘッダとセクション範囲、grammar_id の再計算、dict と grammar の grammar_id の一致
/// - 品詞表の全レコード（UTF-8、昇順、代表 ID の範囲）
/// - 索引の全キー（索引可能な読み、値の範囲が隙間なく連続）と、全エントリ（範囲、UTF-8、ID の範囲、
///   同じ読みの中のコスト昇順）
pub fn verify(dir: &Path) -> Result<VerifyReport, BuildError> {
    let grammar_bytes = fs::read(dir.join(GRAMMAR_FILE_NAME))?;
    let dict_bytes = fs::read(dir.join(SYSTEM_DICT_FILE_NAME))?;

    let g = parse_grammar_layout(&grammar_bytes)?;
    verify_grammar_id(&grammar_bytes, &g)?;
    let d = parse_dict_layout(&dict_bytes)?;
    if d.grammar_id != g.grammar_id {
        return Err(fail(
            "grammar_id",
            format!(
                "system.dict expects {:#018x} but grammar.bin is {:#018x}",
                d.grammar_id, g.grammar_id
            ),
        ));
    }
    if d.kind != DICT_KIND_SYSTEM {
        return Err(fail(
            "header",
            format!("expected a system dictionary, kind={}", d.kind),
        ));
    }
    if std::str::from_utf8(&dict_bytes[d.meta.clone()]).is_err() {
        return Err(fail("meta", "not valid UTF-8"));
    }

    // 品詞表
    let pos_section = &grammar_bytes[g.pos.clone()];
    let mut prev_name: Option<&str> = None;
    for id in 0..g.pos_count {
        let id16 = u16::try_from(id).map_err(|_| fail("pos", "pos_id does not fit in u16"))?;
        let rec = read_pos_record(pos_section, g.pos_count, id16)
            .ok_or_else(|| fail("pos", format!("record {id} is unreadable")))?;
        if rec.rep_left_id >= g.next_left_size || rec.rep_right_id >= g.prev_right_size {
            return Err(fail(
                "pos",
                format!("record {id}: representative id is out of range"),
            ));
        }
        if rec.rep_share_permille > 1000 {
            return Err(fail(
                "pos",
                format!("record {id}: rep_share_permille exceeds 1000"),
            ));
        }
        if prev_name.is_some_and(|p| p >= rec.name) {
            return Err(fail(
                "pos",
                format!("record {id}: names are not strictly ascending"),
            ));
        }
        prev_name = Some(rec.name);
    }

    let conj_rules = read_conj_rules(&grammar_bytes[g.conj.clone()], g.pos_count)?.len();
    verify_dict_id(&dict_bytes, &d)?;

    // lemma 表: 辞書順（重複なし）で読める
    let lemma_table = &dict_bytes[d.lemma_table.clone()];
    let mut prev_lemma: Option<&str> = None;
    for id in 0..d.lemma_count {
        let text = read_lemma(lemma_table, d.lemma_count, id)
            .ok_or_else(|| fail("lemma_table", format!("lemma {id} is unreadable")))?;
        if prev_lemma.is_some_and(|p| p >= text) {
            return Err(fail("lemma_table", format!("lemma {id} is not ascending")));
        }
        prev_lemma = Some(text);
    }
    let lemma_ids = &dict_bytes[d.lemma_ids.clone()];
    for index in 0..d.entry_count {
        match read_lemma_id(lemma_ids, index) {
            Some(id) if id < d.lemma_count => {}
            _ => {
                return Err(fail(
                    "lemma_ids",
                    format!("entry {index}: lemma id is out of range"),
                ));
            }
        }
    }

    // 索引とエントリ
    let entries_section = &dict_bytes[d.entries.clone()];
    let strings_section = &dict_bytes[d.strings.clone()];
    let mut next_first: u64 = 0;
    let mut readings = 0usize;
    let mut problem: Option<BuildError> = None;
    index_for_each(&dict_bytes[d.index.clone()], &mut |key, value| {
        if problem.is_some() {
            return;
        }
        let result = (|| -> Result<(), BuildError> {
            let key = std::str::from_utf8(key).map_err(|_| fail("index", "key is not UTF-8"))?;
            if !is_indexable_with_digits(key) {
                return Err(fail(
                    "index",
                    format!("key {key:?} is not an indexable reading"),
                ));
            }
            let (first, count) = ikari_dict_format::unpack_index_value(value);
            if u64::from(first) != next_first || count == 0 {
                return Err(fail(
                    "index",
                    format!(
                        "key {key:?}: range ({first}, {count}) is not contiguous (expected start {next_first})"
                    ),
                ));
            }
            next_first += u64::from(count);
            if next_first > u64::from(d.entry_count) {
                return Err(fail(
                    "index",
                    format!("key {key:?}: range exceeds entry_count"),
                ));
            }
            let mut prev_cost = i16::MIN;
            for index in first..first + count {
                let e = read_raw_entry(entries_section, index)
                    .ok_or_else(|| fail("entries", format!("entry {index} is unreadable")))?;
                if read_surface(strings_section, &e).is_none() {
                    return Err(fail(
                        "entries",
                        format!("entry {index}: surface is out of range or not UTF-8"),
                    ));
                }
                if e.left_id >= g.next_left_size || e.right_id >= g.prev_right_size {
                    return Err(fail(
                        "entries",
                        format!("entry {index}: left/right id is out of range"),
                    ));
                }
                if u32::from(e.pos_id) >= g.pos_count {
                    return Err(fail(
                        "entries",
                        format!("entry {index}: pos_id is out of range"),
                    ));
                }
                if e.cost < prev_cost {
                    return Err(fail(
                        "entries",
                        format!("entry {index}: costs are not ascending within {key:?}"),
                    ));
                }
                prev_cost = e.cost;
            }
            readings += 1;
            Ok(())
        })();
        if let Err(e) = result {
            problem = Some(e);
        }
    })?;
    if let Some(e) = problem {
        return Err(e);
    }
    if next_first != u64::from(d.entry_count) {
        return Err(fail(
            "index",
            format!(
                "index covers {next_first} entries but entry_count is {}",
                d.entry_count
            ),
        ));
    }

    // 表記の索引: postings が隙間なく連続し、各エントリの表記が表記と一致し、(cost, index) の昇順
    let postings = &dict_bytes[d.postings.clone()];
    let total_postings = postings.len() / 4;
    let mut next_start: u64 = 0;
    let mut surfaces = 0usize;
    let mut problem: Option<BuildError> = None;
    index_for_each(&dict_bytes[d.surface_index.clone()], &mut |key, value| {
        if problem.is_some() {
            return;
        }
        let result = (|| -> Result<(), BuildError> {
            let key =
                std::str::from_utf8(key).map_err(|_| fail("surface_index", "key is not UTF-8"))?;
            if key.is_empty() || key.chars().count() > MAX_READING_CHARS {
                return Err(fail(
                    "surface_index",
                    format!("key {key:?} has an unsupported length"),
                ));
            }
            let (start, count) = unpack_index_value(value);
            if u64::from(start) != next_start || count == 0 {
                return Err(fail(
                    "surface_index",
                    format!("key {key:?}: range ({start}, {count}) is not contiguous"),
                ));
            }
            next_start += u64::from(count);
            if next_start > total_postings as u64 {
                return Err(fail(
                    "surface_index",
                    format!("key {key:?}: range exceeds postings"),
                ));
            }
            let mut prev: Option<(i16, u32)> = None;
            for position in start..start + count {
                let index = read_posting(postings, position)
                    .ok_or_else(|| fail("postings", format!("posting {position} is unreadable")))?;
                let e = read_raw_entry(entries_section, index)
                    .ok_or_else(|| fail("postings", format!("entry {index} is unreadable")))?;
                if read_surface(strings_section, &e) != Some(key) {
                    return Err(fail(
                        "postings",
                        format!("entry {index} does not have surface {key:?}"),
                    ));
                }
                if prev.is_some_and(|p| p >= (e.cost, index)) {
                    return Err(fail(
                        "postings",
                        format!("key {key:?}: not ascending by (cost, index)"),
                    ));
                }
                prev = Some((e.cost, index));
            }
            surfaces += 1;
            Ok(())
        })();
        if let Err(e) = result {
            problem = Some(e);
        }
    })?;
    if let Some(e) = problem {
        return Err(e);
    }
    // 索引に入るべきエントリ（表記が1〜32文字）の数と一致する
    let mut expected_postings = 0usize;
    for index in 0..d.entry_count {
        let surface = read_raw_entry(entries_section, index)
            .and_then(|e| read_surface(strings_section, &e).map(str::to_owned));
        if surface.is_some_and(|s| !s.is_empty() && s.chars().count() <= MAX_READING_CHARS) {
            expected_postings += 1;
        }
    }
    if next_start != total_postings as u64 || total_postings != expected_postings {
        return Err(fail(
            "surface_index",
            format!(
                "index covers {next_start} postings, section has {total_postings}, expected {expected_postings}"
            ),
        ));
    }

    Ok(VerifyReport {
        grammar_id: g.grammar_id,
        entries: d.entry_count as usize,
        readings,
        pos_count: g.pos_count as usize,
        lemmas: d.lemma_count as usize,
        surfaces,
        conj_rules,
    })
}

/// `dir` の vectors.bin を検証する。system.dict があれば dict_id と lemma 数の整合も確かめる。
pub fn verify_vectors(dir: &Path) -> Result<VectorsVerifyReport, BuildError> {
    let bytes = fs::read(dir.join(VECTORS_FILE_NAME))?;
    let dict_bytes = fs::read(dir.join(SYSTEM_DICT_FILE_NAME))?;
    let v = parse_vectors_layout(&bytes)?;
    let d = parse_dict_layout(&dict_bytes)?;
    if v.dict_id != d.dict_id {
        return Err(fail(
            "dict_id",
            format!(
                "vectors.bin was built for {:#018x} but system.dict is {:#018x}",
                v.dict_id, d.dict_id
            ),
        ));
    }
    let lemma_rows = &bytes[v.lemma_rows.clone()];
    if lemma_rows.len() != d.lemma_count as usize * 4 {
        return Err(fail(
            "lemma_rows",
            format!(
                "has {} entries but the dictionary has {} lemmas",
                lemma_rows.len() / 4,
                d.lemma_count
            ),
        ));
    }
    let mut with_vector = 0usize;
    let mut seen = vec![false; v.row_count as usize];
    for lemma in 0..d.lemma_count {
        if let Some(row) = read_lemma_row(lemma_rows, lemma) {
            let slot = seen.get_mut(row as usize).ok_or_else(|| {
                fail(
                    "lemma_rows",
                    format!("lemma {lemma}: row {row} is out of range"),
                )
            })?;
            if *slot {
                return Err(fail("lemma_rows", format!("row {row} is used twice")));
            }
            *slot = true;
            with_vector += 1;
        }
    }
    Ok(VectorsVerifyReport {
        dict_id: v.dict_id,
        dims: usize::from(v.dims),
        rows: v.row_count as usize,
        clusters: usize::from(v.cluster_count),
        lemmas_with_vector: with_vector,
    })
}
