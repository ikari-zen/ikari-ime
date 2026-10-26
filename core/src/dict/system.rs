//! mmap + fst のシステム辞書（ADR 0001 §4.1）。
//!
//! `system.dict` を mmap し、索引（fst::Map）を mmap の部分スライス上にゼロコピーで置く。
//! 開くときの検証は O(1) のものだけ（ヘッダ、セクション範囲、`entries_len == entry_count*16`、
//! `fst::Map::new` の成否、メタの UTF-8）。エントリは読むたびに範囲付きスライスと UTF-8 検査で確かめ、
//! 不正なエントリは飛ばす。全エントリの走査は builder の `--verify` が担う。

use std::borrow::Cow;
use std::fs::File;
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use fst::Streamer;
use fst::raw::Output;
use ikari_dict_format::{
    DictLayout, FormatError, GRAMMAR_FILE_NAME, MAX_READING_CHARS, SYSTEM_DICT_FILE_NAME,
    parse_dict_layout, read_lemma, read_lemma_id, read_posting, read_raw_entry, read_surface,
    unpack_index_value,
};
use memmap2::Mmap;

use super::{DictError, Dictionary, GrammarFile, Lemma, WordEntry, WordId};

/// 単体の辞書が返す `WordId.dict`（`DictionarySet` が付け替える）
const STANDALONE_DICT_ID: u8 = 0;
/// postings の1要素（u32 のエントリ番号）のバイト数
const POSTING_SIZE: usize = 4;
/// `prefault` が読む間隔（バイト）。OS のページ（4 KiB または 16 KiB）より小さければ、全ページに触れる
const PAGE_STRIDE: usize = 4096;

/// ファイルを読み取り専用で mmap する。
///
/// 空のファイルは mmap できない環境があるので、ここで形式エラー（切り詰め）にする。
pub(super) fn map_file(path: &Path, what: &'static str) -> Result<Arc<Mmap>, DictError> {
    let file = File::open(path).map_err(|e| {
        DictError::Io(std::io::Error::new(
            e.kind(),
            format!("cannot open {}: {e}", path.display()),
        ))
    })?;
    if file.metadata()?.len() == 0 {
        return Err(FormatError::Truncated {
            what,
            need: 1,
            actual: 0,
        }
        .into());
    }
    // SAFETY: マップしたファイルが他のプロセスやスレッドによって変更・切り詰めされないことが前提。
    // 変更されると、読み出しが未定義動作（実際には SIGBUS や不整合な値）になりうる。
    // この前提は「辞書の更新は別ファイルを作って rename で切り替える」運用（spec の切り替え方針、
    // builder の tmp -> rename）で守る。rename は既存の inode を書き換えないので、マップ中のファイルは
    // 古い内容のまま読み続けられる。
    let mmap = unsafe { Mmap::map(&file)? };
    Ok(Arc::new(mmap))
}

/// mmap の部分範囲。`fst::Map<Section>` が自己参照なしにスライスを所有するための型。
#[derive(Clone)]
pub(super) struct Section {
    mmap: Arc<Mmap>,
    range: Range<usize>,
}

impl Section {
    /// `range` は呼び出し側が mmap の長さに収まることを検証済みであること（`parse_*_layout` の結果）。
    /// 収まらない場合は空スライスとして振る舞い、パニックしない。
    pub(super) fn new(mmap: Arc<Mmap>, range: Range<usize>) -> Self {
        let range = if range.end <= mmap.len() && range.start <= range.end {
            range
        } else {
            0..0
        };
        Self { mmap, range }
    }
}

impl AsRef<[u8]> for Section {
    fn as_ref(&self) -> &[u8] {
        // new() で範囲を確認済み
        self.mmap.get(self.range.clone()).unwrap_or(&[])
    }
}

pub struct LoadedDict {
    pub dict: Arc<SystemDictionary>,
    pub grammar: Arc<GrammarFile>,
}

/// grammar.bin と system.dict を開き、grammar_id を照合する。
pub fn open_dir(dir: &Path) -> Result<LoadedDict, DictError> {
    let grammar = GrammarFile::open(&dir.join(GRAMMAR_FILE_NAME))?;
    let dict = SystemDictionary::open(&dir.join(SYSTEM_DICT_FILE_NAME))?;
    let (dict_id, grammar_id) = (dict.grammar_id(), grammar_id_of(&grammar));
    if dict_id != grammar_id {
        return Err(DictError::GrammarMismatch {
            dict: dict_id,
            grammar: grammar_id,
        });
    }
    Ok(LoadedDict {
        dict: Arc::new(dict),
        grammar: Arc::new(grammar),
    })
}

fn grammar_id_of(grammar: &GrammarFile) -> u64 {
    use super::Grammar;
    grammar.grammar_id()
}

/// 読みの fst のキー 1 つぶん。エントリ `first..first+count` が、この読みを持つ
struct ReadingSpan {
    first: u32,
    count: u32,
    /// `ReadingIndex::text` の中のバイト位置と長さ
    start: u32,
    len: u16,
}

/// エントリ番号から読みを引く逆引き表（ADR 0006 §9.3）。読みの fst を一度だけ順に読んで作る。
/// エントリは読みの順に並んでいる（fst の値の `first` は単調に増える）ので、二分探索で引く。
struct ReadingIndex {
    /// `first` の昇順
    spans: Vec<ReadingSpan>,
    text: String,
}

impl ReadingIndex {
    fn build(index: &fst::Map<Section>) -> Self {
        let mut spans: Vec<ReadingSpan> = Vec::new();
        let mut text = String::new();
        let mut stream = index.stream();
        while let Some((key, value)) = stream.next() {
            let Ok(reading) = std::str::from_utf8(key) else {
                continue;
            };
            let (Ok(start), Ok(len)) = (u32::try_from(text.len()), u16::try_from(reading.len()))
            else {
                continue;
            };
            let (first, count) = unpack_index_value(value);
            text.push_str(reading);
            spans.push(ReadingSpan {
                first,
                count,
                start,
                len,
            });
        }
        // 通常は fst のキー順で `first` が増える。そうでない辞書でも二分探索が成り立つように並べ直す
        if spans.windows(2).any(|w| w[0].first > w[1].first) {
            spans.sort_by_key(|s| s.first);
        }
        spans.shrink_to_fit();
        text.shrink_to_fit();
        Self { spans, text }
    }

    fn reading(&self, index: u32) -> Option<&str> {
        let at = self.spans.partition_point(|s| s.first <= index);
        let span = self.spans.get(at.checked_sub(1)?)?;
        if u64::from(index) >= u64::from(span.first) + u64::from(span.count) {
            return None;
        }
        let start = span.start as usize;
        self.text.get(start..start + usize::from(span.len))
    }
}

pub struct SystemDictionary {
    mmap: Arc<Mmap>,
    /// キー: 正規化した読み。値: `(first_entry_index << 32) | count`
    index: fst::Map<Section>,
    /// キー: 表記（`MAX_READING_CHARS` 文字以下）。値: `(postings の開始位置 << 32) | count`
    surface_index: fst::Map<Section>,
    layout: DictLayout,
    /// 最初に `reading` が呼ばれたとき（または `warm_reading_index`）に作る
    reading_index: OnceLock<ReadingIndex>,
}

impl SystemDictionary {
    pub fn open(path: &Path) -> Result<Self, DictError> {
        let mmap = map_file(path, "system.dict")?;
        // parse_dict_layout は fst::Map::new の成否も確かめる（O(1)）
        let layout = parse_dict_layout(&mmap)?;
        if std::str::from_utf8(&mmap[layout.meta.clone()]).is_err() {
            return Err(FormatError::BadSection {
                name: "meta",
                message: "not valid UTF-8".to_string(),
            }
            .into());
        }
        let index = fst::Map::new(Section::new(Arc::clone(&mmap), layout.index.clone()))
            .map_err(|e| FormatError::Fst(e.to_string()))?;
        let surface_index = fst::Map::new(Section::new(
            Arc::clone(&mmap),
            layout.surface_index.clone(),
        ))
        .map_err(|e| FormatError::Fst(e.to_string()))?;
        Ok(Self {
            mmap,
            index,
            surface_index,
            layout,
            reading_index: OnceLock::new(),
        })
    }

    /// 逆引き表を作る（作ってあれば何もしない）。起動のあとに裏で呼ぶ想定
    /// （作成は実辞書で約 0.1 秒、表は約 33 MiB。ADR 0006 §16.2）。
    pub fn warm_reading_index(&self) {
        self.reading_index();
    }

    /// mmap 全体のページを 1 度ずつ読んで、OS のページ表に載せる（約 0.1 秒、100 MiB の辞書で）。
    /// 実体は読み捨てる。最初の参照がページフォールトで 1〜数 ms 遅れるのを避ける
    pub fn prefault(&self) {
        let touched = self
            .mmap
            .iter()
            .step_by(PAGE_STRIDE)
            .fold(0u8, |acc, &b| acc ^ b);
        std::hint::black_box(touched);
    }

    fn reading_index(&self) -> &ReadingIndex {
        self.reading_index
            .get_or_init(|| ReadingIndex::build(&self.index))
    }

    /// ビルド時のメタ情報（`key=value\n`）。
    pub fn meta(&self) -> &str {
        self.slice(&self.layout.meta)
            .and_then(|b| std::str::from_utf8(b).ok())
            .unwrap_or("")
    }

    /// エントリ数。
    pub fn entry_count(&self) -> usize {
        self.layout.entry_count as usize
    }

    fn slice(&self, range: &Range<usize>) -> Option<&[u8]> {
        self.mmap.get(range.clone())
    }

    /// 添字からエントリを作る。範囲外や壊れたエントリは None。
    fn word_entry(&self, index: u32) -> Option<WordEntry<'_>> {
        let raw = read_raw_entry(self.slice(&self.layout.entries)?, index)?;
        let surface = read_surface(self.slice(&self.layout.strings)?, &raw)?;
        Some(WordEntry {
            id: WordId {
                dict: STANDALONE_DICT_ID,
                index,
            },
            surface,
            left_id: raw.left_id,
            right_id: raw.right_id,
            cost: raw.cost,
            pos_id: raw.pos_id,
            penalty: raw.ime_penalty,
        })
    }

    /// 表記の索引の値が指す postings の範囲を、エントリにして `f(entry)` に渡す。壊れたものは飛ばす。
    fn emit_postings(&self, value: u64, f: &mut dyn FnMut(WordEntry<'_>)) {
        let Some(postings) = self.slice(&self.layout.postings) else {
            return;
        };
        let (start, count) = unpack_index_value(value);
        // postings の長さを超える範囲は切り詰める（壊れた索引でも走査量を抑える）
        let total = u32::try_from(postings.len() / POSTING_SIZE).unwrap_or(u32::MAX);
        let end = start.saturating_add(count).min(total);
        for position in start..end {
            let entry = read_posting(postings, position).and_then(|i| self.word_entry(i));
            if let Some(entry) = entry {
                f(entry);
            }
        }
    }

    /// 索引の値が指すエントリ範囲を `f(entry)` に渡す。壊れたエントリは飛ばす。
    fn emit_range(&self, value: u64, f: &mut dyn FnMut(WordEntry<'_>)) {
        let (first, count) = unpack_index_value(value);
        // entry_count を超える範囲は切り詰める（壊れた索引でも走査量を抑える）
        let end = first.saturating_add(count).min(self.layout.entry_count);
        for index in first..end {
            if let Some(entry) = self.word_entry(index) {
                f(entry);
            }
        }
    }
}

/// fst の raw ノードを1バイトずつたどり、途中の final ノードごとに `f(一致したバイト長, 値)` を呼ぶ。
/// キーは `MAX_READING_CHARS` 文字までしか入らないので、そこで打ち切る。
fn for_each_prefix_value(map: &fst::Map<Section>, text: &str, f: &mut dyn FnMut(usize, u64)) {
    let fst = map.as_fst();
    let mut node = fst.root();
    let mut output: u64 = 0;
    for (chars, (start, ch)) in text.char_indices().enumerate() {
        if chars >= MAX_READING_CHARS {
            return;
        }
        let end = start + ch.len_utf8();
        for &byte in &text.as_bytes()[start..end] {
            let Some(i) = node.find_input(byte) else {
                return;
            };
            let transition = node.transition(i);
            output = output.wrapping_add(transition.out.value());
            node = fst.node(transition.addr);
        }
        if node.is_final() {
            f(end, output.wrapping_add(Output::value(node.final_output())));
        }
    }
}

impl Dictionary for SystemDictionary {
    /// fst の raw ノードを1バイトずつたどり、途中の final ノードごとに一致を報告する。
    /// 読みは `MAX_READING_CHARS` 文字までしかキーに入らないので、そこで打ち切る。
    ///
    /// 注: 索引の中身（遷移先のアドレス）が壊れている場合の挙動は fst に依存する。open 時の検証は
    /// `fst::Map::new` までで、全体の整合は builder の `--verify` で確かめる。
    fn common_prefix_search(&self, key: &str, f: &mut dyn FnMut(usize, WordEntry<'_>)) {
        for_each_prefix_value(&self.index, key, &mut |end, value| {
            self.emit_range(value, &mut |entry| f(end, entry));
        });
    }

    /// 表記の索引（fst）を同じ方法でたどり、postings の順（表記ごとに (cost, index) の昇順）に返す。
    fn common_prefix_search_surface(&self, text: &str, f: &mut dyn FnMut(usize, WordEntry<'_>)) {
        for_each_prefix_value(&self.surface_index, text, &mut |end, value| {
            self.emit_postings(value, &mut |entry| f(end, entry));
        });
    }

    fn lemma(&self, id: WordId) -> Option<Lemma<'_>> {
        if id.dict != STANDALONE_DICT_ID {
            return None;
        }
        let lemma_id = read_lemma_id(self.slice(&self.layout.lemma_ids)?, id.index)?;
        let text = read_lemma(
            self.slice(&self.layout.lemma_table)?,
            self.layout.lemma_count,
            lemma_id,
        )?;
        Some(Lemma {
            text,
            id: Some(lemma_id),
        })
    }

    fn dict_id(&self) -> Option<u64> {
        Some(self.layout.dict_id)
    }

    fn lookup_exact(&self, reading: &str, f: &mut dyn FnMut(WordEntry<'_>)) {
        if let Some(value) = self.index.get(reading) {
            self.emit_range(value, f);
        }
    }

    fn entry(&self, id: WordId) -> Option<WordEntry<'_>> {
        if id.dict != STANDALONE_DICT_ID {
            return None;
        }
        self.word_entry(id.index)
    }

    fn grammar_id(&self) -> u64 {
        self.layout.grammar_id
    }

    fn reading(&self, id: WordId) -> Option<Cow<'_, str>> {
        if id.dict != STANDALONE_DICT_ID {
            return None;
        }
        // 範囲外や壊れたエントリは `entry` と同じく None
        self.word_entry(id.index)?;
        self.reading_index().reading(id.index).map(Cow::Borrowed)
    }

    fn warm_reading_index(&self) {
        SystemDictionary::warm_reading_index(self);
    }

    fn prefault(&self) {
        SystemDictionary::prefault(self);
    }
}
