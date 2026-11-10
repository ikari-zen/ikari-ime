//! Sudachi CSV から grammar.bin / system.dict を作る（ADR 0001 §3.7, §8）。
//!
//! 流れ: matrix.def を厳格に読む -> lex.csv を読んで行フィルタ -> 品詞表と未知語パラメータを決める
//! -> `ikari_dict_format::{encode_grammar, encode_dict}` -> `*.tmp` に書いて rename。

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use ikari_dict_format::{
    ConjRecord, DICT_KIND_SYSTEM, DictEntryInput, DictSpec, FORMAT_VERSION, GRAMMAR_FILE_NAME,
    GrammarSpec, PosRecord, SYSTEM_DICT_FILE_NAME, encode_dict, encode_grammar,
    parse_grammar_layout,
};

mod conj;
mod connection;
mod decompose;
mod emoji;
mod kun;
mod lex;
mod matrix;
mod numeral;
mod policy;
mod town;
mod vectors;
mod verify;
pub mod webwords;

pub use conj::{ConjStats, MIN_SUPPORT};
pub use connection::{ConnectionReport, SideReport};
pub use emoji::{EmojiError, EmojiReport, build_emoji_file, build_emoji_table};
pub use kun::{KUN_TABLE_TEXT, KunInfo, KunNotation, KunTable};
pub use policy::{
    ChiveVocab, ConnectionDeltas, DEFAULT_POLICY_TEXT, HomophoneInfo, NotcoreInfo, PREDICATES,
    Policy, PolicyRow, PolicyStats, Verdict, is_homophone_peer, is_kanji_peer, is_kanji_place_peer,
    is_noun_peer,
};
pub use town::{
    ChainConfig, DEFAULT_CHAIN_COST, DEFAULT_MUNICIPALITY_CHAIN_COST, DEFAULT_TOWN_COST,
    PostalConfig, PostalReport, TownConfig, TownReport, build_town_lex,
};
pub use vectors::{
    DEFAULT_CLUSTERS, DEFAULT_KMEANS_ITERS, VectorsConfig, VectorsReport, build_vectors,
    read_chive_vocab,
};
pub use verify::{VectorsVerifyReport, VerifyReport, verify, verify_vectors};

/// `BuildReport::skipped` の理由（ADR §3.7 の規則に対応）
pub const SKIP_LEFT_ID_MINUS1: &str = "left_id_minus1";
pub const SKIP_SYMBOL_POS: &str = "pos_whitespace_or_symbol";
pub const SKIP_READING_NOT_INDEXABLE: &str = "reading_not_indexable";
pub const SKIP_READING_TOO_LONG: &str = "reading_too_long";
pub const SKIP_DUPLICATE: &str = "duplicate";
/// notcore の行のうち、Core の品詞表にない品詞の行（ADR 0012 §2.3。notcore は品詞表を増やさない）
pub const SKIP_NOTCORE_UNKNOWN_POS: &str = "notcore_pos_not_in_core";
/// notcore の固有名詞のうち、同じ読みに固有名詞以外の語がある行（d2s。ADR 0012 §3.5）
pub const SKIP_NOTCORE_PROPER_HOMOPHONE: &str = "notcore_proper_same_reading_as_common";
/// notcore の固有名詞のうち、読みが短い行（d2s）
pub const SKIP_NOTCORE_PROPER_SHORT: &str = "notcore_proper_short_reading";

/// BOS/EOS の ID（Sudachi の matrix.def では 0）
const BOS_EOS_ID: u16 = 0;
const TMP_SUFFIX: &str = ".tmp";
/// ユーザー辞書のコストに使う、品詞ごとのコストのパーセンタイル（ADR 0002 §3.3）
const USER_COST_PERCENTILE: usize = 10;

pub struct BuildConfig {
    pub lex: Vec<PathBuf>,
    pub matrix: PathBuf,
    pub out_dir: PathBuf,
    pub unk_pos: String,
    pub unk_cost: i16,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildReport {
    pub rows_read: usize,
    pub skipped: BTreeMap<&'static str, usize>,
    pub entries: usize,
    /// `entries` のうち Full の notcore の行
    pub notcore_entries: usize,
    pub readings: usize,
    pub pos_count: usize,
    pub lemma_count: usize,
    /// ポリシーの述語ごとの集計
    pub policy: PolicyStats,
    pub policy_fingerprint: u64,
    pub conj_rules: usize,
    pub conj_stats: ConjStats,
    /// 連接表の補正（ADR 0054）。δ がどちらも 0 なら `None`
    pub connection: Option<ConnectionReport>,
    pub grammar_bytes: u64,
    pub dict_bytes: u64,
}

impl BuildReport {
    /// notcore の取り込みのレポート（Markdown）。notcore を入力しなかったときは空。
    pub fn notcore_markdown(&self) -> String {
        const SKIP_PREFIX: &str = "notcore_";
        let skipped: Vec<(&&str, &usize)> = self
            .skipped
            .iter()
            .filter(|(reason, _)| reason.starts_with(SKIP_PREFIX))
            .collect();
        if self.notcore_entries == 0 && skipped.is_empty() {
            return String::new();
        }
        let mut out = String::from("\n## Full の notcore の取り込み（ADR 0012 §11.3）\n\n");
        out.push_str(&format!(
            "- 辞書に入れた notcore の行: {}（全体 {}）\n",
            self.notcore_entries, self.entries
        ));
        out.push_str(
            "- 品詞の統計（代表の左右ID、`user_cost`）と活用規則は Core の行だけで数えた（U1）\n",
        );
        for (reason, count) in skipped {
            out.push_str(&format!("- 入れなかった行（`{reason}`）: {count}\n"));
        }
        out
    }

    /// 連接表の補正のレポート（Markdown）。補正しなかったときは空。
    pub fn connection_markdown(&self) -> String {
        self.connection
            .as_ref()
            .map(ConnectionReport::to_markdown)
            .unwrap_or_default()
    }
}

impl fmt::Display for BuildReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "rows read:   {}", self.rows_read)?;
        for (reason, count) in &self.skipped {
            writeln!(f, "  skipped ({reason}): {count}")?;
        }
        writeln!(f, "entries:     {}", self.entries)?;
        if self.notcore_entries > 0 {
            writeln!(f, "  of which notcore: {}", self.notcore_entries)?;
        }
        writeln!(f, "readings:    {}", self.readings)?;
        writeln!(f, "pos count:   {}", self.pos_count)?;
        writeln!(f, "lemmas:      {}", self.lemma_count)?;
        writeln!(
            f,
            "policy:      {:016x} (penalized {})",
            self.policy_fingerprint, self.policy.penalized
        )?;
        for (name, count) in self.policy.hits_by_predicate() {
            writeln!(f, "  policy hit ({name}): {count}")?;
        }
        for rule in policy::EXCLUSION_RULES {
            writeln!(
                f,
                "  excluded by fixed rule ({}): {}",
                rule.name(),
                self.policy.excluded[rule.index()]
            )?;
        }
        writeln!(
            f,
            "conj rules:  {} (candidates {}, no base {}, unusable {}, dropped pairs {})",
            self.conj_rules,
            self.conj_stats.candidates,
            self.conj_stats.skipped_no_base,
            self.conj_stats.skipped_unusable,
            self.conj_stats.dropped_pairs
        )?;
        if let Some(connection) = &self.connection {
            for (key, value) in connection.meta_lines() {
                writeln!(f, "{key}: {value}")?;
            }
        }
        writeln!(f, "grammar.bin: {} bytes", self.grammar_bytes)?;
        write!(f, "system.dict: {} bytes", self.dict_bytes)
    }
}

#[derive(Debug)]
pub enum BuildError {
    Io(std::io::Error),
    Format(ikari_dict_format::FormatError),
    /// 入力 CSV / matrix.def の不正（行番号付き）
    Input {
        file: PathBuf,
        line: usize,
        message: String,
    },
    /// 設定の不正（`--unk-pos` の品詞が見つからないなど）
    Config(String),
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::Io(e) => write!(f, "I/O error: {e}"),
            BuildError::Format(e) => write!(f, "format error: {e}"),
            BuildError::Input {
                file,
                line,
                message,
            } => {
                write!(f, "{}:{line}: {message}", file.display())
            }
            BuildError::Config(message) => write!(f, "config error: {message}"),
        }
    }
}

impl std::error::Error for BuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BuildError::Io(e) => Some(e),
            BuildError::Format(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for BuildError {
    fn from(e: std::io::Error) -> Self {
        BuildError::Io(e)
    }
}

impl From<ikari_dict_format::FormatError> for BuildError {
    fn from(e: ikari_dict_format::FormatError) -> Self {
        BuildError::Format(e)
    }
}

/// 入力ファイルを開く。失敗したらパス付きの I/O エラーにする。
pub(crate) fn open_reader(path: &Path) -> Result<BufReader<File>, BuildError> {
    const READ_BUFFER_BYTES: usize = 1 << 20;
    let file = File::open(path).map_err(|e| {
        BuildError::Io(io::Error::new(
            e.kind(),
            format!("cannot open {}: {e}", path.display()),
        ))
    })?;
    Ok(BufReader::with_capacity(READ_BUFFER_BYTES, file))
}

/// 1行を `buf` に読み（末尾の `\n` / `\r\n` は取り除く）、読んだバイト数を返す。0 なら EOF。
/// UTF-8 として不正な行は行番号付きの入力エラーにする。
pub(crate) fn read_line_at(
    reader: &mut BufReader<File>,
    buf: &mut String,
    path: &Path,
    lineno: usize,
) -> Result<usize, BuildError> {
    buf.clear();
    let n = reader.read_line(buf).map_err(|e| {
        if e.kind() == io::ErrorKind::InvalidData {
            BuildError::Input {
                file: path.to_path_buf(),
                line: lineno,
                message: "invalid UTF-8".to_string(),
            }
        } else {
            BuildError::Io(e)
        }
    })?;
    if buf.ends_with('\n') {
        buf.pop();
        if buf.ends_with('\r') {
            buf.pop();
        }
    }
    Ok(n)
}

/// `build` の追加の設定（`BuildConfig` は他の crate のテストが構造体リテラルで作るので、
/// 後から足す設定はここに置く）。
#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub policy: Policy,
    /// `lemma_rank` の判定に使う chiVe の語彙。`None` なら評価しない
    pub chive_vocab: Option<ChiveVocab>,
    /// メタ情報の `chive_vocab` に入れる版の表記（例: `chive-1.3-mc90`）
    pub chive_vocab_label: Option<String>,
    /// Full の notcore_lex.csv（`BuildConfig::lex` に続けて連結する入力。ADR 0012 §11.3）。
    /// 辞書の行にはなるが、品詞表、代表の左右ID、`user_cost`、活用規則には数えない（grammar.bin は Core だけのときと同じ）
    pub notcore_lex: Vec<PathBuf>,
    /// 住所の連なりの lex.csv（notcore と同じ扱いで、同じ行の Core にも勝つ。ADR 0030 §12）。`notcore_lex` のあとに読む
    pub chain_lex: Vec<PathBuf>,
    /// 連接表の補正の δ が閾値 T 以下でもビルドを失敗にしない（掃引用。ADR 0054 §5.1。既定は偽）
    pub allow_weak_connection: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            policy: Policy::builtin(),
            chive_vocab: None,
            chive_vocab_label: None,
            notcore_lex: Vec::new(),
            chain_lex: Vec::new(),
            allow_weak_connection: false,
        }
    }
}

/// 行ごとの `PolicyRow::kanji_peer_rank`。同じ読みの行（`lexicon.rows` は読みで整列済み）の中で、
/// 漢字の表記の行（`is_kanji_peer`）の lemma の chiVe の頻度順位が最も高いもの。語彙がなければすべて `None`。
fn kanji_peer_ranks(lexicon: &lex::Lexicon, vocab: Option<&ChiveVocab>) -> Vec<Option<usize>> {
    peer_ranks_by(lexicon, vocab, is_kanji_peer, PeerKey::Lemma)
}

/// 行ごとの `PolicyRow::noun_peer_rank`（ADR 0011）。`kanji_peer_ranks` と同じ形で、対象は漢字を含む名詞の行
/// （`is_noun_peer`。固有名詞を含む）。
fn noun_peer_ranks(lexicon: &lex::Lexicon, vocab: Option<&ChiveVocab>) -> Vec<Option<usize>> {
    peer_ranks_by(lexicon, vocab, is_noun_peer, PeerKey::Lemma)
}

/// 行ごとの `PolicyRow::place_peer_rank`（#223）。対象は漢字を含む地名の行（`is_kanji_place_peer`）。
/// 頻度は表記で引く（巴里・露西亜 のような当て字の lemma は ロンドン・ロシア のカタカナで、lemma で引くと高頻度になるため）
fn place_peer_ranks(lexicon: &lex::Lexicon, vocab: Option<&ChiveVocab>) -> Vec<Option<usize>> {
    peer_ranks_by(lexicon, vocab, is_kanji_place_peer, PeerKey::Surface)
}

/// 行ごとの `PolicyRow::place_peer_total`（#223）。同じ読みの行（`lexicon.rows` は読みで整列済み）の中で、
/// 漢字を含む名詞（`is_noun_peer`。地名に限らない。人名の 松田、一般の 大和 が先に出るのでそれにそろえる）のうち表記の chiVe 順位が `KATAKANA_PLACE_PEER_MAX_RANK` より上の行の、
/// 1 語だけを変換するときの経路コスト（BOS からの連接 + Sudachi のコスト + ポリシーの加算 + EOS への連接）の最小値を求め、
/// 読みの各行に「その行自身の連接を引いた値」（その行の コスト + 加算 と比べる値）として配る。
/// 品詞が違うと連接コストが違う（固有名詞の一般 と 地名）ので、語のコストだけでそろえると 1 語の順位が逆になる。語彙がなければすべて `None`。
fn place_peer_totals<'a>(
    lexicon: &lex::Lexicon,
    opts: &BuildOptions,
    matrix: &matrix::Matrix,
    make_row: &dyn Fn(usize, Option<i32>, Option<i32>) -> PolicyRow<'a>,
) -> Vec<Option<i32>> {
    let prev_right_size = usize::from(matrix.prev_right_size);
    // 1 語だけのときの BOS/EOS との連接コスト
    let connection = |i: usize| -> i32 {
        let row = &lexicon.rows[i].0;
        let bos =
            matrix.costs[usize::from(row.left_id) * prev_right_size + usize::from(BOS_EOS_ID)];
        let eos =
            matrix.costs[usize::from(BOS_EOS_ID) * prev_right_size + usize::from(row.right_id)];
        i32::from(bos) + i32::from(eos)
    };
    let mut totals = vec![None; lexicon.rows.len()];
    let Some(vocab) = opts.chive_vocab.as_ref() else {
        return totals;
    };
    let mut start = 0;
    while start < lexicon.rows.len() {
        let reading = &lexicon.rows[start].0.reading;
        let end = start
            + lexicon.rows[start..]
                .iter()
                .take_while(|(row, _)| &row.reading == reading)
                .count();
        let best = (start..end)
            .filter_map(|i| {
                let policy_row = make_row(i, None, None);
                if !is_noun_peer(&policy_row)
                    || vocab.rank_or_last(policy_row.surface)
                        >= policy::KATAKANA_PLACE_PEER_MAX_RANK
                {
                    return None;
                }
                let verdict = opts.policy.evaluate(&policy_row, Some(vocab));
                verdict.excluded.is_none().then(|| {
                    i32::from(lexicon.rows[i].0.cost) + i32::from(verdict.penalty) + connection(i)
                })
            })
            .min();
        for (i, total) in totals.iter_mut().enumerate().take(end).skip(start) {
            *total = best.map(|b| b - connection(i));
        }
        start = end;
    }
    totals
}

/// 行ごとの `PolicyRow::compound_parts_rank`（ADR 0035 規則 F）。`is_homophone_peer` の 4 文字以上（`COMPOUND_PARTS_MIN_CHARS`）の行について、
/// 辞書の全行の `PolicyRow::rare_kanji_variant`（`lexicon.rows` と同じ並び）。辞書形の表記は、行の辞書形 ID から引く（`lexicon.inflect`）。
/// 引けなければ行自身の表記を使う（`kun::kun_infos` と同じ）
fn rare_kanji_variant_flags(
    lexicon: &lex::Lexicon,
    kun_table: &KunTable,
    vocab: Option<&ChiveVocab>,
) -> Vec<bool> {
    let Some(vocab) = vocab else {
        return vec![false; lexicon.rows.len()];
    };
    lexicon
        .rows
        .iter()
        .map(|(row, pos_id)| {
            let dict_surface = row
                .dict_form
                .and_then(|id| {
                    lexicon
                        .inflect
                        .binary_search_by_key(&id, |r| r.id)
                        .ok()
                        .map(|k| lexicon.inflect[k].surface.as_str())
                })
                .unwrap_or(row.surface.as_str());
            policy::is_rare_kanji_variant(
                &lexicon.pos[usize::from(*pos_id)].name,
                &row.lemma,
                dict_surface,
                kun_table.get(&row.lemma).is_some(),
                |c| !vocab.contains(c.encode_utf8(&mut [0; 4])),
            )
        })
        .collect()
}

/// 表記と読みを、前と後がどちらも辞書の `is_homophone_peer` の行になるように 2 つに切る切り方を探し、
/// 切り方ごとの部品の lemma の chiVe の順位の大きいほうの最小値を求める。語彙がなければすべて `None`。
fn compound_parts_ranks(lexicon: &lex::Lexicon, vocab: Option<&ChiveVocab>) -> Vec<Option<usize>> {
    let mut ranks = vec![None; lexicon.rows.len()];
    let Some(vocab) = vocab else {
        return ranks;
    };
    let policy_row = |i: usize| {
        let (row, pos_id) = &lexicon.rows[i];
        PolicyRow {
            surface: &row.surface,
            lemma: &row.lemma,
            reading: &row.reading,
            pos: &lexicon.pos[usize::from(*pos_id)].name,
            kanji_peer_rank: None,
            noun_peer_rank: None,
            place_peer_rank: None,
            place_peer_total: None,
            compound_parts_rank: None,
            default_cost_peer_total: None,
            notcore: None,
            homophone: HomophoneInfo::default(),
            kun: KunInfo::default(),
            rare_kanji_variant: false,
        }
    };
    // (表記, 読み) -> 部品になれる行の lemma の順位の最小
    let mut parts: HashMap<(&str, &str), usize> = HashMap::new();
    for i in 0..lexicon.rows.len() {
        let row = policy_row(i);
        if is_homophone_peer(&row) {
            let rank = vocab.rank_or_last(row.lemma);
            parts
                .entry((row.surface, row.reading))
                .and_modify(|r| *r = (*r).min(rank))
                .or_insert(rank);
        }
    }
    for (i, slot) in ranks.iter_mut().enumerate() {
        let row = policy_row(i);
        if !is_homophone_peer(&row)
            || row.surface.chars().count() < policy::COMPOUND_PARTS_MIN_CHARS
        {
            continue;
        }
        let surface_cuts = row
            .surface
            .char_indices()
            .map(|(at, _)| at)
            .filter(|&at| at > 0);
        let reading_cuts: Vec<usize> = row
            .reading
            .char_indices()
            .map(|(at, _)| at)
            .filter(|&at| at > 0)
            .collect();
        let mut best: Option<usize> = None;
        for s_at in surface_cuts {
            let (s_head, s_tail) = row.surface.split_at(s_at);
            for &r_at in &reading_cuts {
                let (r_head, r_tail) = row.reading.split_at(r_at);
                if let (Some(&a), Some(&b)) =
                    (parts.get(&(s_head, r_head)), parts.get(&(s_tail, r_tail)))
                {
                    let worse = a.max(b);
                    best = Some(best.map_or(worse, |cur| cur.min(worse)));
                }
            }
        }
        *slot = best;
    }
    ranks
}

/// 行ごとの `PolicyRow::default_cost_peer_total`（ADR 0035 規則 H）。同じ読みの `is_homophone_peer` の行を集め、
/// `policy::default_cost_peer_totals` で W の行に V の総コストの最小を配る。総コストは `default_cost_homophone` を除くポリシーの加算を含む。
fn default_cost_peer_totals<'a>(
    lexicon: &lex::Lexicon,
    opts: &BuildOptions,
    make_row: &dyn Fn(usize) -> PolicyRow<'a>,
) -> Vec<Option<i32>> {
    let mut totals = vec![None; lexicon.rows.len()];
    let Some(vocab) = opts.chive_vocab.as_ref() else {
        return totals;
    };
    let mut start = 0;
    while start < lexicon.rows.len() {
        let reading = &lexicon.rows[start].0.reading;
        let end = start
            + lexicon.rows[start..]
                .iter()
                .take_while(|(row, _)| &row.reading == reading)
                .count();
        let mut index = Vec::new();
        let mut peers = Vec::new();
        for i in start..end {
            let row = make_row(i);
            if !is_homophone_peer(&row) {
                continue;
            }
            let verdict = opts.policy.evaluate(&row, Some(vocab));
            if verdict.excluded.is_some() {
                continue;
            }
            index.push(i);
            peers.push(policy::HomophonePeer {
                rank: vocab.rank_or_last(row.lemma),
                cost: lexicon.rows[i].0.cost,
                total: i32::from(lexicon.rows[i].0.cost) + i32::from(verdict.penalty),
                cheap_rare: verdict.hits(policy::CHEAP_RARE_HOMOPHONE),
            });
        }
        if peers.len() >= 2 {
            for (i, total) in index
                .into_iter()
                .zip(policy::default_cost_peer_totals(&peers))
            {
                totals[i] = total;
            }
        }
        start = end;
    }
    totals
}

/// 行ごとの `HomophoneInfo::peer_rank`（ADR 0016）。`kanji_peer_ranks` と同じ形で、対象は `is_homophone_peer` の行
fn homophone_peer_ranks(lexicon: &lex::Lexicon, vocab: Option<&ChiveVocab>) -> Vec<Option<usize>> {
    peer_ranks_by(lexicon, vocab, is_homophone_peer, PeerKey::Lemma)
}

/// `peer_ranks_by` が chiVe の頻度を引く文字列
#[derive(Clone, Copy)]
enum PeerKey {
    Lemma,
    Surface,
}

/// 同じ読みの行のうち `is_peer` を満たす行の lemma（`PeerKey::Surface` なら表記）の chiVe の頻度順位の最小値を、読みの全行に配る。
fn peer_ranks_by(
    lexicon: &lex::Lexicon,
    vocab: Option<&ChiveVocab>,
    is_peer: fn(&PolicyRow<'_>) -> bool,
    key: PeerKey,
) -> Vec<Option<usize>> {
    let mut ranks = vec![None; lexicon.rows.len()];
    let Some(vocab) = vocab else {
        return ranks;
    };
    let mut start = 0;
    while start < lexicon.rows.len() {
        let reading = &lexicon.rows[start].0.reading;
        let end = start
            + lexicon.rows[start..]
                .iter()
                .take_while(|(row, _)| &row.reading == reading)
                .count();
        let best = lexicon.rows[start..end]
            .iter()
            .filter(|(row, pos_id)| {
                is_peer(&PolicyRow {
                    surface: &row.surface,
                    lemma: &row.lemma,
                    reading: &row.reading,
                    pos: &lexicon.pos[usize::from(*pos_id)].name,
                    kanji_peer_rank: None,
                    noun_peer_rank: None,
                    place_peer_rank: None,
                    place_peer_total: None,
                    compound_parts_rank: None,
                    default_cost_peer_total: None,
                    notcore: None,
                    homophone: HomophoneInfo::default(),
                    kun: KunInfo::default(),
                    rare_kanji_variant: false,
                })
            })
            .map(|(row, _)| {
                vocab.rank_or_last(match key {
                    PeerKey::Lemma => &row.lemma,
                    PeerKey::Surface => &row.surface,
                })
            })
            .min();
        ranks[start..end].fill(best);
        start = end;
    }
    ranks
}

/// 行ごとの `NotcoreInfo::core_peer_rank`。同じ読みの行（`lexicon.rows` は読みで整列済み）の中で、notcore でない行の
/// lemma の chiVe の頻度順位が最も高いもの（語彙にある lemma だけ）。語彙がなければすべて `None`。
fn core_peer_ranks(lexicon: &lex::Lexicon, vocab: Option<&ChiveVocab>) -> Vec<Option<usize>> {
    let mut ranks = vec![None; lexicon.rows.len()];
    let Some(vocab) = vocab else {
        return ranks;
    };
    let mut start = 0;
    while start < lexicon.rows.len() {
        let reading = &lexicon.rows[start].0.reading;
        let end = start
            + lexicon.rows[start..]
                .iter()
                .take_while(|(row, _)| &row.reading == reading)
                .count();
        let best = lexicon.rows[start..end]
            .iter()
            .filter(|(row, _)| !row.notcore)
            .filter_map(|(row, _)| vocab.rank(&row.lemma))
            .min()
            .map(|rank| rank as usize);
        ranks[start..end].fill(best);
        start = end;
    }
    ranks
}

/// 品詞ごとの集計（品詞レコードの `user_cost` と `rep_share_permille` の元）。
#[derive(Default)]
struct PosTally {
    costs: Vec<i16>,
    rows: u32,
    rep_rows: u32,
}

/// 昇順に並べた値の10パーセンタイル。
fn percentile_10(sorted: &[i16]) -> i16 {
    let at = sorted.len().saturating_sub(1) * USER_COST_PERCENTILE / 100;
    sorted.get(at).copied().unwrap_or(0)
}

/// `build_with` を既定のオプション（同梱のポリシー、chiVe の語彙なし）で呼ぶ。
pub fn build(cfg: &BuildConfig) -> Result<BuildReport, BuildError> {
    build_with(cfg, &BuildOptions::default())
}

pub fn build_with(cfg: &BuildConfig, opts: &BuildOptions) -> Result<BuildReport, BuildError> {
    if cfg.lex.is_empty() {
        return Err(BuildError::Config(
            "at least one lex file is required".into(),
        ));
    }
    if cfg.label.contains(['\n', '\r']) {
        return Err(BuildError::Config("label must be a single line".into()));
    }
    if opts.policy.uses_chive_vocab() && opts.chive_vocab.is_none() {
        return Err(BuildError::Config(
            "the policy uses a predicate (`lemma_rank`, `katakana_analysis`, `cheap_rare_homophone`, ...) that needs --chive-vocab".into(),
        ));
    }

    let mut matrix = matrix::read_matrix(&cfg.matrix)?;
    let mut acc = lex::LexAccumulator::new(matrix.prev_right_size, matrix.next_left_size);
    for path in &cfg.lex {
        acc.add_file(path)?;
    }
    for path in &opts.notcore_lex {
        acc.add_notcore_file(path)?;
    }
    for path in &opts.chain_lex {
        acc.add_priority_notcore_file(path)?;
    }
    let mut lexicon = acc.finish();

    // 未知語のパラメータ: 指定した品詞とその代表左右ID
    let unk_index = lexicon
        .pos
        .iter()
        .position(|p| p.name == cfg.unk_pos)
        .ok_or_else(|| {
            BuildError::Config(format!(
                "unknown-word part of speech `{}` not found in the input",
                cfg.unk_pos
            ))
        })?;
    let unk_pos_id = u16::try_from(unk_index)
        .map_err(|_| BuildError::Config("too many parts of speech".into()))?;
    let (unk_left_id, unk_right_id) = {
        let unk = &lexicon.pos[unk_index];
        (unk.rep_left_id, unk.rep_right_id)
    };

    // ポリシー（ime_penalty）と、品詞ごとの集計
    let mut policy_stats = PolicyStats::default();
    let mut penalties: Vec<i16> = Vec::with_capacity(lexicon.rows.len());
    let mut tallies: Vec<PosTally> = lexicon.pos.iter().map(|_| PosTally::default()).collect();
    let peer_ranks = kanji_peer_ranks(&lexicon, opts.chive_vocab.as_ref());
    let noun_ranks = noun_peer_ranks(&lexicon, opts.chive_vocab.as_ref());
    let place_ranks = place_peer_ranks(&lexicon, opts.chive_vocab.as_ref());
    let homophone_ranks = homophone_peer_ranks(&lexicon, opts.chive_vocab.as_ref());
    let core_ranks = core_peer_ranks(&lexicon, opts.chive_vocab.as_ref());
    // 異字同訓の表（ADR 0009）。述語が書かれていなければ引かない
    let kun = if opts.policy.uses_kun_table() {
        kun::kun_infos(opts.policy.kun_table(), &lexicon)
    } else {
        vec![KunInfo::default(); lexicon.rows.len()]
    };
    let rare_kanji_variant = if opts.policy.effect(policy::RARE_KANJI_VARIANT).is_some() {
        rare_kanji_variant_flags(&lexicon, opts.policy.kun_table(), opts.chive_vocab.as_ref())
    } else {
        vec![false; lexicon.rows.len()]
    };
    let parts_ranks = compound_parts_ranks(&lexicon, opts.chive_vocab.as_ref());
    let make_row =
        |i: usize, place_peer_total: Option<i32>, default_cost_peer_total: Option<i32>| {
            let (row, pos_id) = &lexicon.rows[i];
            PolicyRow {
                surface: &row.surface,
                lemma: &row.lemma,
                reading: &row.reading,
                pos: &lexicon.pos[usize::from(*pos_id)].name,
                kanji_peer_rank: peer_ranks[i],
                noun_peer_rank: noun_ranks[i],
                place_peer_rank: place_ranks[i],
                place_peer_total,
                compound_parts_rank: parts_ranks[i],
                default_cost_peer_total,
                notcore: row.notcore.then_some(NotcoreInfo {
                    core_peer_rank: core_ranks[i],
                }),
                homophone: HomophoneInfo {
                    cost: row.cost,
                    peer_rank: homophone_ranks[i],
                },
                kun: kun[i],
                rare_kanji_variant: rare_kanji_variant[i],
            }
        };
    // `katakana_place_peer` が総コストをそろえる基準（同じ読みの最安の、頻度が高い漢字の地名の総コスト）
    let place_totals = if opts.policy.effect(policy::KATAKANA_PLACE_PEER).is_some() {
        place_peer_totals(&lexicon, opts, &matrix, &make_row)
    } else {
        vec![None; lexicon.rows.len()]
    };
    // `default_cost_homophone` が総コストをそろえる基準（同じ読みの、既定のコストどうしの、よりまれな同音語の総コストの最小）
    let homophone_totals = if opts.policy.effect(policy::DEFAULT_COST_HOMOPHONE).is_some() {
        default_cost_peer_totals(&lexicon, opts, &|i| make_row(i, place_totals[i], None))
    } else {
        vec![None; lexicon.rows.len()]
    };
    for (i, (row, pos_id)) in lexicon.rows.iter().enumerate() {
        let pos = &lexicon.pos[usize::from(*pos_id)];
        let policy_row = make_row(i, place_totals[i], homophone_totals[i]);
        let verdict = opts.policy.evaluate(&policy_row, opts.chive_vocab.as_ref());
        policy_stats.record(&policy_row, verdict);
        penalties.push(verdict.penalty);
        // 品詞の統計（user_cost、代表の左右ID）は Core の行だけで数える（ADR 0012 §13 の U1）
        // （優先の行が置き換えた Core の行は、その行のコストで数える）
        let stat_cost = match (row.notcore, row.replaced_core_cost) {
            (false, _) => row.cost,
            (true, Some(core_cost)) => core_cost,
            (true, None) => continue,
        };
        let tally = &mut tallies[usize::from(*pos_id)];
        tally.rows += 1;
        tally.costs.push(stat_cost);
        if (row.left_id, row.right_id) == (pos.rep_left_id, pos.rep_right_id) {
            tally.rep_rows += 1;
        }
    }

    // 連接表の補正（ADR 0054）。品詞の統計（代表の左右ID）を数えたあと、辞書の行とポリシーの判定が済んでから行う
    let connection = connection::apply(
        &mut lexicon,
        &penalties,
        &mut matrix,
        opts.policy.connection(),
        !opts.allow_weak_connection,
    )?;

    // 活用規則
    let (conj_rules, conj_stats) = conj::derive(&lexicon.inflect, &lexicon.pos_final);
    let conj_records: Vec<ConjRecord<'_>> = conj_rules
        .iter()
        .map(|r| ConjRecord {
            base_pos: r.base_pos,
            target_pos: r.target_pos,
            strip_chars: r.strip_chars,
            add_surface: &r.add_surface,
            add_reading: &r.add_reading,
        })
        .collect();

    let pos_records: Vec<PosRecord<'_>> = lexicon
        .pos
        .iter()
        .zip(tallies.iter_mut())
        .map(|(p, tally)| {
            tally.costs.sort_unstable();
            let source = &tally.costs;
            let share = if tally.rows == 0 {
                0
            } else {
                // 四捨五入した‰
                (u64::from(tally.rep_rows) * 1000 + u64::from(tally.rows) / 2)
                    / u64::from(tally.rows)
            };
            PosRecord {
                rep_left_id: p.rep_left_id,
                rep_right_id: p.rep_right_id,
                user_cost: percentile_10(source),
                rep_share_permille: share as u16, // 最大 1000
                name: &p.name,
            }
        })
        .collect();
    let grammar_bytes = encode_grammar(&GrammarSpec {
        prev_right_size: matrix.prev_right_size,
        next_left_size: matrix.next_left_size,
        bos_eos_id: BOS_EOS_ID,
        unk_left_id,
        unk_right_id,
        unk_pos_id,
        unk_cost: cfg.unk_cost,
        matrix: &matrix.costs,
        pos: &pos_records,
        conj: &conj_records,
    })?;
    let grammar_id = parse_grammar_layout(&grammar_bytes)?.grammar_id;
    drop(matrix);

    let entries: Vec<DictEntryInput<'_>> = lexicon
        .rows
        .iter()
        .zip(&penalties)
        .map(|((row, pos_id), &penalty)| DictEntryInput {
            reading: &row.reading,
            surface: &row.surface,
            left_id: row.left_id,
            right_id: row.right_id,
            cost: row.cost,
            pos_id: *pos_id,
            ime_penalty: penalty,
            lemma: &row.lemma,
        })
        .collect();
    // lexicon.rows は読みで整列済み
    let readings = entries
        .iter()
        .enumerate()
        .filter(|(i, e)| *i == 0 || entries[*i - 1].reading != e.reading)
        .count();
    let mut lemma_set: Vec<&str> = entries.iter().map(|e| e.lemma).collect();
    lemma_set.sort_unstable();
    lemma_set.dedup();

    let mut report = BuildReport {
        rows_read: lexicon.rows_read,
        skipped: lexicon.skipped.clone(),
        entries: entries.len(),
        notcore_entries: lexicon.rows.iter().filter(|(row, _)| row.notcore).count(),
        readings,
        pos_count: lexicon.pos.len(),
        lemma_count: lemma_set.len(),
        policy: policy_stats,
        policy_fingerprint: opts.policy.fingerprint(),
        conj_rules: conj_rules.len(),
        conj_stats,
        connection,
        grammar_bytes: 0,
        dict_bytes: 0,
    };
    drop(lemma_set);
    let meta = make_meta(cfg, opts, &report);
    let dict_bytes = encode_dict(&DictSpec {
        grammar_id,
        kind: DICT_KIND_SYSTEM,
        entries: &entries,
        meta: &meta,
    })?;
    report.grammar_bytes = grammar_bytes.len() as u64;
    report.dict_bytes = dict_bytes.len() as u64;

    write_atomically(
        &cfg.out_dir,
        &[
            (GRAMMAR_FILE_NAME, &grammar_bytes),
            (SYSTEM_DICT_FILE_NAME, &dict_bytes),
        ],
    )?;
    Ok(report)
}

/// `key=value\n` のメタ情報。日時は入れない（出力を決定的にするため）。
fn make_meta(cfg: &BuildConfig, opts: &BuildOptions, report: &BuildReport) -> String {
    let lex_names: Vec<String> = cfg
        .lex
        .iter()
        .map(|p| {
            p.file_name().map_or_else(
                || p.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            )
        })
        .collect();
    let mut meta = String::new();
    let mut line = |key: &str, value: &dyn fmt::Display| {
        meta.push_str(&format!("{key}={value}\n"));
    };
    line("source", &cfg.label);
    line("builder_version", &env!("CARGO_PKG_VERSION"));
    line("format_version", &FORMAT_VERSION);
    line("lex", &lex_names.join(","));
    line("unk_pos", &cfg.unk_pos);
    line("unk_cost", &cfg.unk_cost);
    line("rows_read", &report.rows_read);
    line("entries", &report.entries);
    if !opts.notcore_lex.is_empty() {
        let names: Vec<String> = opts
            .notcore_lex
            .iter()
            .map(|p| {
                p.file_name().map_or_else(
                    || p.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                )
            })
            .collect();
        line("notcore_lex", &names.join(","));
        line("notcore_entries", &report.notcore_entries);
    }
    if !opts.chain_lex.is_empty() {
        let names: Vec<String> = opts
            .chain_lex
            .iter()
            .map(|p| {
                p.file_name().map_or_else(
                    || p.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                )
            })
            .collect();
        line("chain_lex", &names.join(","));
    }
    line("readings", &report.readings);
    line("pos_count", &report.pos_count);
    line("lemma_count", &report.lemma_count);
    line("conj_rules", &report.conj_rules);
    line(
        "policy",
        &format_args!("{:016x}", report.policy_fingerprint),
    );
    if let (Some(vocab), Some(label)) = (&opts.chive_vocab, &opts.chive_vocab_label) {
        line("chive_vocab", &format_args!("{label}:{}", vocab.len()));
    }
    for (reason, count) in &report.skipped {
        line(&format!("skipped.{reason}"), count);
    }
    if let Some(connection) = &report.connection {
        for (key, value) in connection.meta_lines() {
            line(&key, &value);
        }
    }
    meta
}

/// 各ファイルを `<name>.tmp` に書いて同期し、すべて成功してから rename で置き換える。
/// 途中で失敗しても、壊れたファイルを最終名では残さない。
///
/// 2つの rename の間でクラッシュすると新旧の組み合わせになりうるが、core は open 時に
/// grammar_id を照合して拒否するので、壊れた組で動くことはない。
pub(crate) fn write_atomically(out_dir: &Path, files: &[(&str, &[u8])]) -> Result<(), BuildError> {
    fs::create_dir_all(out_dir)?;
    let tmp_path = |name: &str| out_dir.join(format!("{name}{TMP_SUFFIX}"));

    let write_all = || -> io::Result<()> {
        for (name, bytes) in files {
            let mut file = File::create(tmp_path(name))?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        Ok(())
    };
    if let Err(e) = write_all() {
        remove_tmp_files(out_dir, files);
        return Err(e.into());
    }
    for (name, _) in files {
        if let Err(e) = fs::rename(tmp_path(name), out_dir.join(name)) {
            remove_tmp_files(out_dir, files);
            return Err(e.into());
        }
    }
    Ok(())
}

fn remove_tmp_files(out_dir: &Path, files: &[(&str, &[u8])]) {
    for (name, _) in files {
        // 掃除の失敗は元のエラーを優先するため無視する
        let _ = fs::remove_file(out_dir.join(format!("{name}{TMP_SUFFIX}")));
    }
}
