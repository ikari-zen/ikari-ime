//! `vectors.bin`（lemma 番号で引く単語ベクトル）のエンコードと解析（ADR 0002 §3.4）。
//!
//! 各行は L2 正規化してから ×127 で丸めた i8。行の並びは chiVe のテキストの順（頻度の降順）で、
//! 行番号がそのまま頻度順位になる。`dict_id` は system.dict のものと一致しなければ使わない。

use std::ops::Range;

use crate::util::{HeaderReader, check_preamble, check_section, get_u32, pad_to_align};
use crate::{FormatError, VECTORS_FORMAT_VERSION, VECTORS_HEADER_LEN, VECTORS_MAGIC};

const WHAT: &str = "vectors.bin";

const OFF_DICT_ID: usize = 16;
const OFF_DIMS: usize = 24;
const OFF_CLUSTER_COUNT: usize = 26;
const OFF_ROW_COUNT: usize = 28;
const OFF_LEMMA_ROWS_OFFSET: usize = 32;
const OFF_LEMMA_ROWS_LEN: usize = 40;
const OFF_ROWS_OFFSET: usize = 48;
const OFF_ROWS_LEN: usize = 56;
const OFF_CENTROIDS_OFFSET: usize = 64;
const OFF_CENTROIDS_LEN: usize = 72;
const OFF_META_OFFSET: usize = 80;
const OFF_META_LEN: usize = 88;

const U32_SIZE: usize = 4;
/// `lemma_rows` で「この lemma のベクトルはない」を表す値
pub const NO_VECTOR_ROW: u32 = u32::MAX;

/// `encode_vectors` の入力。
#[derive(Debug, Clone)]
pub struct VectorsSpec<'a> {
    pub dict_id: u64,
    pub dims: u16,
    pub cluster_count: u16,
    /// lemma 番号 -> 行番号（なければ `NO_VECTOR_ROW`）
    pub lemma_rows: &'a [u32],
    /// `dims * row_count` 個。1行ずつ L2 正規化して ×127 で丸めたもの
    pub rows: &'a [i8],
    /// `dims * cluster_count` 個
    pub centroids: &'a [i8],
    /// `key=value\n` の UTF-8
    pub meta: &'a str,
}

/// `parse_vectors_layout` の結果。範囲は入力 `&[u8]` 内のバイト位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorsLayout {
    pub dict_id: u64,
    pub dims: u16,
    pub cluster_count: u16,
    pub row_count: u32,
    /// u32 × lemma_count
    pub lemma_rows: Range<usize>,
    /// i8 × dims × row_count
    pub rows: Range<usize>,
    /// i8 × dims × cluster_count
    pub centroids: Range<usize>,
    pub meta: Range<usize>,
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

fn push_i8s(out: &mut Vec<u8>, values: &[i8]) {
    out.extend(values.iter().map(|&v| v as u8));
}

/// vectors.bin のバイト列を作る。
pub fn encode_vectors(spec: &VectorsSpec<'_>) -> Result<Vec<u8>, FormatError> {
    let dims = usize::from(spec.dims);
    if dims == 0 {
        return Err(invalid("dims must be non-zero"));
    }
    if !spec.rows.len().is_multiple_of(dims) {
        return Err(invalid(format!(
            "rows has {} values, which is not a multiple of dims {dims}",
            spec.rows.len()
        )));
    }
    let row_count = u32::try_from(spec.rows.len() / dims)
        .map_err(|_| invalid("too many rows (more than u32::MAX)"))?;
    if spec.centroids.len() != dims * usize::from(spec.cluster_count) {
        return Err(invalid(format!(
            "centroids has {} values but dims x cluster_count = {} expected",
            spec.centroids.len(),
            dims * usize::from(spec.cluster_count)
        )));
    }
    if let Some(bad) = spec
        .lemma_rows
        .iter()
        .find(|&&r| r != NO_VECTOR_ROW && r >= row_count)
    {
        return Err(invalid(format!(
            "lemma_rows refers to row {bad} but there are only {row_count} rows"
        )));
    }

    let mut lemma_rows = Vec::with_capacity(spec.lemma_rows.len() * U32_SIZE);
    for r in spec.lemma_rows {
        lemma_rows.extend_from_slice(&r.to_le_bytes());
    }
    let mut rows = Vec::with_capacity(spec.rows.len());
    push_i8s(&mut rows, spec.rows);
    let mut centroids = Vec::with_capacity(spec.centroids.len());
    push_i8s(&mut centroids, spec.centroids);

    let sections: [&[u8]; 4] = [&lemma_rows, &rows, &centroids, spec.meta.as_bytes()];
    let mut ranges = [(0usize, 0usize); 4];
    let mut cursor = VECTORS_HEADER_LEN;
    for (range, section) in ranges.iter_mut().zip(sections) {
        *range = (cursor, section.len());
        cursor = crate::util::align_up(cursor + section.len());
    }

    let mut out = Vec::with_capacity(cursor);
    out.extend_from_slice(&VECTORS_MAGIC);
    out.extend_from_slice(&VECTORS_FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&(VECTORS_HEADER_LEN as u32).to_le_bytes());
    out.extend_from_slice(&spec.dict_id.to_le_bytes());
    out.extend_from_slice(&spec.dims.to_le_bytes());
    out.extend_from_slice(&spec.cluster_count.to_le_bytes());
    out.extend_from_slice(&row_count.to_le_bytes());
    for (offset, len) in ranges {
        out.extend_from_slice(&(offset as u64).to_le_bytes());
        out.extend_from_slice(&(len as u64).to_le_bytes());
    }
    debug_assert_eq!(out.len(), VECTORS_HEADER_LEN);
    for section in sections {
        out.extend_from_slice(section);
        pad_to_align(&mut out);
    }
    Ok(out)
}

/// vectors.bin のヘッダとセクション範囲を検証して返す（O(1)）。壊れた入力でもパニックしない。
pub fn parse_vectors_layout(bytes: &[u8]) -> Result<VectorsLayout, FormatError> {
    check_preamble(
        bytes,
        WHAT,
        &VECTORS_MAGIC,
        VECTORS_FORMAT_VERSION,
        VECTORS_HEADER_LEN,
    )?;
    let r = HeaderReader::new(bytes, WHAT);

    let dict_id = r.u64(OFF_DICT_ID)?;
    let dims = r.u16(OFF_DIMS)?;
    let cluster_count = r.u16(OFF_CLUSTER_COUNT)?;
    let row_count = r.u32(OFF_ROW_COUNT)?;
    if dims == 0 {
        return Err(bad_section("header", "dims is zero"));
    }

    let lemma_rows_len = r.u64(OFF_LEMMA_ROWS_LEN)?;
    if !lemma_rows_len.is_multiple_of(U32_SIZE as u64) {
        return Err(bad_section(
            "lemma_rows",
            format!("length {lemma_rows_len} is not a multiple of {U32_SIZE}"),
        ));
    }
    let lemma_rows = check_section(
        "lemma_rows",
        WHAT,
        bytes.len(),
        r.u64(OFF_LEMMA_ROWS_OFFSET)?,
        lemma_rows_len,
        VECTORS_HEADER_LEN,
    )?;
    let rows_len = r.u64(OFF_ROWS_LEN)?;
    let expected = u64::from(row_count) * u64::from(dims);
    if rows_len != expected {
        return Err(bad_section(
            "rows",
            format!("length {rows_len} does not match row_count {row_count} x dims {dims}"),
        ));
    }
    let rows = check_section(
        "rows",
        WHAT,
        bytes.len(),
        r.u64(OFF_ROWS_OFFSET)?,
        rows_len,
        lemma_rows.end,
    )?;
    let centroids_len = r.u64(OFF_CENTROIDS_LEN)?;
    let expected = u64::from(cluster_count) * u64::from(dims);
    if centroids_len != expected {
        return Err(bad_section(
            "centroids",
            format!(
                "length {centroids_len} does not match cluster_count {cluster_count} x dims {dims}"
            ),
        ));
    }
    let centroids = check_section(
        "centroids",
        WHAT,
        bytes.len(),
        r.u64(OFF_CENTROIDS_OFFSET)?,
        centroids_len,
        rows.end,
    )?;
    let meta = check_section(
        "meta",
        WHAT,
        bytes.len(),
        r.u64(OFF_META_OFFSET)?,
        r.u64(OFF_META_LEN)?,
        centroids.end,
    )?;
    Ok(VectorsLayout {
        dict_id,
        dims,
        cluster_count,
        row_count,
        lemma_rows,
        rows,
        centroids,
        meta,
    })
}

/// lemma_rows セクションから lemma 番号の行番号を読む。範囲外や `NO_VECTOR_ROW` なら None。
pub fn read_lemma_row(lemma_rows_section: &[u8], lemma_id: u32) -> Option<u32> {
    let at = (lemma_id as usize).checked_mul(U32_SIZE)?;
    let row = get_u32(lemma_rows_section, at)?;
    (row != NO_VECTOR_ROW).then_some(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIMS: u16 = 4;

    fn sample() -> Vec<u8> {
        let lemma_rows = [NO_VECTOR_ROW, 1, 0, NO_VECTOR_ROW, 2];
        let rows: Vec<i8> = (0..12).map(|v| v as i8 - 6).collect();
        let centroids: Vec<i8> = vec![127, 0, 0, 0, 0, -127, 0, 0];
        encode_vectors(&VectorsSpec {
            dict_id: 0x0123_4567_89ab_cdef,
            dims: DIMS,
            cluster_count: 2,
            lemma_rows: &lemma_rows,
            rows: &rows,
            centroids: &centroids,
            meta: "source=test\n",
        })
        .unwrap()
    }

    #[test]
    fn roundtrip() {
        let bytes = sample();
        assert_eq!(bytes.len() % 8, 0);
        let l = parse_vectors_layout(&bytes).unwrap();
        assert_eq!(l.dict_id, 0x0123_4567_89ab_cdef);
        assert_eq!((l.dims, l.cluster_count, l.row_count), (4, 2, 3));
        assert_eq!(&bytes[l.meta.clone()], b"source=test\n");
        let lr = &bytes[l.lemma_rows.clone()];
        assert_eq!(read_lemma_row(lr, 0), None);
        assert_eq!(read_lemma_row(lr, 1), Some(1));
        assert_eq!(read_lemma_row(lr, 2), Some(0));
        assert_eq!(read_lemma_row(lr, 4), Some(2));
        assert_eq!(read_lemma_row(lr, 5), None);
        assert_eq!(read_lemma_row(lr, u32::MAX), None);
        let rows = &bytes[l.rows.clone()];
        assert_eq!(rows[0] as i8, -6);
        assert_eq!(rows[11] as i8, 5);
        let centroids = &bytes[l.centroids.clone()];
        assert_eq!((centroids[0] as i8, centroids[5] as i8), (127, -127));
    }

    #[test]
    fn encoding_is_deterministic() {
        assert_eq!(sample(), sample());
    }

    #[test]
    fn encode_rejects_inconsistent_inputs() {
        let base = |lemma_rows: &'static [u32], rows: &'static [i8], dims: u16| {
            encode_vectors(&VectorsSpec {
                dict_id: 0,
                dims,
                cluster_count: 0,
                lemma_rows,
                rows,
                centroids: &[],
                meta: "",
            })
        };
        assert!(base(&[], &[1, 2, 3, 4], 4).is_ok());
        assert!(base(&[], &[1, 2, 3], 4).is_err()); // dims の倍数でない
        assert!(base(&[], &[], 0).is_err());
        assert!(base(&[1], &[1, 2, 3, 4], 4).is_err()); // 行番号が範囲外
        assert!(
            encode_vectors(&VectorsSpec {
                dict_id: 0,
                dims: 4,
                cluster_count: 2,
                lemma_rows: &[],
                rows: &[],
                centroids: &[1, 2, 3],
                meta: "",
            })
            .is_err()
        );
    }

    #[test]
    fn bad_preamble() {
        let mut bytes = sample();
        bytes[0] = b'X';
        assert!(matches!(
            parse_vectors_layout(&bytes),
            Err(FormatError::BadMagic { .. })
        ));
        let mut bytes = sample();
        bytes[8..12].copy_from_slice(&9u32.to_le_bytes());
        assert!(matches!(
            parse_vectors_layout(&bytes),
            Err(FormatError::UnsupportedVersion { found: 9, .. })
        ));
        let mut bytes = sample();
        bytes[12..16].copy_from_slice(&80u32.to_le_bytes());
        assert!(matches!(
            parse_vectors_layout(&bytes),
            Err(FormatError::BadHeaderLen { .. })
        ));
    }

    #[test]
    fn every_truncation_is_an_error_without_panic() {
        let bytes = sample();
        let used = parse_vectors_layout(&bytes).unwrap().meta.end;
        for len in 0..used {
            assert!(
                parse_vectors_layout(&bytes[..len]).is_err(),
                "len {len} should fail"
            );
        }
        assert!(parse_vectors_layout(&bytes[..used]).is_ok());
    }

    #[test]
    fn single_byte_flips_never_panic() {
        let bytes = sample();
        for i in 0..bytes.len() {
            for mask in [0x01u8, 0x80, 0xff] {
                let mut b = bytes.clone();
                b[i] ^= mask;
                if let Ok(l) = parse_vectors_layout(&b) {
                    let lr = &b[l.lemma_rows.clone()];
                    for id in 0..8 {
                        let _ = read_lemma_row(lr, id);
                    }
                }
            }
        }
    }

    #[test]
    fn out_of_range_sections_are_errors() {
        let cases: [(usize, u64); 8] = [
            (OFF_LEMMA_ROWS_OFFSET, u64::MAX),
            (OFF_LEMMA_ROWS_OFFSET, 100), // 8 バイト境界でない
            (OFF_LEMMA_ROWS_LEN, 3),      // 4 の倍数でない
            (OFF_ROWS_OFFSET, 8),         // ヘッダと重なる
            (OFF_ROWS_LEN, 1 << 40),
            (OFF_CENTROIDS_LEN, 1),
            (OFF_META_OFFSET, u64::MAX - 7),
            (OFF_META_LEN, u64::MAX),
        ];
        for (off, value) in cases {
            let mut bytes = sample();
            bytes[off..off + 8].copy_from_slice(&value.to_le_bytes());
            assert!(
                parse_vectors_layout(&bytes).is_err(),
                "offset {off} value {value:#x}"
            );
        }
        let mut bytes = sample();
        bytes[OFF_DIMS..OFF_DIMS + 2].copy_from_slice(&0u16.to_le_bytes());
        assert!(parse_vectors_layout(&bytes).is_err());
        let mut bytes = sample();
        bytes[OFF_ROW_COUNT..OFF_ROW_COUNT + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_vectors_layout(&bytes).is_err());
    }
}
