//! ikari-dict-format: バイナリ辞書形式の定義・検証・エンコード、読みの正規化。
//!
//! IO は持たない（ADR 0001 §2）。`grammar.bin` と `system.dict` のレイアウトは ADR §3 を参照。
//! 契約: 定数、型、関数のシグネチャは凍結（内部の型の形は B が調整してよい）。

use std::fmt;

mod dict;
mod grammar;
pub mod postal;
pub mod reading;
mod util;
mod vectors;

pub const DICT_MAGIC: [u8; 8] = *b"IKARIDIC";
pub const GRAMMAR_MAGIC: [u8; 8] = *b"IKARIGRM";
/// system.dict と grammar.bin の形式バージョン（ADR 0002 §3）。v1 のファイルは読み込まない。
pub const FORMAT_VERSION: u32 = 2;
pub const DICT_HEADER_LEN: usize = 176;
pub const GRAMMAR_HEADER_LEN: usize = 96;
pub const ENTRY_SIZE: usize = 16;
pub const SECTION_ALIGN: usize = 8;
/// 索引に入れる読みの最大文字数。ラティスの最大語長も同じ値
pub const MAX_READING_CHARS: usize = 32;
pub const GRAMMAR_FILE_NAME: &str = "grammar.bin";
pub const SYSTEM_DICT_FILE_NAME: &str = "system.dict";
pub const DICT_KIND_SYSTEM: u32 = 0;
/// 分野別の語彙パック用（フェーズ1以降）
pub const DICT_KIND_EXTENSION: u32 = 1;
/// `ime_penalty` がこの値のエントリは、変換では使わない（解析には使える）。
pub const IME_EXCLUDED: i16 = i16::MAX;

pub const VECTORS_MAGIC: [u8; 8] = *b"IKARIVEC";
/// vectors.bin 自身の形式バージョン（system.dict とは独立）
pub const VECTORS_FORMAT_VERSION: u32 = 1;
pub const VECTORS_FILE_NAME: &str = "vectors.bin";
pub const VECTORS_HEADER_LEN: usize = 96;

/// 形式の検証エラー。パニックの代わりに、壊れた入力はすべてこれで返す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    /// ヘッダより短い、またはセクションがファイルに収まらない
    Truncated {
        what: &'static str,
        need: u64,
        actual: u64,
    },
    BadMagic {
        what: &'static str,
    },
    UnsupportedVersion {
        found: u32,
        expected: u32,
    },
    BadHeaderLen {
        found: u32,
        expected: u32,
    },
    /// セクションのオフセット／長さが範囲外、8バイト境界でない、または整合しない
    BadSection {
        name: &'static str,
        message: String,
    },
    /// fst の破損（`fst::Error` の文字列表現）
    Fst(String),
    /// 入力値が不正（エンコード時）
    InvalidInput(String),
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::Truncated { what, need, actual } => {
                write!(f, "{what} is truncated: need {need} bytes, got {actual}")
            }
            FormatError::BadMagic { what } => write!(f, "bad magic in {what}"),
            FormatError::UnsupportedVersion { found, expected } => write!(
                f,
                "unsupported format version {found} (expected {expected}); rebuild the dictionary"
            ),
            FormatError::BadHeaderLen { found, expected } => {
                write!(f, "bad header length {found} (expected {expected})")
            }
            FormatError::BadSection { name, message } => {
                write!(f, "bad section `{name}`: {message}")
            }
            FormatError::Fst(message) => write!(f, "corrupt fst index: {message}"),
            FormatError::InvalidInput(message) => write!(f, "invalid input: {message}"),
        }
    }
}

impl std::error::Error for FormatError {}

pub use dict::{
    DictEntryInput, DictLayout, DictSpec, RawEntry, compute_dict_id, encode_dict, index_for_each,
    index_get, pack_index_value, parse_dict_layout, read_lemma, read_lemma_id, read_posting,
    read_raw_entry, read_surface, unpack_index_value, verify_dict_id,
};
pub use grammar::{
    ConjRecord, GrammarLayout, GrammarSpec, PosRecord, compute_grammar_id, encode_grammar,
    matrix_cost, parse_grammar_layout, read_conj_rules, read_pos_record, verify_grammar_id,
};
pub use util::Fnv1a64;
pub use vectors::{
    NO_VECTOR_ROW, VectorsLayout, VectorsSpec, encode_vectors, parse_vectors_layout, read_lemma_row,
};
