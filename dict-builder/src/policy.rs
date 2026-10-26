//! 変換用のコスト調整ポリシー（ADR 0002 §4）。
//!
//! 辞書の行は落とさず、エントリの `ime_penalty` として持たせる。変換では `cost + penalty`、
//! 文脈解析では `cost` だけを使う。述語はここ（コード）に固定し、値はデータファイル
//! （`policy/default.tsv`。列は `述語<TAB>値`）に書く。値は整数（加算）。
//!
//! ポリシーに除外の仕組みはない（基本原則: 候補の選択肢を不当に制限しない）。例外は [`ExclusionRule`] の
//! 固定の規則だけ（表記と読みの整合。ユーザーが入力していない記号を足した表記を出さないため。値ではなくコードに固定する）。

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use ikari_dict_format::{Fnv1a64, IME_EXCLUDED};

use crate::kun::{KunInfo, KunTable};

/// 同梱の既定ポリシー
pub const DEFAULT_POLICY_TEXT: &str = include_str!("../policy/default.tsv");

pub const ASCII_LOWER: &str = "ascii_lower";
pub const ASCII_OTHER: &str = "ascii_other";
pub const NUMERIC_SURFACE: &str = "numeric_surface";
pub const KATAKANA_VARIANT: &str = "katakana_variant";
pub const SYMBOL_KANA: &str = "symbol_kana";
pub const PROPER_NOUN: &str = "proper_noun";
pub const PERSON_NAME: &str = "person_name";
pub const PLACE_NAME: &str = "place_name";
pub const WAVE_DASH: &str = "wave_dash";
pub const LEMMA_RANK: &str = "lemma_rank";
pub const VARIANT_SURFACE: &str = "variant_surface";
pub const KATAKANA_ANALYSIS: &str = "katakana_analysis";
pub const FUNCTION_WORD_NOTATION: &str = "function_word_notation";
pub const FREQUENT_COMPOUND: &str = "frequent_compound";
pub const KUN_OUTSIDE: &str = "kun_outside";
pub const KUN_LEMMA_DEFAULT: &str = "kun_lemma_default";
pub const KUN_READING_DEFAULT: &str = "kun_reading_default";
pub const EXTENSION_PEER: &str = "extension_peer";
pub const CHEAP_RARE_HOMOPHONE: &str = "cheap_rare_homophone";
pub const OFFICIAL_KA_COUNTER: &str = "official_ka_counter";
pub const KATAKANA_PLACE_PEER: &str = "katakana_place_peer";
pub const DEFAULT_COST_HOMOPHONE: &str = "default_cost_homophone";

/// 述語の一覧。添字が `Verdict::hit_mask` のビット位置。
pub const PREDICATES: [&str; 22] = [
    ASCII_LOWER,
    ASCII_OTHER,
    NUMERIC_SURFACE,
    KATAKANA_VARIANT,
    SYMBOL_KANA,
    PROPER_NOUN,
    PERSON_NAME,
    WAVE_DASH,
    LEMMA_RANK,
    VARIANT_SURFACE,
    KATAKANA_ANALYSIS,
    FUNCTION_WORD_NOTATION,
    FREQUENT_COMPOUND,
    KUN_OUTSIDE,
    KUN_LEMMA_DEFAULT,
    KUN_READING_DEFAULT,
    EXTENSION_PEER,
    CHEAP_RARE_HOMOPHONE,
    PLACE_NAME,
    OFFICIAL_KA_COUNTER,
    KATAKANA_PLACE_PEER,
    DEFAULT_COST_HOMOPHONE,
];
const IDX_ASCII_LOWER: usize = 0;
const IDX_ASCII_OTHER: usize = 1;
const IDX_NUMERIC_SURFACE: usize = 2;
const IDX_KATAKANA_VARIANT: usize = 3;
const IDX_SYMBOL_KANA: usize = 4;
const IDX_PROPER_NOUN: usize = 5;
const IDX_PERSON_NAME: usize = 6;
const IDX_WAVE_DASH: usize = 7;
const IDX_LEMMA_RANK: usize = 8;
const IDX_VARIANT_SURFACE: usize = 9;
const IDX_KATAKANA_ANALYSIS: usize = 10;
const IDX_FUNCTION_WORD_NOTATION: usize = 11;
const IDX_FREQUENT_COMPOUND: usize = 12;
const IDX_KUN_OUTSIDE: usize = 13;
const IDX_KUN_LEMMA_DEFAULT: usize = 14;
const IDX_KUN_READING_DEFAULT: usize = 15;
const IDX_EXTENSION_PEER: usize = 16;
const IDX_CHEAP_RARE_HOMOPHONE: usize = 17;
const IDX_PLACE_NAME: usize = 18;
const IDX_OFFICIAL_KA_COUNTER: usize = 19;
const IDX_KATAKANA_PLACE_PEER: usize = 20;
const IDX_DEFAULT_COST_HOMOPHONE: usize = 21;

const POS_SEPARATOR: char = ',';
/// 長音記号「ー」(U+30FC)
const PROLONGED_SOUND_MARK: char = '\u{30FC}';
const POS1_SYMBOL: &str = "記号";
const POS2_NUMERAL: &str = "数詞";
const POS2_PROPER_NOUN: &str = "固有名詞";
const POS1_NOUN: &str = "名詞";
const POS2_COMMON_NOUN: &str = "普通名詞";
const POS3_PERSON_NAME: &str = "人名";
const POS3_PLACE_NAME: &str = "地名";
const POS3_COUNTABLE: &str = "助数詞可能";
/// 公用文の「か月」「か国」「か所」（ADR 0019 §17）。表記は、ひらがなの「か」と、この漢字 1 字
const KA_COUNTER_HIRAGANA: char = 'か';
/// 異体の「箇」(U+7B87)。公用文の か月・か国・か所 の lemma は 箇月・箇国・箇所
const KA_COUNTER_KANJI: char = '箇';
const KA_COUNTER_SUFFIXES: [char; 3] = ['月', '国', '所'];
/// 補助記号（Sudachi の品詞1。記号そのものの項目）
const POS1_AUX_SYMBOL: &str = "補助記号";
/// 機能語の品詞1（`function_word_notation` の対象）
const FUNCTION_WORD_POS1: [&str; 3] = ["助詞", "助動詞", "接尾辞"];
/// 助詞・助動詞のカタカナ表記（タ、ダ、マス、ネ。Sudachi の解析用の表記）を `function_word_notation` の対象にする品詞1。
/// 接尾辞は除く（カ月 など、実際に使う表記がある）
const FUNCTION_WORD_KATAKANA_POS1: [&str; 2] = ["助詞", "助動詞"];
/// 助詞・助動詞でも、カタカナの表記が正しい字（ヶ U+30F6、ヵ U+30F5。「ヶ月」「ヶ所」）
const KATAKANA_SMALL_KE: [char; 2] = ['\u{30F6}', '\u{30F5}'];
/// 漢数字のゼロ「〇」(U+3007)。読み「れい」「ぜろ」の数詞として辞書にある
const KANJI_ZERO: &str = "\u{3007}";
/// 品詞の5列目（活用型）が活用しないことを表す値
const CONJ_TYPE_NONE: &str = "*";
const POS_CONJ_TYPE_FIELD: usize = 4;
/// ascii_lower に含められる、英小文字と数字以外の文字
const ASCII_LOWER_PUNCT: [char; 5] = [' ', '.', '\'', '&', '-'];
/// `lemma_rank` が無料にする頻度順位の桁数（順位 1000 位までは加算しない）
/// 表記と読みの整合の規則（[`ExclusionRule`]）で、読みの長音と対応する波ダッシュの類（〜 U+301C、～ U+FF5E）
const WAVE_DASH_MARKS: [char; 2] = ['\u{301C}', '\u{FF5E}'];
/// 文末や語末に飾りとして付く記号（読みに対応する音がない）。語の一部になりうる記号（☆印の☆、中点、ハイフン）は入れない
const DECORATION_MARKS: [char; 10] = ['！', '？', '!', '?', '。', '…', '‥', '♪', '♫', '♬'];
/// ハイフンとダッシュの類（‐ U+2010、‑ U+2011、‒ U+2012、– U+2013、— U+2014、― U+2015、− U+2212、- U+002D、－ U+FF0D）。
/// 長音記号「ー」(U+30FC) と波ダッシュは入れない
const HYPHEN_MARKS: [char; 9] = [
    '\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}', '\u{2015}', '\u{2212}', '-',
    '\u{FF0D}',
];
/// `frequent_compound` の対象にする表記の長さの下限（文字数）。1 語の複合語（具体例、勉強会）
const COMPOUND_MIN_CHARS: usize = 3;
/// `frequent_compound` の対象にする、lemma の chiVe の頻度順位の上限（この順位より上の語だけ）。
/// 3 万では 構造化（31404 位）、拡張性（33312 位）、規格外（30606 位）が漏れた。dev では 3.5 万〜5 万が同じ結果で、
/// 6 万から 伊富魚（56691 位）が 解凍 に勝つ。平坦な範囲の手前寄りの 4 万にした（eval/reports/phase3-suffix.md）
const COMPOUND_MAX_RANK: usize = 40_000;
/// `frequent_compound` を、同音の守り（`PolicyRow::noun_peer_rank`）つきで広げる上限（ADR 0011）。
/// 4 万位以上 20 万位未満の熟語は、同じ読みの漢字を含む名詞の中で chiVe の順位が最も高いときだけ対象にする。
/// 守りなしで広げると 伊富魚、総指揮、史料館 などが 解凍、葬式、資料館 に勝つ。形態素（83455 位）を含む。
/// 10 万から 20 万に広げたのは 公用文（194216 位。こうようぶん が 紅葉分 に負けた。#283。ADR 0011 §13）。
/// 20 万を超える語は、守りがあっても当たる語が増えて効き方を見積もれない（30 万、上限なしは 蛙化現象 を直すが、セットの外の効きが読めない）
const COMPOUND_GUARDED_MAX_RANK: usize = 200_000;
/// `frequent_compound` の (f)（部品の頻度。ADR 0035 規則 F）の対象にする表記の長さの下限（文字数）。
/// ADR 0011 §7-1 の崩れ（高修正する、公家領）は 1 文字の部品を含む 3 文字の語だった
pub(crate) const COMPOUND_PARTS_MIN_CHARS: usize = 4;
/// `frequent_compound` の (f): 2 つの部品の lemma の chiVe の順位が、どちらもこの順位より上のとき当たる。
/// 掃引（3000、1 万、3 万、10 万。eval/reports/path-loses-250.md）で、1 万〜10 万が同じ結果（悪化 0）、3000 では 異常気象（気象 7340 位）が漏れる。
/// 救いが同じなかで当たる行がいちばん少ない 1 万にした（ADR 0035 §4.6）
const COMPOUND_PARTS_MAX_RANK: usize = 10_000;
/// 既定のコストどうしの同音語（`default_cost_homophone`。ADR 0035 規則 H）の、Sudachi のコストの既定の帯（両端を含む）。
/// 漢字だけの 2 文字以上の普通名詞（表記 = lemma）のコストの山: サ変可能 6508〜6518（6518 が 6436 行）、一般 7380〜7404（7404 が 28762 行）
const DEFAULT_COST_BAND: std::ops::RangeInclusive<i16> = 6500..=7410;
/// `default_cost_homophone`: W（同じ読みの同音語の中で頻度がいちばん高い語）の lemma の chiVe の順位の上限（この順位より上の語だけ）。
/// 掃引で 5 万〜20 万が同じ結果（看守 は 45724 位）。中心の 10 万
const DEFAULT_COST_PEER_MAX_RANK: usize = 100_000;
/// `default_cost_homophone`: V が W よりまれな桁数の下限（`log10(順位 + 1)` の差）。0 では 正確／性格 が崩れ、0.15〜0.5 は悪化 0。
/// 0.15 は 気象／起床（0.19 桁）を含む
const DEFAULT_COST_MIN_DECADE_GAP: f64 = 0.15;
/// `katakana_place_peer`: 同じ読みの、漢字の地名（`is_kanji_place_peer`）の表記が、chiVe の頻度順位でこの順位より上のとき当たる。
/// 渋谷（3271 位）、横浜（1564 位）、鎌倉（4455 位）、奄美（10927 位）は入り、巴里・倫敦・羅馬・伯林・露西亜（語彙にない）、
/// 雅典（82716 位）、厦門（84864 位）、桑港（314655 位）のような外来語の地名の当て字は入らない。間の 5 万位で分ける（#223）
pub const KATAKANA_PLACE_PEER_MAX_RANK: usize = 50_000;
const LEMMA_RANK_FREE_DECADES: f64 = 3.0;
/// `extension_peer`: 同じ読みの Core の語の lemma が、chiVe の頻度順位でこの順位より上のとき当たる
/// （3万位。試作では 3万〜10万位で 1位の崩れは同じ 0 件。ADR 0012 §2.5）
const EXTENSION_PEER_MAX_RANK: usize = 30_000;
/// `katakana_analysis` の対象にする表記の長さの上限（文字数）。Sudachi の解析用の表記（メイ、ココ、ハナ）は短い
const KATAKANA_ANALYSIS_MAX_CHARS: usize = 3;
/// `katakana_analysis` の対象にする、カタカナ語自身の chiVe の頻度順位の下限。
/// 1 万位より上（ドル、ピン、タブ、テンポ。実際に使うカタカナ語）は対象にしない
const KATAKANA_ANALYSIS_MIN_RANK: usize = 10_000;
/// `katakana_analysis` の対象にする、同じ読みの漢字の語との頻度の差（`log10(順位 + 1)` の差。1.0 = 10 倍）
const KATAKANA_ANALYSIS_MIN_DECADE_GAP: f64 = 1.0;
/// `cheap_rare_homophone` の対象にする表記の長さの下限（文字数）
const HOMOPHONE_MIN_CHARS: usize = 2;
/// `cheap_rare_homophone` の対象にする、lemma の chiVe の頻度順位の下限（2 万位以下。語彙にないものを含む）。
/// 1 万位では 終末（12343 位）が 週末 に負け、1.5 万位でも 一報する が崩れる。2 万位は崩れがない（ADR 0016 §3.2、§9）
const HOMOPHONE_MIN_RANK: usize = 20_000;
/// `cheap_rare_homophone` の対象にする、同じ読みのよく使う語との頻度の差（`log10(順位 + 1)` の差。0.5 = 約 3 倍）。
/// 1 桁にすると 視察、臓器、言動 が漏れる（ADR 0016 §3.2）
const HOMOPHONE_MIN_DECADE_GAP: f64 = 0.5;
/// 加算の値の絶対値の上限（i16 に収まる範囲）
const MAX_ADD: i32 = 30_000;
/// 報告に載せる、述語ごとの例の数
const EXAMPLES_PER_PREDICATE: usize = 8;
/// ペナルティの分布の区切り（下限）。最後の区切り以上は「以上」の階級
const PENALTY_BIN_FLOORS: [i32; 5] = [1, 1000, 2000, 3000, 5000];

/// 述語ごとの加算量。ファイルに書かれていない述語は何もしない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    effects: [Option<i32>; PREDICATES.len()],
    /// 異字同訓の表。`kun_*` の述語があるときだけ使う（同梱の表）
    kun: KunTable,
}

/// chiVe の語彙。行の並びが頻度順なので、行番号がそのまま頻度順位（0 始まり）になる。
#[derive(Debug, Clone, Default)]
pub struct ChiveVocab {
    rank: HashMap<String, u32>,
}

impl ChiveVocab {
    /// 頻度の降順に並んだキーから作る。重複したキーは最初の位置を採る。
    pub fn from_keys<I: IntoIterator<Item = String>>(keys: I) -> Self {
        let mut rank = HashMap::new();
        for (i, key) in keys.into_iter().enumerate() {
            let i = u32::try_from(i).unwrap_or(u32::MAX);
            rank.entry(key).or_insert(i);
        }
        Self { rank }
    }

    pub fn len(&self) -> usize {
        self.rank.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rank.is_empty()
    }

    pub fn contains(&self, lemma: &str) -> bool {
        self.rank.contains_key(lemma)
    }

    /// 頻度順位（0 始まり）。語彙になければ None。
    pub fn rank(&self, lemma: &str) -> Option<u32> {
        self.rank.get(lemma).copied()
    }

    /// 頻度順位（0 始まり）。語彙にないものは語彙の最後より後ろ（順位 = 語彙の大きさ）として扱う。
    pub fn rank_or_last(&self, lemma: &str) -> usize {
        self.rank(lemma).map_or(self.rank.len(), |r| r as usize)
    }

    /// `lemma_rank` の加算量の基になる、頻度順位の桁数（`log10(順位 + 1)` から無料分を引いた値）。
    /// 語彙にない lemma は、語彙の最後より後ろ（順位 = 語彙の大きさ）として扱う。
    fn excess_decades(&self, lemma: &str) -> f64 {
        let rank = self.rank_or_last(lemma);
        ((rank as f64 + 1.0).log10() - LEMMA_RANK_FREE_DECADES).max(0.0)
    }
}

/// 判定の対象になる辞書の行。
#[derive(Debug, Clone, Copy)]
pub struct PolicyRow<'a> {
    pub surface: &'a str,
    /// 正規化表記
    pub lemma: &'a str,
    /// 読み（ひらがなと長音記号。辞書の索引のキー）
    pub reading: &'a str,
    /// 品詞名（`名詞,固有名詞,人名,一般,*,*` の形）
    pub pos: &'a str,
    /// 同じ読みの、漢字を含む表記の行（`is_kanji_peer`）の lemma のうち、chiVe の頻度順位が最も高いもの
    /// （語彙にない lemma は語彙の大きさ）。そういう行がない、または chiVe の語彙がなければ `None`
    pub kanji_peer_rank: Option<usize>,
    /// 同じ読みの、漢字を含む名詞の行（`is_noun_peer`。固有名詞を含む）の lemma のうち、chiVe の頻度順位が最も高いもの
    /// （語彙にない lemma は語彙の大きさ）。そういう行がない、または chiVe の語彙がなければ `None`（ADR 0011）
    pub noun_peer_rank: Option<usize>,
    /// 同じ読みの、漢字を含む地名の行（`is_kanji_place_peer`）の表記のうち、chiVe の頻度順位が最も高いもの
    /// （語彙にない表記は語彙の大きさ。lemma でなく表記で引く。当て字の lemma はカタカナになる）。そういう行がない、または chiVe の語彙がなければ `None`（#223）
    pub place_peer_rank: Option<usize>,
    /// 同じ読みの、頻度が高い（表記の chiVe 順位が `KATAKANA_PLACE_PEER_MAX_RANK` より上の）漢字を含む名詞の行（地名に限らない）のうち、
    /// 1 語だけを変換するときの経路コスト（BOS・EOS との連接を含む）が最小のものに、この行の総コスト（Sudachi のコスト + ポリシーの加算）を
    /// そろえるときの目標。この行自身の連接コストは引いてある。`katakana_place_peer` が総コストをそろえる基準（#223）。
    /// そういう行がない、または `katakana_place_peer` を使わなければ `None`
    pub place_peer_total: Option<i32>,
    /// 表記と読みを、2 つの辞書の語（漢字だけ 2 文字以上の普通名詞、表記 = lemma。`is_homophone_peer`）に切れるとき、
    /// 切り方ごとの「部品の lemma の chiVe の順位の大きいほう」の最小値（語彙にない部品は語彙の大きさ）。
    /// `frequent_compound` の (f)（部品がよく使う語の熟語。ADR 0035 規則 F）が使う。切れない、または語彙がなければ `None`
    pub compound_parts_rank: Option<usize>,
    /// この行が、同じ読みの `is_homophone_peer` の中で頻度がいちばん高い語（W）で、既定のコストどうしの、よりまれな同音語（V）があるとき、
    /// V の総コスト（Sudachi のコスト + ポリシーの加算）の最小値。`default_cost_homophone` が W の総コストをそろえる基準（ADR 0035 規則 H）。
    /// 当たらない、または `default_cost_homophone` を使わなければ `None`
    pub default_cost_peer_total: Option<i32>,
    /// 異字同訓の表（`policy/kun.tsv`）での分類（ADR 0009）。表を使わない行は `KunInfo::default()`
    pub kun: KunInfo,
    /// `cheap_rare_homophone` が使う、コストと同音語の頻度順位（ADR 0016）
    pub homophone: HomophoneInfo,
    /// Full の notcore の行のとき、その情報（ADR 0012 §11.3）。Core の行は `None`
    pub notcore: Option<NotcoreInfo>,
}

/// `cheap_rare_homophone` が使う行の情報（`PolicyRow::homophone`）。使わない行は `HomophoneInfo::default()`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HomophoneInfo {
    /// Sudachi の語のコスト（ポリシーを足す前）
    pub cost: i16,
    /// 同じ読みの、`is_homophone_peer` を満たす行の lemma のうち、chiVe の頻度順位が最も高いもの
    /// （語彙にない lemma は語彙の大きさ）。そういう行がない、または chiVe の語彙がなければ `None`
    pub peer_rank: Option<usize>,
}

/// notcore の行についての情報（`PolicyRow::notcore`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NotcoreInfo {
    /// 同じ読みの Core の行（notcore でない行）の lemma のうち、chiVe の頻度順位が最も高いもの
    /// （語彙にある lemma だけ）。そういう行がない、または chiVe の語彙がなければ `None`
    pub core_peer_rank: Option<usize>,
}

/// 同じ読みの「漢字の表記」の行か（`PolicyRow::kanji_peer_rank` の対象）。固有名詞（人名・地名の当て字）と記号は除く。
pub fn is_kanji_peer(row: &PolicyRow<'_>) -> bool {
    let mut fields = row.pos.split(POS_SEPARATOR);
    let (pos1, pos2) = (fields.next().unwrap_or(""), fields.next().unwrap_or(""));
    row.surface.chars().any(is_han) && pos1 != POS1_SYMBOL && pos2 != POS2_PROPER_NOUN
}

/// 同じ読みの「漢字を含む名詞」の行か（`PolicyRow::noun_peer_rank` の対象）。`is_kanji_peer` と違い、固有名詞も数える
/// （伊富魚 の守りは 伊藤 の人名）。品詞1 が名詞でない行（接尾辞、記号）は除く。
pub fn is_noun_peer(row: &PolicyRow<'_>) -> bool {
    row.pos.split(POS_SEPARATOR).next() == Some(POS1_NOUN) && row.surface.chars().any(is_han)
}

/// 同じ読みの「漢字を含む地名」の行か（`PolicyRow::place_peer_rank` の対象）。品詞が 名詞,固有名詞,地名 で、表記に漢字を含む。
/// 人名（渋谷 の姓）は品詞3 が違うので数えない（人名の当て字を除く）
pub fn is_kanji_place_peer(row: &PolicyRow<'_>) -> bool {
    let mut fields = row.pos.split(POS_SEPARATOR);
    let (pos1, pos2, pos3) = (
        fields.next().unwrap_or(""),
        fields.next().unwrap_or(""),
        fields.next().unwrap_or(""),
    );
    pos1 == POS1_NOUN
        && pos2 == POS2_PROPER_NOUN
        && pos3 == POS3_PLACE_NAME
        && row.surface.chars().any(is_han)
}

/// 漢字だけの2文字以上の普通名詞で、表記が lemma と同じか（`cheap_rare_homophone` の対象と、その同音語の条件。ADR 0016 §3.1 (a)）
fn is_plain_kanji_noun(row: &PolicyRow<'_>) -> bool {
    let mut fields = row.pos.split(POS_SEPARATOR);
    let (pos1, pos2) = (fields.next().unwrap_or(""), fields.next().unwrap_or(""));
    pos1 == POS1_NOUN
        && pos2 == POS2_COMMON_NOUN
        && row.surface == row.lemma
        && row.surface.chars().count() >= HOMOPHONE_MIN_CHARS
        && row.surface.chars().all(is_han)
}

/// 同じ読みの「よく使う同音語」になれる行か（`HomophoneInfo::peer_rank` の対象）。`is_plain_kanji_noun` と同じ条件
pub fn is_homophone_peer(row: &PolicyRow<'_>) -> bool {
    is_plain_kanji_noun(row)
}

/// 1行への判定の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    /// `ime_penalty` に入れる値。`IME_EXCLUDED` にはならない
    pub penalty: i16,
    /// 当たった述語（`PREDICATES` の添字をビット位置とする）
    pub hit_mask: u32,
    /// 固定の規則で変換に使わないとき、その規則。このとき `penalty` は `IME_EXCLUDED`
    pub excluded: Option<ExclusionRule>,
    /// notcore の新語（固有名詞以外で、lemma が chiVe にない）なので、`lemma_rank` を当てなかった
    pub lemma_rank_exempt: bool,
    /// `frequent_compound` に、部品の頻度の条件（(f)。ADR 0035 規則 F）で当たった
    pub compound_by_parts: bool,
}

impl Verdict {
    /// 述語（`PREDICATES` の名前）に当たったか
    pub fn hits(&self, predicate: &str) -> bool {
        PREDICATES
            .iter()
            .position(|p| *p == predicate)
            .is_some_and(|i| self.hit_mask & (1 << i) != 0)
    }
}

/// 表記と読みの整合の固定の規則。ポリシーの値ではなく、辞書を作るときの規則としてコードに固定する。
///
/// 理由（spec の基本原則「ユーザーの入力を正とする」）: Sudachi の解析用の辞書には、読みに対応しない記号を表記に足した項目
/// （`ね〜`/ネ、`あの〜`/アノ。話し言葉の飾りを付けた語）がある。これが候補に出ると、ユーザーが入力していない文字を
/// 先回りして付け足すことになる。ペナルティで後ろに回すだけでは足りないので、`IME_EXCLUDED` にして変換に使わない
/// （文脈の解析には残る）。表記の「ー」が読みの長音に当たる項目（ユーザー、あま〜い/アマーイ）と、記号そのものの項目は対象にしない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExclusionRule {
    /// 2 文字以上で波ダッシュ（〜 ～）を含み、読みに長音記号「ー」がない
    WaveDashWithoutLongVowel,
    /// 2 文字以上で飾りの記号（！？。…♪ など）を含む。固有名詞（`yahoo!`、`ミニモニ。`）は名前の一部なので対象にしない
    DecorationMark,
    /// ハイフンかダッシュ（‐ − - など）の両隣がひらがなか漢字で（`す‐ぐ`/スグ、`脱‐存`/ダツゾン）、読みに長音記号「ー」がない。
    /// 和語や漢語の途中のハイフンは音がなく、表記の一部でもない（Sudachi の解析用の表記）。
    /// カタカナの外来語や英字の綴りの区切り（`セブン‐イレブン`、`Wi-Fi`、`e-文書法`）は名前の一部なので対象にしない
    HyphenInsideNativeWord,
}

/// 規則の一覧（集計の添字）
pub const EXCLUSION_RULES: [ExclusionRule; 3] = [
    ExclusionRule::WaveDashWithoutLongVowel,
    ExclusionRule::DecorationMark,
    ExclusionRule::HyphenInsideNativeWord,
];

impl ExclusionRule {
    pub fn name(self) -> &'static str {
        match self {
            Self::WaveDashWithoutLongVowel => "wave_dash_without_long_vowel",
            Self::DecorationMark => "decoration_mark",
            Self::HyphenInsideNativeWord => "hyphen_inside_native_word",
        }
    }

    pub(crate) fn index(self) -> usize {
        match self {
            Self::WaveDashWithoutLongVowel => 0,
            Self::DecorationMark => 1,
            Self::HyphenInsideNativeWord => 2,
        }
    }

    /// この行が規則に当たるか
    pub fn check(row: &PolicyRow<'_>) -> Option<Self> {
        let mut fields = row.pos.split(POS_SEPARATOR);
        let (pos1, pos2) = (fields.next().unwrap_or(""), fields.next().unwrap_or(""));
        // 記号そのものの項目（〜 キゴウ、!? ）と、1 文字の表記は対象にしない
        if pos1 == POS1_AUX_SYMBOL || pos1 == POS1_SYMBOL || row.surface.chars().count() < 2 {
            return None;
        }
        if row.surface.chars().any(|c| WAVE_DASH_MARKS.contains(&c))
            && !row.reading.contains(PROLONGED_SOUND_MARK)
        {
            return Some(Self::WaveDashWithoutLongVowel);
        }
        if pos2 != POS2_PROPER_NOUN && row.surface.chars().any(|c| DECORATION_MARKS.contains(&c)) {
            return Some(Self::DecorationMark);
        }
        if has_hyphen_between_kana_or_kanji(row.surface)
            && !row.reading.contains(PROLONGED_SOUND_MARK)
        {
            return Some(Self::HyphenInsideNativeWord);
        }
        None
    }
}

/// ハイフンかダッシュ（`HYPHEN_MARKS`）のうち、両隣がひらがなか漢字のものがあるか
fn has_hyphen_between_kana_or_kanji(surface: &str) -> bool {
    let is_native = |c: char| is_hiragana(c) || is_han(c);
    let chars: Vec<char> = surface.chars().collect();
    chars
        .windows(3)
        .any(|w| HYPHEN_MARKS.contains(&w[1]) && is_native(w[0]) && is_native(w[2]))
}

impl Policy {
    /// `述語<TAB>値` の行からなるテキストを読む。`#` で始まる行と空行は無視する。
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut effects = [None; PREDICATES.len()];
        for (i, line) in text.lines().enumerate() {
            let lineno = i + 1;
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (name, value) = line
                .split_once('\t')
                .ok_or_else(|| format!("line {lineno}: expected `predicate<TAB>value`"))?;
            let (name, value) = (name.trim(), value.trim());
            let index = PREDICATES
                .iter()
                .position(|p| *p == name)
                .ok_or_else(|| format!("line {lineno}: unknown predicate `{name}`"))?;
            if effects[index].is_some() {
                return Err(format!(
                    "line {lineno}: predicate `{name}` is defined twice"
                ));
            }
            let add: i32 = value
                .parse()
                .map_err(|_| format!("line {lineno}: value `{value}` is not an integer"))?;
            if add.abs() > MAX_ADD {
                return Err(format!(
                    "line {lineno}: value {add} is outside -{MAX_ADD}..={MAX_ADD}"
                ));
            }
            effects[index] = Some(add);
        }
        Ok(Self {
            effects,
            kun: KunTable::builtin(),
        })
    }

    /// 同梱の既定ポリシー。
    pub fn builtin() -> Self {
        // 同梱のファイルは単体テストで検証している
        Self::parse(DEFAULT_POLICY_TEXT).unwrap_or(Self {
            effects: [None; PREDICATES.len()],
            kun: KunTable::builtin(),
        })
    }

    pub fn effect(&self, predicate: &str) -> Option<i32> {
        let index = PREDICATES.iter().position(|p| *p == predicate)?;
        self.effects[index]
    }

    /// chiVe の語彙が要る述語（`lemma_rank`、`katakana_analysis` など）が書かれているか。
    pub fn uses_chive_vocab(&self) -> bool {
        self.effects[IDX_LEMMA_RANK].is_some()
            || self.effects[IDX_KATAKANA_ANALYSIS].is_some()
            || self.effects[IDX_FREQUENT_COMPOUND].is_some()
            || self.effects[IDX_EXTENSION_PEER].is_some()
            || self.effects[IDX_CHEAP_RARE_HOMOPHONE].is_some()
            || self.effects[IDX_KATAKANA_PLACE_PEER].is_some()
            || self.effects[IDX_DEFAULT_COST_HOMOPHONE].is_some()
    }

    /// 異字同訓の表の述語（`kun_*`）が書かれているか。
    pub fn uses_kun_table(&self) -> bool {
        [
            IDX_KUN_OUTSIDE,
            IDX_KUN_LEMMA_DEFAULT,
            IDX_KUN_READING_DEFAULT,
        ]
        .iter()
        .any(|&i| self.effects[i].is_some())
    }

    /// 異字同訓の表（同梱）。
    pub fn kun_table(&self) -> &KunTable {
        &self.kun
    }

    /// 内容のハッシュ（メタ情報の `policy=`）。コメントや空白の違いでは変わらない。
    /// `kun_*` の述語があるときは、異字同訓の表の内容も入る。
    pub fn fingerprint(&self) -> u64 {
        let mut h = Fnv1a64::new();
        h.update(self.canonical_text().as_bytes());
        if self.uses_kun_table() {
            h.update(self.kun.canonical_text().as_bytes());
        }
        h.finish()
    }

    /// 述語の並びを固定した、正規化したテキスト。
    pub fn canonical_text(&self) -> String {
        let mut out = String::new();
        for (name, effect) in PREDICATES.iter().zip(self.effects) {
            if let Some(v) = effect {
                let _ = writeln!(out, "{name}\t{v}");
            }
        }
        out
    }

    /// 行を判定する。`chive_vocab` が `None` のとき、`lemma_rank` は当たらない。
    pub fn evaluate(&self, row: &PolicyRow<'_>, chive_vocab: Option<&ChiveVocab>) -> Verdict {
        let mut fields = row.pos.split(POS_SEPARATOR);
        let (pos1, pos2, pos3) = (
            fields.next().unwrap_or(""),
            fields.next().unwrap_or(""),
            fields.next().unwrap_or(""),
        );
        if let Some(rule) = ExclusionRule::check(row) {
            return Verdict {
                penalty: IME_EXCLUDED,
                hit_mask: 0,
                excluded: Some(rule),
                lemma_rank_exempt: false,
                compound_by_parts: false,
            };
        }
        let lower = is_ascii_lower_surface(row.surface);
        let mut hit = [false; PREDICATES.len()];
        hit[IDX_ASCII_LOWER] = lower;
        hit[IDX_ASCII_OTHER] = !lower && row.surface.chars().any(|c| c.is_ascii_alphabetic());
        // 数字の表記。漢数字のゼロ「〇」（れい／ぜろ）だけは数字として扱う（一、二 は数字の読みで正しい）
        hit[IDX_NUMERIC_SURFACE] = pos2 == POS2_NUMERAL
            && (row.surface == KANJI_ZERO || row.surface.chars().any(is_digit));
        hit[IDX_KATAKANA_VARIANT] =
            row.surface.chars().any(is_katakana) && !row.lemma.chars().any(is_katakana);
        hit[IDX_SYMBOL_KANA] = pos1 == POS1_SYMBOL;
        hit[IDX_PROPER_NOUN] = pos2 == POS2_PROPER_NOUN;
        hit[IDX_PERSON_NAME] = pos3 == POS3_PERSON_NAME;
        hit[IDX_PLACE_NAME] = pos2 == POS2_PROPER_NOUN && pos3 == POS3_PLACE_NAME;
        hit[IDX_OFFICIAL_KA_COUNTER] = is_official_ka_counter(row, pos1, pos2, pos3);
        hit[IDX_KATAKANA_PLACE_PEER] =
            is_katakana_proper_noun_with_kanji_place_peer(row, pos1, pos2, chive_vocab);
        // 活用しない語で、漢字を含む表記が lemma（正規化表記）と違う: 異体字・送り仮名のゆれ（浜死/瀕死）
        hit[IDX_VARIANT_SURFACE] = row.pos.split(POS_SEPARATOR).nth(POS_CONJ_TYPE_FIELD)
            == Some(CONJ_TYPE_NONE)
            && row.surface != row.lemma
            && row.surface.chars().any(is_han);
        hit[IDX_WAVE_DASH] =
            row.surface.chars().count() > 1 && row.surface.chars().any(is_wave_dash);
        hit[IDX_KATAKANA_ANALYSIS] = is_katakana_analysis(row, pos1, pos2, chive_vocab);
        // 機能語（助詞、助動詞、接尾辞）の英字・波ダッシュ入りの表記（よ → YO、ね → ね〜）。名詞の英字表記（PC、API）は対象外
        hit[IDX_FUNCTION_WORD_NOTATION] = (FUNCTION_WORD_POS1.contains(&pos1)
            && row.surface.chars().count() > 1
            && row
                .surface
                .chars()
                .any(|c| c.is_ascii_alphabetic() || WAVE_DASH_MARKS.contains(&c)))
            || is_function_word_katakana(row, pos1);
        let compound = frequent_compound_kind(row, pos1, pos2, chive_vocab);
        hit[IDX_FREQUENT_COMPOUND] = compound.is_some();
        // 解析用に安い語が、よく使う同音語に勝つ（製鋼 → 成功）。値はコストの下限で、足すのは「下限 - コスト」
        let cost_floor = self.effects[IDX_CHEAP_RARE_HOMOPHONE];
        hit[IDX_CHEAP_RARE_HOMOPHONE] =
            cost_floor.is_some_and(|floor| is_cheap_rare_homophone(row, floor, chive_vocab));
        // 異字同訓の表（ADR 0009）。lemma の中の既定は、ほかの表記との差が足りないときだけ当たる
        let lemma_default_excess = row.kun.lemma_default_excess.filter(|excess| *excess >= 0);
        hit[IDX_KUN_OUTSIDE] = row.kun.outside;
        hit[IDX_KUN_LEMMA_DEFAULT] = lemma_default_excess.is_some();
        hit[IDX_KUN_READING_DEFAULT] = row.kun.reading_default;

        // 追加の語（notcore）の規則（ADR 0012 §2.5、§11.3）。
        // chiVe は 2024 年の Core で分かち書きして学習しているので、新しい語は「まれ」と区別できない。
        // 固有名詞以外で chiVe にない lemma は、`lemma_rank` を当てない（沼る、メロい、チルい）。
        // 代わりに、同じ読みによく使われる Core の語があるときだけ `extension_peer` を足す（体制 に タイ製 が勝たないように）。
        let lemma_rank_exempt = row.notcore.is_some()
            && pos2 != POS2_PROPER_NOUN
            && chive_vocab.is_some_and(|vocab| !vocab.contains(row.lemma));
        hit[IDX_EXTENSION_PEER] = row
            .notcore
            .and_then(|n| n.core_peer_rank)
            .is_some_and(|rank| rank < EXTENSION_PEER_MAX_RANK);

        // 頻度順位は、順位の桁数に比例して加算する（当たるのは無料の桁数を超えたとき）
        let rank_excess = if lemma_rank_exempt {
            0.0
        } else {
            chive_vocab.map_or(0.0, |vocab| vocab.excess_decades(row.lemma))
        };
        hit[IDX_LEMMA_RANK] = rank_excess > 0.0;

        let mut sum: i32 = 0;
        let mut hit_mask = 0u32;
        for (i, effect) in self.effects.iter().enumerate() {
            let (true, Some(effect)) = (hit[i], effect) else {
                continue;
            };
            hit_mask |= 1 << i;
            if i == IDX_LEMMA_RANK {
                sum += (f64::from(*effect) * rank_excess).round() as i32;
            } else if i == IDX_CHEAP_RARE_HOMOPHONE {
                // 値はコストの下限。コストをそこまで上げる（差を足す。当たるのは下限より安い行だけ）
                sum += effect - i32::from(row.homophone.cost);
            } else if i == IDX_KUN_LEMMA_DEFAULT {
                // 値は、既定の表記をほかの表記より安くする差（負）。いまの差を引いて、その差にする
                sum += effect - lemma_default_excess.unwrap_or(0);
            } else if i == IDX_KATAKANA_PLACE_PEER || i == IDX_DEFAULT_COST_HOMOPHONE {
                // 値は余白 M。加算ではなく、下の「総コストをそろえる」で使う
            } else {
                sum += effect;
            }
        }
        // 対象のカタカナ表記の総コストを「同じ読みの最安の漢字の地名の総コスト + M」にそろえる（上げるだけ。下げない）。
        // 固定の加算では、語ごとのコストの差（ヨコハマ 3437、ナガノ はもっと大きい）で、1 回の学習（1500）で 1 位になる語とならない語が出た。
        // そろえると、カタカナは常に漢字の直後（M だけ後ろ）に残り、1 回の学習で M が 1500 未満なら 1 位になる
        if let (true, Some(margin), Some(peer_total)) = (
            hit[IDX_KATAKANA_PLACE_PEER],
            self.effects[IDX_KATAKANA_PLACE_PEER],
            row.place_peer_total,
        ) {
            let target = peer_total + margin;
            let current = i32::from(row.homophone.cost) + sum;
            if target > current {
                sum += target - current;
            }
        }
        // 既定のコストどうしの同音語（ADR 0035 規則 H）: 頻度がいちばん高い語 W の総コストを「よりまれな同音語 V の総コストの最小 − M」まで下げる
        // （下げるだけ。上げない）。Sudachi の既定のコストの差（サ変可能 6518、一般 7404）は品詞の違いだけから来て、頻度の情報がないため
        if let (Some(margin), Some(peer_total)) = (
            self.effects[IDX_DEFAULT_COST_HOMOPHONE],
            row.default_cost_peer_total,
        ) {
            let target = peer_total - margin;
            let current = i32::from(row.homophone.cost) + sum;
            if current > target {
                sum += target - current;
                hit_mask |= 1 << IDX_DEFAULT_COST_HOMOPHONE;
            }
        }
        // 加算は i16 に飽和させる。IME_EXCLUDED と区別するため、上限は IME_EXCLUDED - 1
        let penalty = sum.clamp(i32::from(i16::MIN), i32::from(IME_EXCLUDED) - 1) as i16;
        Verdict {
            penalty,
            hit_mask,
            excluded: None,
            lemma_rank_exempt,
            compound_by_parts: compound == Some(CompoundKind::ByParts),
        }
    }
}

/// カタカナだけの表記の固有名詞（地名、人名、一般）で、同じ読みに頻度の高い漢字の地名があるか（シブヤ に 渋谷、ヨコハマ に 横浜。#223）。
/// Sudachi のカタカナの地名は解析用に漢字より 2000〜3800 安く、地名のペナルティ（`place_name`）は両方に同じだけ効くので、
/// 順位が変わらず シブヤ が 渋谷 に勝っていた。後ろに回す量は固定でなく、総コストを「最安の頻度が高い漢字の名詞の、1 語だけを変換するときの
/// 総コスト + 余白 M」にそろえる（`PolicyRow::place_peer_total`。学習 1 回の加点より小さい M なら、1 回選んだだけでカタカナが 1 位になる）。固有名詞の一般（カワサキ）と人名（サカエ）も、地名の漢字（川崎、栄）の後ろに回す
/// （ユーザー決定 2026-10-23: 地名は第一候補。固有名詞よりも明らかに優先する）。後ろに回すのは、漢字の地名が日常に使われる（chiVe の頻度が上位の）ときだけ。
/// ロンドン、ニューヨーク、パリ のように同じ読みに漢字の地名がない（巴里 などの当て字は頻度が低くて対象外）外来語の地名は対象にしない。
/// カタカナの表記のほうが漢字の地名より頻度が高いとき（カナダ 4460 位 と 金田 22468 位、タイ と 泰）も対象にしない
fn is_katakana_proper_noun_with_kanji_place_peer(
    row: &PolicyRow<'_>,
    pos1: &str,
    pos2: &str,
    chive_vocab: Option<&ChiveVocab>,
) -> bool {
    pos1 == POS1_NOUN
        && pos2 == POS2_PROPER_NOUN
        && !row.surface.is_empty()
        && row
            .surface
            .chars()
            .all(|c| is_katakana(c) || c == PROLONGED_SOUND_MARK)
        && row.place_peer_rank.is_some_and(|peer| {
            peer < KATAKANA_PLACE_PEER_MAX_RANK
                && chive_vocab.is_some_and(|vocab| peer < vocab.rank_or_last(row.surface))
        })
}

/// 助数詞可能の名詞で、表記が「か」+ 月・国・所 の 2 文字で、lemma が「箇」+ 同じ漢字か（公用文の か月・か国・か所）。
/// 数字のあと（3か月、3か所、3か国語）で、公用文の表記を異体の 箇月・箇国・箇所・個所 より上に出すために使う（ADR 0019 §17）。
/// カ月・ヶ月 などのカタカナの表記は対象にしない（`katakana_variant` が下げている）
fn is_official_ka_counter(row: &PolicyRow<'_>, pos1: &str, pos2: &str, pos3: &str) -> bool {
    if pos1 != POS1_NOUN || pos2 != POS2_COMMON_NOUN || pos3 != POS3_COUNTABLE {
        return false;
    }
    let mut surface = row.surface.chars();
    let (Some(first), Some(second), None) = (surface.next(), surface.next(), surface.next()) else {
        return false;
    };
    first == KA_COUNTER_HIRAGANA
        && KA_COUNTER_SUFFIXES.contains(&second)
        && row.lemma.chars().eq([KA_COUNTER_KANJI, second])
}

/// 助詞・助動詞で、カタカナを含み lemma にはカタカナがない表記か（タ、ダ、デス、マス、ネ。治っタ のように
/// 読みの一部に混じって上位に出る）。ヶ・ヵ は正しい表記なので除く。1 文字でも当たる
fn is_function_word_katakana(row: &PolicyRow<'_>, pos1: &str) -> bool {
    FUNCTION_WORD_KATAKANA_POS1.contains(&pos1)
        && row
            .surface
            .chars()
            .any(|c| is_katakana(c) && !KATAKANA_SMALL_KE.contains(&c))
        && !row.lemma.chars().any(is_katakana)
}

/// 1 語の普通名詞の複合語で、lemma の chiVe の頻度が高いか（具体例、勉強会）、または部品がよく使う語か（異常気象、仕様変更）。
/// Sudachi は解析のために複合語の項目のコストを高くしている（具体例 15422 は 具体 6762 + 礼 4397 に負ける）。
/// 値は負（加点）にして、分割した経路と公平に比べる。表記が lemma と同じ、漢字だけの語に限る。
///
/// 順位が `COMPOUND_MAX_RANK` より上なら対象。`COMPOUND_MAX_RANK` 以上 `COMPOUND_GUARDED_MAX_RANK` 未満の語は、
/// 同じ読みの漢字を含む名詞に自身より上位のものがない（`noun_peer_rank` が自身の順位以上）ときだけ対象にする
/// （形態素解析 の 形態素 は対象、葬式 に対する 総指揮 は対象外。ADR 0011）。
///
/// chiVe にない語は、`COMPOUND_PARTS_MIN_CHARS` 文字以上で、表記と読みを 2 つの部品（`is_homophone_peer` の辞書の行）に切れて、
/// 両方の部品の順位が `COMPOUND_PARTS_MAX_RANK` より上（`PolicyRow::compound_parts_rank`）で、同じ読みに chiVe にある漢字の名詞がない
/// （`noun_peer_rank` が語彙の大きさ）ときに対象にする（ADR 0035 規則 F。異常気象 = 異常 + 気象。chiVe は複合語の頻度を持たないことが多い）
fn frequent_compound_kind(
    row: &PolicyRow<'_>,
    pos1: &str,
    pos2: &str,
    chive_vocab: Option<&ChiveVocab>,
) -> Option<CompoundKind> {
    let vocab = chive_vocab?;
    if !(pos1 == POS1_NOUN
        && pos2 == POS2_COMMON_NOUN
        && row.surface == row.lemma
        && row.surface.chars().count() >= COMPOUND_MIN_CHARS
        && row.surface.chars().all(is_han))
    {
        return None;
    }
    match vocab.rank(row.lemma).map(|r| r as usize) {
        Some(rank) if rank < COMPOUND_MAX_RANK => Some(CompoundKind::ByRank),
        Some(rank) => (rank < COMPOUND_GUARDED_MAX_RANK
            && row.noun_peer_rank.is_some_and(|peer| peer >= rank))
        .then_some(CompoundKind::ByRank),
        // 20 万位以下の語まで (f) を広げても救いは同じで、当たる行が増える（ADR 0035 の掃引）ので、chiVe にない語だけ
        None => (row.surface.chars().count() >= COMPOUND_PARTS_MIN_CHARS
            && row
                .compound_parts_rank
                .is_some_and(|parts| parts < COMPOUND_PARTS_MAX_RANK)
            && row.noun_peer_rank.is_some_and(|peer| peer >= vocab.len()))
        .then_some(CompoundKind::ByParts),
    }
}

/// `frequent_compound` に当たった理由
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompoundKind {
    /// 熟語自身の chiVe の順位（ADR 0011 の (d)(e)）
    ByRank,
    /// 部品の chiVe の順位（ADR 0035 の (f)）
    ByParts,
}

/// 既定のコストどうしの同音語（ADR 0035 規則 H）の判定に使う、同じ読みの `is_homophone_peer` の行の情報
#[derive(Debug, Clone, Copy)]
pub(crate) struct HomophonePeer {
    /// lemma の chiVe の順位（語彙にない lemma は語彙の大きさ）
    pub rank: usize,
    /// Sudachi のコスト
    pub cost: i16,
    /// 総コスト（Sudachi のコスト + `default_cost_homophone` を除くポリシーの加算）
    pub total: i32,
    /// 規則 R（`cheap_rare_homophone`）に当たった
    pub cheap_rare: bool,
}

/// 同じ読みの `is_homophone_peer` の行（`peers`）ごとの `PolicyRow::default_cost_peer_total`。
/// W（順位がいちばん高い lemma の行）で、W の順位が上限より上で Sudachi のコストが既定の帯にあるとき、
/// W より一定の桁以上まれで、Sudachi のコストが既定の帯にあり、規則 R に当たっていない行（V）の総コストの最小値を W の行に返す
pub(crate) fn default_cost_peer_totals(peers: &[HomophonePeer]) -> Vec<Option<i32>> {
    let in_band = |cost: i16| DEFAULT_COST_BAND.contains(&cost);
    let mut out = vec![None; peers.len()];
    let Some(top) = peers.iter().map(|p| p.rank).min() else {
        return out;
    };
    if top >= DEFAULT_COST_PEER_MAX_RANK {
        return out;
    }
    let decades = |rank: usize| (rank as f64 + 1.0).log10();
    let best_v = peers
        .iter()
        .filter(|p| {
            p.rank != top
                && decades(p.rank) - decades(top) >= DEFAULT_COST_MIN_DECADE_GAP
                && in_band(p.cost)
                && !p.cheap_rare
        })
        .map(|p| p.total)
        .min();
    let Some(best_v) = best_v else {
        return out;
    };
    for (slot, peer) in out.iter_mut().zip(peers) {
        if peer.rank == top && in_band(peer.cost) {
            *slot = Some(best_v);
        }
    }
    out
}

/// 解析用に安い語で、同じ読みのよく使う語に勝つ恐れがあるか（製鋼 → 成功、右中 → 宇宙。ADR 0016 規則 R）。次をすべて満たす:
/// - 漢字だけの2文字以上の普通名詞で、表記が lemma と同じ（`is_plain_kanji_noun`）
/// - Sudachi のコストが `cost_floor` 未満（解析のために既定の値より安くされている）
/// - lemma の chiVe の頻度順位が `HOMOPHONE_MIN_RANK` 以下（語彙にない語を含む）
/// - 同じ読みの `is_homophone_peer` の中で最も頻度の高い語より、`HOMOPHONE_MIN_DECADE_GAP`（約 3 倍）以上まれ
///
/// 当たった行は、コストを `cost_floor`（サ変可能の既定の値 6518）まで上げる。消さない。
fn is_cheap_rare_homophone(
    row: &PolicyRow<'_>,
    cost_floor: i32,
    chive_vocab: Option<&ChiveVocab>,
) -> bool {
    let (Some(vocab), Some(peer_rank)) = (chive_vocab, row.homophone.peer_rank) else {
        return false;
    };
    if !is_plain_kanji_noun(row) || i32::from(row.homophone.cost) >= cost_floor {
        return false;
    }
    let own_rank = vocab.rank_or_last(row.lemma);
    own_rank >= HOMOPHONE_MIN_RANK
        && (own_rank as f64 + 1.0).log10() - (peer_rank as f64 + 1.0).log10()
            >= HOMOPHONE_MIN_DECADE_GAP
}

/// Sudachi の解析用のカタカナ名詞か。次をすべて満たす:
/// - 普通名詞で、表記が lemma と同じ、長音記号を含むカタカナだけの短い語（メイ、ハナ。ドルのような実際のカタカナ語を除くため、
///   自身の頻度順位が低いことも要る）
/// - 同じ読みに漢字の表記（名、花）があり、その lemma の方が桁違いに頻度が高い
///
/// 「ユーザ名」の「名」に、安い「メイ」が勝つのを防ぐ。`lemma_rank` だけでは足りない（頻度の差が小さい、コストの差が大きい）。
fn is_katakana_analysis(
    row: &PolicyRow<'_>,
    pos1: &str,
    pos2: &str,
    chive_vocab: Option<&ChiveVocab>,
) -> bool {
    let (Some(vocab), Some(peer_rank)) = (chive_vocab, row.kanji_peer_rank) else {
        return false;
    };
    let len = row.surface.chars().count();
    if pos1 != POS1_NOUN
        || pos2 != POS2_COMMON_NOUN
        || row.surface != row.lemma
        || len == 0
        || len > KATAKANA_ANALYSIS_MAX_CHARS
        || !row
            .surface
            .chars()
            .all(|c| is_katakana(c) || c == PROLONGED_SOUND_MARK)
    {
        return false;
    }
    let own_rank = vocab.rank_or_last(row.lemma);
    own_rank >= KATAKANA_ANALYSIS_MIN_RANK
        && (own_rank as f64 + 1.0).log10() - (peer_rank as f64 + 1.0).log10()
            >= KATAKANA_ANALYSIS_MIN_DECADE_GAP
}

/// CJK 統合漢字（拡張A を含む）と々。
pub(crate) fn is_han(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '々')
}

/// ひらがな（ぁ U+3041 〜 ゖ U+3096、ゝ ゞ）
fn is_hiragana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{3096}' | 'ゝ' | 'ゞ')
}

/// 波ダッシュ類（〜 U+301C、～ U+FF5E、~）。感嘆の装飾（`あの〜`、`なった〜`）に使われる。
fn is_wave_dash(c: char) -> bool {
    matches!(c, '\u{301C}' | '\u{FF5E}' | '~')
}

/// ASCII と全角の数字、ローマ数字（Ⅰ〜ↈ。`ⅰ`/`Ⅲ` など。数詞として辞書に入っている）。
fn is_digit(c: char) -> bool {
    c.is_ascii_digit() || ('０'..='９').contains(&c) || ('\u{2160}'..='\u{2188}').contains(&c)
}

/// 長音記号「ー」(U+30FC) は含まない（ひらがなの語にも使うため）。小書きのカタカナ拡張（ㇲ など）は含む。
fn is_katakana(c: char) -> bool {
    matches!(
        c,
        '\u{30A1}'..='\u{30FA}' | '\u{30FD}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' | '\u{FF66}'..='\u{FF9D}'
    )
}

/// 英小文字、数字、空白、`.'&-` だけで構成され、英小文字を含む。
fn is_ascii_lower_surface(s: &str) -> bool {
    let mut has_lower = false;
    for c in s.chars() {
        if c.is_ascii_lowercase() {
            has_lower = true;
        } else if !(c.is_ascii_digit() || ASCII_LOWER_PUNCT.contains(&c)) {
            return false;
        }
    }
    has_lower
}

/// ビルド全体での述語ごとの集計。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PolicyStats {
    pub rows: usize,
    pub hits: [usize; PREDICATES.len()],
    pub examples: [Vec<String>; PREDICATES.len()],
    /// ペナルティが正のもの
    pub penalized: usize,
    /// ペナルティの階級（`PENALTY_BIN_FLOORS` の各下限以上）ごとの件数
    pub penalty_bins: [usize; PENALTY_BIN_FLOORS.len()],
    /// 加算が負のもの
    pub boosted: usize,
    /// 固定の規則（`ExclusionRule`）で変換に使わないものの件数と例（`EXCLUSION_RULES` の順）
    pub excluded: [usize; EXCLUSION_RULES.len()],
    pub excluded_examples: [Vec<String>; EXCLUSION_RULES.len()],
    /// notcore の新語として `lemma_rank` を当てなかった行の件数と例（`Verdict::lemma_rank_exempt`）
    pub lemma_rank_exempt: usize,
    pub lemma_rank_exempt_examples: Vec<String>,
    /// 判定した行のうち notcore の行
    pub notcore_rows: usize,
    /// `frequent_compound` に部品の頻度（(f)。ADR 0035 規則 F）で当たった行の件数と例
    pub compound_by_parts: usize,
    pub compound_by_parts_examples: Vec<String>,
}

impl PolicyStats {
    pub fn record(&mut self, row: &PolicyRow<'_>, verdict: Verdict) {
        self.rows += 1;
        if row.notcore.is_some() {
            self.notcore_rows += 1;
        }
        if let Some(rule) = verdict.excluded {
            let i = rule.index();
            self.excluded[i] += 1;
            if self.excluded_examples[i].len() < EXAMPLES_PER_PREDICATE {
                self.excluded_examples[i].push(format!("{}（{}）", row.surface, row.reading));
            }
            return;
        }
        if verdict.lemma_rank_exempt {
            self.lemma_rank_exempt += 1;
            if self.lemma_rank_exempt_examples.len() < EXAMPLES_PER_PREDICATE {
                self.lemma_rank_exempt_examples
                    .push(format!("{}（{}）", row.surface, row.lemma));
            }
        }
        if verdict.compound_by_parts {
            self.compound_by_parts += 1;
            if self.compound_by_parts_examples.len() < EXAMPLES_PER_PREDICATE {
                self.compound_by_parts_examples
                    .push(format!("{}（{}）", row.surface, row.reading));
            }
        }
        for i in 0..PREDICATES.len() {
            if verdict.hit_mask & (1 << i) != 0 {
                self.hits[i] += 1;
                if self.examples[i].len() < EXAMPLES_PER_PREDICATE {
                    self.examples[i].push(format!("{}（{}）", row.surface, row.lemma));
                }
            }
        }
        if verdict.penalty < 0 {
            self.boosted += 1;
        } else if verdict.penalty > 0 {
            self.penalized += 1;
            let bin = PENALTY_BIN_FLOORS
                .iter()
                .rposition(|&floor| i32::from(verdict.penalty) >= floor)
                .unwrap_or(0);
            self.penalty_bins[bin] += 1;
        }
    }

    /// 述語ごとに当たった件数（当たらなかった述語も 0 で含む）。
    pub fn hits_by_predicate(&self) -> BTreeMap<&'static str, usize> {
        PREDICATES.iter().copied().zip(self.hits).collect()
    }

    /// Markdown のレポート。
    pub fn to_markdown(&self, policy: &Policy, chive_vocab: Option<usize>) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# 辞書ポリシーのレポート（ADR 0002 §4）\n");
        let _ = writeln!(out, "- ポリシーのハッシュ: `{:016x}`", policy.fingerprint());
        match chive_vocab {
            Some(n) => {
                let _ = writeln!(out, "- chiVe の語彙: {n} 語（`lemma_rank` を評価した）");
            }
            None => {
                let _ = writeln!(
                    out,
                    "- chiVe の語彙: 指定なし（`lemma_rank` は評価していない）"
                );
            }
        }
        let _ = writeln!(out, "- 判定した行（辞書に入る行）: {}", self.rows);
        if policy.uses_kun_table() {
            let _ = writeln!(
                out,
                "- 異字同訓の表（ADR 0009）: {} 語（`kun_*` を評価した）",
                policy.kun.len()
            );
        }
        let _ = writeln!(
            out,
            "- ペナルティを加算: {}（{:.2}%）",
            self.penalized,
            percent(self.penalized, self.rows)
        );
        if self.notcore_rows > 0 {
            let _ = writeln!(
                out,
                "- うち Full の notcore の行: {}（品詞の統計と活用規則には数えない。ADR 0012 §11.3）",
                self.notcore_rows
            );
            let _ = writeln!(
                out,
                "- notcore の新語（固有名詞以外で chiVe にない lemma）に `lemma_rank` を当てなかった行: {}（例: {}）",
                self.lemma_rank_exempt,
                self.lemma_rank_exempt_examples.join("、")
            );
        }
        let _ = writeln!(out, "- 加点（負の値）: {}", self.boosted);
        let _ = writeln!(
            out,
            "- 固定の規則で変換に使わない（`IME_EXCLUDED`）: {}\n",
            self.excluded.iter().sum::<usize>()
        );

        let _ = writeln!(out, "## 述語ごとの当たった件数\n");
        let _ = writeln!(out, "| 述語 | 値 | 当たった件数 | 例 |");
        let _ = writeln!(out, "| --- | --- | ---: | --- |");
        for (i, name) in PREDICATES.iter().enumerate() {
            let value = match policy.effects[i] {
                Some(v) => format!("{v:+}"),
                None => "（未設定）".to_string(),
            };
            let _ = writeln!(
                out,
                "| `{name}` | {value} | {} | {} |",
                self.hits[i],
                self.examples[i].join("、")
            );
        }

        let _ = writeln!(
            out,
            "\n- `frequent_compound` のうち、部品の頻度（ADR 0035 規則 F）で当たった行: {}（例: {}）",
            self.compound_by_parts,
            self.compound_by_parts_examples.join("、")
        );

        let _ = writeln!(out, "\n## 表記と読みの整合の固定の規則\n");
        let _ = writeln!(out, "| 規則 | 件数 | 例（表記（読み）） |");
        let _ = writeln!(out, "| --- | ---: | --- |");
        for rule in EXCLUSION_RULES {
            let i = rule.index();
            let _ = writeln!(
                out,
                "| `{}` | {} | {} |",
                rule.name(),
                self.excluded[i],
                self.excluded_examples[i].join("、")
            );
        }

        let _ = writeln!(out, "\n## 加算したペナルティの分布\n");
        let _ = writeln!(out, "| 階級 | 件数 |");
        let _ = writeln!(out, "| --- | ---: |");
        for (i, floor) in PENALTY_BIN_FLOORS.iter().enumerate() {
            let label = match PENALTY_BIN_FLOORS.get(i + 1) {
                Some(next) => format!("{floor} 〜 {}", next - 1),
                None => format!("{floor} 以上"),
            };
            let _ = writeln!(out, "| {label} | {} |", self.penalty_bins[i]);
        }
        out
    }
}

fn percent(n: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        n as f64 * 100.0 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(policy: &Policy, surface: &str, lemma: &str, pos: &str) -> Verdict {
        policy.evaluate(
            &PolicyRow {
                surface,
                lemma,
                reading: "ー",
                pos,
                kanji_peer_rank: None,
                noun_peer_rank: None,
                place_peer_rank: None,
                place_peer_total: None,
                compound_parts_rank: None,
                default_cost_peer_total: None,
                notcore: None,
                homophone: HomophoneInfo::default(),
                kun: KunInfo::default(),
            },
            None,
        )
    }

    const NOUN: &str = "名詞,普通名詞,一般,*,*,*";
    const PERSON: &str = "名詞,固有名詞,人名,一般,*,*";
    const PLACE: &str = "名詞,固有名詞,地名,一般,*,*";
    /// 既定の `place_name`（proper_noun に加算される。ADR 0002 §4.1）
    const PLACE_NAME_ADD: i16 = -750;

    #[test]
    fn builtin_policy_has_every_predicate_with_adr_defaults() {
        let p = Policy::builtin();
        assert_eq!(p.effect(ASCII_LOWER), Some(10000));
        assert_eq!(p.effect(ASCII_OTHER), Some(3000));
        assert_eq!(p.effect(NUMERIC_SURFACE), Some(10000));
        assert_eq!(p.effect(KATAKANA_VARIANT), Some(5000));
        assert_eq!(p.effect(SYMBOL_KANA), Some(3000));
        assert_eq!(p.effect(PROPER_NOUN), Some(1000));
        assert_eq!(p.effect(PERSON_NAME), Some(1000));
        assert_eq!(p.effect(PLACE_NAME), Some(-750));
        assert!(!p.uses_chive_vocab());
        assert_eq!(p.effect(WAVE_DASH), Some(3000));
        assert_eq!(p.effect(VARIANT_SURFACE), Some(1500));
        assert_eq!(p.effect(KUN_OUTSIDE), Some(3000));
        assert_eq!(p.effect(KUN_LEMMA_DEFAULT), Some(-50));
        assert_eq!(p.effect(KUN_READING_DEFAULT), Some(-500));
        assert_eq!(p.effect(OFFICIAL_KA_COUNTER), Some(-6000));
        // chiVe が要る述語は既定に書かない（chive.tsv に書く）
        assert_eq!(p.effect(LEMMA_RANK), None);
        assert_eq!(p.effect(KATAKANA_ANALYSIS), None);
        assert_eq!(p.effect(CHEAP_RARE_HOMOPHONE), None);
    }

    #[test]
    fn chive_policy_is_the_default_plus_the_chive_predicates() {
        let text = include_str!("../policy/chive.tsv");
        let chive = Policy::parse(text).unwrap();
        assert!(chive.uses_chive_vocab());
        assert_eq!(chive.effect(LEMMA_RANK), Some(2000));
        assert!(chive.effect(KATAKANA_ANALYSIS).is_some());
        assert_eq!(chive.effect(EXTENSION_PEER), Some(3000));
        assert_eq!(chive.effect(CHEAP_RARE_HOMOPHONE), Some(6518));
        assert_eq!(chive.effect(DEFAULT_COST_HOMOPHONE), Some(600));
        let base = Policy::builtin();
        for name in PREDICATES.iter().filter(|n| {
            ![
                LEMMA_RANK,
                KATAKANA_ANALYSIS,
                FREQUENT_COMPOUND,
                EXTENSION_PEER,
                CHEAP_RARE_HOMOPHONE,
                KATAKANA_PLACE_PEER,
                DEFAULT_COST_HOMOPHONE,
            ]
            .contains(n)
        }) {
            assert_eq!(chive.effect(name), base.effect(name), "{name}");
        }
    }

    #[test]
    fn parse_accepts_comments_and_rejects_bad_lines() {
        let p = Policy::parse("# c\n\nproper_noun\t-500\nascii_lower\t7\n").unwrap();
        assert_eq!(p.effect(PROPER_NOUN), Some(-500));
        assert_eq!(p.effect(ASCII_LOWER), Some(7));
        for bad in [
            "proper_noun 100",                // タブがない
            "unknown\t100",                   // 未知の述語
            "proper_noun\tabc",               // 値が不正
            "ascii_lower\texclude",           // 除外の仕組みはない
            "proper_noun\t1\nproper_noun\t2", // 重複
            "proper_noun\t99999",             // 範囲外
        ] {
            assert!(Policy::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn fingerprint_ignores_comments_and_order() {
        let a = Policy::parse("proper_noun\t1000\nascii_lower\t10\n").unwrap();
        let b = Policy::parse("# x\nascii_lower\t10\n\nproper_noun\t1000\n").unwrap();
        let c = Policy::parse("proper_noun\t1001\nascii_lower\t10\n").unwrap();
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_ne!(a.fingerprint(), c.fingerprint());
    }

    #[test]
    fn ascii_lower_is_pushed_far_back_and_ascii_other_penalizes() {
        let p = Policy::builtin();
        for s in [
            "hennkann", "relax", "a.b", "it's", "r&d", "x-ray", "mp3", "go home",
        ] {
            let v = eval(&p, s, s, NOUN);
            // 除外はしない（候補には残すが、上位には出さない）
            assert_ne!(v.penalty, IME_EXCLUDED, "{s}");
            assert_eq!(v.penalty, 10000, "{s}");
        }
        // 大文字や日本語を含むものは ascii_lower ではなく ascii_other
        for s in ["iPhone", "ABC", "x線", "Web2.0"] {
            let v = eval(&p, s, s, NOUN);
            assert_eq!(v.penalty, 3000, "{s}");
            assert_ne!(v.hit_mask & (1 << IDX_ASCII_OTHER), 0);
        }
        // 数字だけ、記号だけは英字を含まない
        assert_eq!(eval(&p, "123", "123", NOUN).penalty, 0);
        assert_eq!(eval(&p, "ひらがな", "ひらがな", NOUN).penalty, 0);
    }

    #[test]
    fn numeric_surface_requires_numeral_pos_and_digit() {
        let p = Policy::builtin();
        let numeral = "名詞,数詞,*,*,*,*";
        assert_eq!(eval(&p, "4", "4", numeral).penalty, 10000);
        assert_eq!(eval(&p, "４", "4", numeral).penalty, 10000);
        assert_eq!(eval(&p, "3つ", "3つ", numeral).penalty, 10000);
        // ローマ数字は数字として扱う
        assert_eq!(eval(&p, "ⅲ", "Ⅲ", numeral).penalty, 10000);
        assert_eq!(eval(&p, "Ⅹ", "Ⅹ", numeral).penalty, 10000);
        // 漢数字は数字を含まないので残る
        assert_eq!(eval(&p, "四", "四", numeral).penalty, 0);
        // 数詞でなければ対象外
        assert_eq!(eval(&p, "100円", "100円", NOUN).penalty, 0);
    }

    #[test]
    fn katakana_variant_needs_katakana_surface_and_non_katakana_lemma() {
        let p = Policy::builtin();
        assert_eq!(eval(&p, "コウ", "こう", NOUN).penalty, 5000);
        assert_eq!(eval(&p, "カキ", "かき", NOUN).penalty, 5000);
        // 本来カタカナの語は対象外
        assert_eq!(eval(&p, "ラーメン", "ラーメン", NOUN).penalty, 0);
        assert_eq!(eval(&p, "ユーザ", "ユーザー", NOUN).penalty, 0);
        // 漢字の lemma でも、カタカナ表記なら対象（スズメ/雀）
        assert_eq!(eval(&p, "スズメ", "雀", NOUN).penalty, 5000);
        // 小書きのカタカナ拡張（ㇲもう/すもう）
        assert_eq!(eval(&p, "ㇲもう", "すもう", NOUN).penalty, 5000);
        // 長音記号だけではカタカナとみなさない
        assert_eq!(eval(&p, "ー", "ー", NOUN).penalty, 0);
        // 漢字かな混じりの表記はカタカナを含まない
        assert_eq!(eval(&p, "構成", "構成", NOUN).penalty, 0);
    }

    #[test]
    fn proper_and_person_names_accumulate() {
        let p = Policy::builtin();
        assert_eq!(
            eval(&p, "更生", "更生", PLACE).penalty,
            1000 + PLACE_NAME_ADD
        );
        assert_eq!(eval(&p, "更生", "更生", PERSON).penalty, 2000);
        assert_eq!(eval(&p, "更生", "更生", NOUN).penalty, 0);
        // カタカナの人名: 5000 + 1000 + 1000
        assert_eq!(eval(&p, "ナッ", "なっ", PERSON).penalty, 7000);
    }

    #[test]
    fn official_ka_counters_hit_only_the_two_character_countable_nouns_with_a_kanji_lemma() {
        const COUNTABLE: &str = "名詞,普通名詞,助数詞可能,*,*,*";
        let p = Policy::parse("official_ka_counter\t-6000\n").unwrap();
        // 公用文の表記は下げる（加点）。異体の 箇月・箇国・箇所 は変えない
        for (ka, kanji) in [("か月", "箇月"), ("か国", "箇国"), ("か所", "箇所")] {
            assert_eq!(eval(&p, ka, kanji, COUNTABLE).penalty, -6000, "{ka}");
            assert_eq!(eval(&p, kanji, kanji, COUNTABLE).penalty, 0, "{kanji}");
        }
        // カタカナ・小書きの表記、箇条 など対象の漢字でないもの、品詞が違うもの、lemma が箇でないものは当たらない
        for (surface, lemma, pos) in [
            ("カ月", "箇月", COUNTABLE),
            ("ヶ所", "箇所", COUNTABLE),
            ("か条", "箇条", COUNTABLE),
            ("か月", "箇月", NOUN),
            ("か月", "か月", COUNTABLE),
            ("か月間", "箇月間", COUNTABLE),
            ("個所", "箇所", COUNTABLE),
        ] {
            assert_eq!(eval(&p, surface, lemma, pos).penalty, 0, "{surface}");
        }
    }

    #[test]
    fn place_name_adds_to_proper_noun_and_only_for_place_names() {
        let p = Policy::parse("proper_noun\t1000\nperson_name\t1000\nplace_name\t-750\n").unwrap();
        // 地名の合計は proper_noun + place_name
        assert_eq!(eval(&p, "梅田", "梅田", PLACE).penalty, 250);
        // 人名は place_name の影響を受けない
        assert_eq!(eval(&p, "梅田", "梅田", PERSON).penalty, 2000);
        // 固有名詞でも地名・人名でなければ proper_noun だけ
        let other = "名詞,固有名詞,一般,*,*,*";
        assert_eq!(eval(&p, "梅田", "梅田", other).penalty, 1000);
        // 品詞2が固有名詞でない地名は対象外
        assert_eq!(
            eval(&p, "梅田", "梅田", "名詞,普通名詞,地名,*,*,*").penalty,
            0
        );
        // 値を書かなければ当たらない
        let q = Policy::parse("proper_noun\t1000\n").unwrap();
        assert_eq!(eval(&q, "梅田", "梅田", PLACE).penalty, 1000);
    }

    #[test]
    fn variant_surface_needs_a_kanji_surface_that_differs_from_the_lemma_of_an_uninflected_word() {
        let p = Policy::parse("variant_surface\t1500\n").unwrap();
        assert_eq!(eval(&p, "浜死", "瀕死", NOUN).penalty, 1500);
        assert_eq!(eval(&p, "ひん死", "瀕死", NOUN).penalty, 1500);
        assert_eq!(eval(&p, "瀕死", "瀕死", NOUN).penalty, 0);
        // ひらがなだけの表記（この/此の）は対象外
        assert_eq!(eval(&p, "この", "此の", "連体詞,*,*,*,*,*").penalty, 0);
        // 活用する語（成っ/成る）は対象外
        let verb = "動詞,非自立可能,*,*,五段-ラ行,連用形-促音便";
        assert_eq!(eval(&p, "成っ", "成る", verb).penalty, 0);
    }

    #[test]
    fn wave_dash_penalizes_decorated_surfaces_but_not_the_mark_itself() {
        let p = Policy::builtin();
        for s in ["の〜", "た〜", "あ～あ", "あれ~"] {
            let v = eval(&p, s, s, NOUN);
            assert_eq!(v.penalty, 3000, "{s}");
            assert_ne!(v.hit_mask & (1 << IDX_WAVE_DASH), 0);
        }
        // 「〜」そのもの（から）は対象外
        assert_eq!(eval(&p, "〜", "〜", "助詞,格助詞,*,*,*,*").penalty, 0);
        assert_eq!(eval(&p, "の", "の", NOUN).penalty, 0);
    }

    fn eval_with_reading(policy: &Policy, surface: &str, reading: &str, pos: &str) -> Verdict {
        policy.evaluate(
            &PolicyRow {
                surface,
                lemma: surface,
                reading,
                pos,
                kanji_peer_rank: None,
                noun_peer_rank: None,
                place_peer_rank: None,
                place_peer_total: None,
                compound_parts_rank: None,
                default_cost_peer_total: None,
                notcore: None,
                homophone: HomophoneInfo::default(),
                kun: KunInfo::default(),
            },
            None,
        )
    }

    #[test]
    fn a_wave_dash_without_a_long_vowel_in_the_reading_is_excluded() {
        let p = Policy::builtin();
        let particle = "助詞,終助詞,*,*,*,*";
        for (surface, reading) in [("ね〜", "ね"), ("あの〜", "あの"), ("よ～", "よ")] {
            let v = eval_with_reading(&p, surface, reading, particle);
            assert_eq!(
                v.excluded,
                Some(ExclusionRule::WaveDashWithoutLongVowel),
                "{surface}"
            );
            assert_eq!(v.penalty, IME_EXCLUDED);
        }
        // 読みの長音に当たる波ダッシュ（あま〜い/あまーい）は対象外
        let v = eval_with_reading(&p, "あま〜い", "あまーい", "形容詞,一般,*,*,*,*");
        assert_eq!(v.excluded, None);
        // 記号そのもの（〜 から、補助記号）は対象外
        assert_eq!(eval_with_reading(&p, "〜", "から", particle).excluded, None);
        assert_eq!(
            eval_with_reading(&p, "〜〜", "きごう", "補助記号,一般,*,*,*,*").excluded,
            None
        );
        // 長音記号「ー」の表記（ユーザー、データ）は対象外
        assert_eq!(
            eval_with_reading(&p, "ユーザー", "ゆーざー", NOUN).excluded,
            None
        );
        assert_eq!(
            eval_with_reading(&p, "データ", "でーた", NOUN).excluded,
            None
        );
    }

    #[test]
    fn a_decoration_mark_is_excluded_except_in_proper_nouns() {
        let p = Policy::builtin();
        let v = eval_with_reading(&p, "ありがとう！", "ありがとう", "感動詞,一般,*,*,*,*");
        assert_eq!(v.excluded, Some(ExclusionRule::DecorationMark));
        assert_eq!(
            eval_with_reading(&p, "ミニモニ。", "みにもに", PERSON).excluded,
            None
        );
        assert_eq!(
            eval_with_reading(&p, "!?", "きごう", "補助記号,一般,*,*,*,*").excluded,
            None
        );
        // 読みにない中点やハイフンは語の一部（wi-fi、ニューヨーク・タイムズ）
        assert_eq!(
            eval_with_reading(&p, "ニューヨーク・タイムズ", "にゅーよーくたいむず", NOUN).excluded,
            None
        );
    }

    #[test]
    fn a_hyphen_inside_a_native_word_is_excluded() {
        let p = Policy::builtin();
        // Sudachi の解析用の表記（読みに対応する音がない）。ダッシュの類はどれも同じ
        for (surface, reading, pos) in [
            ("す‐ぐ", "すぐ", "副詞,*,*,*,*,*"),
            ("げん‐ぷ", "げんぷ", NOUN),
            ("脱‐存", "だつぞん", NOUN),
            ("す-ぐ", "すぐ", "副詞,*,*,*,*,*"),
            ("す－ぐ", "すぐ", "副詞,*,*,*,*,*"),
            ("す−ぐ", "すぐ", "副詞,*,*,*,*,*"),
            ("す―ぐ", "すぐ", "副詞,*,*,*,*,*"),
            ("お‐茶", "おちゃ", NOUN),
        ] {
            let v = eval_with_reading(&p, surface, reading, pos);
            assert_eq!(
                v.excluded,
                Some(ExclusionRule::HyphenInsideNativeWord),
                "{surface}"
            );
            assert_eq!(v.penalty, IME_EXCLUDED);
        }
        // カタカナの外来語、英字の綴り、英字と漢字の境のハイフンは名前の一部
        for (surface, reading, pos) in [
            ("セブン‐イレブン", "せぶんいれぶん", PERSON),
            ("セブン-イレブン", "せぶんいれぶん", PERSON),
            ("フォンテ‐コロンボ", "ふぉんてころんぼ", PERSON),
            ("Wi-Fi", "わいふぁい", PERSON),
            ("HANA‐BI", "はなび", NOUN),
            ("e-文書法", "いーぶんしょほう", NOUN),
            // 長音に当たるダッシュ（アンサ−/アンサー、すご−い/スゴーイ）
            ("アンサ−", "あんさー", NOUN),
            ("すご−い", "すごーい", "形容詞,一般,*,*,*,*"),
            // 長音記号と波ダッシュはこの規則の対象ではない
            ("らーめん", "らーめん", NOUN),
        ] {
            assert_eq!(
                eval_with_reading(&p, surface, reading, pos).excluded,
                None,
                "{surface}"
            );
        }
        // 記号そのものの項目と 1 文字の表記は対象外
        assert_eq!(
            eval_with_reading(&p, "−", "から", "助詞,格助詞,*,*,*,*").excluded,
            None
        );
        assert_eq!(
            eval_with_reading(&p, "あ‐い", "きごう", "補助記号,一般,*,*,*,*").excluded,
            None
        );
    }

    #[test]
    fn function_words_with_ascii_or_wave_dash_go_far_back_but_nouns_do_not() {
        let p = Policy::builtin();
        let particle = "助詞,終助詞,*,*,*,*";
        // よ → YO: ascii_other 3000 + function_word_notation 10000
        assert_eq!(eval(&p, "YO", "YO", particle).penalty, 13000);
        assert_eq!(eval(&p, "yo", "yo", particle).penalty, 20000);
        // 助動詞の波ダッシュ入り（読みに長音がある ね〜/ネー）: wave_dash 3000 + 10000
        assert_eq!(
            eval_with_reading(&p, "ね〜", "ねー", "助動詞,*,*,*,*,*").penalty,
            13000
        );
        // 名詞の英字表記は下げない（ascii_other の 3000 のまま）
        assert_eq!(eval(&p, "PC", "PC", NOUN).penalty, 3000);
        assert_eq!(eval(&p, "API", "API", NOUN).penalty, 3000);
        // ひらがなの機能語、1 文字の記号は対象外
        assert_eq!(eval(&p, "よ", "よ", particle).penalty, 0);
    }

    #[test]
    fn function_word_katakana_goes_far_back_except_small_ke() {
        let p = Policy::builtin();
        let aux = "助動詞,*,*,*,助動詞-タ,終止形-一般";
        let particle = "助詞,副助詞,*,*,*,*";
        // 治っタ のタ: katakana_variant 5000 + function_word_notation 10000（1 文字でも当たる）
        assert_eq!(eval(&p, "タ", "た", aux).penalty, 15000);
        assert_eq!(eval(&p, "マス", "ます", aux).penalty, 15000);
        assert_eq!(eval(&p, "ネ", "ね", particle).penalty, 15000);
        // lemma もカタカナなら対象外。ひらがなも対象外
        assert_eq!(eval(&p, "タ", "タ", aux).penalty, 0);
        assert_eq!(eval(&p, "た", "た", aux).penalty, 0);
        // ヶ・ヵ は正しい表記。接尾辞（カ月）と名詞は対象にしない
        assert_eq!(eval(&p, "ヶ", "か", particle).penalty, 5000);
        assert_eq!(eval(&p, "ヵ", "か", particle).penalty, 5000);
        // （カ月 は katakana_variant 5000 + variant_surface 1500。function_word_notation は足さない）
        assert_eq!(
            eval(&p, "カ月", "か月", "接尾辞,名詞的,助数詞,*,*,*").penalty,
            6500
        );
        assert_eq!(eval(&p, "タ", "た", NOUN).penalty, 5000);
        let v = eval(&p, "タ", "た", aux);
        assert_ne!(v.hit_mask & (1 << IDX_FUNCTION_WORD_NOTATION), 0);
    }

    #[test]
    fn kun_predicates_use_the_precomputed_notation() {
        let p = Policy::builtin();
        assert_eq!(p.effect(KUN_OUTSIDE), Some(3000));
        assert_eq!(p.effect(KUN_LEMMA_DEFAULT), Some(-50));
        assert_eq!(p.effect(KUN_READING_DEFAULT), Some(-500));
        assert!(p.uses_kun_table());
        let verb = "動詞,一般,*,*,五段-ラ行,終止形-一般";
        let eval_kun = |kun: KunInfo| {
            p.evaluate(
                &PolicyRow {
                    surface: "取る",
                    lemma: "取る",
                    reading: "とる",
                    pos: verb,
                    kanji_peer_rank: None,
                    noun_peer_rank: None,
                    place_peer_rank: None,
                    place_peer_total: None,
                    compound_parts_rank: None,
                    default_cost_peer_total: None,
                    notcore: None,
                    homophone: HomophoneInfo::default(),
                    kun,
                },
                None,
            )
        };
        assert_eq!(eval_kun(KunInfo::default()).penalty, 0);
        // 表外: +3000
        let outside = eval_kun(KunInfo {
            outside: true,
            ..KunInfo::default()
        });
        assert_eq!(outside.penalty, 3000);
        assert_ne!(outside.hit_mask & (1 << IDX_KUN_OUTSIDE), 0);
        // lemma の中の既定: ほかの表記より 1734 高いとき、50 だけ安くなるように -1784
        let lemma_default = |excess| {
            eval_kun(KunInfo {
                lemma_default_excess: Some(excess),
                ..KunInfo::default()
            })
        };
        assert_eq!(lemma_default(1734).penalty, -1784);
        assert_eq!(lemma_default(0).penalty, -50);
        // すでに安い（差が負）なら変えない。当たりもしない
        for excess in [-1, -50, -3000] {
            let v = lemma_default(excess);
            assert_eq!(v.penalty, 0, "{excess}");
            assert_eq!(v.hit_mask & (1 << IDX_KUN_LEMMA_DEFAULT), 0);
        }
        // 読みの組の既定: -500。lemma の中の既定と加算する
        let both = eval_kun(KunInfo {
            outside: false,
            lemma_default_excess: Some(100),
            reading_default: true,
        });
        assert_eq!(both.penalty, -50 - 100 - 500);
        // 値が書かれていなければ何もしない（表を使わない）
        let none = Policy::parse("proper_noun\t1000\n").unwrap();
        assert!(!none.uses_kun_table());
        let v = none.evaluate(
            &PolicyRow {
                surface: "取る",
                lemma: "取る",
                reading: "とる",
                pos: verb,
                kanji_peer_rank: None,
                noun_peer_rank: None,
                place_peer_rank: None,
                place_peer_total: None,
                compound_parts_rank: None,
                default_cost_peer_total: None,
                notcore: None,
                homophone: HomophoneInfo::default(),
                kun: KunInfo {
                    outside: true,
                    lemma_default_excess: Some(10),
                    reading_default: true,
                },
            },
            None,
        );
        assert_eq!((v.penalty, v.hit_mask), (0, 0));
    }

    #[test]
    fn fingerprint_includes_the_kun_table_only_when_it_is_used() {
        // 表を使わない述語だけのポリシーは、表の内容に左右されない（正規化したテキストだけで決まる）
        let a = Policy::parse("proper_noun\t1000\n").unwrap();
        assert_eq!(a.fingerprint(), {
            let mut h = Fnv1a64::new();
            h.update(a.canonical_text().as_bytes());
            h.finish()
        });
        let b = Policy::parse("proper_noun\t1000\nkun_outside\t3000\n").unwrap();
        let mut h = Fnv1a64::new();
        h.update(b.canonical_text().as_bytes());
        assert_ne!(b.fingerprint(), h.finish());
    }

    #[test]
    fn the_kanji_zero_is_a_numeric_surface_but_other_kanji_numerals_are_not() {
        let p = Policy::builtin();
        let numeral = "名詞,数詞,*,*,*,*";
        assert_eq!(eval(&p, "〇", "〇", numeral).penalty, 10000);
        assert_eq!(eval(&p, "一", "一", numeral).penalty, 0);
        assert_eq!(eval(&p, "二", "二", numeral).penalty, 0);
        // 数詞でなければ当たらない
        assert_eq!(eval(&p, "〇", "〇", "補助記号,一般,*,*,*,*").penalty, 0);
    }

    #[test]
    fn frequent_compounds_get_a_boost_only_when_one_word_noun_of_kanji_and_frequent() {
        let p = Policy::parse("frequent_compound\t-8000\n").unwrap();
        let vocab = ChiveVocab::from_keys(["具体", "具体例", "礼"].map(String::from));
        let eval = |surface: &str, pos: &str| {
            p.evaluate(
                &PolicyRow {
                    surface,
                    lemma: surface,
                    reading: "ー",
                    pos,
                    kanji_peer_rank: None,
                    noun_peer_rank: None,
                    place_peer_rank: None,
                    place_peer_total: None,
                    compound_parts_rank: None,
                    default_cost_peer_total: None,
                    notcore: None,
                    homophone: HomophoneInfo::default(),
                    kun: KunInfo::default(),
                },
                Some(&vocab),
            )
            .penalty
        };
        assert_eq!(eval("具体例", NOUN), -8000);
        assert_eq!(eval("具体", NOUN), 0, "2 文字は対象外");
        assert_eq!(eval("具体例", PERSON), 0, "固有名詞は対象外");
        assert_eq!(eval("未登録語", NOUN), 0, "chiVe の語彙にない語は対象外");
        assert_eq!(eval("ぐたい例", NOUN), 0, "漢字だけの語に限る");
    }

    #[test]
    fn frequent_compounds_stop_at_the_rank_limit() {
        let p = Policy::parse("frequent_compound\t-8000\n").unwrap();
        // 頻度順位は行番号。上限の直前（構造化 31404 位のような語）は当たり、上限からは当たらない
        let mut keys: Vec<String> = (0..COMPOUND_MAX_RANK - 1)
            .map(|i| format!("k{i}"))
            .collect();
        keys.push("構造化".to_string());
        keys.push("伊富魚".to_string());
        let vocab = ChiveVocab::from_keys(keys);
        assert_eq!(vocab.rank("構造化"), Some((COMPOUND_MAX_RANK - 1) as u32));
        let eval = |surface: &str| {
            p.evaluate(
                &PolicyRow {
                    surface,
                    lemma: surface,
                    reading: "ー",
                    pos: NOUN,
                    kanji_peer_rank: None,
                    noun_peer_rank: None,
                    place_peer_rank: None,
                    place_peer_total: None,
                    compound_parts_rank: None,
                    default_cost_peer_total: None,
                    notcore: None,
                    homophone: HomophoneInfo::default(),
                    kun: KunInfo::default(),
                },
                Some(&vocab),
            )
            .penalty
        };
        assert_eq!(eval("構造化"), -8000);
        assert_eq!(eval("伊富魚"), 0, "上限の順位からは対象外");
    }

    #[test]
    fn katakana_place_goes_back_only_with_a_frequent_kanji_place_peer() {
        let p = Policy::parse("katakana_place_peer\t600\n").unwrap();
        assert!(p.uses_chive_vocab());
        const PLACE: &str = "名詞,固有名詞,地名,一般,*,*";
        // 順位 = 並びの位置。シブヤ はカタカナのほうが頻度が低い（漢字の 渋谷 は 100 位）、カナダ は高い
        let mut keys: Vec<String> = (0..KATAKANA_PLACE_PEER_MAX_RANK + 10)
            .map(|i| format!("k{i}"))
            .collect();
        for (rank, word) in [(20, "カナダ"), (30_000, "シブヤ")] {
            keys[rank] = word.to_string();
        }
        let vocab = ChiveVocab::from_keys(keys);
        let penalty = |surface: &str, pos: &str, peer: Option<usize>| {
            p.evaluate(
                &PolicyRow {
                    surface,
                    lemma: surface,
                    reading: "ー",
                    pos,
                    kanji_peer_rank: None,
                    noun_peer_rank: None,
                    place_peer_rank: peer,
                    place_peer_total: peer.map(|_| 8000),
                    compound_parts_rank: None,
                    default_cost_peer_total: None,
                    notcore: None,
                    homophone: HomophoneInfo {
                        cost: 4000,
                        ..HomophoneInfo::default()
                    },
                    kun: KunInfo::default(),
                },
                Some(&vocab),
            )
            .penalty
        };
        // 総コストを「漢字の地名の総コスト 8000 + 余白 600」にそろえる: 4000 + 4600 = 8600。
        // シブヤ（渋谷 100 位。カタカナは 3 万位）、長音を含む表記（語彙にない）も対象
        assert_eq!(penalty("シブヤ", PLACE, Some(100)), 4600);
        assert_eq!(penalty("ニューヨーク", PLACE, Some(100)), 4600);
        // カタカナのほうが頻度が高い（カナダ 20 位 と 金田 22468 位）は対象外
        assert_eq!(penalty("カナダ", PLACE, Some(22_468)), 0);
        // 同じ読みに漢字の地名がない（ロンドン）、頻度が低い当て字だけ（雅典 82716 位）は対象外
        assert_eq!(penalty("ロンドン", PLACE, None), 0);
        assert_eq!(penalty("アテネ", PLACE, Some(82_716)), 0);
        assert_eq!(
            penalty("アテネ", PLACE, Some(KATAKANA_PLACE_PEER_MAX_RANK)),
            0
        );
        // 漢字の地名自身、人名、カタカナでない表記は対象外
        assert_eq!(penalty("渋谷", PLACE, Some(100)), 0);
        // 固有名詞の一般（カワサキ）と人名（サカエ）も対象。固有名詞でない語は対象外
        assert_eq!(
            penalty("シブヤ", "名詞,固有名詞,人名,姓,*,*", Some(100)),
            4600
        );
        assert_eq!(
            penalty("カワサキ", "名詞,固有名詞,一般,*,*,*", Some(100)),
            4600
        );
        assert_eq!(
            penalty("カマクラ", "名詞,普通名詞,一般,*,*,*", Some(100)),
            0
        );
    }

    /// 順位 = 並びの位置。4 万位、10 万位の前後に語を置いた語彙（ADR 0011）
    fn guarded_vocab() -> ChiveVocab {
        let mut keys: Vec<String> = (0..COMPOUND_GUARDED_MAX_RANK + 1)
            .map(|i| format!("k{i}"))
            .collect();
        for (rank, lemma) in [
            (10, "葬式"),
            (COMPOUND_MAX_RANK - 1, "消火器"),
            (COMPOUND_MAX_RANK, "総指揮"),
            (83_455, "形態素"),
            (COMPOUND_GUARDED_MAX_RANK - 1, "守備範囲"),
            (COMPOUND_GUARDED_MAX_RANK, "圏外語"),
        ] {
            keys[rank] = lemma.to_string();
        }
        ChiveVocab::from_keys(keys)
    }

    fn guarded_penalty(surface: &str, peer_rank: Option<usize>) -> i16 {
        let p = Policy::parse("frequent_compound\t-8000\n").unwrap();
        p.evaluate(
            &PolicyRow {
                surface,
                lemma: surface,
                reading: "ー",
                pos: NOUN,
                kanji_peer_rank: None,
                noun_peer_rank: peer_rank,
                place_peer_rank: None,
                place_peer_total: None,
                compound_parts_rank: None,
                default_cost_peer_total: None,
                notcore: None,
                homophone: HomophoneInfo::default(),
                kun: KunInfo::default(),
            },
            Some(&guarded_vocab()),
        )
        .penalty
    }

    #[test]
    fn frequent_compounds_between_the_limits_need_to_be_the_top_noun_of_the_reading() {
        // 形態素（83455 位）: 同じ読みの名詞の最小の順位が自身（読みで最上位）なら当たる
        assert_eq!(guarded_penalty("形態素", Some(83_455)), -8000);
        // 同じ読みにもっと上位の名詞（葬式 10 位）があれば当たらない
        assert_eq!(guarded_penalty("総指揮", Some(10)), 0);
        // 守りの値が求まっていない（None）なら当たらない
        assert_eq!(guarded_penalty("形態素", None), 0);
        // 上限の直前は当たり、上限の順位からは守りがあっても当たらない
        assert_eq!(
            guarded_penalty("守備範囲", Some(COMPOUND_GUARDED_MAX_RANK - 1)),
            -8000
        );
        assert_eq!(
            guarded_penalty("圏外語", Some(COMPOUND_GUARDED_MAX_RANK)),
            0
        );
        // chiVe にない語は当たらない
        assert_eq!(guarded_penalty("未登録語", Some(0)), 0);
        // 4 万位以内は守りによらず当たる（今の規則。消火器 は同じ読みに上位の名詞があっても当たる）
        assert_eq!(guarded_penalty("消火器", Some(10)), -8000);
        // 4 万位ちょうどからは守りが要る
        assert_eq!(guarded_penalty("総指揮", Some(COMPOUND_MAX_RANK)), -8000);
    }

    #[test]
    fn noun_peers_include_proper_nouns_but_not_symbols_suffixes_or_kana_only_surfaces() {
        let peer = |surface, pos| {
            is_noun_peer(&PolicyRow {
                surface,
                lemma: surface,
                reading: "ー",
                pos,
                kanji_peer_rank: None,
                noun_peer_rank: None,
                place_peer_rank: None,
                place_peer_total: None,
                compound_parts_rank: None,
                default_cost_peer_total: None,
                notcore: None,
                homophone: HomophoneInfo::default(),
                kun: KunInfo::default(),
            })
        };
        assert!(peer("葬式", NOUN));
        assert!(peer("伊藤", PERSON), "固有名詞も数える");
        assert!(!peer("そうしき", NOUN), "漢字を含まない");
        assert!(!peer("素", "接尾辞,名詞的,一般,*,*,*"), "名詞以外");
        assert!(!peer("〇", "記号,一般,*,*,*,*"));
    }

    #[test]
    fn symbol_pos_is_penalized() {
        let p = Policy::builtin();
        assert_eq!(eval(&p, "の", "の", "記号,一般,*,*,*,*").penalty, 3000);
        assert_eq!(eval(&p, "の", "の", "助詞,格助詞,*,*,*,*").penalty, 0);
    }

    #[test]
    fn lemma_rank_grows_with_the_decades_of_rank() {
        let p = Policy::parse("lemma_rank\t1000\n").unwrap();
        assert!(p.uses_chive_vocab());
        // 順位 0〜9999 の 10000 語。「語N」の順位は N
        let vocab = ChiveVocab::from_keys((0..10_000).map(|i| format!("語{i}")));
        let penalty = |lemma: &str| {
            let row = PolicyRow {
                surface: "x",
                lemma,
                reading: "ー",
                pos: NOUN,
                kanji_peer_rank: None,
                noun_peer_rank: None,
                place_peer_rank: None,
                place_peer_total: None,
                compound_parts_rank: None,
                default_cost_peer_total: None,
                notcore: None,
                homophone: HomophoneInfo::default(),
                kun: KunInfo::default(),
            };
            p.evaluate(&row, Some(&vocab)).penalty
        };
        // 1000 位まで（log10(順位 + 1) <= 3）は無料
        assert_eq!(penalty("語0"), 0);
        assert_eq!(penalty("語999"), 0);
        // 9999 位は log10(10000) - 3 = 1 桁分
        assert_eq!(penalty("語9999"), 1000);
        // 1 桁の途中は比例する（99 → log10(100) = 2 で無料。4999 → log10(5000) - 3 = 0.699）
        assert_eq!(penalty("語4999"), 699);
        // 語彙にないものは最後より後ろ（順位 = 語彙の大きさ）
        assert_eq!(penalty("無い語"), 1000);
        // 語彙がなければ当たらない
        let row = PolicyRow {
            surface: "x",
            lemma: "語9999",
            reading: "ー",
            pos: NOUN,
            kanji_peer_rank: None,
            noun_peer_rank: None,
            place_peer_rank: None,
            place_peer_total: None,
            compound_parts_rank: None,
            default_cost_peer_total: None,
            notcore: None,
            homophone: HomophoneInfo::default(),
            kun: KunInfo::default(),
        };
        assert_eq!(p.evaluate(&row, None).penalty, 0);
    }

    /// 語彙: 順位 0 = 名（頻度が高い漢字の語）、1 = ここ。ほかは「語N」で順位 N（メイ = 15000、ペン = 4000、ドル = 9999）
    fn analysis_vocab() -> ChiveVocab {
        let mut keys: Vec<String> = (0..20_000).map(|i| format!("語{i}")).collect();
        for (rank, lemma) in [
            (10, "名"),
            (15_000, "メイ"),
            (4_000, "ペン"),
            (9_999, "ドル"),
            (15_500, "メモリ"),
        ] {
            keys[rank] = lemma.to_string();
        }
        ChiveVocab::from_keys(keys)
    }

    fn analysis_penalty(p: &Policy, surface: &str, pos: &str, peer_rank: Option<usize>) -> i16 {
        let row = PolicyRow {
            surface,
            lemma: surface,
            reading: "ー",
            pos,
            kanji_peer_rank: peer_rank,
            noun_peer_rank: None,
            place_peer_rank: None,
            place_peer_total: None,
            compound_parts_rank: None,
            default_cost_peer_total: None,
            notcore: None,
            homophone: HomophoneInfo::default(),
            kun: KunInfo::default(),
        };
        p.evaluate(&row, Some(&analysis_vocab())).penalty
    }

    #[test]
    fn katakana_analysis_hits_short_rare_katakana_nouns_with_a_much_more_frequent_kanji_peer() {
        let p = Policy::parse("katakana_analysis\t3000\n").unwrap();
        assert!(p.uses_chive_vocab());
        // メイ（15000 位）に、同じ読みの 名（10 位）がある
        assert_eq!(analysis_penalty(&p, "メイ", NOUN, Some(10)), 3000);
        // 同じ読みに漢字の語がなければ対象外（ココア、メイン）
        assert_eq!(analysis_penalty(&p, "メイ", NOUN, None), 0);
        // 漢字の語との頻度の差が 10 倍未満（メイ 15000 位、漢字 5000 位）なら対象外
        assert_eq!(analysis_penalty(&p, "メイ", NOUN, Some(5_000)), 0);
        // 自身が 1 万位より上のカタカナ語（ペン 4000 位、ドル 9999 位）は、漢字の語がもっと上でも対象外
        assert_eq!(analysis_penalty(&p, "ペン", NOUN, Some(10)), 0);
        assert_eq!(analysis_penalty(&p, "ドル", NOUN, Some(10)), 0);
        // 4 文字以上は対象外
        assert_eq!(analysis_penalty(&p, "メモリー", NOUN, Some(10)), 0);
        assert_eq!(analysis_penalty(&p, "メイメイメ", NOUN, Some(10)), 0);
        // 普通名詞以外（固有名詞）は対象外
        assert_eq!(analysis_penalty(&p, "メイ", PERSON, Some(10)), 0);
        // 語彙がなければ当たらない
        let row = PolicyRow {
            surface: "メイ",
            lemma: "メイ",
            reading: "ー",
            pos: NOUN,
            kanji_peer_rank: Some(10),
            noun_peer_rank: None,
            place_peer_rank: None,
            place_peer_total: None,
            compound_parts_rank: None,
            default_cost_peer_total: None,
            notcore: None,
            homophone: HomophoneInfo::default(),
            kun: KunInfo::default(),
        };
        assert_eq!(p.evaluate(&row, None).penalty, 0);
        // lemma が表記と違う行（コウ/こう）は対象外（katakana_variant の担当）
        let variant = PolicyRow {
            surface: "メイ",
            lemma: "名",
            reading: "ー",
            pos: NOUN,
            kanji_peer_rank: Some(10),
            noun_peer_rank: None,
            place_peer_rank: None,
            place_peer_total: None,
            compound_parts_rank: None,
            default_cost_peer_total: None,
            notcore: None,
            homophone: HomophoneInfo::default(),
            kun: KunInfo::default(),
        };
        assert_eq!(p.evaluate(&variant, Some(&analysis_vocab())).penalty, 0);
    }

    #[test]
    fn kanji_peers_exclude_proper_nouns_and_symbols_and_kana_only_surfaces() {
        let peer = |surface, pos| {
            is_kanji_peer(&PolicyRow {
                surface,
                lemma: surface,
                reading: "ー",
                pos,
                kanji_peer_rank: None,
                noun_peer_rank: None,
                place_peer_rank: None,
                place_peer_total: None,
                compound_parts_rank: None,
                default_cost_peer_total: None,
                notcore: None,
                homophone: HomophoneInfo::default(),
                kun: KunInfo::default(),
            })
        };
        assert!(peer("名", NOUN));
        assert!(peer("名", "接頭辞,*,*,*,*,*"));
        assert!(!peer("名", PERSON));
        assert!(!peer("名", "記号,一般,*,*,*,*"));
        assert!(!peer("メイ", NOUN));
        assert!(!peer("めい", NOUN));
    }

    #[test]
    fn duplicate_chive_keys_keep_the_first_rank() {
        let vocab = ChiveVocab::from_keys(["a".to_string(), "b".to_string(), "a".to_string()]);
        assert_eq!(vocab.rank("a"), Some(0));
        assert_eq!(vocab.rank("b"), Some(1));
        assert_eq!(vocab.len(), 2);
    }

    #[test]
    fn sum_saturates_below_the_excluded_marker() {
        let p =
            Policy::parse("symbol_kana\t30000\nproper_noun\t30000\nperson_name\t30000\n").unwrap();
        let v = eval(&p, "x", "x", "記号,固有名詞,人名,*,*,*");
        assert_eq!(v.penalty, IME_EXCLUDED - 1);
        let neg = Policy::parse("symbol_kana\t-30000\nproper_noun\t-30000\nperson_name\t-30000\n")
            .unwrap();
        assert_eq!(
            eval(&neg, "x", "x", "記号,固有名詞,人名,*,*,*").penalty,
            i16::MIN
        );
    }

    #[test]
    fn stats_count_hits_and_bins() {
        let p = Policy::builtin();
        let mut stats = PolicyStats::default();
        let rows = [
            ("コウ", "こう", NOUN),
            ("abc", "abc", NOUN),
            ("更生", "更生", PERSON),
            ("犬", "犬", NOUN),
        ];
        for (surface, lemma, pos) in rows {
            let row = PolicyRow {
                surface,
                lemma,
                reading: "ー",
                kanji_peer_rank: None,
                noun_peer_rank: None,
                place_peer_rank: None,
                place_peer_total: None,
                compound_parts_rank: None,
                default_cost_peer_total: None,
                notcore: None,
                homophone: HomophoneInfo::default(),
                kun: KunInfo::default(),
                pos,
            };
            let v = p.evaluate(&row, None);
            stats.record(&row, v);
        }
        assert_eq!(stats.rows, 4);
        assert_eq!(stats.penalized, 3);
        let by = stats.hits_by_predicate();
        assert_eq!(by[ASCII_LOWER], 1);
        assert_eq!(by[KATAKANA_VARIANT], 1);
        assert_eq!(by[PERSON_NAME], 1);
        assert_eq!(by[LEMMA_RANK], 0);
        let md = stats.to_markdown(&p, None);
        assert!(md.contains("`ascii_lower`"), "{md}");
        assert!(md.contains("コウ（こう）"), "{md}");
        // 異字同訓の表の述語は、当たらなくても表に出る。表の語数も出る
        for name in [KUN_OUTSIDE, KUN_LEMMA_DEFAULT, KUN_READING_DEFAULT] {
            assert!(md.contains(&format!("`{name}`")), "{md}");
        }
        assert!(md.contains("異字同訓の表（ADR 0009）"), "{md}");
    }

    // ---- cheap_rare_homophone（ADR 0016 規則 R） ----

    const HOMOPHONE_FLOOR: i32 = 6518;

    /// 語彙の大きさ。この順位以上は「2 万位以下」の判定に使う
    const HOMOPHONE_VOCAB: usize = 60_000;

    /// 順位 `peer_rank` に 成功、`own_rank` に 製鋼 を置いた chiVe の語彙（残りは埋め草）
    fn homophone_vocab(peer_rank: usize, own_rank: usize) -> ChiveVocab {
        let mut keys: Vec<String> = (0..HOMOPHONE_VOCAB).map(|i| format!("k{i}")).collect();
        keys[peer_rank] = "成功".to_string();
        keys[own_rank] = "製鋼".to_string();
        ChiveVocab::from_keys(keys)
    }

    /// 製鋼（読み せいこう）の行を評価した加算量。`peer` は同じ読みの最も頻度の高い語の順位
    fn cheap_rare_penalty(
        surface: &str,
        pos: &str,
        cost: i16,
        peer: Option<usize>,
        vocab: &ChiveVocab,
    ) -> i16 {
        let p = Policy::parse("cheap_rare_homophone\t6518\n").unwrap();
        p.evaluate(
            &PolicyRow {
                surface,
                lemma: surface,
                reading: "せいこう",
                pos,
                kanji_peer_rank: None,
                noun_peer_rank: None,
                place_peer_rank: None,
                place_peer_total: None,
                compound_parts_rank: None,
                default_cost_peer_total: None,
                notcore: None,
                homophone: HomophoneInfo {
                    cost,
                    peer_rank: peer,
                },
                kun: KunInfo::default(),
            },
            Some(vocab),
        )
        .penalty
    }

    #[test]
    fn cheap_rare_homophone_raises_the_cost_to_the_floor() {
        let vocab = homophone_vocab(956, 48_327);
        // 製鋼（コスト 5268、48327 位）は 成功（956 位）より 1.7 桁まれ。6518 - 5268 を足す
        assert_eq!(
            cheap_rare_penalty("製鋼", NOUN, 5268, Some(956), &vocab),
            1250
        );
        // 加算後のコストは下限にそろう
        assert_eq!(5268 + 1250, HOMOPHONE_FLOOR as i16);
    }

    #[test]
    fn cheap_rare_homophone_needs_a_cost_below_the_floor() {
        let vocab = homophone_vocab(956, 48_327);
        assert_eq!(cheap_rare_penalty("製鋼", NOUN, 6517, Some(956), &vocab), 1);
        assert_eq!(cheap_rare_penalty("製鋼", NOUN, 6518, Some(956), &vocab), 0);
        assert_eq!(cheap_rare_penalty("製鋼", NOUN, 7404, Some(956), &vocab), 0);
    }

    #[test]
    fn cheap_rare_homophone_rank_boundary_is_20000() {
        // 同音語は 1 位。順位の条件だけを見る
        let at = |own: usize| {
            let vocab = homophone_vocab(0, own);
            cheap_rare_penalty("製鋼", NOUN, 5000, Some(0), &vocab)
        };
        assert_eq!(at(HOMOPHONE_MIN_RANK - 1), 0, "19999 位は対象外");
        assert_eq!(at(HOMOPHONE_MIN_RANK), 1518, "2 万位は対象");
        assert_eq!(at(HOMOPHONE_MIN_RANK + 1), 1518);
    }

    #[test]
    fn cheap_rare_homophone_gap_boundary_is_half_a_decade() {
        // 同音語 9000 位（log10 = 3.954）。差 0.5 桁になる順位は 約 28464
        let at = |own: usize| {
            let vocab = homophone_vocab(9000, own);
            cheap_rare_penalty("製鋼", NOUN, 5000, Some(9000), &vocab)
        };
        assert_eq!(at(28_000), 0, "差 0.493 桁は対象外");
        assert_eq!(at(29_000), 1518, "差 0.508 桁は対象");
        // 同音語がずっと上位でなければ、安い語でも困らない（頻度の近い同音語）
        assert_eq!(at(HOMOPHONE_MIN_RANK), 0);
        // 同音語が自分自身（差 0）
        let vocab = homophone_vocab(30_000, 30_000);
        assert_eq!(
            cheap_rare_penalty("製鋼", NOUN, 5000, Some(30_000), &vocab),
            0
        );
    }

    #[test]
    fn cheap_rare_homophone_uses_words_outside_the_vocab_as_the_last_rank() {
        let vocab = homophone_vocab(956, 48_327);
        assert_eq!(
            cheap_rare_penalty("右中", NOUN, 3144, Some(956), &vocab),
            6518 - 3144
        );
    }

    #[test]
    fn cheap_rare_homophone_skips_rows_outside_the_target() {
        let vocab = homophone_vocab(956, 48_327);
        let hit = |surface, pos, peer| cheap_rare_penalty(surface, pos, 5000, peer, &vocab);
        assert_eq!(hit("製鋼", NOUN, Some(956)), 1518);
        assert_eq!(hit("製鋼", NOUN, None), 0, "同音語の頻度がなければ対象外");
        assert_eq!(hit("製鋼", PERSON, Some(956)), 0, "固有名詞は対象外");
        assert_eq!(hit("製鋼", "名詞,数詞,*,*,*,*", Some(956)), 0);
        assert_eq!(hit("製", NOUN, Some(956)), 0, "1 文字は対象外");
        assert_eq!(hit("せい鋼", NOUN, Some(956)), 0, "漢字だけの語に限る");
        assert_eq!(hit("製鋼", "動詞,一般,*,*,*,*", Some(956)), 0);
    }

    #[test]
    fn cheap_rare_homophone_needs_surface_equal_to_lemma_and_a_vocab() {
        let p = Policy::parse("cheap_rare_homophone\t6518\n").unwrap();
        assert!(p.uses_chive_vocab());
        let vocab = homophone_vocab(956, 48_327);
        let row = |surface, lemma| PolicyRow {
            surface,
            lemma,
            reading: "せいこう",
            pos: NOUN,
            kanji_peer_rank: None,
            noun_peer_rank: None,
            place_peer_rank: None,
            place_peer_total: None,
            compound_parts_rank: None,
            default_cost_peer_total: None,
            notcore: None,
            homophone: HomophoneInfo {
                cost: 5000,
                peer_rank: Some(956),
            },
            kun: KunInfo::default(),
        };
        assert_eq!(p.evaluate(&row("製鋼", "製鋼"), Some(&vocab)).penalty, 1518);
        assert_eq!(p.evaluate(&row("製剛", "製鋼"), Some(&vocab)).penalty, 0);
        assert_eq!(p.evaluate(&row("製鋼", "製鋼"), None).penalty, 0);
        // 述語が書かれていなければ何もしない
        let off = Policy::parse("proper_noun\t1000\n").unwrap();
        assert_eq!(off.evaluate(&row("製鋼", "製鋼"), Some(&vocab)).penalty, 0);
    }

    // ---- frequent_compound の (f)（ADR 0035 規則 F） ----

    fn parts_row<'a>(
        surface: &'a str,
        parts: Option<usize>,
        noun_peer: Option<usize>,
    ) -> PolicyRow<'a> {
        PolicyRow {
            surface,
            lemma: surface,
            reading: "ー",
            pos: NOUN,
            kanji_peer_rank: None,
            noun_peer_rank: noun_peer,
            place_peer_rank: None,
            place_peer_total: None,
            compound_parts_rank: parts,
            default_cost_peer_total: None,
            notcore: None,
            homophone: HomophoneInfo::default(),
            kun: KunInfo::default(),
        }
    }

    #[test]
    fn compounds_missing_from_chive_get_the_boost_when_both_parts_are_frequent() {
        let p = Policy::parse("frequent_compound\t-8000\n").unwrap();
        let vocab = guarded_vocab();
        let oov = vocab.len();
        let verdict = |row: PolicyRow<'_>| p.evaluate(&row, Some(&vocab));
        // 異常気象（chiVe になし）: 部品 異常・気象 の順位の大きいほうが 1 万位より上、同じ読みに chiVe にある名詞がない
        let hit = verdict(parts_row("異常気象", Some(7_338), Some(oov)));
        assert_eq!(hit.penalty, -8000);
        assert!(hit.compound_by_parts);
        assert!(hit.hits(FREQUENT_COMPOUND));
        let penalty = |row| verdict(row).penalty;
        assert_eq!(
            penalty(parts_row(
                "異常気象",
                Some(COMPOUND_PARTS_MAX_RANK - 1),
                Some(oov)
            )),
            -8000
        );
        // 部品の上限の順位から、切れない語、3 文字の語、同じ読みに chiVe にある名詞がある語は当たらない
        assert_eq!(
            penalty(parts_row(
                "異常気象",
                Some(COMPOUND_PARTS_MAX_RANK),
                Some(oov)
            )),
            0
        );
        assert_eq!(penalty(parts_row("異常気象", None, Some(oov))), 0);
        assert_eq!(penalty(parts_row("異常気", Some(10), Some(oov))), 0);
        assert_eq!(penalty(parts_row("異常気象", Some(10), Some(oov - 1))), 0);
        assert_eq!(penalty(parts_row("異常気象", Some(10), None)), 0);
        // chiVe にある語は (f) の対象にしない（20 万位以下の 圏外語、守りに外れた 総指揮）
        assert_eq!(
            penalty(parts_row(
                "圏外語",
                Some(10),
                Some(COMPOUND_GUARDED_MAX_RANK)
            )),
            0
        );
        assert_eq!(penalty(parts_row("総指揮", Some(10), Some(10))), 0);
        // 熟語自身の順位で当たる語は (f) に数えない
        let by_rank = verdict(parts_row("消火器", Some(10), Some(10)));
        assert_eq!(by_rank.penalty, -8000);
        assert!(!by_rank.compound_by_parts);
        // 仮名を含む語（推し活）は対象にしない（ADR 0035 の F2 は採らなかった）
        assert_eq!(penalty(parts_row("推し活", Some(10), Some(oov))), 0);
    }

    // ---- default_cost_homophone（ADR 0035 規則 H） ----

    fn peer(rank: usize, cost: i16, total: i32) -> HomophonePeer {
        HomophonePeer {
            rank,
            cost,
            total,
            cheap_rare: false,
        }
    }

    #[test]
    fn default_cost_peer_totals_give_the_most_frequent_word_the_cheapest_rarer_total() {
        // 看守（45724 位、一般 7401）と 看取（117971 位、サ変可能 6519）、感取（語彙にない、サ変可能 6518）
        let kanshu = peer(45_724, 7401, 7401 + 1320);
        let mitori = peer(117_971, 6519, 6519 + 4143);
        let kantori = peer(2_000_000, 6518, 6518 + 5200);
        assert_eq!(
            default_cost_peer_totals(&[mitori, kanshu, kantori]),
            vec![None, Some(6519 + 4143), None]
        );
        // 同じ lemma の行（同じ順位）はどれも W
        assert_eq!(
            default_cost_peer_totals(&[kanshu, kanshu, mitori]),
            vec![Some(10_662), Some(10_662), None]
        );
        // 1 行だけ、または空なら何もしない
        assert_eq!(default_cost_peer_totals(&[kanshu]), vec![None]);
        assert!(default_cost_peer_totals(&[]).is_empty());
    }

    #[test]
    fn default_cost_peer_totals_need_default_costs_rarity_and_a_frequent_top() {
        let w = peer(7_340, 7379, 9100);
        let v = peer(11_400, 6516, 8600);
        // 気象／起床（0.19 桁）は当たる
        assert_eq!(default_cost_peer_totals(&[w, v]), vec![Some(8600), None]);
        // 桁の差が足りない V は使わない
        let close = peer(8_000, 6516, 8600);
        assert_eq!(default_cost_peer_totals(&[w, close]), vec![None, None]);
        // 帯の外（Sudachi が既定より高くした 再開 8691、安い語）の W や V は使わない。帯の端は含む
        let w_high = peer(7_340, 8691, 9100);
        assert_eq!(default_cost_peer_totals(&[w_high, v]), vec![None, None]);
        let v_cheap = peer(11_400, 5268, 8600);
        assert_eq!(default_cost_peer_totals(&[w, v_cheap]), vec![None, None]);
        let w_edge = peer(7_340, *DEFAULT_COST_BAND.end(), 9100);
        assert_eq!(
            default_cost_peer_totals(&[w_edge, v]),
            vec![Some(8600), None]
        );
        // 規則 R に当たった V は使わない
        let v_r = HomophonePeer {
            cheap_rare: true,
            ..v
        };
        assert_eq!(default_cost_peer_totals(&[w, v_r]), vec![None, None]);
        // W の順位が上限以下なら何もしない
        let w_rare = peer(DEFAULT_COST_PEER_MAX_RANK, 7379, 9100);
        let v_rarer = peer(DEFAULT_COST_PEER_MAX_RANK * 10, 6516, 8600);
        assert_eq!(
            default_cost_peer_totals(&[w_rare, v_rarer]),
            vec![None, None]
        );
    }

    #[test]
    fn default_cost_homophone_only_lowers_the_total_to_the_peer_minus_the_margin() {
        let p = Policy::parse("default_cost_homophone\t600\n").unwrap();
        assert!(p.uses_chive_vocab());
        let row = |peer_total: Option<i32>| PolicyRow {
            surface: "看守",
            lemma: "看守",
            reading: "かんしゅ",
            pos: NOUN,
            kanji_peer_rank: None,
            noun_peer_rank: None,
            place_peer_rank: None,
            place_peer_total: None,
            compound_parts_rank: None,
            default_cost_peer_total: peer_total,
            notcore: None,
            homophone: HomophoneInfo {
                cost: 7401,
                peer_rank: None,
            },
            kun: KunInfo::default(),
        };
        // 目標（10662 − 600 = 10062）が今の総コスト 7401 より高いときは上げない
        let above = p.evaluate(&row(Some(10_662)), None);
        assert_eq!(above.penalty, 0);
        assert!(!above.hits(DEFAULT_COST_HOMOPHONE));
        // V の総コストが 7000 なら 6400 まで下げる
        let lowered = p.evaluate(&row(Some(7_000)), None);
        assert_eq!(lowered.penalty, 6400 - 7401);
        assert!(lowered.hits(DEFAULT_COST_HOMOPHONE));
        // 基準がなければ何もしない。述語が書かれていなければ何もしない
        assert_eq!(p.evaluate(&row(None), None).penalty, 0);
        let off = Policy::parse("proper_noun\t1000\n").unwrap();
        assert_eq!(off.evaluate(&row(Some(7_000)), None).penalty, 0);
    }
}
