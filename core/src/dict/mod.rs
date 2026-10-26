//! 辞書と文法の抽象（ADR 0001 §4.1）。

use std::borrow::Cow;
use std::fmt;

mod grammar_file;
mod memory;
mod set;
mod system;

pub use grammar_file::GrammarFile;
pub use memory::{DenseGrammar, InMemoryDictionary};
pub use set::DictionarySet;
pub(crate) use set::SYSTEM_SLOT;
pub use system::{LoadedDict, SystemDictionary, open_dir};

/// `ime_penalty` がこの値のエントリは変換では使わない（解析には使う）。ラティスはノードにしない。
pub use ikari_dict_format::{IME_EXCLUDED, VECTORS_FILE_NAME};

/// 単体の辞書は dict=0 で返し、DictionarySet が付け替える。永続化には使わない。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WordId {
    pub dict: u8,
    pub index: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct WordEntry<'a> {
    pub id: WordId,
    pub surface: &'a str,
    pub left_id: u16,
    pub right_id: u16,
    pub cost: i16,
    pub pos_id: u16,
    /// 変換時に cost へ足す値。`IME_EXCLUDED` なら変換では使わない。文脈解析では足さない。
    /// 構築は辞書の実装の中だけで行う（フィールドを足しても外部は壊れない）。
    pub penalty: i16,
}

/// lemma（Sudachi の正規化表記）。
#[derive(Clone, Copy, Debug)]
pub struct Lemma<'a> {
    pub text: &'a str,
    /// システム辞書の lemma 番号（ベクトルを引く用）。それ以外の辞書では None
    pub id: Option<u32>,
}

pub trait Dictionary: Send + Sync {
    /// key の先頭から始まり、読みが key の接頭辞と完全一致するエントリをすべて列挙する。
    /// f(一致した読みのバイト長, entry)。同じ読みの中ではコスト昇順。読みは MAX_READING_CHARS 以下。
    fn common_prefix_search(&self, key: &str, f: &mut dyn FnMut(usize, WordEntry<'_>));
    /// 読みが完全一致するエントリを列挙する。
    fn lookup_exact(&self, reading: &str, f: &mut dyn FnMut(WordEntry<'_>));
    /// 語IDからエントリを引く。範囲外や壊れたエントリなら None。
    fn entry(&self, id: WordId) -> Option<WordEntry<'_>>;
    fn grammar_id(&self) -> u64;

    /// 表記が text の接頭辞と一致するエントリを列挙する。f(一致した表記のバイト長, entry)。
    /// 文脈解析（左文脈の表記から語を引く）に使う。表記は MAX_READING_CHARS 文字以下のものだけが対象。
    /// 既定の実装は何も返さない。
    fn common_prefix_search_surface(&self, text: &str, f: &mut dyn FnMut(usize, WordEntry<'_>)) {
        let _ = (text, f);
    }

    /// lemma（正規化表記）。id は「この辞書の lemma 番号」で、DictionarySet では slot 0 のものだけ Some。
    fn lemma(&self, id: WordId) -> Option<Lemma<'_>> {
        let _ = id;
        None
    }

    /// この語がユーザー自身が登録した辞書（ユーザー辞書）の語か。同梱の語彙集やシステム辞書は false。
    /// 英字を1位に残す判断（ADR 0020 E3）に使う。既定は false。
    fn is_user_authored(&self, id: WordId) -> bool {
        let _ = id;
        false
    }

    /// system.dict の dict_id（vectors.bin の照合用）。なければ None。
    fn dict_id(&self) -> Option<u64> {
        None
    }

    /// 語の読み（正規化済みのひらがな）。再変換で表記から読みを求めるために使う（ADR 0006 §9.3）。
    /// 既定は None。`SystemDictionary` は読みの fst から作った逆引き表で、`InMemoryDictionary` は持っている読みで返す。
    fn reading(&self, id: WordId) -> Option<Cow<'_, str>> {
        let _ = id;
        None
    }

    /// `reading` の逆引き表を先に作る（作ってあれば何もしない）。既定は何もしない。
    /// 起動のあとに裏で呼び、最初の再変換が表の作成を待たないようにする。
    fn warm_reading_index(&self) {}

    /// mmap した辞書のページを先に読み込む（既定は何もしない）。起動のあとに裏で呼び、最初の変換や再変換が
    /// ページフォールトで遅れないようにする（ファイルが OS のキャッシュにあっても、初回の参照は 1〜数 ms かかる）。
    fn prefault(&self) {}
}

#[derive(Clone, Copy, Debug)]
pub struct UnknownParams {
    pub left_id: u16,
    pub right_id: u16,
    pub pos_id: u16,
    pub cost_per_char: i16,
}

/// 品詞表の v2 レコードのうち、ユーザー辞書が使う部分（ADR 0002 §3.3）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PosInfo {
    pub rep_left_id: u16,
    pub rep_right_id: u16,
    /// ユーザー辞書の語のコスト（その品詞のシステム辞書のコストの10パーセンタイル）
    pub user_cost: i16,
    /// 代表の左右IDと一致するエントリの割合（‰）
    pub rep_share_permille: u16,
}

/// 活用規則 1件（ADR 0002 §3.3）。終止形から別の活用形を導く。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConjRule {
    pub base_pos: u16,
    pub target_pos: u16,
    /// 終止形の末尾から削る文字数
    pub strip_chars: u8,
    pub add_surface: String,
    pub add_reading: String,
}

pub trait Grammar: Send + Sync {
    /// ID が範囲外なら i16::MAX を返す（その接続は事実上禁止。パニックしない）。
    fn connection_cost(&self, prev_right_id: u16, next_left_id: u16) -> i16;
    fn bos_eos_id(&self) -> u16;
    fn unknown(&self) -> UnknownParams;
    /// 例: "名詞,普通名詞,一般,*,*,*"
    fn pos_name(&self, pos_id: u16) -> Option<&str>;
    fn grammar_id(&self) -> u64;

    /// 品詞名から pos_id を引く。既定は None（v2 の品詞表を持たない文法）。
    fn pos_id(&self, name: &str) -> Option<u16> {
        let _ = name;
        None
    }

    /// 品詞の代表左右IDとユーザー辞書のコスト。既定は None。
    fn pos_info(&self, pos_id: u16) -> Option<PosInfo> {
        let _ = pos_id;
        None
    }

    /// 活用規則（`base_pos` ごとの目標の活用形）。既定は空。
    fn conj_rules(&self) -> &[ConjRule] {
        &[]
    }
}

#[derive(Debug)]
pub enum DictError {
    Io(std::io::Error),
    /// マジック、バージョン、範囲外のオフセット、fst の破損など
    Format(ikari_dict_format::FormatError),
    GrammarMismatch {
        dict: u64,
        grammar: u64,
    },
}

impl fmt::Display for DictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DictError::Io(e) => write!(f, "dictionary I/O error: {e}"),
            DictError::Format(e) => write!(f, "dictionary format error: {e}"),
            DictError::GrammarMismatch { dict, grammar } => write!(
                f,
                "grammar mismatch: dictionary expects grammar_id={dict:#018x}, grammar has {grammar:#018x}"
            ),
        }
    }
}

impl std::error::Error for DictError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DictError::Io(e) => Some(e),
            DictError::Format(e) => Some(e),
            DictError::GrammarMismatch { .. } => None,
        }
    }
}

impl From<std::io::Error> for DictError {
    fn from(e: std::io::Error) -> Self {
        DictError::Io(e)
    }
}

impl From<ikari_dict_format::FormatError> for DictError {
    fn from(e: ikari_dict_format::FormatError) -> Self {
        DictError::Format(e)
    }
}
