//! 形式の実装が共有する小さな補助（リトルエンディアンの読み取り、セクション範囲の検証、FNV-1a）。

use std::ops::Range;

use crate::{FormatError, SECTION_ALIGN};

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a 64 の逐次ハッシュ。grammar_id、ポリシーの指紋、学習の特徴量などに使う（暗号用ではない）。ワークスペースでの唯一の実装。
#[derive(Debug, Clone)]
pub struct Fnv1a64(u64);

impl Fnv1a64 {
    pub fn new() -> Self {
        Self(FNV_OFFSET_BASIS)
    }

    /// `finish()` の値から続きを計算する（`from_state(h(a).finish())` に `b` を足すと `h(a ++ b)`）。
    pub fn from_state(state: u64) -> Self {
        Self(state)
    }

    pub fn update(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

impl Default for Fnv1a64 {
    fn default() -> Self {
        Self::new()
    }
}

/// `n` を `SECTION_ALIGN` の倍数に切り上げる。
pub(crate) fn align_up(n: usize) -> usize {
    n.div_ceil(SECTION_ALIGN) * SECTION_ALIGN
}

/// `bytes` を 8 バイト境界まで 0 で埋める。
pub(crate) fn pad_to_align(bytes: &mut Vec<u8>) {
    bytes.resize(align_up(bytes.len()), 0);
}

// 範囲外でもパニックせず None を返す読み取り。

pub(crate) fn get_u16(b: &[u8], off: usize) -> Option<u16> {
    let s = b.get(off..off.checked_add(2)?)?;
    Some(u16::from_le_bytes(s.try_into().ok()?))
}

pub(crate) fn get_u32(b: &[u8], off: usize) -> Option<u32> {
    let s = b.get(off..off.checked_add(4)?)?;
    Some(u32::from_le_bytes(s.try_into().ok()?))
}

pub(crate) fn get_u64(b: &[u8], off: usize) -> Option<u64> {
    let s = b.get(off..off.checked_add(8)?)?;
    Some(u64::from_le_bytes(s.try_into().ok()?))
}

/// ヘッダ読み取り用。長さは `check_preamble` で検証済みのはずだが、念のため Truncated にする。
pub(crate) struct HeaderReader<'a> {
    bytes: &'a [u8],
    what: &'static str,
}

impl<'a> HeaderReader<'a> {
    pub(crate) fn new(bytes: &'a [u8], what: &'static str) -> Self {
        Self { bytes, what }
    }

    fn truncated(&self, off: usize, size: usize) -> FormatError {
        FormatError::Truncated {
            what: self.what,
            need: (off + size) as u64,
            actual: self.bytes.len() as u64,
        }
    }

    pub(crate) fn u16(&self, off: usize) -> Result<u16, FormatError> {
        get_u16(self.bytes, off).ok_or_else(|| self.truncated(off, 2))
    }

    pub(crate) fn u32(&self, off: usize) -> Result<u32, FormatError> {
        get_u32(self.bytes, off).ok_or_else(|| self.truncated(off, 4))
    }

    pub(crate) fn u64(&self, off: usize) -> Result<u64, FormatError> {
        get_u64(self.bytes, off).ok_or_else(|| self.truncated(off, 8))
    }
}

/// マジック、バージョン、ヘッダ長を、この順に検証する。短い入力でもパニックしない。
pub(crate) fn check_preamble(
    bytes: &[u8],
    what: &'static str,
    magic: &[u8; 8],
    expected_version: u32,
    header_len: usize,
) -> Result<(), FormatError> {
    let truncated = |need: usize| FormatError::Truncated {
        what,
        need: need as u64,
        actual: bytes.len() as u64,
    };
    let found_magic = bytes.get(..8).ok_or_else(|| truncated(8))?;
    if found_magic != magic {
        return Err(FormatError::BadMagic { what });
    }
    let version = get_u32(bytes, 8).ok_or_else(|| truncated(12))?;
    if version != expected_version {
        return Err(FormatError::UnsupportedVersion {
            found: version,
            expected: expected_version,
        });
    }
    let found_len = get_u32(bytes, 12).ok_or_else(|| truncated(16))?;
    if found_len as usize != header_len {
        return Err(FormatError::BadHeaderLen {
            found: found_len,
            expected: header_len as u32,
        });
    }
    if bytes.len() < header_len {
        return Err(truncated(header_len));
    }
    Ok(())
}

/// セクション (offset, len) を検証して範囲にする。
/// 8バイト境界、`min_start` 以降（ヘッダや前のセクションと重ならない）、ファイル内に収まること。
pub(crate) fn check_section(
    name: &'static str,
    what: &'static str,
    file_len: usize,
    offset: u64,
    len: u64,
    min_start: usize,
) -> Result<Range<usize>, FormatError> {
    let bad = |message: String| FormatError::BadSection { name, message };
    let start = usize::try_from(offset).map_err(|_| bad(format!("offset {offset} too large")))?;
    if !start.is_multiple_of(SECTION_ALIGN) {
        return Err(bad(format!(
            "offset {start} is not {SECTION_ALIGN}-byte aligned"
        )));
    }
    if start < min_start {
        return Err(bad(format!(
            "offset {start} overlaps the preceding data (ends at {min_start})"
        )));
    }
    let len_usize = usize::try_from(len).map_err(|_| bad(format!("length {len} too large")))?;
    let end = start
        .checked_add(len_usize)
        .ok_or_else(|| bad("offset + length overflows".to_string()))?;
    if end > file_len {
        return Err(FormatError::Truncated {
            what,
            need: end as u64,
            actual: file_len as u64,
        });
    }
    Ok(start..end)
}
