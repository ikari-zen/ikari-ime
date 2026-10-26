//! ひらがな／漢字の表記の既定（ADR 0002 §22。文化庁「公用文作成の考え方」2022 の書き表し方）。
//!
//! 辞書のコストは解析用で、「あえて」より「会えて」、「ください」より「下さい」が安い。
//! 変換では、補助動詞、形式名詞、接続詞、一部の副詞はひらがなを、本動詞と実質名詞は漢字を優先したい。
//! 語のリストは `core/assets/kana_style.tsv` に置き、この Ranker が次の3つの規則で score を調整する。
//!
//! - `phrase`: 読みと同じ全ひらがなの候補（接続詞、一部の副詞、複合助詞）に、文脈によらず加点する。
//! - `formal_noun`: 形式名詞は、直前が連体修飾のときだけひらがなに加点する。それ以外は漢字（時、物、所、訳）に加点する。
//! - `aux_verb`: 補助動詞は、直前が「て／で」のときだけひらがなに加点する。それ以外の本動詞は漢字に加点する。
//!
//! 規則は候補のすべての語を見る（`phrase` は候補全体）。
//!
//! 本動詞の漢字への加点（`aux_kanji_bonus`）は表記の選択であって、区切りの選択ではない（ADR 0010）。
//! そこで、各語の読みと品詞1が同じ候補（同じ形）のまとまりの中だけで効かせ、
//! まとまりの最良のスコアは加点の前の値に戻す。区切りの違う候補との比較は、この加点で変わらない。
//!
//! 形式名詞の漢字への加点（`formal_kanji_bonus`）も、次の語が助詞（「の」を除く）でないときは同じ扱い（ADR 0017）。
//! 次が助詞のとき（物が足りない、時を忘れる）は、実質名詞が文節になる区切りを選ぶ加点として、区切りをまたいで効く。
//! 本動詞と形式名詞の漢字の加点は、別々に `max_total` で頭打ちにしてから足す。
//!
//! 常用漢字表にない動物名（兎、鯨、狸 など）は、公用文の書き表し方ではひらがな（またはカタカナ）なので、
//! `animal_kana` の表にある語は、ひらがなの表記（同じ lemma）に加点する（`animal_kana_bonus`。#239）。
//! これも表記の選択なので、同じ形の候補のまとまりの中だけで効かせる。ひらがなは、まとまりの最良のスコアの手前（−1）で止める
//! （まとまりの最良のスコアも、ほかの候補のスコアも変えない）。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use super::common::apply_bonus;
use super::config::ScopedKv;
use super::names;
use super::{RankInput, Ranker, RankerConfigError};
use crate::candidate::{Candidate, CandidateOrigin, Segment};
use crate::pos::pos1;

const BUNDLED_TABLE: &str = include_str!("../../assets/kana_style.tsv");
const COMMENT_PREFIX: char = '#';
const HEADER_PREFIX: &str = "#ikari-kana-style";
const KANJI_SEPARATOR: char = '=';
const CLASS_PHRASE: &str = "phrase";
const CLASS_FORMAL_NOUN: &str = "formal_noun";
const CLASS_AUX_VERB: &str = "aux_verb";
const CLASS_AUX_VERB_TE_ONLY: &str = "aux_verb_te_only";
const CLASS_ANIMAL_KANA: &str = "animal_kana";
const CONFIG_PREFIX: &str = names::KANA_STYLE;
const CONFIG_KEYS: [&str; 7] = [
    "phrase_bonus",
    "formal_bonus",
    "formal_kanji_bonus",
    "aux_bonus",
    "aux_kanji_bonus",
    "animal_kana_bonus",
    "max_total",
];
/// 「て／で」（接続助詞）
const TE_FORMS: [&str; 2] = ["て", "で"];
const POS1_VERB: &str = "動詞";
const POS1_ADJECTIVE: &str = "形容詞";
const POS1_AUXILIARY: &str = "助動詞";
const POS1_ADNOMINAL: &str = "連体詞";
const POS1_PARTICLE: &str = "助詞";
const POS1_NOUN: &str = "名詞";
const POS1_PREFIX: &str = "接頭辞";
/// 形式名詞のすぐあとに続くと、形式名詞として完結していない品詞1
const CONTENT_POS1: [&str; 4] = [POS1_NOUN, POS1_VERB, POS1_ADJECTIVE, POS1_PREFIX];
const POS2_NUMERAL: &str = "数詞";
const POS2_NON_INDEPENDENT: &str = "非自立可能";
const POS_SEPARATOR: char = ',';
const ATTRIBUTIVE_NO: &str = "の";
const CONJ_FORM_FIELD: usize = 5;
const FORM_CONTINUATIVE: &str = "連用形";
const FORM_ATTRIBUTIVE: &str = "連体形";
const FORM_TERMINAL: &str = "終止形";

#[derive(Debug, PartialEq, Eq)]
pub struct KanaStyleParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for KanaStyleParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for KanaStyleParseError {}

/// 語のリスト（`core/assets/kana_style.tsv`）
#[derive(Clone, Debug, Default)]
pub struct KanaStyleTable {
    phrases: HashSet<String>,
    /// ひらがな -> 実質名詞の漢字（なければ None）
    formal_nouns: HashMap<String, Option<String>>,
    /// 補助動詞の lemma（本動詞では漢字を優先するもの）
    aux_verbs: HashSet<String>,
    /// 「て／で」の直後だけひらがなを優先する lemma
    aux_verbs_te_only: HashSet<String>,
    /// ひらがな -> 常用漢字表にない動物名の漢字（lemma。ひらがなを優先する）
    animal_kana: HashMap<String, String>,
}

impl KanaStyleTable {
    pub fn builtin() -> Self {
        // 同梱のファイルは単体テストで検証している
        Self::parse(BUNDLED_TABLE).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Result<Self, KanaStyleParseError> {
        let err = |line: usize, message: &str| KanaStyleParseError {
            line,
            message: message.to_owned(),
        };
        let mut table = Self::default();
        let mut saw_header = false;
        for (i, line) in text.lines().enumerate() {
            let n = i + 1;
            if line.starts_with(HEADER_PREFIX) {
                if !line.contains("schema=1") {
                    return Err(err(n, "unsupported schema (expected schema=1)"));
                }
                saw_header = true;
                continue;
            }
            if line.trim().is_empty() || line.starts_with(COMMENT_PREFIX) {
                continue;
            }
            if !saw_header {
                return Err(err(n, "missing `#ikari-kana-style\tschema=1` header"));
            }
            let mut cols = line.split('\t');
            let class = cols.next().unwrap_or("");
            let items: Vec<&str> = cols.map(str::trim).filter(|s| !s.is_empty()).collect();
            if items.is_empty() {
                return Err(err(n, "no items"));
            }
            match class {
                CLASS_PHRASE => table.phrases.extend(items.iter().map(|s| (*s).to_owned())),
                CLASS_FORMAL_NOUN => {
                    for item in items {
                        let (kana, kanji) = match item.split_once(KANJI_SEPARATOR) {
                            Some((k, j)) if !k.is_empty() && !j.is_empty() => {
                                (k, Some(j.to_owned()))
                            }
                            Some(_) => return Err(err(n, "empty side of `kana=kanji`")),
                            None => (item, None),
                        };
                        table.formal_nouns.insert(kana.to_owned(), kanji);
                    }
                }
                CLASS_AUX_VERB => table
                    .aux_verbs
                    .extend(items.iter().map(|s| (*s).to_owned())),
                CLASS_AUX_VERB_TE_ONLY => table
                    .aux_verbs_te_only
                    .extend(items.iter().map(|s| (*s).to_owned())),
                CLASS_ANIMAL_KANA => {
                    for item in items {
                        match item.split_once(KANJI_SEPARATOR) {
                            Some((k, j)) if !k.is_empty() && !j.is_empty() => {
                                table.animal_kana.insert(k.to_owned(), j.to_owned());
                            }
                            _ => return Err(err(n, "animal_kana needs `kana=kanji`")),
                        }
                    }
                }
                _ => return Err(err(n, "unknown class")),
            }
        }
        if !saw_header {
            return Err(err(1, "missing `#ikari-kana-style\tschema=1` header"));
        }
        Ok(table)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct KanaStyleConfig {
    /// `phrase` のひらがな候補への加点
    pub phrase_bonus: i32,
    /// 連体修飾の直後の形式名詞（ひらがな）への加点
    pub formal_bonus: i32,
    /// 連体修飾の直後でない形式名詞の漢字への加点
    pub formal_kanji_bonus: i32,
    /// 「て／で」の直後の補助動詞（ひらがな）への加点
    pub aux_bonus: i32,
    /// 「て／で」の直後でない本動詞の漢字への加点
    pub aux_kanji_bonus: i32,
    /// 常用漢字表にない動物名のひらがな（同じ lemma）への加点（同じ形の候補の中だけ）
    pub animal_kana_bonus: i32,
    /// 1候補あたりの加点の上限
    pub max_total: i32,
}

impl Default for KanaStyleConfig {
    fn default() -> Self {
        Self {
            phrase_bonus: 4000,
            formal_bonus: 4000,
            formal_kanji_bonus: 3500,
            aux_bonus: 4000,
            aux_kanji_bonus: 3500,
            animal_kana_bonus: 4000,
            max_total: 6000,
        }
    }
}

impl KanaStyleConfig {
    /// `kana_style.phrase_bonus` のように、`kana_style.` を前置したキーだけを読む。
    pub fn from_kv(kv: &BTreeMap<String, String>) -> Result<Self, RankerConfigError> {
        let scope = ScopedKv::new(CONFIG_PREFIX, &CONFIG_KEYS, kv)?;
        let d = Self::default();
        Ok(Self {
            phrase_bonus: scope.get("phrase_bonus", d.phrase_bonus)?,
            formal_bonus: scope.get("formal_bonus", d.formal_bonus)?,
            formal_kanji_bonus: scope.get("formal_kanji_bonus", d.formal_kanji_bonus)?,
            aux_bonus: scope.get("aux_bonus", d.aux_bonus)?,
            aux_kanji_bonus: scope.get("aux_kanji_bonus", d.aux_kanji_bonus)?,
            animal_kana_bonus: scope.get("animal_kana_bonus", d.animal_kana_bonus)?,
            max_total: scope.get("max_total", d.max_total)?,
        })
    }
}

pub struct KanaStyleRanker {
    table: KanaStyleTable,
    cfg: KanaStyleConfig,
}

/// 直前の語（左文脈の最後のトークン）が、次の語にどうつながるか
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Before {
    /// 左文脈がない
    Nothing,
    /// 「て／で」（接続助詞）
    Te,
    /// 動詞、形容詞、助動詞、連体詞、「の」、数詞（連体修飾）
    Modifier,
    /// それ以外（「が」「を」など）
    Other,
}

/// 候補への加点。表記の選択のための加点（本動詞の漢字、次が助詞でない形式名詞の漢字）は、
/// 同じ形の候補のまとまりの中で扱う（ADR 0010、ADR 0017）
#[derive(Clone, Copy, Debug, Default)]
struct Bonus {
    general: i32,
    /// 本動詞の漢字（同じ形の組の中だけ）
    main_verb_kanji: i32,
    /// 次が助詞でない形式名詞の漢字（同じ形の組の中だけ。`main_verb_kanji` とは別に頭打ちにする）
    formal_kanji: i32,
    /// 常用漢字表にない動物名のひらがな（同じ形の組の中だけ。別に頭打ちにする）
    animal_kana: i32,
}

impl Bonus {
    fn add(&mut self, other: Bonus) {
        self.general += other.general;
        self.main_verb_kanji += other.main_verb_kanji;
        self.formal_kanji += other.formal_kanji;
        self.animal_kana += other.animal_kana;
    }
}

/// 候補の形: 各語の（読み, 品詞1）の列
type Shape = Vec<(String, String)>;

impl KanaStyleRanker {
    pub fn new(table: KanaStyleTable, cfg: KanaStyleConfig) -> Self {
        Self { table, cfg }
    }

    /// 同梱の表を使う
    pub fn with_defaults() -> Self {
        Self::new(KanaStyleTable::builtin(), KanaStyleConfig::default())
    }

    /// 左文脈の最後のトークンが、次の語にどうつながるか
    fn before_context(&self, input: &RankInput<'_>) -> Before {
        match input.analysis.tokens.last() {
            None => Before::Nothing,
            Some(last) => classify(input.grammar.pos_name(last.pos_id), &last.surface),
        }
    }

    /// 候補への加点を、(本動詞の漢字以外, 本動詞の漢字) に分けて返す。
    /// 後者は同じ形の候補のまとまりの中でだけ使う（ADR 0010）
    fn bonus(&self, input: &RankInput<'_>, context: Before, cand: &Candidate) -> Bonus {
        if !matches!(
            cand.origin,
            CandidateOrigin::Dictionary | CandidateOrigin::Hiragana
        ) {
            return Bonus::default();
        }
        let cfg = &self.cfg;
        let mut total = Bonus::default();
        // 全体が読みと同じひらがな
        if cand.surface == input.reading && self.table.phrases.contains(input.reading) {
            total.general += cfg.phrase_bonus;
        }
        // 語ごとに、直前の語（先頭は左文脈の最後のトークン）との関係で見る
        let mut before = context;
        for (i, seg) in cand.segments.iter().enumerate() {
            let next = cand.segments.get(i + 1);
            let b = self.segment_bonus(input, before, seg, next);
            total.add(b);
            before = classify(input.grammar.pos_name(seg.pos_id), &seg.surface);
        }
        total
    }

    fn segment_bonus(
        &self,
        input: &RankInput<'_>,
        before: Before,
        seg: &Segment,
        next: Option<&Segment>,
    ) -> Bonus {
        let cfg = &self.cfg;
        let is_kana = seg.surface == seg.reading;
        let mut total = Bonus::default();
        if let Some(kanji) = self.table.formal_nouns.get(seg.reading.as_str()) {
            match (before, is_kana) {
                (Before::Modifier, true) => {
                    // 実質語が続く候補（大きな + もの + 置き）は、形式名詞として完結していない
                    if !self.is_followed_by_content_word(input, next) {
                        total.general += cfg.formal_bonus;
                    }
                }
                // 文頭や助詞の直後は実質名詞として漢字
                (Before::Nothing | Before::Other, false)
                    if kanji.as_deref() == Some(seg.surface.as_str()) =>
                {
                    if self.is_separating_particle(input, next) {
                        // 実質名詞が助詞を伴って文節になる位置: 区切りの選択にも効く
                        total.general += cfg.formal_kanji_bonus;
                    } else {
                        total.formal_kanji += cfg.formal_kanji_bonus;
                    }
                }
                _ => {}
            }
        }
        total.add(self.aux_bonus(input, before, seg, is_kana, next));
        if is_kana && self.is_animal_kana(input, seg) {
            total.animal_kana += cfg.animal_kana_bonus;
        }
        total
    }

    /// 表にある動物名のひらがなで、辞書の lemma が表の漢字か読み自身（Sudachi は かたつむり のように lemma が仮名のものがある）。
    /// 動詞の仮名書き（さる = 去る など）は lemma が合わないので対象にしない
    fn is_animal_kana(&self, input: &RankInput<'_>, seg: &Segment) -> bool {
        let Some(kanji) = self.table.animal_kana.get(seg.reading.as_str()) else {
            return false;
        };
        seg.word
            .and_then(|w| input.dict.lemma(w))
            .is_some_and(|l| l.text == kanji || l.text == seg.reading)
    }

    /// 次の語が名詞、動詞、形容詞、接頭辞（形式名詞は、助詞や助動詞が続くか、末尾で終わるのが普通）
    fn is_followed_by_content_word(&self, input: &RankInput<'_>, next: Option<&Segment>) -> bool {
        next.is_some_and(|n| {
            input
                .grammar
                .pos_name(n.pos_id)
                .is_some_and(|p| CONTENT_POS1.contains(&pos1(p)))
        })
    }

    /// 次の語が助詞で、「の」ではない（「の」は物の怪、時の間のように熟語の中にも現れる）
    fn is_separating_particle(&self, input: &RankInput<'_>, next: Option<&Segment>) -> bool {
        next.is_some_and(|n| {
            n.surface != ATTRIBUTIVE_NO
                && input
                    .grammar
                    .pos_name(n.pos_id)
                    .is_some_and(|p| pos1(p) == POS1_PARTICLE)
        })
    }

    fn aux_bonus(
        &self,
        input: &RankInput<'_>,
        before: Before,
        seg: &Segment,
        is_kana: bool,
        next: Option<&Segment>,
    ) -> Bonus {
        let cfg = &self.cfg;
        let general = |general| Bonus {
            general,
            ..Bonus::default()
        };
        let lemma = match seg.word.and_then(|w| input.dict.lemma(w)) {
            Some(l) => l.text,
            None => return Bonus::default(),
        };
        let listed = self.table.aux_verbs.contains(lemma);
        if !listed && !self.table.aux_verbs_te_only.contains(lemma) {
            return Bonus::default();
        }
        match before {
            Before::Te => {
                let pos = input.grammar.pos_name(seg.pos_id).unwrap_or("");
                let non_independent = pos1(pos) == POS1_VERB && pos.contains(POS2_NON_INDEPENDENT);
                // 連用形の補助動詞（「いる」の「い」など）は、助動詞か「て／で」が続くときだけ。
                // 末尾（「こう+て+い」）や「し」「が」が続く候補は、補助動詞として完結していない
                let complete = !is_continuative(pos)
                    || next.is_some_and(|n| {
                        TE_FORMS.contains(&n.surface.as_str())
                            || input
                                .grammar
                                .pos_name(n.pos_id)
                                .is_some_and(|p| pos1(p) == POS1_AUXILIARY)
                    });
                if is_kana && non_independent && complete {
                    general(cfg.aux_bonus)
                } else {
                    Bonus::default()
                }
            }
            // 直前が「て／で」でない: 本動詞なので漢字
            Before::Modifier | Before::Other if listed && !is_kana => Bonus {
                main_verb_kanji: cfg.aux_kanji_bonus,
                ..Bonus::default()
            },
            _ => Bonus::default(),
        }
    }
}

/// 品詞名の6列目（活用形）
fn conjugation_form(pos: &str) -> &str {
    pos.split(POS_SEPARATOR).nth(CONJ_FORM_FIELD).unwrap_or("")
}

fn is_continuative(pos: &str) -> bool {
    conjugation_form(pos).starts_with(FORM_CONTINUATIVE)
}

/// 直前の語の品詞と表記から、次の語へのつながりを分類する。
/// 連体修飾になるのは、終止形・連体形（連用形や未然形ではない）の動詞、形容詞、助動詞
fn classify(pos_name: Option<&str>, surface: &str) -> Before {
    let pos = pos_name.unwrap_or("");
    let mut fields = pos.split(POS_SEPARATOR);
    let (p1, p2) = (fields.next().unwrap_or(""), fields.next().unwrap_or(""));
    if p1 == POS1_PARTICLE && TE_FORMS.contains(&surface) {
        return Before::Te;
    }
    let form = conjugation_form(pos);
    let attributive_form = form.starts_with(FORM_ATTRIBUTIVE) || form.starts_with(FORM_TERMINAL);
    let modifier = (matches!(p1, POS1_VERB | POS1_ADJECTIVE | POS1_AUXILIARY) && attributive_form)
        || p1 == POS1_ADNOMINAL
        || (p1 == POS1_PARTICLE && surface == ATTRIBUTIVE_NO)
        || (p1 == POS1_NOUN && p2 == POS2_NUMERAL);
    if modifier {
        Before::Modifier
    } else {
        Before::Other
    }
}

impl KanaStyleRanker {
    /// 表記の選択のための漢字の加点（本動詞、次が助詞でない形式名詞）を、同じ形の候補のまとまりの中だけで効かせる（ADR 0010 §2.1）。
    /// まとまりの中の順位と差は「加点して比べる」と同じで、まとまりの最良のスコアは加点の前の値に保つ
    fn apply_shape_local(&self, input: &RankInput<'_>, cands: &mut [Candidate], kanji: &[i32]) {
        if kanji.iter().all(|&k| k == 0) {
            return;
        }
        let mut groups: HashMap<Shape, Vec<usize>> = HashMap::new();
        for (i, cand) in cands.iter().enumerate() {
            if cand.origin == CandidateOrigin::Dictionary {
                groups.entry(shape(input, cand)).or_default().push(i);
            }
        }
        for members in groups.values() {
            if members.iter().all(|&i| kanji[i] == 0) {
                continue;
            }
            let best = members.iter().map(|&i| cands[i].score).min().unwrap_or(0);
            let best_adjusted = members
                .iter()
                .map(|&i| cands[i].score.saturating_sub(kanji[i]))
                .min()
                .unwrap_or(0);
            let shift = best.saturating_sub(best_adjusted);
            for &i in members {
                let adjusted = cands[i].score.saturating_sub(kanji[i]);
                cands[i].score = adjusted.saturating_add(shift);
            }
        }
    }
}

impl KanaStyleRanker {
    /// 常用漢字表にない動物名のひらがなを、同じ形の候補のまとまりの中で先頭に出す。
    /// 加点した値がまとまりの最良のスコアより良くなるときは、最良のスコアの直前（−1）で止める。
    /// まとまりの外の候補との順位は変えない（ほかの漢字の候補を押し下げもしない）
    fn apply_animal_kana(&self, input: &RankInput<'_>, cands: &mut [Candidate], animal: &[i32]) {
        if animal.iter().all(|&a| a == 0) {
            return;
        }
        let mut groups: HashMap<Shape, Vec<usize>> = HashMap::new();
        for (i, cand) in cands.iter().enumerate() {
            if cand.origin == CandidateOrigin::Dictionary {
                groups.entry(shape(input, cand)).or_default().push(i);
            }
        }
        for members in groups.values() {
            let Some(best) = members.iter().map(|&i| cands[i].score).min() else {
                continue;
            };
            let floor = best.saturating_sub(1);
            for &i in members {
                if animal[i] > 0 {
                    let boosted = cands[i].score.saturating_sub(animal[i]).max(floor);
                    cands[i].score = cands[i].score.min(boosted);
                }
            }
        }
    }
}

fn shape(input: &RankInput<'_>, cand: &Candidate) -> Shape {
    cand.segments
        .iter()
        .map(|seg| {
            let pos = input.grammar.pos_name(seg.pos_id).unwrap_or("");
            (seg.reading.clone(), pos1(pos).to_owned())
        })
        .collect()
}

impl Ranker for KanaStyleRanker {
    fn rank(&self, input: &RankInput<'_>, cands: &mut [Candidate]) {
        if input.context.secure {
            return;
        }
        let before = self.before_context(input);
        let mut kanji_bonus = vec![0; cands.len()];
        let mut animal = Vec::with_capacity(cands.len());
        let cap = self.cfg.max_total;
        for (cand, kanji) in cands.iter_mut().zip(kanji_bonus.iter_mut()) {
            let bonus = self.bonus(input, before, cand);
            let general = bonus.general.min(self.cfg.max_total);
            if general > 0 {
                apply_bonus(cand, general);
            }
            // 本動詞と形式名詞は別々に頭打ちにして足す（ADR 0017 §2.1）
            *kanji = bonus.main_verb_kanji.min(cap) + bonus.formal_kanji.min(cap);
            animal.push(bonus.animal_kana.min(cap));
        }
        self.apply_shape_local(input, cands, &kanji_bonus);
        self.apply_animal_kana(input, cands, &animal);
    }

    fn name(&self) -> &str {
        names::KANA_STYLE
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::analysis::AnalyzedContext;
    use crate::context::Context;
    use crate::dict::WordId;

    const POS_VERB_AUX_END: &str = "動詞,非自立可能,*,*,上一段-マ行,終止形-一般";
    const POS_VERB_AUX_CONT: &str = "動詞,非自立可能,*,*,上一段-マ行,連用形-一般";
    const POS_VERB_END: &str = "動詞,一般,*,*,五段-カ行,終止形-一般";
    const POS_CONJ_PARTICLE: &str = "助詞,接続助詞,*,*,*,*";
    const POS_CASE_PARTICLE: &str = "助詞,格助詞,*,*,*,*";
    const POS_AUXILIARY_TA: &str = "助動詞,*,*,*,助動詞-タ,終止形-一般";

    struct Fx {
        env: Env,
        verb_aux_end: u16,
        verb_aux_cont: u16,
        verb_end: u16,
        conj_particle: u16,
        case_particle: u16,
        aux_pos: u16,
    }

    fn fx() -> Fx {
        let mut env = Env::new();
        let verb_aux_end = env.grammar.add_pos(POS_VERB_AUX_END);
        let verb_aux_cont = env.grammar.add_pos(POS_VERB_AUX_CONT);
        let verb_end = env.grammar.add_pos(POS_VERB_END);
        let conj_particle = env.grammar.add_pos(POS_CONJ_PARTICLE);
        let case_particle = env.grammar.add_pos(POS_CASE_PARTICLE);
        let aux_pos = env.grammar.add_pos(POS_AUXILIARY_TA);
        Fx {
            env,
            verb_aux_end,
            verb_aux_cont,
            verb_end,
            conj_particle,
            case_particle,
            aux_pos,
        }
    }

    impl Fx {
        /// 辞書に語を足して、読みつきのセグメントにする
        fn seg(&mut self, reading: &str, surface: &str, lemma: &str, pos: u16) -> Segment {
            let index = self.env.dict.add(reading, surface, 1, 1, 1000, pos);
            self.env.dict.set_lemma(index, lemma, None);
            Segment {
                reading: reading.to_owned(),
                surface: surface.to_owned(),
                word: Some(WordId { dict: 0, index }),
                left_id: 1,
                right_id: 1,
                pos_id: pos,
            }
        }

        fn rank(&self, last: Option<(&str, u16)>, reading: &str, cands: &mut [Candidate]) {
            let tokens = last
                .map(|(surface, pos)| {
                    let mut t = token(surface, surface, None, false, 0);
                    t.pos_id = pos;
                    vec![t]
                })
                .unwrap_or_default();
            let analysis: AnalyzedContext = analysis(tokens);
            let input = RankInput {
                reading,
                context: &Context::new(),
                grammar: &self.env.grammar,
                analysis: &analysis,
                dict: &self.env.dict,
                deadline: None,
            };
            KanaStyleRanker::with_defaults().rank(&input, cands);
        }
    }

    fn scores(c: &[Candidate]) -> Vec<i32> {
        c.iter().map(|c| c.score).collect()
    }

    #[test]
    fn bundled_table_parses_and_has_every_class() {
        let t = KanaStyleTable::parse(BUNDLED_TABLE).expect("bundled table");
        assert!(t.phrases.contains("あえて") && t.phrases.contains("ただし"));
        assert_eq!(t.formal_nouns.get("とき"), Some(&Some("時".to_owned())));
        assert_eq!(t.formal_nouns.get("こと"), Some(&None));
        assert!(t.aux_verbs.contains("下さる") && t.aux_verbs.contains("頂く"));
        assert!(t.aux_verbs_te_only.contains("居る"));
        // フェーズ2 Q: 「時が経つ」（本動詞の 経つ は漢字）と、複合助詞
        assert!(t.aux_verbs.contains("経つ"));
        for p in ["において", "によると", "によれば", "にとって"] {
            assert!(t.phrases.contains(p), "{p}");
        }
    }

    #[test]
    fn animal_kana_table_has_expression_outside_joyo_only() {
        let t = KanaStyleTable::parse(BUNDLED_TABLE).expect("bundled table");
        assert_eq!(t.animal_kana.get("うさぎ"), Some(&"兎".to_owned()));
        // 常用漢字表にある動物名と、同音の別語が多い読みは載せない
        for kana in ["いぬ", "とり", "うま", "さかな", "かえる", "さる", "はち"] {
            assert!(!t.animal_kana.contains_key(kana), "{kana}");
        }
        assert!(
            KanaStyleTable::parse("#ikari-kana-style\tschema=1\nanimal_kana\tうさぎ\n").is_err()
        );
    }

    #[test]
    fn animal_kana_goes_first_only_within_the_same_shape_and_keeps_the_group_best() {
        let mut f = fx();
        let noun = f.env.grammar.add_pos("名詞,普通名詞,一般,*,*,*");
        let kana = f.seg("うさぎ", "うさぎ", "兎", noun);
        let kanji = f.seg("うさぎ", "兎", "兎", noun);
        let katakana = f.seg("うさぎ", "ウサギ", "兎", noun);
        let other = f.seg("うさぎ", "う詐欺", "詐欺", f.verb_end);
        let mut cands = vec![
            cand(vec![kanji], 8000),
            cand(vec![kana], 10000),
            cand(vec![katakana], 11000),
            cand(vec![other], 9000),
        ];
        f.rank(None, "うさぎ", &mut cands);
        // ひらがなは同じ形の組の最良（8000）の手前で止まる。ほかの候補の score は変えない
        assert_eq!(scores(&cands), vec![8000, 7999, 11000, 9000]);
    }

    #[test]
    fn animal_kana_ignores_a_homophone_with_another_lemma() {
        let mut f = fx();
        let noun = f.env.grammar.add_pos("名詞,普通名詞,一般,*,*,*");
        // 表にない読み、または lemma が違う仮名書きには効かない
        let other = f.seg("うさぎ", "うさぎ", "別の語", noun);
        let unlisted = f.seg("いぬ", "いぬ", "犬", noun);
        let mut cands = vec![cand(vec![other], 9000)];
        f.rank(None, "うさぎ", &mut cands);
        assert_eq!(scores(&cands), vec![9000]);
        let mut cands = vec![cand(vec![unlisted], 9000)];
        f.rank(None, "いぬ", &mut cands);
        assert_eq!(scores(&cands), vec![9000]);
    }

    #[test]
    fn parse_rejects_bad_tables() {
        for bad in [
            "phrase\tあ\n",                                      // ヘッダがない
            "#ikari-kana-style\tschema=2\nphrase\tあ\n",         // 知らない版
            "#ikari-kana-style\tschema=1\nunknown\tあ\n",        // 知らない種類
            "#ikari-kana-style\tschema=1\nphrase\n",             // 項目がない
            "#ikari-kana-style\tschema=1\nformal_noun\tとき=\n", // 漢字が空
        ] {
            assert!(KanaStyleTable::parse(bad).is_err(), "{bad:?}");
        }
        assert!(KanaStyleTable::parse("").is_err());
    }

    #[test]
    fn phrase_boosts_the_whole_kana_candidate_regardless_of_context() {
        let mut f = fx();
        let kana = f.seg("あえて", "あえて", "敢えて", 0);
        let kanji = f.seg("あえ", "会え", "会う", f.verb_end);
        let mut cands = vec![cand(vec![kana], 7000), cand(vec![kanji], 2900)];
        cands[0].surface = "あえて".to_owned();
        cands[1].surface = "会えて".to_owned();
        f.rank(Some(("を", f.case_particle)), "あえて", &mut cands);
        assert_eq!(scores(&cands), vec![7000 - 4000, 2900]);
    }

    #[test]
    fn aux_verb_is_kana_after_te_and_kanji_otherwise() {
        let mut f = fx();
        let kana = f.seg("みる", "みる", "見る", f.verb_aux_end);
        let kanji = f.seg("みる", "見る", "見る", f.verb_aux_end);
        let make = || {
            vec![
                cand(vec![kana.clone()], 5000),
                cand(vec![kanji.clone()], 5000),
            ]
        };
        // 直前が「て」: ひらがなに加点
        let mut c = make();
        f.rank(Some(("て", f.conj_particle)), "みる", &mut c);
        assert_eq!(scores(&c), vec![5000 - 4000, 5000]);
        // 直前が「を」: 本動詞なので漢字に加点
        let mut c = make();
        f.rank(Some(("を", f.case_particle)), "みる", &mut c);
        // 同じ形の組なので、最良（漢字）は加点の前のまま、ひらがなとの差が 3500 開く
        assert_eq!(scores(&c), vec![5000 + 3500, 5000]);
        // 左文脈がなければ何もしない
        let mut c = make();
        f.rank(None, "みる", &mut c);
        assert_eq!(scores(&c), vec![5000, 5000]);
    }

    #[test]
    fn aux_verb_left_unfinished_in_continuative_form_gets_nothing() {
        let mut f = fx();
        // 「こう + て + い」: 「い」が連用形で終わる候補は補助動詞として完結していない
        let ko = f.seg("こう", "こう", "請う", f.verb_end);
        let te = f.seg("て", "て", "て", f.conj_particle);
        let i = f.seg("い", "い", "居る", f.verb_aux_cont);
        let mut c = vec![cand(vec![ko.clone(), te.clone(), i.clone()], 4000)];
        f.rank(None, "こうてい", &mut c);
        assert_eq!(scores(&c), vec![4000]);
        // 助動詞（い + た）か「て／で」が続くなら、補助動詞として扱う
        let ta = f.seg("た", "た", "た", f.aux_pos);
        let mut c = vec![cand(vec![te.clone(), i.clone(), ta], 4000)];
        f.rank(None, "ていた", &mut c);
        assert_eq!(scores(&c), vec![4000 - 4000]);
        // 接続助詞「し」が続く候補（「晴れて + い + し」）には加点しない
        let shi = f.seg("し", "し", "し", f.conj_particle);
        let mut c = vec![cand(vec![te, i, shi], 4000)];
        f.rank(None, "ていし", &mut c);
        assert_eq!(scores(&c), vec![4000]);
    }

    #[test]
    fn formal_noun_is_kana_after_a_modifier_and_kanji_elsewhere() {
        let mut f = fx();
        let kana = f.seg("とき", "とき", "時", 0);
        let kanji = f.seg("とき", "時", "時", 0);
        let make = || {
            vec![
                cand(vec![kana.clone()], 5000),
                cand(vec![kanji.clone()], 5000),
            ]
        };
        // 動詞の終止形・連体形の直後: ひらがな
        let mut c = make();
        f.rank(Some(("困る", f.verb_end)), "とき", &mut c);
        assert_eq!(scores(&c), vec![5000 - 4000, 5000]);
        // 「が」の直後や文頭: 実質名詞の漢字
        let mut c = make();
        f.rank(Some(("が", f.case_particle)), "とき", &mut c);
        // 候補の末尾の 時 は同じ形の組の中だけ（ADR 0017）。最良（漢字）は加点の前のまま、ひらがなとの差が 3500
        assert_eq!(scores(&c), vec![5000 + 3500, 5000]);
        // 動詞の連用形の直後は連体修飾ではない（ひらがなには加点しない。実質名詞の漢字になる）
        let mut c = make();
        f.rank(Some(("買い", f.verb_aux_cont)), "とき", &mut c);
        assert_eq!(scores(&c), vec![5000 + 3500, 5000]);
    }

    const POS_NOUN: &str = "名詞,普通名詞,一般,*,*,*";

    /// 連体修飾のあとの もの に実質語が続く候補（大きな + もの + 置き）には、ひらがなの加点をしない。
    /// 続くのが助詞や末尾なら、これまでどおり加点する（困った + とき + は）
    #[test]
    fn formal_noun_kana_bonus_skips_when_a_content_word_follows() {
        let mut f = fx();
        let noun = f.env.grammar.add_pos(POS_NOUN);
        let mono = f.seg("もの", "もの", "物", noun);
        let oki = f.seg("おき", "置き", "置く", f.verb_aux_cont);
        let wa = f.seg("は", "は", "は", f.case_particle);
        let mut c = vec![cand(vec![mono.clone(), oki], 9000)];
        f.rank(Some(("大きな", f.verb_end)), "ものおき", &mut c);
        assert_eq!(scores(&c), vec![9000]);
        let mut c = vec![cand(vec![mono.clone(), wa], 9000)];
        f.rank(Some(("大きな", f.verb_end)), "ものは", &mut c);
        assert_eq!(scores(&c), vec![9000 - 4000]);
        let mut c = vec![cand(vec![mono], 9000)];
        f.rank(Some(("大きな", f.verb_end)), "もの", &mut c);
        assert_eq!(scores(&c), vec![9000 - 4000]);
    }

    /// 区切りの違う候補を、本動詞の漢字の加点が追い越さない（もう行っ海 と もう一階）
    #[test]
    fn main_verb_kanji_bonus_does_not_overtake_a_different_shape() {
        let mut f = fx();
        let noun = f.env.grammar.add_pos(POS_NOUN);
        let go = f.seg("いっ", "行っ", "行く", f.verb_aux_cont);
        let sea = f.seg("かい", "海", "海", noun);
        let floor = f.seg("いっかい", "一階", "一階", noun);
        let mut c = vec![cand(vec![go, sea], 6000), cand(vec![floor], 5000)];
        f.rank(Some(("を", f.case_particle)), "いっかい", &mut c);
        // 加点しても形が違うので、どちらも元のスコアのまま
        assert_eq!(scores(&c), vec![6000, 5000]);
    }

    /// 同じ形の中では今までどおり漢字が勝ち、行った と 言った の差も加点どおり
    #[test]
    fn main_verb_kanji_bonus_still_decides_within_the_same_shape() {
        let mut f = fx();
        let kana = f.seg("おく", "おく", "置く", f.verb_aux_end);
        let kanji = f.seg("おく", "置く", "置く", f.verb_aux_end);
        let mut c = vec![cand(vec![kana], 5000), cand(vec![kanji], 5000)];
        f.rank(Some(("を", f.case_particle)), "おく", &mut c);
        assert_eq!(scores(&c), vec![8500, 5000]);

        // 行った（動詞,非自立可能）と 言った（動詞,一般）は同じ形: 行った が 言った に勝つ差は今と同じ
        let went = f.seg("いった", "行った", "行く", f.verb_aux_end);
        let said = f.seg("いった", "言った", "言う", f.verb_end);
        let mut c = vec![cand(vec![said], 5000), cand(vec![went], 5200)];
        f.rank(Some(("を", f.case_particle)), "いった", &mut c);
        assert_eq!(scores(&c), vec![5000 + 3300, 5000]);
    }

    /// 組が1件だけなら、加点は効かない
    #[test]
    fn main_verb_kanji_bonus_needs_a_rival_of_the_same_shape() {
        let mut f = fx();
        let kanji = f.seg("きて", "来て", "来る", f.verb_aux_end);
        let mut c = vec![cand(vec![kanji], 5000)];
        f.rank(Some(("を", f.case_particle)), "きて", &mut c);
        assert_eq!(scores(&c), vec![5000]);
    }

    const POS_SUFFIX: &str = "接尾辞,名詞的,一般,*,*,*";

    struct NounFx {
        f: Fx,
        noun: u16,
        suffix: u16,
    }

    fn noun_fx() -> NounFx {
        let mut f = fx();
        let noun = f.env.grammar.add_pos(POS_NOUN);
        let suffix = f.env.grammar.add_pos(POS_SUFFIX);
        NounFx { f, noun, suffix }
    }

    /// ADR 0017: 物 + 後（接尾辞）+ と が、1語の ものごと を加点で追い越さない
    #[test]
    fn formal_kanji_bonus_does_not_overtake_when_a_suffix_follows() {
        let mut n = noun_fx();
        let mono = n.f.seg("もの", "物", "物", n.noun);
        let go = n.f.seg("ご", "後", "後", n.suffix);
        let to = n.f.seg("と", "と", "と", n.f.case_particle);
        let whole = n.f.seg("ものごと", "ものごと", "ものごと", n.noun);
        let mut c = vec![cand(vec![mono, go, to], 9000), cand(vec![whole], 7000)];
        n.f.rank(Some(("を", n.f.case_particle)), "ものごと", &mut c);
        assert_eq!(scores(&c), vec![9000, 7000]);
    }

    /// 次が名詞・動詞・形容詞のときも、区切りの違う候補を追い越さない
    #[test]
    fn formal_kanji_bonus_stays_local_before_noun_or_verb() {
        let mut n = noun_fx();
        let hoshi = n.f.seg("ほし", "星", "星", n.noun);
        let mono = n.f.seg("もの", "物", "物", n.noun);
        let tokuyo = n.f.seg("ものほし", "物干し", "物干し", n.noun);
        let mut c = vec![cand(vec![mono, hoshi], 9000), cand(vec![tokuyo], 8000)];
        n.f.rank(None, "ものほし", &mut c);
        assert_eq!(scores(&c), vec![9000, 8000]);
        // 次が動詞（時 + 聞かせる）
        let toki = n.f.seg("とき", "時", "時", n.noun);
        let kikaseru = n.f.seg("きかせる", "聞かせる", "聞かせる", n.f.verb_end);
        let whole =
            n.f.seg("ときぎかせる", "説き聞かせる", "説き聞かせる", n.f.verb_end);
        let mut c = vec![cand(vec![toki, kikaseru], 9000), cand(vec![whole], 8000)];
        n.f.rank(None, "ときぎかせる", &mut c);
        assert_eq!(scores(&c), vec![9000, 8000]);
    }

    /// 次が助詞（「の」を除く）なら、今までどおり区切りをまたいで効く（物が足りない）
    #[test]
    fn formal_kanji_bonus_crosses_shapes_before_a_particle() {
        let mut n = noun_fx();
        let mono = n.f.seg("もの", "物", "物", n.noun);
        let ga = n.f.seg("が", "が", "が", n.f.case_particle);
        let tari = n.f.seg("たりない", "足りない", "足りない", n.f.verb_end);
        let story = n.f.seg("ものがたり", "物語", "物語", n.noun);
        let nai = n.f.seg("ない", "ない", "ない", n.f.aux_pos);
        let mut c = vec![
            cand(vec![mono, ga, tari], 9000),
            cand(vec![story, nai], 7000),
        ];
        n.f.rank(Some(("を", n.f.case_particle)), "ものがたりない", &mut c);
        assert_eq!(scores(&c), vec![9000 - 3500, 7000]);
    }

    /// 「の」は助詞に含めない（物の怪 と 物の毛）
    #[test]
    fn formal_kanji_bonus_treats_no_as_not_a_particle() {
        let mut n = noun_fx();
        let no_pos = n.f.env.grammar.add_pos("助詞,格助詞,*,*,*,*");
        let mono = n.f.seg("もの", "物", "物", n.noun);
        let no = n.f.seg("の", "の", "の", no_pos);
        let ke = n.f.seg("け", "毛", "毛", n.noun);
        let whole = n.f.seg("もののけ", "物の怪", "物の怪", n.noun);
        let mut c = vec![cand(vec![mono, no, ke], 8000), cand(vec![whole], 7500)];
        n.f.rank(Some(("を", n.f.case_particle)), "もののけ", &mut c);
        assert_eq!(scores(&c), vec![8000, 7500]);
    }

    /// 同じ形の中（候補の末尾の 物 と もの）では、今と同じ差で漢字が勝つ
    #[test]
    fn formal_kanji_bonus_still_decides_within_the_same_shape() {
        let mut n = noun_fx();
        let kana = n.f.seg("もの", "もの", "物", n.noun);
        let kanji = n.f.seg("もの", "物", "物", n.noun);
        let mut c = vec![cand(vec![kana], 5000), cand(vec![kanji], 5000)];
        n.f.rank(Some(("を", n.f.case_particle)), "もの", &mut c);
        assert_eq!(scores(&c), vec![8500, 5000]);
    }

    /// 本動詞の漢字と形式名詞の漢字は、別々に `max_total` で頭打ちにして足す（時 + 経つ で 7000）
    #[test]
    fn verb_and_formal_kanji_bonuses_are_capped_separately() {
        let mut n = noun_fx();
        let toki_kana = n.f.seg("とき", "とき", "時", n.noun);
        let tatsu_kana = n.f.seg("たつ", "たつ", "たつ", n.f.verb_end);
        let toki = n.f.seg("とき", "時", "時", n.noun);
        let tatsu = n.f.seg("たつ", "経つ", "経つ", n.f.verb_end);
        let mut c = vec![
            cand(vec![toki_kana, tatsu_kana], 5000),
            cand(vec![toki, tatsu], 5000),
        ];
        n.f.rank(Some(("を", n.f.case_particle)), "ときたつ", &mut c);
        // 同じ形の組の中で 3500 + 3500 = 7000 の差（1つの枠なら 6000 に削られる）
        assert_eq!(scores(&c), vec![5000 + 7000, 5000]);
    }

    #[test]
    fn secure_and_other_origins_are_left_alone() {
        let mut f = fx();
        let kana = f.seg("あえて", "あえて", "敢えて", 0);
        let mut c = cand(vec![kana], 7000);
        c.surface = "あえて".to_owned();
        let mut secure = vec![c.clone()];
        let analysis = analysis(Vec::new());
        let secure_ctx = Context::new().with_secure(true);
        let input = RankInput {
            reading: "あえて",
            context: &secure_ctx,
            grammar: &f.env.grammar,
            analysis: &analysis,
            dict: &f.env.dict,
            deadline: None,
        };
        KanaStyleRanker::with_defaults().rank(&input, &mut secure);
        assert_eq!(scores(&secure), vec![7000]);
        // Rewriter の候補は対象外
        c.origin = CandidateOrigin::Rewriter {
            name: "x",
            format: 1,
        };
        let mut rw = vec![c];
        f.rank(None, "あえて", &mut rw);
        assert_eq!(scores(&rw), vec![7000]);
    }

    #[test]
    fn config_reads_only_its_own_keys() {
        let kv: BTreeMap<String, String> =
            [("kana_style.aux_bonus", "1234"), ("topic.weight", "1")]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect();
        let cfg = KanaStyleConfig::from_kv(&kv).unwrap();
        assert_eq!(cfg.aux_bonus, 1234);
        assert_eq!(cfg.phrase_bonus, KanaStyleConfig::default().phrase_bonus);
        let bad: BTreeMap<String, String> = [("kana_style.nope".to_owned(), "1".to_owned())]
            .into_iter()
            .collect();
        assert!(KanaStyleConfig::from_kv(&bad).is_err());
    }
}
