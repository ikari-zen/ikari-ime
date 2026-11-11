//! ラティス構築と前向き Viterbi（crate 内部。ADR 0001 §4.6-2〜4, §11）。
//!
//! ノードは `Vec<Node>` のアリーナに置き、WordId と数値だけを持つ（ラティスの中で String は作らない）。
//! 表記は候補を作るときに `Dictionary::entry` で引く。

use std::sync::Arc;

use crate::candidate::Segment;
use crate::converter::{
    AdverbNounRule, CompoundWordRule, NounSuffixRule, NounSuruRule, NumeralCounterRule,
    NumeralCoverRule, VolitionalRule,
};
use crate::dict::{Dictionary, Grammar, IME_EXCLUDED, WordId};
use ikari_dict_format::reading::fold_fullwidth_digits;

/// ラティスのノード。位置は文字単位（`key` の何文字目か）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Node {
    /// 開始位置（文字単位、含む）
    pub start: u32,
    /// 終了位置（文字単位、含まない）
    pub end: u32,
    /// 未知語ノードなら None
    pub word: Option<WordId>,
    pub left_id: u16,
    pub right_id: u16,
    pub pos_id: u16,
    pub cost: i16,
    /// 住所・番号の助数詞の表記（番・番地 など。`ADDRESS_COUNTER_SURFACES`）の辞書のノードか。数詞 → これの辺を軽くする
    pub address_counter: bool,
    /// 語（読み ご。`LANGUAGE_SUFFIX`）の辞書のノードか。か国 → これの辺を軽くする（`numeral_counter.language_bonus`。#222）
    pub language_suffix: bool,
    /// 「だい」の直後から始まる数（数字の並びの各桁と、直後の辞書のノード）か。数詞 → 序数の助数詞の辺を軽くする
    /// （`numeral_counter.ordinal_bonus`。#240）
    pub ordinal_numeral: bool,
    /// 序数に使う助数詞（`ORDINAL_COUNTER_SURFACES`）の辞書のノードか
    pub ordinal_counter: bool,
    /// BOS からこのノード（自身の単語コストを含む）までの最小コスト
    pub best: i32,
}

/// 品詞の分類（`EdgeRules` の品詞の表の値）。品詞1 だけを見る
const POS_OTHER: u8 = 0;
const POS_ADVERB: u8 = 1;
const POS_NOUN: u8 = 2;
const POS_SUFFIX: u8 = 3;
/// 規則 N が「数」を見分ける読み（ADR 0019 §2.1）
const NUMERAL_COVER_READING: &str = "すう";
/// 品詞2 の数詞（`名詞,数詞,…`）
const POS_DETAIL_NUMERAL: &str = "数詞";
/// 読みの中の数字（未知語になる1文字）に当てる品詞（ADR 0019 §12）
const NUMERAL_POS_NAME: &str = "名詞,数詞,*,*,*,*";
/// 読みの中の漢数字（未知語になる1文字）。全角・半角の算用数字は `is_numeral_char` が別に見る
const KANJI_NUMERALS: &str = "〇一二三四五六七八九十百千万億兆";

/// 読みの中の数字か: 半角・全角の算用数字と漢数字
fn is_numeral_char(c: char) -> bool {
    c.is_ascii_digit() || ('０'..='９').contains(&c) || KANJI_NUMERALS.contains(c)
}

/// 未知語のうち数字の1文字に当てる左右ID・品詞（名詞,数詞 の代表）。品詞表がなければ None（既定の未知語のまま）
fn numeral_unknown(grammar: &dyn Grammar) -> Option<(u16, u16, u16)> {
    let pos_id = grammar.pos_id(NUMERAL_POS_NAME)?;
    let info = grammar.pos_info(pos_id)?;
    Some((info.rep_left_id, info.rep_right_id, pos_id))
}

/// 規則 S（ADR 0016）の品詞の分類。サ変でない名詞（品詞3 が サ変可能・サ変形状詞可能 以外）と、
/// する・す（サ行変格・文語サ行変格の動詞、文語下二段-サ行 の助動詞）
const SURU_OTHER: u8 = 0;
const SURU_NON_SAHEN_NOUN: u8 = 1;
const SURU_VERB: u8 = 2;

/// 規則 V（ADR 0016 §10。B63）の品詞の分類
const VOL_OTHER: u8 = 0;
/// 前の語になれる品詞1: 助詞・副詞・接続詞
const VOL_PREV: u8 = 1;
/// サ行変格の動詞の意志推量形
const VOL_VERB: u8 = 2;
/// サ変可能の普通名詞（ADR 0016 §12。#230）。前の語にも、しよう の後ろの語にもなる
const VOL_SAHEN: u8 = 4;
/// 後ろの語になれる品詞1: 動詞・名詞・接尾辞・接頭辞・助動詞（助動詞-ダ の じゃ を除く）（意志形の しよう の直後には来ない。ADR 0016 §12）
const VOL_NEXT: u8 = 3;
/// 規則 V が見分ける読み。`しょう` などは対象にしない（こうしょう が 副詞＋しょう になるのを防ぐ）
const VOLITIONAL_READING: &str = "しよう";

/// 品詞名の規則 V の分類
fn volitional_class_of(name: &str) -> u8 {
    let f: Vec<&str> = name.split(',').collect();
    match f.first().copied() {
        Some("動詞") if f.get(4) == Some(&"サ行変格") && f.get(5) == Some(&"意志推量形") => {
            VOL_VERB
        }
        Some("名詞") if f.get(1) == Some(&"普通名詞") && f.get(2) == Some(&"サ変可能") => {
            VOL_SAHEN
        }
        Some("助詞" | "副詞" | "接続詞") => VOL_PREV,
        Some("動詞" | "名詞" | "接尾辞" | "接頭辞") => VOL_NEXT,
        Some("助動詞") if f.get(4) != Some(&"助動詞-ダ") => VOL_NEXT,
        _ => VOL_OTHER,
    }
}

/// 品詞名（"品詞1,品詞2,品詞3,品詞4,活用型,活用形"）の規則 S の分類
fn suru_class_of(name: &str) -> u8 {
    let mut fields = name.split(',');
    let pos1 = fields.next();
    let _pos2 = fields.next();
    let pos3 = fields.next();
    let _pos4 = fields.next();
    let conj_type = fields.next();
    match pos1 {
        Some("名詞") if !matches!(pos3, Some("サ変可能" | "サ変形状詞可能")) => {
            SURU_NON_SAHEN_NOUN
        }
        Some("動詞") if matches!(conj_type, Some("サ行変格" | "文語サ行変格")) => {
            SURU_VERB
        }
        Some("助動詞") if conj_type == Some("文語下二段-サ行") => SURU_VERB,
        _ => SURU_OTHER,
    }
}

/// 候補の分割に、規則 S（ADR 0016）が加点した辺（サ変でない名詞 → する・す）があるか。
/// 辞書の語どうしの隣り合う 2 語だけを見る（ラティスの `noun_suru_extra` と同じ条件。ADR 0024）。
pub(crate) fn has_noun_suru_edge(grammar: &dyn Grammar, segments: &[Segment]) -> bool {
    let class = |s: &Segment| {
        if s.word.is_none() {
            return SURU_OTHER;
        }
        grammar.pos_name(s.pos_id).map_or(SURU_OTHER, suru_class_of)
    };
    segments
        .windows(2)
        .any(|w| class(&w[0]) == SURU_NON_SAHEN_NOUN && class(&w[1]) == SURU_VERB)
}

/// 変換のラティスの辺の補正（ADR 0008）。`Converter` の組み立てで1度だけ作り、ラティスごとに複製する
/// （品詞の表は `Arc` で共有するので、複製は安い）。
#[derive(Clone, Debug)]
pub(crate) struct EdgeRules {
    adverb_noun: AdverbNounRule,
    /// pos_id -> `POS_*`。範囲外の pos_id は `POS_OTHER`
    pos_class: Arc<[u8]>,
    noun_suru: NounSuruRule,
    noun_suffix: NounSuffixRule,
    numeral_cover: NumeralCoverRule,
    /// pos_id -> 品詞が 名詞,数詞 か（規則 N が止まっていれば空）
    numeral_class: Arc<[bool]>,
    /// pos_id -> `SURU_*`（規則 S が止まっていれば空）
    suru_class: Arc<[u8]>,
    volitional: VolitionalRule,
    /// pos_id -> `VOL_*`（規則 V が止まっていれば空）
    volitional_class: Arc<[u8]>,
    numeral_counter: NumeralCounterRule,
    /// pos_id -> `COUNTER_*`（規則 C2 が止まっていれば空）
    counter_class: Arc<[u8]>,
    /// 仮名まじりの 1 語の複合名詞の補正（ADR 0035 §12。#2）
    compound_word: CompoundWordRule,
}

/// 仮名まじりの複合名詞（`CompoundWordRule`）の表記・読みの文字数の下限
const COMPOUND_WORD_MIN_CHARS: usize = 3;

fn is_hiragana(c: char) -> bool {
    ('\u{3041}'..='\u{309F}').contains(&c)
}

fn is_kanji(c: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{3400}'..='\u{4DBF}').contains(&c)
}

/// 表記の先頭と末尾が漢字で、途中に平仮名があり（見た目、結び目、立ち位置）、表記と読みがどちらも
/// `COMPOUND_WORD_MIN_CHARS` 文字以上か。末尾が仮名の表記（想ぞう、家てい。常用外の字を開いた表記ゆれ）は除く
fn is_kana_mixed_compound(surface: &str, reading_chars: usize) -> bool {
    let (Some(first), Some(last)) = (surface.chars().next(), surface.chars().last()) else {
        return false;
    };
    reading_chars >= COMPOUND_WORD_MIN_CHARS
        && surface.chars().count() >= COMPOUND_WORD_MIN_CHARS
        && is_kanji(first)
        && is_kanji(last)
        && surface.chars().any(is_hiragana)
}

/// 数詞 → 助数詞可能の名詞（#219）の品詞の分類
const COUNTER_OTHER: u8 = 0;
/// 名詞,数詞
const COUNTER_NUMERAL: u8 = 1;
/// 名詞,普通名詞,助数詞可能
const COUNTER_NOUN: u8 = 2;
/// 接尾辞,名詞的,助数詞（個・年生・刷・日 など）
const COUNTER_SUFFIX: u8 = 3;
/// 数詞のあとで `suffix_bonus` を引く 接尾辞,助数詞 の読み（個・年生。同音の戸・湖・年性に負ける）
const FAVORED_COUNTER_SUFFIX_READINGS: [&str; 2] = ["こ", "ねんせい"];
/// 数詞のあとで `ka_penalty` を足す 接尾辞,助数詞 の読み（日・カ。か月・か国・か所の「か」を 3日 に取られる）
const DAY_KA_READING: &str = "か";
/// 数詞のあとで `address_bonus` を引く（読み, 表記）。住所・番号の助数詞のうち、同音の別の助数詞に負けるもの。
/// 番（ばん）は 版・板 に、番地（ばんち）は 版地 に負ける（ADR 0030 §13）。表記は辞書の語を作るときに見て、`Node::address_counter` に持つ
const ADDRESS_COUNTER_SURFACES: [(&str, &str); 2] = [("ばん", "番"), ("ばんち", "番地")];

/// 「第」+ 数詞のあとで `ordinal_bonus` を引く（読み, 表記）。順番を表す助数詞。階（個数にも使う）、年生（第3年生と言わない）は入れない。
/// 番・版 は番号の助数詞の規則（`ADDRESS_COUNTER_SURFACES`）と重なるが、第 + 数詞 のあとでは序数として扱う（2026-10-23 ユーザー決定）
const ORDINAL_COUNTER_SURFACES: [(&str, &str); 15] = [
    ("かん", "巻"),
    ("かい", "回"),
    ("しょう", "章"),
    ("せつ", "節"),
    ("わ", "話"),
    ("じょう", "条"),
    ("こう", "項"),
    ("ごう", "号"),
    ("い", "位"),
    ("ばん", "番"),
    ("き", "期"),
    ("じ", "次"),
    ("ぶ", "部"),
    ("か", "課"),
    ("はん", "版"),
];
/// 「第」の読み。この直後の数を序数とみなす
const ORDINAL_PREFIX_READING: &str = "だい";

/// か国（読み かこく。表記は か国・箇国・カ国 など）のあとの 語（読み, 表記）。3かこくご が 3か国後 になるのを防ぐ（#222）
const COUNTRY_COUNTER_READING: &str = "かこく";
const LANGUAGE_SUFFIX: (&str, &str) = ("ご", "語");

fn counter_class_of(name: &str) -> u8 {
    let mut f = name.split(',');
    match (f.next(), f.next(), f.next()) {
        (Some("名詞"), Some(POS_DETAIL_NUMERAL), _) => COUNTER_NUMERAL,
        (Some("名詞"), Some("普通名詞"), Some("助数詞可能")) => COUNTER_NOUN,
        (Some("接尾辞"), Some("名詞的"), Some("助数詞")) => COUNTER_SUFFIX,
        _ => COUNTER_OTHER,
    }
}

/// pos_id -> `POS_*`（品詞1 だけを見る）
fn pos_class_table(grammar: &dyn Grammar) -> Arc<[u8]> {
    (0..=u16::MAX)
        .map_while(|id| grammar.pos_name(id))
        .map(|name| match name.split(',').next() {
            Some("副詞") => POS_ADVERB,
            Some("名詞") => POS_NOUN,
            Some("接尾辞") => POS_SUFFIX,
            _ => POS_OTHER,
        })
        .collect()
}

impl EdgeRules {
    /// `grammar.pos_name` から品詞の表を作る（pos_id は 0 から連続し、範囲外は None を返す）。
    /// 規則が止まっている（`penalty == 0`）なら表を作らない。
    #[cfg(test)]
    pub fn new(
        grammar: &dyn Grammar,
        adverb_noun: AdverbNounRule,
        noun_suru: NounSuruRule,
        noun_suffix: NounSuffixRule,
        numeral_cover: NumeralCoverRule,
    ) -> Self {
        Self::new_with_volitional(
            grammar,
            adverb_noun,
            noun_suru,
            noun_suffix,
            numeral_cover,
            VolitionalRule::disabled(),
            NumeralCounterRule::disabled(),
        )
    }

    /// `new` に規則 V（助詞＋しよう）を足したもの
    pub fn new_with_volitional(
        grammar: &dyn Grammar,
        adverb_noun: AdverbNounRule,
        noun_suru: NounSuruRule,
        noun_suffix: NounSuffixRule,
        numeral_cover: NumeralCoverRule,
        volitional: VolitionalRule,
        numeral_counter: NumeralCounterRule,
    ) -> Self {
        let pos_class: Arc<[u8]> =
            if adverb_noun.is_enabled() || noun_suffix.is_enabled() || numeral_cover.is_enabled() {
                pos_class_table(grammar)
            } else {
                Arc::from([])
            };
        let numeral_class: Arc<[bool]> = if numeral_cover.is_enabled() {
            (0..=u16::MAX)
                .map_while(|id| grammar.pos_name(id))
                .map(|name| {
                    let mut parts = name.split(',');
                    parts.next() == Some("名詞") && parts.next() == Some(POS_DETAIL_NUMERAL)
                })
                .collect()
        } else {
            Arc::from([])
        };
        let suru_class: Arc<[u8]> = if noun_suru.is_enabled() {
            (0..=u16::MAX)
                .map_while(|id| grammar.pos_name(id))
                .map(suru_class_of)
                .collect()
        } else {
            Arc::from([])
        };
        let volitional_class: Arc<[u8]> = if volitional.is_enabled() {
            (0..=u16::MAX)
                .map_while(|id| grammar.pos_name(id))
                .map(volitional_class_of)
                .collect()
        } else {
            Arc::from([])
        };
        let counter_class: Arc<[u8]> = if numeral_counter.is_enabled() {
            (0..=u16::MAX)
                .map_while(|id| grammar.pos_name(id))
                .map(counter_class_of)
                .collect()
        } else {
            Arc::from([])
        };
        Self {
            numeral_counter,
            counter_class,
            volitional,
            volitional_class,
            adverb_noun,
            pos_class,
            noun_suru,
            noun_suffix,
            numeral_cover,
            numeral_class,
            suru_class,
            compound_word: CompoundWordRule::disabled(),
        }
    }

    /// 仮名まじりの 1 語の複合名詞の補正を足す（既定は止まっている）。名詞を見分ける品詞の表がなければ作る
    pub fn with_compound_word(mut self, grammar: &dyn Grammar, rule: CompoundWordRule) -> Self {
        if rule.is_enabled() && self.pos_class.is_empty() {
            self.pos_class = pos_class_table(grammar);
        }
        self.compound_word = rule;
        self
    }

    /// 辞書の語のコストから引く値。名詞で、表記が仮名まじりの複合語（`is_kana_mixed_compound`）のときだけ 0 でない
    #[inline]
    fn compound_word_bonus(&self, pos_id: u16, surface: &str, reading_chars: usize) -> i32 {
        if !self.compound_word.is_enabled()
            || self.pos_class.get(usize::from(pos_id)).copied() != Some(POS_NOUN)
            || !is_kana_mixed_compound(surface, reading_chars)
        {
            return 0;
        }
        self.compound_word.bonus
    }

    /// 補正なし（今までの変換と同じ）
    #[cfg(test)]
    pub fn none() -> Self {
        Self {
            adverb_noun: AdverbNounRule::disabled(),
            pos_class: Arc::from([]),
            noun_suru: NounSuruRule::disabled(),
            noun_suffix: NounSuffixRule::disabled(),
            numeral_cover: NumeralCoverRule::disabled(),
            numeral_class: Arc::from([]),
            suru_class: Arc::from([]),
            volitional: VolitionalRule::disabled(),
            volitional_class: Arc::from([]),
            numeral_counter: NumeralCounterRule::disabled(),
            counter_class: Arc::from([]),
            compound_word: CompoundWordRule::disabled(),
        }
    }

    /// 辞書のノードの品詞の分類。未知語は `POS_OTHER`
    fn class(&self, node: &Node) -> u8 {
        if node.word.is_none() {
            return POS_OTHER;
        }
        self.pos_class
            .get(usize::from(node.pos_id))
            .copied()
            .unwrap_or(POS_OTHER)
    }

    /// 名詞,数詞 の辞書のノードか。未知語は false
    fn is_numeral(&self, node: &Node) -> bool {
        node.word.is_some()
            && self
                .numeral_class
                .get(usize::from(node.pos_id))
                .copied()
                .unwrap_or(false)
    }

    /// 規則 S の分類。未知語は `SURU_OTHER`
    fn suru_class(&self, node: &Node) -> u8 {
        if node.word.is_none() {
            return SURU_OTHER;
        }
        self.suru_class
            .get(usize::from(node.pos_id))
            .copied()
            .unwrap_or(SURU_OTHER)
    }

    /// 「覆う名詞」の候補になれるノードか（品詞1 = 名詞の辞書のノードで、変換用のコストが、
    /// 有効な規則の上限のどれかの以下）。規則ごとの上限は辺を見るときに比べる（`Lattice::covered`）
    fn is_cover(&self, node: &Node) -> bool {
        self.class(node) == POS_NOUN && i32::from(node.cost) <= self.cover_cap()
    }

    /// 有効な規則の「覆う名詞」のコストの上限のうち、大きい方。有効な規則がなければ負（誰も覆わない）
    fn cover_cap(&self) -> i32 {
        let adverb = self
            .adverb_noun
            .is_enabled()
            .then_some(self.adverb_noun.cover_max_cost);
        let suffix = self
            .noun_suffix
            .is_enabled()
            .then_some(self.noun_suffix.cover_max_cost);
        let numeral = self
            .numeral_cover
            .is_enabled()
            .then_some(self.numeral_cover.cover_max_cost);
        adverb
            .into_iter()
            .chain(suffix)
            .chain(numeral)
            .max()
            .unwrap_or(-1)
    }

    /// 覆う名詞の表が要るか（どれかの規則が有効）
    fn needs_cover(&self) -> bool {
        self.adverb_noun.is_enabled()
            || self.noun_suffix.is_enabled()
            || self.numeral_cover.is_enabled()
    }
}

pub(crate) struct Lattice {
    key: String,
    /// 文字位置 -> バイト位置。長さは文字数 + 1
    char_bytes: Vec<u32>,
    nodes: Vec<Node>,
    /// 終了位置 -> その位置で終わるノードの添字
    ends_at: Vec<Vec<u32>>,
    rules: EdgeRules,
    /// 開始位置 -> その位置から始まる「覆う名詞」の（終了位置、変換用のコスト）（ADR 0008 §3.1、§13）。
    /// 規則がどちらも止まっていれば空
    cover_ends: Vec<Vec<(u32, i32)>>,
}

impl Lattice {
    /// ノードの間の辺 `prev -> next` に、連接コストのほかに足す値（ADR 0008 §2.1、§3.1）。
    /// 前向き Viterbi（`build_inner`）と後ろ向き A*（`NBest::next`）が、どちらもこの関数を使う。
    /// BOS からの辺と EOS への辺には使わない。
    ///
    /// 副詞の直後の名詞: `prev` が副詞、`next` が名詞（どちらも辞書のノード）で、
    /// `prev` の開始位置から `next` の終了位置までをちょうど覆う名詞（コストが上限以下）があれば加算する。
    ///
    /// サ変でない名詞＋する（ADR 0016 規則 S）: `prev` がサ変でない名詞、`next` がサ行変格・文語サ行変格の動詞か
    /// 文語下二段-サ行 の助動詞（どちらも辞書のノード）なら `noun_suru.penalty` を加算する。
    ///
    /// 名詞＋接尾辞（ADR 0008 §13。B25）: `prev` が名詞、`next` が接尾辞（どちらも辞書のノード）で、
    /// `prev` の開始位置から `next` の終了位置までをちょうど覆う名詞（コストが `noun_suffix.cover_max_cost` 以下）があれば
    /// `noun_suffix.penalty` を加算する。
    ///
    /// 前の語 → 数詞「数」（ADR 0019 規則 N。B33）: `prev` が数詞でない辞書のノード、`next` が 名詞,数詞 で読みが すう の
    /// 辞書のノードで、`prev` の開始位置から `next` の終了位置までをちょうど覆う名詞（コストが `numeral_cover.cover_max_cost` 以下）が
    /// あれば `numeral_cover.penalty` を加算する。
    /// 副詞→名詞、名詞→する、名詞→接尾辞 の 3 つの規則は `prev` と `next` の品詞の組が違うので重ならない。
    /// 規則 N の `next` は名詞（数詞）なので、`prev` が副詞なら副詞→名詞 と同じ辺に重なりうる（両方の条件を満たすときは両方を足す）。
    #[inline]
    pub fn edge_extra(&self, prev: &Node, next: &Node) -> i32 {
        self.adverb_noun_extra(prev, next)
            + self.noun_suru_extra(prev, next)
            + self.noun_suffix_extra(prev, next)
            + self.numeral_cover_extra(prev, next)
            + self.volitional_extra(prev, next)
            + self.numeral_counter_extra(prev, next)
    }

    /// 数詞 → 助数詞可能の名詞（#219）: `prev` が読みの中の数字（算用数字・漢数字の未知語のノード。名詞,数詞）、`next` が辞書の
    /// `名詞,普通名詞,助数詞可能` なら `-bonus`。辞書の数詞（いち・く などの読みから作る 一・九）には当てない
    /// （もういちど → もう一℃ のように、かな読みを数詞＋助数詞に割るのを防ぐ。実測は ADR 0019 §15）。負の値を返す辺の補正（規則 V と同じく、前向きと後ろ向きが同じ関数を使うので
    /// A* のヒューリスティックは正確なまま）。
    /// 「第」+ 数詞 → 序数の助数詞（#240）。負の値
    #[inline]
    fn ordinal_extra(&self, prev: &Node, next: &Node) -> i32 {
        if prev.ordinal_numeral && next.ordinal_counter {
            -self.rules.numeral_counter.ordinal_bonus
        } else {
            0
        }
    }

    #[inline]
    fn numeral_counter_extra(&self, prev: &Node, next: &Node) -> i32 {
        let rule = &self.rules.numeral_counter;
        if !rule.is_enabled() || next.word.is_none() {
            return 0;
        }
        let class = |n: &Node| {
            self.rules
                .counter_class
                .get(usize::from(n.pos_id))
                .copied()
                .unwrap_or(COUNTER_OTHER)
        };
        if prev.word.is_some() {
            if next.language_suffix
                && class(prev) == COUNTER_NOUN
                && self.slice(prev) == COUNTRY_COUNTER_READING
            {
                return -rule.language_bonus;
            }
            // かな読みから作った辞書の数詞（さん → 三）は、住所・番号の助数詞の表記と、第 + 数詞 のあとの序数にだけ当てる
            if class(prev) != COUNTER_NUMERAL {
                return 0;
            }
            let address = if next.address_counter {
                -rule.address_kana_bonus
            } else {
                0
            };
            return address + self.ordinal_extra(prev, next);
        }
        if class(prev) != COUNTER_NUMERAL {
            return 0;
        }
        let address = if next.address_counter {
            -rule.address_bonus
        } else {
            0
        };
        address
            + self.ordinal_extra(prev, next)
            + match class(next) {
                COUNTER_NOUN => -rule.bonus,
                COUNTER_SUFFIX => {
                    let reading = self.slice(next);
                    if FAVORED_COUNTER_SUFFIX_READINGS.contains(&reading) {
                        -rule.suffix_bonus
                    } else if reading == DAY_KA_READING {
                        rule.ka_penalty
                    } else {
                        0
                    }
                }
                _ => 0,
            }
    }

    /// 〜としよう／〜にしよう（ADR 0016 §10 規則 V。B63）: `prev` が助詞・副詞・接続詞、`next` が読み しよう のサ行変格の
    /// 意志推量形なら `-bonus`。`prev` がその しよう、`next` が動詞・名詞・接尾辞なら `+bonus`（でしようする などを防ぐ）。
    /// 負の値を返す辺の補正（数詞 → 助数詞も同じ）。前向きと後ろ向きが同じ関数を使うので、A* のヒューリスティックは正確なまま。
    #[inline]
    fn volitional_extra(&self, prev: &Node, next: &Node) -> i32 {
        let rule = &self.rules.volitional;
        if !rule.is_enabled() || prev.word.is_none() || next.word.is_none() {
            return 0;
        }
        let class = |n: &Node| {
            self.rules
                .volitional_class
                .get(usize::from(n.pos_id))
                .copied()
                .unwrap_or(VOL_OTHER)
        };
        match (class(prev), class(next)) {
            (VOL_PREV, VOL_VERB) if self.slice(next) == VOLITIONAL_READING => -rule.bonus,
            (VOL_SAHEN, VOL_VERB) if self.slice(next) == VOLITIONAL_READING => -rule.sahen_bonus,
            (VOL_VERB, VOL_NEXT | VOL_SAHEN) if self.slice(prev) == VOLITIONAL_READING => {
                rule.bonus
            }
            _ => 0,
        }
    }

    /// `start` から `end` までをちょうど覆う、コストが `cap` 以下の名詞があるか
    #[inline]
    fn covered(&self, start: u32, end: u32, cap: i32) -> bool {
        self.cover_ends
            .get(start as usize)
            .is_some_and(|ends| ends.iter().any(|&(e, cost)| e == end && cost <= cap))
    }

    #[inline]
    fn adverb_noun_extra(&self, prev: &Node, next: &Node) -> i32 {
        let rule = &self.rules.adverb_noun;
        if !rule.is_enabled()
            || self.rules.class(prev) != POS_ADVERB
            || self.rules.class(next) != POS_NOUN
        {
            return 0;
        }
        if self.covered(prev.start, next.end, rule.cover_max_cost) {
            rule.penalty
        } else {
            0
        }
    }

    #[inline]
    fn noun_suffix_extra(&self, prev: &Node, next: &Node) -> i32 {
        let rule = &self.rules.noun_suffix;
        if !rule.is_enabled()
            || self.rules.class(next) != POS_SUFFIX
            || self.rules.class(prev) != POS_NOUN
        {
            return 0;
        }
        if self.covered(prev.start, next.end, rule.cover_max_cost) {
            rule.penalty
        } else {
            0
        }
    }

    #[inline]
    fn numeral_cover_extra(&self, prev: &Node, next: &Node) -> i32 {
        let rule = &self.rules.numeral_cover;
        // 数詞の側（まれ）を先に見て、ほとんどの辺は表を 1 回引くだけで抜ける
        if !rule.is_enabled()
            || !self.rules.is_numeral(next)
            || prev.word.is_none()
            || self.rules.is_numeral(prev)
            || self.slice(next) != NUMERAL_COVER_READING
        {
            return 0;
        }
        if self.covered(prev.start, next.end, rule.cover_max_cost) {
            rule.penalty
        } else {
            0
        }
    }

    #[inline]
    fn noun_suru_extra(&self, prev: &Node, next: &Node) -> i32 {
        let rule = &self.rules.noun_suru;
        if rule.is_enabled()
            // する・す の側（まれ）を先に見て、ほとんどの辺は表を 1 回引くだけで抜ける
            && self.rules.suru_class(next) == SURU_VERB
            && self.rules.suru_class(prev) == SURU_NON_SAHEN_NOUN
        {
            rule.penalty
        } else {
            0
        }
    }

    /// このラティスの辺の補正（同じ設定で別の読みのラティスを作るため）
    pub fn rules(&self) -> &EdgeRules {
        &self.rules
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    /// 文字数
    pub fn len_chars(&self) -> usize {
        self.char_bytes.len() - 1
    }

    pub fn node(&self, id: u32) -> &Node {
        &self.nodes[id as usize]
    }

    /// 位置 `pos`（文字単位）で終わるノード
    pub fn ending_at(&self, pos: u32) -> &[u32] {
        self.ends_at.get(pos as usize).map_or(&[], Vec::as_slice)
    }

    /// ノードが覆う読み（key の部分文字列）
    pub fn slice(&self, node: &Node) -> &str {
        let s = self.char_bytes[node.start as usize] as usize;
        let e = self.char_bytes[node.end as usize] as usize;
        &self.key[s..e]
    }

    #[cfg(test)]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

/// `key`（`normalize_key` 済み）のラティスを作り、前向き Viterbi で各ノードの `best` を求める。
///
/// どの位置にも1文字の未知語ノードを必ず1つ追加するので、EOS まで必ずつながる。
/// `bos_right_id` は BOS の右ID（フェーズ0は `grammar.bos_eos_id()`、フェーズ1は左文脈の末尾の語の右ID）。
///
/// 変換用のラティスなので、ノードのコストは `cost + penalty`（i16 に飽和）にし、
/// `penalty == IME_EXCLUDED` のエントリはノードにしない（ADR 0002 §4.1, §6.2）。
#[cfg(test)]
pub(crate) fn build(
    key: &str,
    dict: &dyn Dictionary,
    grammar: &dyn Grammar,
    bos_right_id: u16,
) -> Lattice {
    build_with_starts(key, dict, grammar, &[(bos_right_id, 0)], &EdgeRules::none())
}

/// `build` の BOS を複数の右ID（と追加コスト）にしたもの。最初の語への連接は、候補のうち最小を使う。
/// 空なら `grammar.bos_eos_id()` だけを使う。`rules` はノードの間の辺の補正（`Lattice::edge_extra`）。
pub(crate) fn build_with_starts(
    key: &str,
    dict: &dyn Dictionary,
    grammar: &dyn Grammar,
    starts: &[(u16, i32)],
    rules: &EdgeRules,
) -> Lattice {
    build_inner(key, dict, grammar, starts, &[], rules)
}

/// `build_with_starts` に、読みの文字位置 `boundaries` をまたぐ辞書のノードを作らない制約を足したもの
/// （区切り直し。ADR 0006 §4.2-2）。1文字の未知語は境界をまたがないので、EOS まで必ずつながる。
/// `boundaries` は読み（`key`）の先頭からの文字位置。範囲外の値（0、文字数以上）は境界にならない。
/// 境界をまたぐ名詞はノードにならないので、境界をまたぐ「覆う名詞」もない（ADR 0008 §4.3）。
pub(crate) fn build_constrained(
    key: &str,
    dict: &dyn Dictionary,
    grammar: &dyn Grammar,
    starts: &[(u16, i32)],
    boundaries: &[u32],
    rules: &EdgeRules,
) -> Lattice {
    build_inner(key, dict, grammar, starts, boundaries, rules)
}

fn build_inner(
    key: &str,
    dict: &dyn Dictionary,
    grammar: &dyn Grammar,
    starts: &[(u16, i32)],
    boundaries: &[u32],
    rules: &EdgeRules,
) -> Lattice {
    let default_start = [(grammar.bos_eos_id(), 0)];
    let starts = if starts.is_empty() {
        &default_start[..]
    } else {
        starts
    };
    let char_bytes: Vec<u32> = key
        .char_indices()
        .map(|(i, _)| i as u32)
        .chain(std::iter::once(key.len() as u32))
        .collect();
    let n = char_bytes.len() - 1;
    let unk = grammar.unknown();
    let numeral_unk = if key.chars().any(is_numeral_char) {
        numeral_unknown(grammar)
    } else {
        None
    };

    let counter_address =
        rules.numeral_counter.address_bonus != 0 || rules.numeral_counter.address_kana_bonus != 0;
    let counter_language = rules.numeral_counter.language_bonus != 0;
    let counter_ordinal = rules.numeral_counter.ordinal_bonus != 0;
    // 位置 i から始まる数が、「だい」の直後の数か（数字の並びは各桁が同じ）
    let ordinal_at: Vec<bool> = if counter_ordinal {
        let chars: Vec<char> = key.chars().collect();
        let prefix: Vec<char> = ORDINAL_PREFIX_READING.chars().collect();
        let mut v = vec![false; n];
        for i in 0..n {
            v[i] = chars[..i].ends_with(&prefix)
                || (i > 0
                    && v[i - 1]
                    && is_numeral_char(chars[i])
                    && is_numeral_char(chars[i - 1]));
        }
        v
    } else {
        Vec::new()
    };
    let ordinal_here = |i: usize| ordinal_at.get(i).copied().unwrap_or(false);
    let mut lat = Lattice {
        key: key.to_owned(),
        char_bytes,
        nodes: Vec::with_capacity(n * 8),
        ends_at: vec![Vec::new(); n + 1],
        rules: rules.clone(),
        cover_ends: if rules.needs_cover() {
            vec![Vec::new(); n + 1]
        } else {
            Vec::new()
        },
    };

    // 辞書は半角の数字で索引を作ってある（数字を含む町名の連なり。ADR 0030 §3.1）ので、全角の数字は引くときだけ半角にする。
    // 1 文字を 1 文字に替えるので、ノードの位置は `key` のまま。全角の数字がなければ `key` をそのまま引く
    let folded = fold_fullwidth_digits(key);
    let folded_bytes: Option<Vec<u32>> = match &folded {
        std::borrow::Cow::Borrowed(_) => None,
        std::borrow::Cow::Owned(text) => Some(text.char_indices().map(|(i, _)| i as u32).collect()),
    };
    for i in 0..n {
        let first = lat.nodes.len();
        let tail = match &folded_bytes {
            Some(bytes) => &folded[bytes[i] as usize..],
            None => &key[lat.char_bytes[i] as usize..],
        };
        // i より後ろで最も近い境界。この位置を越える辞書のノードは作らない
        let next_boundary = boundaries
            .iter()
            .copied()
            .filter(|&b| b as usize > i)
            .min()
            .map_or(usize::MAX, |b| b as usize);
        let nodes = &mut lat.nodes;
        dict.common_prefix_search(tail, &mut |len, e| {
            // 辞書の契約違反（境界でないバイト長）は無視する
            let Some(matched) = tail.get(..len) else {
                return;
            };
            let chars = matched.chars().count();
            if chars == 0 || e.penalty == IME_EXCLUDED || i + chars > next_boundary {
                return;
            }
            nodes.push(Node {
                start: i as u32,
                end: (i + chars) as u32,
                word: Some(e.id),
                left_id: e.left_id,
                right_id: e.right_id,
                pos_id: e.pos_id,
                cost: e.cost.saturating_add(e.penalty).saturating_sub(
                    i16::try_from(rules.compound_word_bonus(e.pos_id, e.surface, chars))
                        .unwrap_or(i16::MAX),
                ),
                address_counter: counter_address
                    && ADDRESS_COUNTER_SURFACES
                        .iter()
                        .any(|&(reading, surface)| reading == matched && surface == e.surface),
                language_suffix: counter_language
                    && matched == LANGUAGE_SUFFIX.0
                    && e.surface == LANGUAGE_SUFFIX.1,
                ordinal_numeral: ordinal_here(i),
                ordinal_counter: counter_ordinal
                    && ORDINAL_COUNTER_SURFACES
                        .iter()
                        .any(|&(reading, surface)| reading == matched && surface == e.surface),
                best: 0,
            });
        });
        // 数字の未知語は、名詞,数詞 の連接を受ける（助数詞が続けられる。ADR 0019 §12）。コストは未知語のまま
        let (left_id, right_id, pos_id) = match (numeral_unk, tail.chars().next()) {
            (Some(numeral), Some(c)) if is_numeral_char(c) => numeral,
            _ => (unk.left_id, unk.right_id, unk.pos_id),
        };
        nodes.push(Node {
            start: i as u32,
            end: (i + 1) as u32,
            word: None,
            left_id,
            right_id,
            pos_id,
            cost: unk.cost_per_char,
            address_counter: false,
            language_suffix: false,
            ordinal_numeral: ordinal_here(i),
            ordinal_counter: false,
            best: 0,
        });

        // 位置 i から始まる覆う名詞。この位置から始まる副詞の後ろの辺は、すべて i より後ろで計算する
        if !lat.cover_ends.is_empty() {
            for id in first..lat.nodes.len() {
                let node = &lat.nodes[id];
                if lat.rules.is_cover(node) {
                    let entry = (node.end, i32::from(node.cost));
                    lat.cover_ends[i].push(entry);
                }
            }
        }

        // このノード群の前側（終了位置 == i）は、開始位置が i より小さいので確定済み
        for id in first..lat.nodes.len() {
            let node = &lat.nodes[id];
            let left = node.left_id;
            let word_cost = i32::from(node.cost);
            let from_prev = if i == 0 {
                starts
                    .iter()
                    .map(|&(right, extra)| {
                        extra.saturating_add(i32::from(grammar.connection_cost(right, left)))
                    })
                    .min()
                    .unwrap_or(i32::MAX / 2)
            } else {
                lat.ends_at[i]
                    .iter()
                    .map(|&p| {
                        let prev = &lat.nodes[p as usize];
                        prev.best
                            + i32::from(grammar.connection_cost(prev.right_id, left))
                            + lat.edge_extra(prev, node)
                    })
                    .min()
                    .unwrap_or(i32::MAX / 2)
            };
            let end = node.end as usize;
            lat.nodes[id].best = from_prev + word_cost;
            lat.ends_at[end].push(id as u32);
        }
    }

    lat
}

#[cfg(test)]
pub(crate) mod testutil {
    //! lattice / nbest / converter のテストが共有する小さな辞書と文法。
    use std::collections::HashMap;

    use crate::dict::{
        DenseGrammar, Dictionary, InMemoryDictionary, Lemma, UnknownParams, WordEntry, WordId,
    };

    pub const ID_NOUN: u16 = 1;
    pub const ID_VERB_SURU: u16 = 2;
    pub const ID_PARTICLE: u16 = 3;
    pub const ID_UNK: u16 = 4;
    pub const UNK_COST: i16 = 5000;

    /// 5x5 の文法。連接コストは 0 を基本にして、必要なテストが `set` で上書きする。
    pub fn grammar() -> DenseGrammar {
        let mut g = DenseGrammar::new(
            5,
            5,
            0,
            UnknownParams {
                left_id: ID_UNK,
                right_id: ID_UNK,
                pos_id: 0,
                cost_per_char: UNK_COST,
            },
        );
        g.add_pos("名詞,普通名詞,一般,*,*,*");
        g.add_pos("動詞,非自立可能,*,*,サ行変格,終止形-一般");
        g.add_pos("助詞,格助詞,*,*,*,*");
        g
    }

    /// 副詞の直後の名詞の規則（ADR 0008）のテストの品詞。`grammar()` の品詞（名詞 0、動詞 1、助詞 2）に足す
    pub const ADVERB_POS: &str = "副詞,*,*,*,*,*";
    pub const POS_ID_NOUN: u16 = 0;
    pub const POS_ID_PARTICLE: u16 = 2;

    /// `grammar()` に副詞の品詞を足し、その pos_id を返す
    pub fn grammar_with_adverb() -> (DenseGrammar, u16) {
        let mut g = grammar();
        let adverb = g.add_pos(ADVERB_POS);
        (g, adverb)
    }

    /// そう（副詞 1000）、じき（時期、名詞 1000）、そうじき（掃除機、`cover_pos` で `cover_cost`）。
    /// 連接コストはすべて 0。そう＋時期 = 2000、掃除機 = `cover_cost`。
    pub fn adverb_dict(g: &DenseGrammar, adverb: u16, cover_cost: i16, cover_pos: u16) -> TestDict {
        use crate::dict::Grammar;
        let mut d = TestDict::new(g.grammar_id());
        d.add("そう", "そう", ID_NOUN, ID_NOUN, 1000, adverb);
        d.add("じき", "時期", ID_NOUN, ID_NOUN, 1000, POS_ID_NOUN);
        d.add(
            "そうじき",
            "掃除機",
            ID_NOUN,
            ID_NOUN,
            cover_cost,
            cover_pos,
        );
        d
    }

    pub fn dict_for(g: &DenseGrammar) -> InMemoryDictionary {
        use crate::dict::Grammar;
        InMemoryDictionary::new(g.grammar_id())
    }

    /// 読みと表記の両方で引ける、テスト専用の小さな辞書（penalty と lemma も持てる）。
    /// 表記の検索ができる最小の辞書として、ここで持つ（`InMemoryDictionary` に依らない）。
    pub struct TestDict {
        grammar_id: u64,
        entries: Vec<TestEntry>,
        by_reading: HashMap<String, Vec<u32>>,
        by_surface: HashMap<String, Vec<u32>>,
        max_reading_chars: usize,
        max_surface_chars: usize,
    }

    struct TestEntry {
        surface: String,
        left_id: u16,
        right_id: u16,
        cost: i16,
        pos_id: u16,
        penalty: i16,
        lemma: Option<String>,
        lemma_id: Option<u32>,
    }

    impl TestDict {
        pub fn new(grammar_id: u64) -> Self {
            Self {
                grammar_id,
                entries: Vec::new(),
                by_reading: HashMap::new(),
                by_surface: HashMap::new(),
                max_reading_chars: 0,
                max_surface_chars: 0,
            }
        }

        /// エントリを足し、その添字（`WordId.index`）を返す。
        pub fn add(
            &mut self,
            reading: &str,
            surface: &str,
            left_id: u16,
            right_id: u16,
            cost: i16,
            pos_id: u16,
        ) -> u32 {
            let id = self.entries.len() as u32;
            self.entries.push(TestEntry {
                surface: surface.to_owned(),
                left_id,
                right_id,
                cost,
                pos_id,
                penalty: 0,
                lemma: None,
                lemma_id: None,
            });
            self.max_reading_chars = self.max_reading_chars.max(reading.chars().count());
            self.max_surface_chars = self.max_surface_chars.max(surface.chars().count());
            for (map, key) in [
                (&mut self.by_reading, reading),
                (&mut self.by_surface, surface),
            ] {
                let ids = map.entry(key.to_owned()).or_default();
                let entries = &self.entries;
                let at = ids.partition_point(|&i| entries[i as usize].cost <= cost);
                ids.insert(at, id);
            }
            id
        }

        pub fn set_penalty(&mut self, id: u32, penalty: i16) {
            self.entries[id as usize].penalty = penalty;
        }

        pub fn set_lemma(&mut self, id: u32, lemma: &str, lemma_id: Option<u32>) {
            let e = &mut self.entries[id as usize];
            e.lemma = Some(lemma.to_owned());
            e.lemma_id = lemma_id;
        }

        fn word_entry(&self, index: u32) -> Option<WordEntry<'_>> {
            let e = self.entries.get(index as usize)?;
            Some(WordEntry {
                id: WordId { dict: 0, index },
                surface: &e.surface,
                left_id: e.left_id,
                right_id: e.right_id,
                cost: e.cost,
                pos_id: e.pos_id,
                penalty: e.penalty,
            })
        }

        fn prefix_search(
            &self,
            map: &HashMap<String, Vec<u32>>,
            max_chars: usize,
            text: &str,
            f: &mut dyn FnMut(usize, WordEntry<'_>),
        ) {
            for (i, c) in text.char_indices().take(max_chars) {
                let end = i + c.len_utf8();
                if let Some(ids) = map.get(&text[..end]) {
                    for &id in ids {
                        if let Some(e) = self.word_entry(id) {
                            f(end, e);
                        }
                    }
                }
            }
        }
    }

    impl Dictionary for TestDict {
        fn common_prefix_search(&self, key: &str, f: &mut dyn FnMut(usize, WordEntry<'_>)) {
            self.prefix_search(&self.by_reading, self.max_reading_chars, key, f);
        }

        fn lookup_exact(&self, reading: &str, f: &mut dyn FnMut(WordEntry<'_>)) {
            for &id in self.by_reading.get(reading).into_iter().flatten() {
                if let Some(e) = self.word_entry(id) {
                    f(e);
                }
            }
        }

        fn entry(&self, id: WordId) -> Option<WordEntry<'_>> {
            if id.dict != 0 {
                return None;
            }
            self.word_entry(id.index)
        }

        fn grammar_id(&self) -> u64 {
            self.grammar_id
        }

        fn common_prefix_search_surface(
            &self,
            text: &str,
            f: &mut dyn FnMut(usize, WordEntry<'_>),
        ) {
            self.prefix_search(&self.by_surface, self.max_surface_chars, text, f);
        }

        fn lemma(&self, id: WordId) -> Option<Lemma<'_>> {
            if id.dict != 0 {
                return None;
            }
            let e = self.entries.get(id.index as usize)?;
            Some(Lemma {
                text: e.lemma.as_deref().unwrap_or(&e.surface),
                id: e.lemma_id,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;
    use crate::dict::{Grammar, InMemoryDictionary};

    fn eos_best(lat: &Lattice, g: &dyn Grammar) -> Option<i32> {
        lat.ending_at(lat.len_chars() as u32)
            .iter()
            .map(|&id| {
                let n = lat.node(id);
                n.best + i32::from(g.connection_cost(n.right_id, g.bos_eos_id()))
            })
            .min()
    }

    #[test]
    fn every_position_has_an_unknown_node() {
        let g = grammar();
        let d = dict_for(&g);
        let lat = build("あ←い", &d, &g, 0);
        assert_eq!(lat.len_chars(), 3);
        assert_eq!(lat.node_count(), 3);
        for pos in 1..=3u32 {
            let ends = lat.ending_at(pos);
            assert_eq!(ends.len(), 1);
            let n = lat.node(ends[0]);
            assert!(n.word.is_none());
            assert_eq!(n.end - n.start, 1);
            assert_eq!(lat.slice(n).chars().count(), 1);
        }
        assert_eq!(lat.slice(lat.node(lat.ending_at(2)[0])), "←");
        assert_eq!(eos_best(&lat, &g), Some(3 * i32::from(UNK_COST)));
    }

    #[test]
    fn viterbi_picks_cheaper_split_using_connection_cost() {
        let mut g = grammar();
        // か+った は 単語コストでは安いが、名詞->名詞 の連接が高い
        g.set(ID_NOUN, ID_NOUN, 3000);
        g.set(ID_NOUN, ID_PARTICLE, 100);
        let mut d = dict_for(&g);
        d.insert("かった", "勝った", ID_NOUN, ID_NOUN, 4000, 0);
        d.insert("か", "買", ID_NOUN, ID_NOUN, 1000, 0);
        d.insert("った", "った", ID_PARTICLE, ID_PARTICLE, 1000, 2);
        let lat = build("かった", &d, &g, 0);
        // 買+った = 1000 + 100 + 1000 = 2100 、勝った = 4000
        assert_eq!(eos_best(&lat, &g), Some(2100));
    }

    #[test]
    fn bos_right_id_affects_first_connection() {
        let mut g = grammar();
        g.set(ID_PARTICLE, ID_NOUN, -500);
        let mut d = InMemoryDictionary::new(g.grammar_id());
        d.insert("あ", "亜", ID_NOUN, ID_NOUN, 100, 0);
        let from_bos = eos_best(&build("あ", &d, &g, 0), &g).unwrap();
        let from_particle = eos_best(&build("あ", &d, &g, ID_PARTICLE), &g).unwrap();
        assert_eq!(from_bos, 100);
        assert_eq!(from_particle, 100 - 500);
    }

    #[test]
    fn several_bos_options_use_the_cheapest_connection_plus_its_extra_cost() {
        let mut g = grammar();
        g.set(ID_PARTICLE, ID_NOUN, -500);
        let mut d = InMemoryDictionary::new(g.grammar_id());
        d.insert("あ", "亜", ID_NOUN, ID_NOUN, 100, 0);
        let best = |starts: &[(u16, i32)]| {
            eos_best(
                &build_with_starts("あ", &d, &g, starts, &EdgeRules::none()),
                &g,
            )
            .unwrap()
        };
        // 追加コスト 200 を払っても、-500 の接続の方が安い
        assert_eq!(best(&[(0, 0), (ID_PARTICLE, 200)]), 100 - 500 + 200);
        // 追加コストが大きければ、BOS 側が選ばれる
        assert_eq!(best(&[(0, 0), (ID_PARTICLE, 900)]), 100);
        // 空なら bos_eos_id だけ
        assert_eq!(best(&[]), 100);
    }

    #[test]
    fn empty_key_builds_empty_lattice() {
        let g = grammar();
        let d = dict_for(&g);
        let lat = build("", &d, &g, 0);
        assert_eq!(lat.len_chars(), 0);
        assert_eq!(lat.node_count(), 0);
        assert_eq!(eos_best(&lat, &g), None);
    }

    #[test]
    fn penalty_is_added_to_node_cost() {
        let g = grammar();
        let mut d = TestDict::new(g.grammar_id());
        let id = d.add("あ", "亜", ID_NOUN, ID_NOUN, 100, 0);
        d.set_penalty(id, 3000);
        let lat = build("あ", &d, &g, 0);
        let costs: Vec<i16> = lat
            .ending_at(1)
            .iter()
            .filter(|&&n| lat.node(n).word.is_some())
            .map(|&n| lat.node(n).cost)
            .collect();
        assert_eq!(costs, [3100]);
    }

    #[test]
    fn penalty_saturates_instead_of_overflowing() {
        let g = grammar();
        let mut d = TestDict::new(g.grammar_id());
        let id = d.add("あ", "亜", ID_NOUN, ID_NOUN, 30000, 0);
        d.set_penalty(id, 30000);
        let lat = build("あ", &d, &g, 0);
        let max = lat
            .ending_at(1)
            .iter()
            .map(|&n| lat.node(n).cost)
            .max()
            .unwrap();
        assert_eq!(max, i16::MAX);
    }

    #[test]
    fn excluded_entries_are_not_nodes() {
        let g = grammar();
        let mut d = TestDict::new(g.grammar_id());
        let id = d.add("あ", "亜", ID_NOUN, ID_NOUN, 100, 0);
        d.set_penalty(id, IME_EXCLUDED);
        let lat = build("あ", &d, &g, 0);
        // 未知語の1文字ノードだけが残る
        assert_eq!(lat.node_count(), 1);
        assert!(lat.node(0).word.is_none());
    }

    /// へんかん／へんか／へん／か／かん を持つ辞書
    fn henkan_dict(g: &crate::dict::DenseGrammar) -> InMemoryDictionary {
        let mut d = dict_for(g);
        d.insert("へんかん", "変換", ID_NOUN, ID_NOUN, 1000, 0);
        d.insert("へんか", "変化", ID_NOUN, ID_NOUN, 1100, 0);
        d.insert("へん", "変", ID_NOUN, ID_NOUN, 2000, 0);
        d.insert("か", "課", ID_NOUN, ID_NOUN, 2500, 0);
        d.insert("かん", "缶", ID_NOUN, ID_NOUN, 2600, 0);
        d
    }

    fn spans(lat: &Lattice) -> Vec<(u32, u32, bool)> {
        (0..lat.node_count() as u32)
            .map(|id| {
                let n = lat.node(id);
                (n.start, n.end, n.word.is_some())
            })
            .collect()
    }

    #[test]
    fn constrained_without_boundaries_equals_unconstrained() {
        let g = grammar();
        let d = henkan_dict(&g);
        let a = build("へんかん", &d, &g, 0);
        let b = build_constrained("へんかん", &d, &g, &[(0, 0)], &[], &EdgeRules::none());
        assert_eq!(spans(&a), spans(&b));
        assert_eq!(eos_best(&a, &g), eos_best(&b, &g));
        // 範囲外の境界（0 と文字数以上）は境界にならない
        let c = build_constrained(
            "へんかん",
            &d,
            &g,
            &[(0, 0)],
            &[0, 4, 9],
            &EdgeRules::none(),
        );
        assert_eq!(spans(&a), spans(&c));
    }

    #[test]
    fn constrained_never_crosses_a_boundary() {
        let g = grammar();
        let d = henkan_dict(&g);
        // へん|かん: 変換（0..4）と 変化（0..3）は境界 2 をまたぐ
        let lat = build_constrained("へんかん", &d, &g, &[(0, 0)], &[2], &EdgeRules::none());
        for (start, end, _) in spans(&lat) {
            assert!(!(start < 2 && end > 2), "crosses: {start}..{end}");
        }
        let dict_nodes: Vec<(u32, u32)> = spans(&lat)
            .into_iter()
            .filter(|n| n.2)
            .map(|n| (n.0, n.1))
            .collect();
        assert!(dict_nodes.contains(&(0, 2))); // へん
        assert!(dict_nodes.contains(&(2, 4))); // かん
        assert!(dict_nodes.contains(&(2, 3))); // か
        assert!(!dict_nodes.contains(&(0, 4)));
        assert!(!dict_nodes.contains(&(0, 3)));
        // 1文字の未知語は各位置に残るので、EOS までつながる
        assert!(eos_best(&lat, &g).is_some());
        assert!(
            lat.ending_at(4)
                .iter()
                .any(|&id| lat.node(id).word.is_some())
        );
    }

    #[test]
    fn constrained_with_every_position_as_boundary_uses_only_single_chars() {
        let g = grammar();
        let d = henkan_dict(&g);
        let lat = build_constrained(
            "へんかん",
            &d,
            &g,
            &[(0, 0)],
            &[1, 2, 3],
            &EdgeRules::none(),
        );
        for (start, end, _) in spans(&lat) {
            assert_eq!(end - start, 1);
        }
        assert_eq!(eos_best(&lat, &g), Some(i32::from(UNK_COST) * 3 + 2500));
    }

    // ---- 副詞の直後の名詞（ADR 0008） ----

    const RULE: AdverbNounRule = AdverbNounRule {
        penalty: 6000,
        cover_max_cost: 10_000,
    };

    fn build_r(key: &str, d: &dyn Dictionary, g: &dyn Grammar, r: &EdgeRules) -> Lattice {
        build_with_starts(key, d, g, &[(g.bos_eos_id(), 0)], r)
    }

    /// 開始位置と終了位置で辞書のノードを探す（最初に見つかったもの）
    fn word_node(lat: &Lattice, start: u32, end: u32) -> &Node {
        (0..lat.node_count() as u32)
            .map(|id| lat.node(id))
            .find(|n| n.word.is_some() && n.start == start && n.end == end)
            .unwrap_or_else(|| panic!("no dictionary node {start}..{end}"))
    }

    fn unknown_node(lat: &Lattice, start: u32) -> &Node {
        (0..lat.node_count() as u32)
            .map(|id| lat.node(id))
            .find(|n| n.word.is_none() && n.start == start)
            .unwrap()
    }

    #[test]
    fn numeral_characters_in_the_key_become_numeral_unknown_nodes() {
        use crate::dict::PosInfo;
        let mut g = grammar();
        let numeral_pos = g.add_pos(NUMERAL_POS_NAME);
        let (left, right) = (3, 4);
        g.set_pos_info(
            numeral_pos,
            PosInfo {
                rep_left_id: left,
                rep_right_id: right,
                user_cost: 0,
                rep_share_permille: 1000,
            },
        );
        let d = TestDict::new(g.grammar_id());
        let r = EdgeRules::new(
            &g,
            AdverbNounRule::disabled(),
            NounSuruRule::disabled(),
            NounSuffixRule::disabled(),
            NumeralCoverRule::disabled(),
        );
        // 半角・全角の算用数字と漢数字は、名詞,数詞 の左右ID・品詞を持つ（コストは未知語のまま）
        let lat = build_r("3３三じ", &d, &g, &r);
        for start in 0..3 {
            let n = unknown_node(&lat, start);
            assert_eq!(
                (n.left_id, n.right_id, n.pos_id),
                (left, right, numeral_pos)
            );
            assert_eq!(i32::from(n.cost), i32::from(UNK_COST));
        }
        let kana = unknown_node(&lat, 3);
        assert_eq!((kana.left_id, kana.right_id), (ID_UNK, ID_UNK));
        // 品詞表に 名詞,数詞 がない文法では、未知語の既定のまま
        let g = grammar();
        let d = TestDict::new(g.grammar_id());
        let lat = build_r("3", &d, &g, &r);
        assert_eq!(unknown_node(&lat, 0).left_id, ID_UNK);
    }

    #[test]
    fn pos_table_classifies_by_the_first_pos_field() {
        let (mut g, adverb) = grammar_with_adverb();
        let proper = g.add_pos("名詞,固有名詞,地名,一般,*,*");
        let r = EdgeRules::new(
            &g,
            RULE,
            NounSuruRule::disabled(),
            NounSuffixRule::disabled(),
            NumeralCoverRule::disabled(),
        );
        let classes: Vec<u8> = r.pos_class.to_vec();
        assert_eq!(usize::from(proper) + 1, classes.len());
        assert_eq!(classes[usize::from(POS_ID_NOUN)], POS_NOUN);
        assert_eq!(classes[usize::from(proper)], POS_NOUN);
        assert_eq!(classes[usize::from(adverb)], POS_ADVERB);
        assert_eq!(classes[usize::from(POS_ID_PARTICLE)], POS_OTHER);
        // 規則が止まっていれば表を作らない
        assert!(
            EdgeRules::new(
                &g,
                AdverbNounRule::disabled(),
                NounSuruRule::disabled(),
                NounSuffixRule::disabled(),
                NumeralCoverRule::disabled(),
            )
            .pos_class
            .is_empty()
        );
    }

    #[test]
    fn adverb_then_noun_is_penalized_only_when_a_covering_noun_exists() {
        let (g, adverb) = grammar_with_adverb();
        let r = EdgeRules::new(
            &g,
            RULE,
            NounSuruRule::disabled(),
            NounSuffixRule::disabled(),
            NumeralCoverRule::disabled(),
        );
        // 掃除機（4000）が そう＋時期（2000）を覆う
        let d = adverb_dict(&g, adverb, 4000, POS_ID_NOUN);
        let lat = build_r("そうじき", &d, &g, &r);
        let (sou, jiki) = (word_node(&lat, 0, 2), word_node(&lat, 2, 4));
        assert_eq!(lat.edge_extra(sou, jiki), 6000);
        assert_eq!(jiki.best, 1000 + 1000 + 6000);
        // 前向きの最良は 掃除機
        assert_eq!(eos_best(&lat, &g), Some(4000));

        // 覆う名詞がなければ（同じ位置から始まる名詞が別の位置で終わる）足さない
        let mut d2 = TestDict::new(g.grammar_id());
        d2.add("そう", "そう", ID_NOUN, ID_NOUN, 1000, adverb);
        d2.add("じき", "時期", ID_NOUN, ID_NOUN, 1000, POS_ID_NOUN);
        d2.add("そうじ", "掃除", ID_NOUN, ID_NOUN, 500, POS_ID_NOUN);
        let lat = build_r("そうじき", &d2, &g, &r);
        let (sou, jiki) = (word_node(&lat, 0, 2), word_node(&lat, 2, 4));
        assert_eq!(lat.edge_extra(sou, jiki), 0);
        assert_eq!(jiki.best, 2000);
    }

    #[test]
    fn covering_word_must_be_a_noun_within_the_cost_cap() {
        let (g, adverb) = grammar_with_adverb();
        let r = EdgeRules::new(
            &g,
            RULE,
            NounSuruRule::disabled(),
            NounSuffixRule::disabled(),
            NumeralCoverRule::disabled(),
        );
        let extra = |d: &TestDict, r: &EdgeRules| {
            let lat = build_r("そうじき", d, &g, r);
            lat.edge_extra(word_node(&lat, 0, 2), word_node(&lat, 2, 4))
        };
        // ちょうど上限は覆う名詞になる。超えるとならない
        assert_eq!(
            extra(&adverb_dict(&g, adverb, 10_000, POS_ID_NOUN), &r),
            6000
        );
        assert_eq!(extra(&adverb_dict(&g, adverb, 10_001, POS_ID_NOUN), &r), 0);
        // 上限は変換用のコスト（cost + penalty）で比べる
        let mut d = adverb_dict(&g, adverb, 4000, POS_ID_NOUN);
        d.set_penalty(2, 7000);
        assert_eq!(extra(&d, &r), 0);
        // 名詞でない語は覆う名詞にならない
        assert_eq!(
            extra(&adverb_dict(&g, adverb, 4000, POS_ID_PARTICLE), &r),
            0
        );
        // 上限と加算の値は設定で変わる
        let wide = EdgeRules::new(
            &g,
            AdverbNounRule {
                penalty: 4000,
                cover_max_cost: 20_000,
            },
            NounSuruRule::disabled(),
            NounSuffixRule::disabled(),
            NumeralCoverRule::disabled(),
        );
        assert_eq!(
            extra(&adverb_dict(&g, adverb, 15_000, POS_ID_NOUN), &wide),
            4000
        );
    }

    #[test]
    fn only_adverb_to_noun_edges_between_dictionary_nodes() {
        let (g, adverb) = grammar_with_adverb();
        let r = EdgeRules::new(
            &g,
            RULE,
            NounSuruRule::disabled(),
            NounSuffixRule::disabled(),
            NumeralCoverRule::disabled(),
        );
        let d = adverb_dict(&g, adverb, 4000, POS_ID_NOUN);
        let lat = build_r("そうじき", &d, &g, &r);
        let sou = word_node(&lat, 0, 2);
        let jiki = word_node(&lat, 2, 4);
        // 名詞 → 名詞、副詞 → 未知語（未知語の品詞が名詞でも数えない）、未知語 → 名詞
        assert_eq!(lat.edge_extra(jiki, jiki), 0);
        assert_eq!(lat.edge_extra(sou, unknown_node(&lat, 2)), 0);
        assert_eq!(lat.edge_extra(unknown_node(&lat, 1), jiki), 0);
        // 副詞 → 助詞（同じ範囲を覆う名詞があっても、次が名詞でなければ足さない）
        let mut d = adverb_dict(&g, adverb, 4000, POS_ID_NOUN);
        d.add("じき", "じき", ID_NOUN, ID_NOUN, 900, POS_ID_PARTICLE);
        let lat = build_r("そうじき", &d, &g, &r);
        let particle = (0..lat.node_count() as u32)
            .map(|id| lat.node(id))
            .find(|n| n.word.is_some() && n.start == 2 && n.pos_id == POS_ID_PARTICLE)
            .unwrap();
        assert_eq!(lat.edge_extra(word_node(&lat, 0, 2), particle), 0);
    }

    #[test]
    fn bos_and_eos_edges_are_not_penalized() {
        let (g, adverb) = grammar_with_adverb();
        let r = EdgeRules::new(
            &g,
            RULE,
            NounSuruRule::disabled(),
            NounSuffixRule::disabled(),
            NumeralCoverRule::disabled(),
        );
        let d = adverb_dict(&g, adverb, 4000, POS_ID_NOUN);
        // 左文脈の最後の語（BOS）が副詞の そう でも、最初の語 時期 への辺には足さない
        let lat = build_with_starts("じき", &d, &g, &[(ID_NOUN, 0)], &r);
        assert_eq!(eos_best(&lat, &g), Some(1000));
        // 読みが副詞で終わるときの EOS への辺にも足さない
        let lat = build_r("そう", &d, &g, &r);
        assert_eq!(eos_best(&lat, &g), Some(1000));
    }

    #[test]
    fn disabled_rule_matches_the_plain_lattice() {
        let (g, adverb) = grammar_with_adverb();
        let d = adverb_dict(&g, adverb, 4000, POS_ID_NOUN);
        let off = build_r(
            "そうじき",
            &d,
            &g,
            &EdgeRules::new(
                &g,
                AdverbNounRule::disabled(),
                NounSuruRule::disabled(),
                NounSuffixRule::disabled(),
                NumeralCoverRule::disabled(),
            ),
        );
        let none = build_r("そうじき", &d, &g, &EdgeRules::none());
        let bests = |lat: &Lattice| {
            (0..lat.node_count() as u32)
                .map(|id| lat.node(id).best)
                .collect::<Vec<_>>()
        };
        assert_eq!(bests(&off), bests(&none));
        assert_eq!(eos_best(&off, &g), Some(2000));
        assert!(off.cover_ends.is_empty());
    }

    #[test]
    fn user_boundary_inside_the_covering_noun_keeps_adverb_plus_noun() {
        let (g, adverb) = grammar_with_adverb();
        let r = EdgeRules::new(
            &g,
            RULE,
            NounSuruRule::disabled(),
            NounSuffixRule::disabled(),
            NumeralCoverRule::disabled(),
        );
        let d = adverb_dict(&g, adverb, 4000, POS_ID_NOUN);
        // そう｜じき に区切ると、掃除機 は境界をまたぐので、ノードにも覆う名詞にもならない（ADR 0008 §4.3）
        let lat = build_constrained("そうじき", &d, &g, &[(0, 0)], &[2], &r);
        let (sou, jiki) = (word_node(&lat, 0, 2), word_node(&lat, 2, 4));
        assert_eq!(lat.edge_extra(sou, jiki), 0);
        assert_eq!(eos_best(&lat, &g), Some(2000));
        // 境界が覆う名詞の外なら、規則はそのまま働く
        let lat = build_constrained("そうじきそう", &d, &g, &[(0, 0)], &[4], &r);
        let (sou, jiki) = (word_node(&lat, 0, 2), word_node(&lat, 2, 4));
        assert_eq!(lat.edge_extra(sou, jiki), 6000);
    }

    // ---- サ変でない名詞＋する（ADR 0016 規則 S） ----

    const SURU_PENALTY: i32 = 3000;
    const GENERAL_NOUN: &str = "名詞,普通名詞,一般,*,*,*";
    const SAHEN_NOUN: &str = "名詞,普通名詞,サ変可能,*,*,*";
    const SAHEN_ADJ_NOUN: &str = "名詞,普通名詞,サ変形状詞可能,*,*,*";
    const NUMERAL: &str = "名詞,数詞,*,*,*,*";
    const SURU_VERB_POS: &str = "動詞,非自立可能,*,*,サ行変格,終止形-一般";
    const LITERARY_SURU_VERB: &str = "動詞,非自立可能,*,*,文語サ行変格,連体形-一般";
    const LITERARY_SU_AUX: &str = "助動詞,*,*,*,文語下二段-サ行,終止形-一般";
    const OTHER_VERB: &str = "動詞,一般,*,*,五段-カ行,終止形-一般";
    const OTHER_AUX: &str = "助動詞,*,*,*,助動詞-タ,終止形-一般";

    #[test]
    fn suru_class_follows_pos3_and_the_conjugation_type() {
        assert_eq!(suru_class_of(GENERAL_NOUN), SURU_NON_SAHEN_NOUN);
        assert_eq!(suru_class_of(NUMERAL), SURU_NON_SAHEN_NOUN);
        assert_eq!(
            suru_class_of("名詞,固有名詞,地名,一般,*,*"),
            SURU_NON_SAHEN_NOUN
        );
        assert_eq!(suru_class_of(SAHEN_NOUN), SURU_OTHER);
        assert_eq!(suru_class_of(SAHEN_ADJ_NOUN), SURU_OTHER);
        assert_eq!(suru_class_of(SURU_VERB_POS), SURU_VERB);
        assert_eq!(suru_class_of(LITERARY_SURU_VERB), SURU_VERB);
        assert_eq!(suru_class_of(LITERARY_SU_AUX), SURU_VERB);
        assert_eq!(suru_class_of(OTHER_VERB), SURU_OTHER);
        assert_eq!(suru_class_of(OTHER_AUX), SURU_OTHER);
        assert_eq!(suru_class_of("副詞,*,*,*,*,*"), SURU_OTHER);
        assert_eq!(suru_class_of(""), SURU_OTHER);
    }

    /// 「あい」(prev_pos) + 「する」(next_pos) の辺の補正。ノードの辞書は 2 語だけ
    fn suru_extra(prev_pos: &str, next_pos: &str, rule: NounSuruRule) -> i32 {
        let mut g = grammar();
        let p = g.add_pos(prev_pos);
        let n = g.add_pos(next_pos);
        let mut d = TestDict::new(g.grammar_id());
        d.add("あい", "愛", ID_NOUN, ID_NOUN, 1000, p);
        d.add("する", "する", ID_NOUN, ID_NOUN, 1000, n);
        let r = EdgeRules::new(
            &g,
            AdverbNounRule::disabled(),
            rule,
            NounSuffixRule::disabled(),
            NumeralCoverRule::disabled(),
        );
        let lat = build_r("あいする", &d, &g, &r);
        lat.edge_extra(word_node(&lat, 0, 2), word_node(&lat, 2, 4))
    }

    #[test]
    fn non_sahen_noun_then_suru_is_penalized() {
        let rule = NounSuruRule::default();
        assert_eq!(rule.penalty, SURU_PENALTY);
        for prev in [GENERAL_NOUN, NUMERAL] {
            for next in [SURU_VERB_POS, LITERARY_SURU_VERB, LITERARY_SU_AUX] {
                assert_eq!(
                    suru_extra(prev, next, rule),
                    SURU_PENALTY,
                    "{prev} -> {next}"
                );
            }
        }
    }

    #[test]
    fn sahen_nouns_and_other_followers_are_not_penalized() {
        let rule = NounSuruRule::default();
        for prev in [SAHEN_NOUN, SAHEN_ADJ_NOUN] {
            assert_eq!(suru_extra(prev, SURU_VERB_POS, rule), 0, "{prev}");
        }
        for next in [OTHER_VERB, OTHER_AUX, GENERAL_NOUN] {
            assert_eq!(suru_extra(GENERAL_NOUN, next, rule), 0, "{next}");
        }
        // 前が名詞でなければ（動詞＋する）足さない
        assert_eq!(suru_extra(OTHER_VERB, SURU_VERB_POS, rule), 0);
    }

    #[test]
    fn noun_suru_rule_off_adds_nothing_and_builds_no_table() {
        assert_eq!(
            suru_extra(GENERAL_NOUN, SURU_VERB_POS, NounSuruRule::disabled()),
            0
        );
        let g = grammar();
        assert!(
            EdgeRules::new(
                &g,
                AdverbNounRule::disabled(),
                NounSuruRule::disabled(),
                NounSuffixRule::disabled(),
                NumeralCoverRule::disabled(),
            )
            .suru_class
            .is_empty()
        );
    }

    #[test]
    fn noun_suru_ignores_unknown_nodes() {
        let g = grammar();
        let d = dict_for(&g);
        let r = EdgeRules::new(
            &g,
            AdverbNounRule::disabled(),
            NounSuruRule::default(),
            NounSuffixRule::disabled(),
            NumeralCoverRule::disabled(),
        );
        let lat = build_r("あい", &d, &g, &r);
        let unk = unknown_node(&lat, 0);
        assert_eq!(lat.edge_extra(unk, unk), 0);
    }

    // ---- 名詞 → 接尾辞（ADR 0008 §13。B25） ----

    const SUFFIX_RULE: NounSuffixRule = NounSuffixRule {
        penalty: 6000,
        cover_max_cost: 10_000,
    };
    const SUFFIX_POS: &str = "接尾辞,名詞的,一般,*,*,*";
    const PREFIX_POS: &str = "接頭辞,*,*,*,*,*";

    fn suffix_rules(g: &crate::dict::DenseGrammar, rule: NounSuffixRule) -> EdgeRules {
        EdgeRules::new(
            g,
            AdverbNounRule::disabled(),
            NounSuruRule::disabled(),
            rule,
            NumeralCoverRule::disabled(),
        )
    }

    /// ていすう（低数、`prev_pos`）＋めい（名、`next_pos`）と、ていすうめい（定数名、`cover_pos` で `cover_cost`）。
    /// 辺 低数 -> 名 の補正を返す
    fn suffix_extra(
        prev_pos: &str,
        next_pos: &str,
        cover_pos: u16,
        cover_cost: i16,
        rule: NounSuffixRule,
    ) -> i32 {
        let mut g = grammar();
        let p = g.add_pos(prev_pos);
        let n = g.add_pos(next_pos);
        let mut d = TestDict::new(g.grammar_id());
        d.add("てい", "低", ID_NOUN, ID_NOUN, 1000, p);
        d.add("すう", "数", ID_NOUN, ID_NOUN, 1000, n);
        d.add("ていすう", "定数", ID_NOUN, ID_NOUN, cover_cost, cover_pos);
        let r = suffix_rules(&g, rule);
        let lat = build_r("ていすう", &d, &g, &r);
        lat.edge_extra(word_node(&lat, 0, 2), word_node(&lat, 2, 4))
    }

    #[test]
    fn pos_table_marks_suffixes_when_only_the_suffix_rule_is_on() {
        let mut g = grammar();
        let suffix = g.add_pos(SUFFIX_POS);
        let prefix = g.add_pos(PREFIX_POS);
        let r = suffix_rules(&g, SUFFIX_RULE);
        assert_eq!(r.pos_class[usize::from(suffix)], POS_SUFFIX);
        assert_eq!(r.pos_class[usize::from(prefix)], POS_OTHER);
        assert_eq!(r.pos_class[usize::from(POS_ID_NOUN)], POS_NOUN);
        assert!(
            suffix_rules(&g, NounSuffixRule::disabled())
                .pos_class
                .is_empty()
        );
    }

    #[test]
    fn noun_then_suffix_is_penalized_only_when_a_covering_noun_exists() {
        let mut g = grammar();
        let suffix = g.add_pos(SUFFIX_POS);
        let mut d = TestDict::new(g.grammar_id());
        d.add("てい", "低", ID_NOUN, ID_NOUN, 1000, POS_ID_NOUN);
        d.add("すう", "数", ID_NOUN, ID_NOUN, 1000, suffix);
        d.add("ていすう", "定数", ID_NOUN, ID_NOUN, 4000, POS_ID_NOUN);
        let r = suffix_rules(&g, SUFFIX_RULE);
        let lat = build_r("ていすう", &d, &g, &r);
        let (tei, suu) = (word_node(&lat, 0, 2), word_node(&lat, 2, 4));
        assert_eq!(lat.edge_extra(tei, suu), 6000);
        assert_eq!(suu.best, 1000 + 1000 + 6000);
        // 前向きの最良は 定数
        assert_eq!(eos_best(&lat, &g), Some(4000));

        // 覆う名詞がなければ足さない
        let mut d2 = TestDict::new(g.grammar_id());
        d2.add("てい", "低", ID_NOUN, ID_NOUN, 1000, POS_ID_NOUN);
        d2.add("すう", "数", ID_NOUN, ID_NOUN, 1000, suffix);
        let lat = build_r("ていすう", &d2, &g, &r);
        assert_eq!(
            lat.edge_extra(word_node(&lat, 0, 2), word_node(&lat, 2, 4)),
            0
        );
    }

    #[test]
    fn suffix_edge_classification_and_cover_boundary() {
        let rule = SUFFIX_RULE;
        let pen = rule.penalty;
        // 名詞 -> 接尾辞 だけが対象
        assert_eq!(suffix_extra(GENERAL_NOUN, SUFFIX_POS, 0, 4000, rule), pen);
        assert_eq!(
            suffix_extra("名詞,固有名詞,地名,一般,*,*", SUFFIX_POS, 0, 4000, rule),
            pen
        );
        // 名詞 -> 名詞、接頭辞 -> 名詞、動詞 -> 接尾辞 は対象外
        assert_eq!(suffix_extra(GENERAL_NOUN, GENERAL_NOUN, 0, 4000, rule), 0);
        assert_eq!(suffix_extra(PREFIX_POS, GENERAL_NOUN, 0, 4000, rule), 0);
        assert_eq!(suffix_extra(OTHER_VERB, SUFFIX_POS, 0, 4000, rule), 0);
        // 接尾辞 -> 接尾辞（前が名詞でない）も対象外
        assert_eq!(suffix_extra(SUFFIX_POS, SUFFIX_POS, 0, 4000, rule), 0);
        // 覆う語の境界: ちょうど上限は覆う、超えると覆わない、名詞でない語は覆わない
        assert_eq!(suffix_extra(GENERAL_NOUN, SUFFIX_POS, 0, 10_000, rule), pen);
        assert_eq!(suffix_extra(GENERAL_NOUN, SUFFIX_POS, 0, 10_001, rule), 0);
        assert_eq!(
            suffix_extra(GENERAL_NOUN, SUFFIX_POS, POS_ID_PARTICLE, 4000, rule),
            0
        );
        // 上限と加算は設定で変わる。止めれば 0
        let wide = NounSuffixRule {
            penalty: 4000,
            cover_max_cost: 20_000,
        };
        assert_eq!(
            suffix_extra(GENERAL_NOUN, SUFFIX_POS, 0, 15_000, wide),
            4000
        );
        assert_eq!(
            suffix_extra(
                GENERAL_NOUN,
                SUFFIX_POS,
                0,
                4000,
                NounSuffixRule::disabled(),
            ),
            0
        );
    }

    #[test]
    fn each_rule_uses_its_own_cover_cap() {
        let (mut g, adverb) = grammar_with_adverb();
        let suffix = g.add_pos(SUFFIX_POS);
        let mut d = TestDict::new(g.grammar_id());
        d.add("そう", "そう", ID_NOUN, ID_NOUN, 1000, adverb);
        d.add("じき", "時期", ID_NOUN, ID_NOUN, 1000, POS_ID_NOUN);
        d.add("そうじ", "掃除", ID_NOUN, ID_NOUN, 1000, POS_ID_NOUN);
        d.add("き", "機", ID_NOUN, ID_NOUN, 1000, suffix);
        // 覆う名詞: そうじき（9000、副詞の規則の上限 8000 は超え、接尾辞の規則の上限 10000 以下）
        d.add("そうじき", "掃除機", ID_NOUN, ID_NOUN, 9000, POS_ID_NOUN);
        let r = EdgeRules::new(
            &g,
            AdverbNounRule {
                penalty: 6000,
                cover_max_cost: 8000,
            },
            NounSuruRule::disabled(),
            SUFFIX_RULE,
            NumeralCoverRule::disabled(),
        );
        let lat = build_r("そうじき", &d, &g, &r);
        // 掃除 -> 機（0..3 -> 3..4）は接尾辞の規則で足す
        assert_eq!(
            lat.edge_extra(word_node(&lat, 0, 3), word_node(&lat, 3, 4)),
            6000
        );
        // そう -> 時期 は副詞の規則の上限（8000）を超えるので足さない
        assert_eq!(
            lat.edge_extra(word_node(&lat, 0, 2), word_node(&lat, 2, 4)),
            0
        );
    }

    #[test]
    fn noun_suffix_rule_off_builds_no_cover_table_when_both_rules_are_off() {
        let g = grammar();
        let d = dict_for(&g);
        let off = suffix_rules(&g, NounSuffixRule::disabled());
        assert!(build_r("あい", &d, &g, &off).cover_ends.is_empty());
        let on = suffix_rules(&g, SUFFIX_RULE);
        assert!(!build_r("あい", &d, &g, &on).cover_ends.is_empty());
    }

    // ---- 前の語 → 数詞「数」（ADR 0019 規則 N。B33） ----

    const NUMERAL_RULE: NumeralCoverRule = NumeralCoverRule {
        penalty: 6000,
        cover_max_cost: 10_000,
    };
    const NUMERAL_POS: &str = "名詞,数詞,*,*,*,*";

    fn numeral_rules(g: &crate::dict::DenseGrammar, rule: NumeralCoverRule) -> EdgeRules {
        EdgeRules::new(
            g,
            AdverbNounRule::disabled(),
            NounSuruRule::disabled(),
            NounSuffixRule::disabled(),
            rule,
        )
    }

    /// へん（`prev_pos`）＋すう（`next_reading`、`next_pos`）と、へんすう（変数、`cover_pos` で `cover_cost`）。
    /// 辺 へん -> すう の補正を返す。`next_reading` が すう 以外のときは、読みの長さを同じにして比べる
    fn numeral_extra(
        prev_pos: &str,
        next_pos: &str,
        next_reading: &str,
        cover_pos: u16,
        cover_cost: i16,
        rule: NumeralCoverRule,
    ) -> i32 {
        let mut g = grammar();
        let p = g.add_pos(prev_pos);
        let n = g.add_pos(next_pos);
        let mut d = TestDict::new(g.grammar_id());
        d.add("へん", "変", ID_NOUN, ID_NOUN, 1000, p);
        d.add(next_reading, "数", ID_NOUN, ID_NOUN, 1000, n);
        let key = format!("へん{next_reading}");
        d.add(&key, "変数", ID_NOUN, ID_NOUN, cover_cost, cover_pos);
        let r = numeral_rules(&g, rule);
        let lat = build_r(&key, &d, &g, &r);
        lat.edge_extra(word_node(&lat, 0, 2), word_node(&lat, 2, 4))
    }

    #[test]
    fn pos_table_marks_numerals_only_when_the_numeral_rule_is_on() {
        let mut g = grammar();
        let numeral = g.add_pos(NUMERAL_POS);
        let r = numeral_rules(&g, NUMERAL_RULE);
        assert!(r.numeral_class[usize::from(numeral)]);
        assert!(!r.numeral_class[usize::from(POS_ID_NOUN)]);
        // 数詞は、既存の規則から見て名詞のまま
        assert_eq!(r.pos_class[usize::from(numeral)], POS_NOUN);
        let off = numeral_rules(&g, NumeralCoverRule::disabled());
        assert!(off.numeral_class.is_empty());
        assert!(off.pos_class.is_empty());
    }

    #[test]
    fn numeral_suu_after_a_word_is_penalized_only_when_a_covering_noun_exists() {
        let on = |prev, cover_pos, cost| {
            numeral_extra(prev, NUMERAL_POS, "すう", cover_pos, cost, NUMERAL_RULE)
        };
        // 変数 が覆うとき
        assert_eq!(on(GENERAL_NOUN, POS_ID_NOUN, 4000), 6000);
        // 前の語の品詞は問わない（接頭辞、動詞など）
        assert_eq!(on(PREFIX_POS, POS_ID_NOUN, 4000), 6000);
        // 覆う名詞がコストの上限を超える／名詞でない
        assert_eq!(on(GENERAL_NOUN, POS_ID_NOUN, 10_000), 6000);
        assert_eq!(on(GENERAL_NOUN, POS_ID_NOUN, 10_001), 0);
        assert_eq!(on(GENERAL_NOUN, POS_ID_PARTICLE, 4000), 0);
    }

    #[test]
    fn numeral_rule_ignores_other_numerals_readings_and_previous_numerals() {
        // 読みが すう でない数詞（ここでは ずう）は対象外
        assert_eq!(
            numeral_extra(
                GENERAL_NOUN,
                NUMERAL_POS,
                "ずう",
                POS_ID_NOUN,
                4000,
                NUMERAL_RULE
            ),
            0
        );
        // 数詞でない すう（名詞）は対象外
        assert_eq!(
            numeral_extra(
                GENERAL_NOUN,
                GENERAL_NOUN,
                "すう",
                POS_ID_NOUN,
                4000,
                NUMERAL_RULE
            ),
            0
        );
        // 前の語が数詞（十 + 数）は対象外
        assert_eq!(
            numeral_extra(
                NUMERAL_POS,
                NUMERAL_POS,
                "すう",
                POS_ID_NOUN,
                4000,
                NUMERAL_RULE
            ),
            0
        );
    }

    #[test]
    fn numeral_rule_can_be_stopped_and_has_its_own_cap() {
        assert_eq!(
            numeral_extra(
                GENERAL_NOUN,
                NUMERAL_POS,
                "すう",
                POS_ID_NOUN,
                4000,
                NumeralCoverRule::disabled()
            ),
            0
        );
        let narrow = NumeralCoverRule {
            penalty: 6000,
            cover_max_cost: 3000,
        };
        assert_eq!(
            numeral_extra(GENERAL_NOUN, NUMERAL_POS, "すう", POS_ID_NOUN, 4000, narrow),
            0
        );
    }

    #[test]
    fn numeral_rule_does_not_touch_bos_or_unknown_nodes() {
        let mut g = grammar();
        let n = g.add_pos(NUMERAL_POS);
        let mut d = TestDict::new(g.grammar_id());
        d.add("すう", "数", ID_NOUN, ID_NOUN, 1000, n);
        d.add("へんすう", "変数", ID_NOUN, ID_NOUN, 4000, POS_ID_NOUN);
        let r = numeral_rules(&g, NUMERAL_RULE);
        let lat = build_r("へんすう", &d, &g, &r);
        // 未知語の へ・ん（辞書のノードでない）→ すう
        let unk = unknown_node(&lat, 0);
        assert_eq!(lat.edge_extra(unk, word_node(&lat, 2, 4)), 0);
    }

    // ---- 数詞 → 助数詞可能の名詞（ADR 0019 §15。#219） ----

    const COUNTER_NOUN_POS: &str = "名詞,普通名詞,助数詞可能,*,*,*";
    const COUNTER_SUFFIX_POS: &str = "接尾辞,名詞的,助数詞,*,*,*";
    const COUNTER_RULE: NumeralCounterRule = NumeralCounterRule {
        bonus: 3000,
        suffix_bonus: 0,
        ka_penalty: 0,
        address_bonus: 0,
        address_kana_bonus: 0,
        language_bonus: 0,
        ordinal_bonus: 0,
    };

    #[test]
    fn numeral_counter_bonus_applies_to_counter_nouns_after_a_numeral() {
        let mut g = grammar();
        let numeral = g.add_pos(NUMERAL_POS);
        g.set_pos_info(
            numeral,
            crate::dict::PosInfo {
                rep_left_id: ID_NOUN,
                rep_right_id: ID_NOUN,
                user_cost: 0,
                rep_share_permille: 1000,
            },
        );
        let counter = g.add_pos(COUNTER_NOUN_POS);
        let mut d = TestDict::new(g.grammar_id());
        d.add("に", "二", ID_NOUN, ID_NOUN, 1000, numeral);
        d.add("けん", "件", ID_NOUN, ID_NOUN, 8000, counter);
        d.add("けん", "県", ID_NOUN, ID_NOUN, 2000, POS_ID_NOUN);
        let rules = |rule| {
            EdgeRules::new_with_volitional(
                &g,
                AdverbNounRule::disabled(),
                NounSuruRule::disabled(),
                NounSuffixRule::disabled(),
                NumeralCoverRule::disabled(),
                VolitionalRule::disabled(),
                rule,
            )
        };
        let by_pos = |lat: &Lattice, pos: u16| -> Node {
            *(0..lat.node_count() as u32)
                .map(|id| lat.node(id))
                .find(|n| n.word.is_some() && n.start >= 1 && n.pos_id == pos)
                .unwrap()
        };
        let on = rules(COUNTER_RULE);
        // 読みの中の算用数字（未知語のノード。名詞,数詞）→ 助数詞可能の名詞は引く。普通の名詞は 0
        let lat = build_r("2けん", &d, &g, &on);
        let two = unknown_node(&lat, 0);
        assert_eq!(lat.edge_extra(two, &by_pos(&lat, counter)), -3000);
        assert_eq!(lat.edge_extra(two, &by_pos(&lat, POS_ID_NOUN)), 0);
        // かな読みから作った辞書の数詞（に → 二）には当てない
        let lat = build_r("にけん", &d, &g, &on);
        let ni = word_node(&lat, 0, 1);
        assert_eq!(lat.edge_extra(ni, &by_pos(&lat, counter)), 0);
        // 数詞でない前の語（件 → 件）は引かない。規則を止めれば 0
        let off = rules(NumeralCounterRule::disabled());
        let lat = build_r("2けん", &d, &g, &off);
        assert_eq!(
            lat.edge_extra(unknown_node(&lat, 0), &by_pos(&lat, counter)),
            0
        );
    }
    #[test]
    fn numeral_counter_address_bonus_picks_words_by_surface() {
        let mut g = grammar();
        let numeral = g.add_pos(NUMERAL_POS);
        g.set_pos_info(
            numeral,
            crate::dict::PosInfo {
                rep_left_id: ID_NOUN,
                rep_right_id: ID_NOUN,
                user_cost: 0,
                rep_share_permille: 1000,
            },
        );
        let counter = g.add_pos(COUNTER_NOUN_POS);
        let mut d = TestDict::new(g.grammar_id());
        d.add("ばん", "版", ID_NOUN, ID_NOUN, 7000, counter);
        d.add("ばん", "番", ID_NOUN, ID_NOUN, 8000, counter);
        let rule = NumeralCounterRule {
            bonus: 0,
            suffix_bonus: 0,
            ka_penalty: 0,
            address_bonus: 900,
            address_kana_bonus: 0,
            language_bonus: 0,
            ordinal_bonus: 0,
        };
        let rules = EdgeRules::new_with_volitional(
            &g,
            AdverbNounRule::disabled(),
            NounSuruRule::disabled(),
            NounSuffixRule::disabled(),
            NumeralCoverRule::disabled(),
            VolitionalRule::disabled(),
            rule,
        );
        let lat = build_r("2ばん", &d, &g, &rules);
        let two = unknown_node(&lat, 0);
        let edge = |surface_cost: i16| {
            let n = (0..lat.node_count() as u32)
                .map(|id| lat.node(id))
                .find(|n| n.word.is_some() && n.start == 1 && n.cost == surface_cost)
                .unwrap();
            lat.edge_extra(two, n)
        };
        assert_eq!(edge(8000), -900);
        assert_eq!(edge(7000), 0);
    }
    #[test]
    fn numeral_counter_ordinal_bonus_applies_after_dai_and_to_ordinal_counters_only() {
        let mut g = grammar();
        let numeral = g.add_pos(NUMERAL_POS);
        g.set_pos_info(
            numeral,
            crate::dict::PosInfo {
                rep_left_id: ID_NOUN,
                rep_right_id: ID_NOUN,
                user_cost: 0,
                rep_share_permille: 1000,
            },
        );
        let counter = g.add_pos(COUNTER_NOUN_POS);
        let mut d = TestDict::new(g.grammar_id());
        d.add("かん", "巻", ID_NOUN, ID_NOUN, 7000, counter);
        d.add("かん", "缶", ID_NOUN, ID_NOUN, 6000, counter);
        let rule = NumeralCounterRule {
            bonus: 0,
            suffix_bonus: 0,
            ka_penalty: 0,
            address_bonus: 0,
            address_kana_bonus: 0,
            language_bonus: 0,
            ordinal_bonus: 700,
        };
        let rules = EdgeRules::new_with_volitional(
            &g,
            AdverbNounRule::disabled(),
            NounSuruRule::disabled(),
            NounSuffixRule::disabled(),
            NumeralCoverRule::disabled(),
            VolitionalRule::disabled(),
            rule,
        );
        // 「だい」の直後の数字の並び（10）は、最後の桁から 巻 へ引く。缶 には当てない
        let lat = build_r("だい10かん", &d, &g, &rules);
        let zero = unknown_node(&lat, 3);
        let edge = |lat: &Lattice, prev: &Node, start: u32, cost: i16| {
            let n = (0..lat.node_count() as u32)
                .map(|id| lat.node(id))
                .find(|n| n.word.is_some() && n.start == start && n.cost == cost)
                .unwrap();
            lat.edge_extra(prev, n)
        };
        assert_eq!(edge(&lat, zero, 4, 7000), -700);
        assert_eq!(edge(&lat, zero, 4, 6000), 0);
        // 「だい」がなければ当てない
        let lat = build_r("10かん", &d, &g, &rules);
        let zero = unknown_node(&lat, 1);
        assert_eq!(edge(&lat, zero, 2, 7000), 0);
    }
    #[test]
    fn numeral_counter_suffix_rules_pick_words_by_reading() {
        let mut g = grammar();
        let numeral = g.add_pos(NUMERAL_POS);
        g.set_pos_info(
            numeral,
            crate::dict::PosInfo {
                rep_left_id: ID_NOUN,
                rep_right_id: ID_NOUN,
                user_cost: 0,
                rep_share_permille: 1000,
            },
        );
        let suffix = g.add_pos(COUNTER_SUFFIX_POS);
        let mut d = TestDict::new(g.grammar_id());
        d.add("こ", "個", ID_NOUN, ID_NOUN, 9000, suffix);
        d.add("さつ", "刷", ID_NOUN, ID_NOUN, 9000, suffix);
        d.add("か", "日", ID_NOUN, ID_NOUN, 9000, suffix);
        let rule = NumeralCounterRule {
            bonus: 3000,
            suffix_bonus: 700,
            ka_penalty: 800,
            address_bonus: 0,
            address_kana_bonus: 0,
            language_bonus: 0,
            ordinal_bonus: 0,
        };
        let rules = EdgeRules::new_with_volitional(
            &g,
            AdverbNounRule::disabled(),
            NounSuruRule::disabled(),
            NounSuffixRule::disabled(),
            NumeralCoverRule::disabled(),
            VolitionalRule::disabled(),
            rule,
        );
        let extra = |key: &str, reading_len: u32| -> i32 {
            let lat = build_r(key, &d, &g, &rules);
            let node = *(0..lat.node_count() as u32)
                .map(|id| lat.node(id))
                .find(|n| n.word.is_some() && n.start == 1 && n.end == 1 + reading_len)
                .unwrap();
            lat.edge_extra(unknown_node(&lat, 0), &node)
        };
        // こ（個）は引く、か（日）は足す、それ以外の接尾辞,助数詞（刷）は 0
        assert_eq!(extra("2こ", 1), -700);
        assert_eq!(extra("2か", 1), 800);
        assert_eq!(extra("2さつ", 2), 0);
    }
    #[test]
    fn numeral_counter_language_bonus_applies_to_go_after_ka_koku_only() {
        let mut g = grammar();
        let counter = g.add_pos(COUNTER_NOUN_POS);
        let mut d = TestDict::new(g.grammar_id());
        d.add("かこく", "か国", ID_NOUN, ID_NOUN, 9000, counter);
        d.add("かげつ", "か月", ID_NOUN, ID_NOUN, 9000, counter);
        d.add("ご", "語", ID_NOUN, ID_NOUN, 5000, POS_ID_NOUN);
        d.add("ご", "後", ID_NOUN, ID_NOUN, 4000, POS_ID_NOUN);
        let rule = NumeralCounterRule {
            bonus: 0,
            suffix_bonus: 0,
            ka_penalty: 0,
            address_bonus: 0,
            address_kana_bonus: 0,
            language_bonus: 600,
            ordinal_bonus: 0,
        };
        let rules = EdgeRules::new_with_volitional(
            &g,
            AdverbNounRule::disabled(),
            NounSuruRule::disabled(),
            NounSuffixRule::disabled(),
            NumeralCoverRule::disabled(),
            VolitionalRule::disabled(),
            rule,
        );
        let extra = |key: &str, prev_end: u32, next_cost: i16| -> i32 {
            let lat = build_r(key, &d, &g, &rules);
            let find = |pred: &dyn Fn(&Node) -> bool| {
                *(0..lat.node_count() as u32)
                    .map(|id| lat.node(id))
                    .find(|n| n.word.is_some() && pred(n))
                    .unwrap()
            };
            let prev = find(&|n| n.start == 0 && n.end == prev_end);
            let next = find(&|n| n.start == prev_end && n.cost == next_cost);
            lat.edge_extra(&prev, &next)
        };
        // か国 → 語 は引く。か国 → 後、か月 → 語 は 0
        assert_eq!(extra("かこくご", 3, 5000), -600);
        assert_eq!(extra("かこくご", 3, 4000), 0);
        assert_eq!(extra("かげつご", 3, 5000), 0);
    }

    /// 見た目(1 語)と 見 + た + 目(3 語)。連接 0 で、1 語は 3 語より `gap` 高い
    fn mitame_cost(bonus: i32, gap: i16, surface: &str) -> i32 {
        let g = grammar();
        let mut d = TestDict::new(g.grammar_id());
        d.add("み", "見", ID_NOUN, ID_NOUN, 1000, POS_ID_NOUN);
        d.add("た", "た", ID_NOUN, ID_NOUN, 1000, POS_ID_NOUN);
        d.add("め", "目", ID_NOUN, ID_NOUN, 1000, POS_ID_NOUN);
        d.add("みため", surface, ID_NOUN, ID_NOUN, 3000 + gap, POS_ID_NOUN);
        let r = EdgeRules::none().with_compound_word(&g, CompoundWordRule { bonus });
        let lat = build_r("みため", &d, &g, &r);
        eos_best(&lat, &g).unwrap()
    }

    #[test]
    fn compound_word_bonus_is_off_by_default_and_lowers_kana_mixed_nouns_only() {
        // 既定(bonus 0)は変化なし: 3 語の経路 3000 が最小
        assert_eq!(mitame_cost(0, 500, "見た目"), 3000);
        // bonus を引くと、1 語(3500 - bonus)が勝つ
        assert_eq!(mitame_cost(1000, 500, "見た目"), 2500);
        // 末尾が仮名の表記ゆれ・漢字だけ・仮名だけには当てない
        assert_eq!(mitame_cost(1000, 500, "見ため"), 3000);
        assert_eq!(mitame_cost(1000, 500, "見多目"), 3000);
        assert_eq!(mitame_cost(1000, 500, "みため"), 3000);
    }
}
