//! 左文脈との連接の重み付け（ADR 0002 §0.3, §6）と、左文脈の語との複合語の加点（ADR 0007）。
//!
//! 変換の探索（`lattice::build`）は、BOS の右ID（左文脈の最後の語）から最初の語への連接コストを
//! 他の連接と同じ重さで足す。ところが辞書のコストは解析用で、同音異義語を比べる用途には粗い
//! （例: 単語コストは 使用 4672 に対して「しよう」の意志形 7517）。左文脈は実際に書かれた語なので、
//! 文法の手がかりとして他の連接より信頼できる。この Ranker は、最初の語の連接コストに
//! `weight` 倍の差分を足して、文法に合う候補（と → しよう）を押し上げる。
//!
//! 増幅するのは次の場合だけにしている（辞書の dev 分割で確かめた。`eval/reports/phase1-*.md`）。
//! - 名詞で始まらない候補だけ: 文法（左文脈）が決めるのは、動詞、助動詞、副詞などの品詞。
//!   名詞どうし（同音異義語）は、左文脈との連接の細かな差が乱れの元になる。
//! - 候補が1つの辞書の語でできている: 複数に分かれた候補は、短い拘束形（き、来）が
//!   連接表で有利なだけで上位に来ることが多い。
//! - 連接コストの絶対値が `threshold` を超えた分だけ: 名詞どうしの細かな差を増やさない。
//!
//! 複合語の加点（ADR 0007 §3）: 左文脈の最後の語（名詞）と 1語の候補をつないだ表記が、辞書の 1語
//! （普通名詞か地名。ユーザー辞書の語は人名以外の固有名詞も）で、読みもつないだものと一致するとき
//! （洗濯 + き → 洗濯機、自動 + か → 自動化）、その候補の差分を上限（`-max_total`）まで引き上げてから
//! `compound_bonus` を足す。接尾辞（器、家）は上の増幅で上限が付き、名詞（機、化）は付かないという、
//! 品詞の分け方だけによる差を、複合語が当たった候補について消すため。当たった複合語が複数あれば、
//! 変換のコスト（cost + penalty）がいちばん小さい複合語と同じ lemma の候補だけに加点する（年 + かん → 間）。

use std::borrow::Cow;
use std::collections::BTreeMap;

use super::config::ScopedKv;
use super::names;
use super::{RankInput, Ranker, RankerConfigError};
use crate::candidate::{Candidate, CandidateOrigin, Segment};
use crate::dict::{Dictionary, Grammar, IME_EXCLUDED, SYSTEM_SLOT, WordEntry, WordId};
use crate::pos::pos1;

const POS1_NOUN: &str = "名詞";
const POS1_SUFFIX: &str = "接尾辞";
const POS2_NOUN_LIKE_SUFFIX: &str = "名詞的";
/// 複合語の左の語に数える形状詞（ADR 0029 §2。最適、簡単、重要 など）。品詞1,品詞2 の前方一致。
/// 形状詞,助動詞語幹（そう、よう、みたい）は入れない
const COMPOUND_LEFT_ADJECTIVAL_PREFIXES: [&str; 2] = ["形状詞,一般", "形状詞,タリ"];
/// 複合語として数える品詞（ADR 0007 §3.1 の条件 4。§12.1 Q2 で地名を含め、人名は含めない）
const COMPOUND_POS_PREFIXES: [&str; 2] = ["名詞,普通名詞,", "名詞,固有名詞,地名,"];
/// ユーザー辞書の語だけ、人名を除く固有名詞（簡略品詞の「固有名詞」「組織」）も数える（§12.1 Q4）
const USER_COMPOUND_POS_PREFIX: &str = "名詞,固有名詞,";
const PERSON_NAME_POS_PREFIX: &str = "名詞,固有名詞,人名,";

const CONFIG_PREFIX: &str = names::LEFT_CONTEXT;
const CONFIG_KEYS: [&str; 5] = [
    "weight",
    "threshold",
    "max_total",
    "compound_bonus",
    "numeral_counter",
];

/// 数詞の品詞2（ADR 0019 §2.2 の規則 C）
const POS2_NUMERAL: &str = "数詞";
/// 助数詞の品詞（名詞,普通名詞,助数詞可能 と 接尾辞,名詞的,助数詞）の品詞3 の前方一致
const POS3_COUNTER_PREFIX: &str = "助数詞";
const POS_FIELD_SEPARATOR: char = ',';
/// `numeral_counter` の値: 0 で無効、それ以外で有効
const NUMERAL_COUNTER_OFF: u8 = 0;
const NUMERAL_COUNTER_ON: u8 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct LeftContextConfig {
    /// 探索で足された連接コストに、さらに足す倍率（1.0 なら合計で 2 倍の重さ）
    pub weight: f32,
    /// 連接コストの絶対値のうち、この値までは増幅しない（名詞どうしの細かな差を増やさないため）
    pub threshold: i32,
    /// 1候補あたりの連接の差分の絶対値の上限
    pub max_total: i32,
    /// 左文脈の最後の語との複合語が辞書にある候補に、上限の差分に加えて足す加点（ADR 0007 §3.3）。0 で無効
    pub compound_bonus: i32,
    /// 左文脈の最後の語が数詞のとき、助数詞の名詞（時、分、円）も接尾辞（次、墳、園）と同じく増幅する
    /// （ADR 0019 §2.2 の規則 C）。false で無効
    pub numeral_counter: bool,
}

impl Default for LeftContextConfig {
    fn default() -> Self {
        Self {
            weight: 4.0,
            threshold: 1000,
            max_total: 6000,
            compound_bonus: 3000,
            numeral_counter: true,
        }
    }
}

impl LeftContextConfig {
    /// `left_context.weight` のように、`left_context.` を前置したキーだけを読む。
    pub fn from_kv(kv: &BTreeMap<String, String>) -> Result<Self, RankerConfigError> {
        let scope = ScopedKv::new(CONFIG_PREFIX, &CONFIG_KEYS, kv)?;
        let d = Self::default();
        Ok(Self {
            weight: scope.get_float("weight", d.weight)?,
            threshold: scope.get("threshold", d.threshold)?,
            max_total: scope.get("max_total", d.max_total)?,
            compound_bonus: scope.get("compound_bonus", d.compound_bonus)?,
            numeral_counter: scope.get(
                "numeral_counter",
                if d.numeral_counter {
                    NUMERAL_COUNTER_ON
                } else {
                    NUMERAL_COUNTER_OFF
                },
            )? != NUMERAL_COUNTER_OFF,
        })
    }
}

pub struct LeftContextRanker {
    cfg: LeftContextConfig,
}

/// 当たった複合語（ADR 0007 §3.2）
struct Compound {
    /// 変換で使うコスト（cost + penalty）
    cost: i32,
    lemma: String,
}

impl LeftContextRanker {
    pub fn new(cfg: LeftContextConfig) -> Self {
        Self { cfg }
    }

    /// 左文脈の最後の語から、1語の候補 `first` への連接の差分（ADR 0002 §22）。名詞の候補と接続禁止は 0。
    fn connection_delta(&self, input: &RankInput<'_>, first: &Segment) -> i32 {
        // 名詞で始まる候補は増幅しない。希少な名詞（聖核、製革）が左文脈の語との連接で偶然有利になり、
        // 同音異義語の名詞どうしの比較を乱すため。左文脈が文法で決めるのは、動詞、助動詞、副詞などの品詞。
        // 例外（規則 C）: 左文脈の最後の語が数詞なら、助数詞の名詞（時、分、円）も増幅する。
        // 助数詞が接尾辞（次、墳、園）か名詞かという品詞の違いだけで 6000 の差が付くのを消すため
        if is_noun(input.grammar, first.pos_id) && !self.is_counter_after_numeral(input, first) {
            return 0;
        }
        // 名詞的な接尾辞（海、界、家、化）も名詞と同じく内容を担う語で、同音の名詞（解、会、か）との比較は
        // 文法では決まらない。増幅しない（ADR 0029 §3）。助数詞の接尾辞（歳、次）は文法で決まるので残す。
        // 名詞に付く正しい接尾辞は、複合語の加点（ADR 0007）が支える
        if is_noun_like_suffix(input.grammar, first.pos_id) {
            return 0;
        }
        let conn = input.analysis.connection_to(input.grammar, first.left_id);
        // 範囲外の ID は i16::MAX（接続禁止）が返る。禁止の接続を重くしても意味がないので無視する
        if conn >= i32::from(i16::MAX) {
            return 0;
        }
        let excess = (conn.abs() - self.cfg.threshold).max(0) * conn.signum();
        let delta = (f64::from(self.cfg.weight) * f64::from(excess)).round() as i32;
        delta.clamp(-self.cfg.max_total, self.cfg.max_total)
    }

    /// 規則 C: 左文脈の最後のトークンが 名詞,数詞 で、`first` の品詞が助数詞か
    fn is_counter_after_numeral(&self, input: &RankInput<'_>, first: &Segment) -> bool {
        if !self.cfg.numeral_counter {
            return false;
        }
        let Some(left) = input.analysis.tokens.last() else {
            return false;
        };
        pos_field(input.grammar, left.pos_id, 1) == Some(POS2_NUMERAL)
            && pos_field(input.grammar, first.pos_id, 2)
                .is_some_and(|f| f.starts_with(POS3_COUNTER_PREFIX))
    }

    /// 候補ごとに、複合語の加点を受けるか（ADR 0007 §3.1、§3.2）。どれも受けなければ空。
    fn compound_hits(&self, input: &RankInput<'_>, cands: &[Candidate]) -> Vec<bool> {
        if self.cfg.compound_bonus == 0 {
            return Vec::new();
        }
        // 条件 1: 左文脈の最後のトークンが名詞（固有名詞を含む）か、形状詞,一般 / 形状詞,タリ（ADR 0029 §2）。
        // そうでなければ表記の検索もしない
        let Some(left) = input.analysis.tokens.last() else {
            return Vec::new();
        };
        if !is_compound_left(input.grammar, left.pos_id) {
            return Vec::new();
        }
        // 条件 5 の L: 左の語と同じ表記の項目の読みのどれでもよい（解析器が選んだ読み 1つに絞ると 日本 + ご が直らない）
        let left_readings = readings_of_surface(input.dict, &left.surface);
        if left_readings.is_empty() {
            return Vec::new();
        }
        // 候補の（表記, 読み）ごとに 1回だけ引く
        let mut looked_up: Vec<(&str, &str, Option<usize>)> = Vec::new();
        let mut compounds: Vec<Compound> = Vec::new();
        let mut per_cand: Vec<Option<usize>> = Vec::with_capacity(cands.len());
        let mut joined = String::new();
        for cand in cands {
            let Some(seg) = single_dictionary_word(cand) else {
                per_cand.push(None);
                continue;
            };
            let surface = seg.surface.as_str();
            let reading = seg.reading.as_str();
            if let Some(&(_, _, hit)) = looked_up
                .iter()
                .find(|(s, r, _)| *s == surface && *r == reading)
            {
                per_cand.push(hit);
                continue;
            }
            joined.clear();
            joined.push_str(&left.surface);
            joined.push_str(surface);
            let hit = cheapest_compound(input, &joined, &left_readings, reading).map(|c| {
                compounds.push(c);
                compounds.len() - 1
            });
            looked_up.push((surface, reading, hit));
            per_cand.push(hit);
        }
        // 当たった複合語のうち、いちばん安いものと同じ lemma の候補だけ（年間 と 年官 なら 年間。子供たち と 子供達 は両方）
        let Some(best) = compounds.iter().min_by_key(|c| c.cost) else {
            return Vec::new();
        };
        per_cand
            .iter()
            .map(|hit| hit.is_some_and(|i| compounds[i].lemma == best.lemma))
            .collect()
    }
}

fn is_noun(grammar: &dyn Grammar, pos_id: u16) -> bool {
    grammar
        .pos_name(pos_id)
        .is_some_and(|name| pos1(name) == POS1_NOUN)
}

/// 接尾辞,名詞的 で、助数詞でないもの（一般、サ変可能、副詞可能）
fn is_noun_like_suffix(grammar: &dyn Grammar, pos_id: u16) -> bool {
    pos_field(grammar, pos_id, 0) == Some(POS1_SUFFIX)
        && pos_field(grammar, pos_id, 1) == Some(POS2_NOUN_LIKE_SUFFIX)
        && !pos_field(grammar, pos_id, 2).is_some_and(|f| f.starts_with(POS3_COUNTER_PREFIX))
}

/// 条件 1: 複合語の左の語の品詞か（名詞、または 形状詞,一般 / 形状詞,タリ）
fn is_compound_left(grammar: &dyn Grammar, pos_id: u16) -> bool {
    is_noun(grammar, pos_id)
        || grammar.pos_name(pos_id).is_some_and(|name| {
            COMPOUND_LEFT_ADJECTIVAL_PREFIXES
                .iter()
                .any(|p| name.starts_with(p))
        })
}

/// 品詞名の `index` 番目の列（0 始まり）
fn pos_field(grammar: &dyn Grammar, pos_id: u16, index: usize) -> Option<&str> {
    grammar
        .pos_name(pos_id)?
        .split(POS_FIELD_SEPARATOR)
        .nth(index)
}

/// 条件 2: 辞書の候補で、ちょうど 1つの辞書の語（未知語でない）でできている
fn single_dictionary_word(cand: &Candidate) -> Option<&Segment> {
    if cand.origin != CandidateOrigin::Dictionary {
        return None;
    }
    match cand.segments.as_slice() {
        [seg] if seg.word.is_some() => Some(seg),
        _ => None,
    }
}

/// 表記が `surface` と完全に一致する項目の読み（重複を除く）
fn readings_of_surface(dict: &dyn Dictionary, surface: &str) -> Vec<String> {
    let mut readings: Vec<String> = Vec::new();
    dict.common_prefix_search_surface(surface, &mut |len, e| {
        if len != surface.len() {
            return;
        }
        if let Some(r) = dict.reading(e.id)
            && !readings.iter().any(|x| x == r.as_ref())
        {
            readings.push(r.into_owned());
        }
    });
    readings
}

/// 条件 4: 複合語として数える品詞か
fn is_compound_pos(grammar: &dyn Grammar, e: &WordEntry<'_>) -> bool {
    let Some(name) = grammar.pos_name(e.pos_id) else {
        return false;
    };
    if COMPOUND_POS_PREFIXES.iter().any(|p| name.starts_with(p)) {
        return true;
    }
    e.id.dict != SYSTEM_SLOT
        && name.starts_with(USER_COMPOUND_POS_PREFIX)
        && !name.starts_with(PERSON_NAME_POS_PREFIX)
}

/// 条件 5: `reading` が `left + cand` と一致する
fn is_joined_reading(reading: &str, left: &str, cand: &str) -> bool {
    reading.len() == left.len() + cand.len() && reading.starts_with(left) && reading.ends_with(cand)
}

/// 条件 3〜5 を満たす、表記が `joined` の項目のうち、変換のコストがいちばん小さいもの
fn cheapest_compound(
    input: &RankInput<'_>,
    joined: &str,
    left_readings: &[String],
    cand_reading: &str,
) -> Option<Compound> {
    let mut best: Option<(i32, WordId)> = None;
    input
        .dict
        .common_prefix_search_surface(joined, &mut |len, e| {
            if len != joined.len() || e.penalty == IME_EXCLUDED {
                return;
            }
            let cost = i32::from(e.cost) + i32::from(e.penalty);
            if best.is_some_and(|(c, _)| c <= cost) || !is_compound_pos(input.grammar, &e) {
                return;
            }
            let joined_reading: Option<Cow<'_, str>> = input.dict.reading(e.id);
            if joined_reading.is_some_and(|r| {
                left_readings
                    .iter()
                    .any(|l| is_joined_reading(&r, l, cand_reading))
            }) {
                best = Some((cost, e.id));
            }
        });
    let (cost, id) = best?;
    let lemma = input
        .dict
        .lemma(id)
        .map_or_else(|| joined.to_owned(), |l| l.text.to_owned());
    Some(Compound { cost, lemma })
}

impl Ranker for LeftContextRanker {
    fn rank(&self, input: &RankInput<'_>, cands: &mut [Candidate]) {
        // 左文脈がなければ BOS は文頭の `bos_eos_id` で、足す手がかりがない
        let bos = input.analysis.bos_right_id;
        if input.context.secure
            || input.analysis.tokens.is_empty()
            || bos == input.grammar.bos_eos_id()
        {
            return;
        }
        let hits = self.compound_hits(input, cands);
        for (i, cand) in cands.iter_mut().enumerate() {
            if cand.origin != CandidateOrigin::Dictionary {
                continue;
            }
            // 複数の語に分かれた候補は、短い拘束形（き、来 など）が連接表で有利なだけの
            // ことが多い。左文脈の手がかりは、1つの辞書の語でできた候補にだけ使う
            let [first] = cand.segments.as_slice() else {
                continue;
            };
            let conn = self.connection_delta(input, first);
            let delta = if hits.get(i).copied().unwrap_or(false) {
                // 複合語が当たった候補は、品詞による連接の差（接尾辞は上限、名詞は 0）をなくしてから加点する（ADR 0007 §3.3）
                conn.min(-self.cfg.max_total)
                    .saturating_sub(self.cfg.compound_bonus)
            } else {
                conn
            };
            cand.score = cand.score.saturating_add(delta);
        }
    }

    fn name(&self) -> &str {
        names::LEFT_CONTEXT
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::analysis::AnalyzedContext;
    use crate::context::Context;

    const BOS_AFTER_PARTICLE: u16 = 1;

    fn ranker(weight: f32) -> LeftContextRanker {
        LeftContextRanker::new(LeftContextConfig {
            weight,
            threshold: 0,
            ..LeftContextConfig::default()
        })
    }

    /// 「粒子」の右ID(1) から 左ID 1 は -2000、左ID 2 は +1000 に接続する環境
    fn env() -> Env {
        let mut env = Env::new();
        env.grammar.set(BOS_AFTER_PARTICLE, 1, -2000);
        env.grammar.set(BOS_AFTER_PARTICLE, 2, 1000);
        env
    }

    fn analysis_after_particle() -> AnalyzedContext {
        let mut a = analysis(vec![token("に", "に", None, false, 0)]);
        a.bos_right_id = BOS_AFTER_PARTICLE;
        a
    }

    fn word_with_left(env: &mut Env, surface: &str, left_id: u16) -> crate::candidate::Segment {
        let mut seg = env.word(surface, surface, None, POS_PARTICLE);
        seg.left_id = left_id;
        seg
    }

    #[test]
    fn adds_weight_times_the_connection_from_the_left_context() {
        let mut env = env();
        let good = word_with_left(&mut env, "良", 1);
        let bad = word_with_left(&mut env, "悪", 2);
        let mut cands = vec![cand(vec![good], 5000), cand(vec![bad], 5000)];
        env.rank(
            &ranker(1.0),
            &Context::new(),
            &analysis_after_particle(),
            &mut cands,
        );
        assert_eq!(cands[0].score, 5000 - 2000);
        assert_eq!(cands[1].score, 5000 + 1000);
        assert_eq!(cands[0].base_cost, 5000);
    }

    #[test]
    fn only_the_part_over_the_threshold_is_amplified() {
        let mut env = env();
        let good = word_with_left(&mut env, "良", 1);
        let bad = word_with_left(&mut env, "悪", 2);
        let mut cands = vec![cand(vec![good], 5000), cand(vec![bad], 5000)];
        let r = LeftContextRanker::new(LeftContextConfig {
            weight: 1.0,
            threshold: 1000,
            ..LeftContextConfig::default()
        });
        env.rank(&r, &Context::new(), &analysis_after_particle(), &mut cands);
        // -2000 は 1000 を超えた -1000 だけ。+1000 は超えないので変わらない
        assert_eq!(cands[0].score, 5000 - 1000);
        assert_eq!(cands[1].score, 5000);
    }

    #[test]
    fn candidates_made_of_several_words_are_left_alone() {
        let mut env = env();
        let a = word_with_left(&mut env, "良", 1);
        let b = word_with_left(&mut env, "好", 1);
        let mut cands = vec![cand(vec![a, b], 5000)];
        env.rank(
            &ranker(1.0),
            &Context::new(),
            &analysis_after_particle(),
            &mut cands,
        );
        assert_eq!(cands[0].score, 5000);
    }

    #[test]
    fn candidates_starting_with_a_noun_are_left_alone() {
        let mut env = env();
        let mut noun = env.word("名", "名", None, POS_NOUN);
        noun.left_id = 1;
        let mut cands = vec![cand(vec![noun], 5000)];
        env.rank(
            &ranker(1.0),
            &Context::new(),
            &analysis_after_particle(),
            &mut cands,
        );
        assert_eq!(cands[0].score, 5000);
    }

    #[test]
    fn zero_weight_changes_nothing() {
        let mut env = env();
        let good = word_with_left(&mut env, "良", 1);
        let mut cands = vec![cand(vec![good], 5000)];
        env.rank(
            &ranker(0.0),
            &Context::new(),
            &analysis_after_particle(),
            &mut cands,
        );
        assert_eq!(cands[0].score, 5000);
    }

    #[test]
    fn nothing_without_a_left_context_or_when_secure() {
        let mut env = env();
        let good = word_with_left(&mut env, "良", 1);
        let mut cands = vec![cand(vec![good], 5000)];
        // トークンがない（左文脈なし）
        env.rank(
            &ranker(1.0),
            &Context::new(),
            &AnalyzedContext::default(),
            &mut cands,
        );
        assert_eq!(cands[0].score, 5000);
        // 文頭（BOS が bos_eos_id）
        let mut sentence_start = analysis_after_particle();
        sentence_start.bos_right_id = 0;
        env.rank(&ranker(1.0), &Context::new(), &sentence_start, &mut cands);
        assert_eq!(cands[0].score, 5000);
        // secure
        let mut secure = Context::new();
        secure.secure = true;
        env.rank(
            &ranker(1.0),
            &secure,
            &analysis_after_particle(),
            &mut cands,
        );
        assert_eq!(cands[0].score, 5000);
    }

    #[test]
    fn non_dictionary_candidates_are_left_alone() {
        let mut env = env();
        let good = word_with_left(&mut env, "良", 1);
        let mut c = cand(vec![good], 5000);
        c.origin = CandidateOrigin::Hiragana;
        let mut cands = vec![c];
        env.rank(
            &ranker(1.0),
            &Context::new(),
            &analysis_after_particle(),
            &mut cands,
        );
        assert_eq!(cands[0].score, 5000);
    }

    #[test]
    fn delta_is_capped() {
        let mut env = Env::new();
        env.grammar.set(BOS_AFTER_PARTICLE, 1, -9000);
        let good = word_with_left(&mut env, "良", 1);
        let mut cands = vec![cand(vec![good], 5000)];
        env.rank(
            &ranker(1.0),
            &Context::new(),
            &analysis_after_particle(),
            &mut cands,
        );
        assert_eq!(cands[0].score, 5000 - 6000);
    }

    #[test]
    fn config_reads_only_its_own_keys() {
        let kv: BTreeMap<String, String> = [("left_context.weight", "0.5"), ("topic.weight", "1")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let cfg = LeftContextConfig::from_kv(&kv).unwrap();
        assert_eq!(cfg.weight, 0.5);
        assert_eq!(cfg.max_total, LeftContextConfig::default().max_total);
        let bad: BTreeMap<String, String> = [("left_context.nope".to_owned(), "1".to_owned())]
            .into_iter()
            .collect();
        assert!(LeftContextConfig::from_kv(&bad).is_err());
    }
}

/// 複合語の加点（ADR 0007 §3）のテスト。表記の検索と読みが要るので、`InMemoryDictionary` で組む
#[cfg(test)]
mod compound_tests {
    use std::sync::Arc;

    use super::*;
    use crate::analysis::{AnalyzedContext, ContextToken};
    use crate::context::Context;
    use crate::dict::{DenseGrammar, DictionarySet, InMemoryDictionary, UnknownParams};

    /// 左文脈の最後の語の右ID
    const LEFT_RIGHT_ID: u16 = 1;
    /// 名詞の左ID（左文脈の語からの連接 0）
    const NOUN_LEFT: u16 = 1;
    /// 接尾辞の左ID（左文脈の語からの連接 -4895。増幅すると上限の -6000）
    const SUFFIX_LEFT: u16 = 2;
    /// 左文脈の語からの連接 -1500（増幅すると -2000。上限に届かない）
    const WEAK_LEFT: u16 = 3;
    /// 左文脈の語からの連接 +3000（増幅すると +6000）
    const BAD_LEFT: u16 = 4;
    const SUFFIX_CONN: i16 = -4895;
    const BASE: i32 = 10_000;
    const CAP: i32 = 6000;
    const BONUS: i32 = 3000;

    const COMMON_NOUN: &str = "名詞,普通名詞,一般,*,*,*";
    /// 増幅を受ける接尾辞（的 のような 接尾辞,形状詞的）。複合語の機構の試験で、連接の上限が付く側として使う
    const SUFFIX: &str = "接尾辞,形状詞的,*,*,*,*";
    /// 増幅を受けない名詞的な接尾辞（海、界、家、化。ADR 0029 §3）
    const NOUN_SUFFIX: &str = "接尾辞,名詞的,一般,*,*,*";
    const ADJECTIVAL_GENERAL: &str = "形状詞,一般,*,*,*,*";
    const ADJECTIVAL_TARI: &str = "形状詞,タリ,*,*,*,*";
    const ADJECTIVAL_STEM: &str = "形状詞,助動詞語幹,*,*,*,*";
    const PARTICLE: &str = "助詞,格助詞,*,*,*,*";
    const PERSON_GIVEN: &str = "名詞,固有名詞,人名,名,*,*";
    const PERSON_GENERAL: &str = "名詞,固有名詞,人名,一般,*,*";
    const PERSON_SURNAME: &str = "名詞,固有名詞,人名,姓,*,*";
    const PLACE: &str = "名詞,固有名詞,地名,一般,*,*";
    const PROPER: &str = "名詞,固有名詞,一般,*,*,*";
    const NUMERAL: &str = "名詞,数詞,*,*,*,*";
    const COUNTER_NOUN: &str = "名詞,普通名詞,助数詞可能,*,*,*";
    const COUNTER_SUFFIX: &str = "接尾辞,名詞的,助数詞,*,*,*";

    struct Env {
        grammar: DenseGrammar,
        system: InMemoryDictionary,
        user: InMemoryDictionary,
    }

    impl Env {
        fn new() -> Self {
            let mut grammar = DenseGrammar::new(
                5,
                5,
                0,
                UnknownParams {
                    left_id: 0,
                    right_id: 0,
                    pos_id: 0,
                    cost_per_char: 5000,
                },
            );
            for name in [
                COMMON_NOUN,
                SUFFIX,
                NOUN_SUFFIX,
                ADJECTIVAL_GENERAL,
                ADJECTIVAL_TARI,
                ADJECTIVAL_STEM,
                PARTICLE,
                PERSON_GIVEN,
                PERSON_GENERAL,
                PERSON_SURNAME,
                PLACE,
                PROPER,
                NUMERAL,
                COUNTER_NOUN,
                COUNTER_SUFFIX,
            ] {
                grammar.add_pos(name);
            }
            grammar.set(LEFT_RIGHT_ID, SUFFIX_LEFT, SUFFIX_CONN);
            grammar.set(LEFT_RIGHT_ID, WEAK_LEFT, -1500);
            grammar.set(LEFT_RIGHT_ID, BAD_LEFT, 3000);
            let id = grammar.grammar_id();
            Self {
                grammar,
                system: InMemoryDictionary::new(id),
                user: InMemoryDictionary::new(id),
            }
        }

        fn pos(&self, name: &str) -> u16 {
            (0..)
                .find(|&i| self.grammar.pos_name(i) == Some(name))
                .unwrap()
        }

        /// システム辞書に語を足す
        fn add(&mut self, reading: &str, surface: &str, pos: &str, cost: i16) -> WordId {
            let pos = self.pos(pos);
            self.system.insert(reading, surface, 1, 1, cost, pos)
        }

        /// ユーザー辞書（`DictionarySet` の slot 1）に語を足す
        fn add_user(&mut self, reading: &str, surface: &str, pos: &str) -> WordId {
            let pos = self.pos(pos);
            self.user.insert(reading, surface, 1, 1, 3000, pos)
        }

        /// 1語の候補（辞書の語）。`left_id` で左文脈の語からの連接を決める
        fn cand(&self, reading: &str, surface: &str, pos: &str, left_id: u16) -> Candidate {
            let segment = Segment {
                reading: reading.to_owned(),
                surface: surface.to_owned(),
                word: Some(WordId {
                    dict: 0,
                    index: 9999,
                }),
                left_id,
                right_id: 1,
                pos_id: self.pos(pos),
            };
            Candidate {
                surface: surface.to_owned(),
                segments: vec![segment],
                base_cost: BASE,
                score: BASE,
                origin: CandidateOrigin::Dictionary,
            }
        }

        fn analysis(&self, surface: &str, pos: &str) -> AnalyzedContext {
            AnalyzedContext {
                tokens: vec![ContextToken {
                    surface: surface.to_owned(),
                    lemma: surface.to_owned(),
                    lemma_id: None,
                    word: None,
                    left_id: 1,
                    right_id: LEFT_RIGHT_ID,
                    pos_id: self.pos(pos),
                    is_content: true,
                    char_end: 0,
                }],
                bos_right_id: LEFT_RIGHT_ID,
                bos_alternatives: Vec::new(),
                content_count: 1,
                carried_from: None,
            }
        }

        /// 候補を並べ、各候補の差分（score - base_cost）を返す
        fn deltas(
            self,
            cfg: LeftContextConfig,
            ctx: &Context,
            analysis: &AnalyzedContext,
            cands: &mut [Candidate],
        ) -> Vec<i32> {
            let mut set = DictionarySet::new();
            set.push(Arc::new(self.system)).unwrap();
            set.push(Arc::new(self.user)).unwrap();
            let input = RankInput {
                reading: "てすと",
                context: ctx,
                grammar: &self.grammar,
                analysis,
                dict: &set,
                deadline: None,
            };
            LeftContextRanker::new(cfg).rank(&input, cands);
            cands.iter().map(|c| c.score - BASE).collect()
        }

        /// 左文脈の最後の語が `left`（品詞 `left_pos`）のとき、既定の設定で並べた差分
        fn run(self, left: &str, left_pos: &str, cands: &mut [Candidate]) -> Vec<i32> {
            let analysis = self.analysis(left, left_pos);
            self.deltas(
                LeftContextConfig::default(),
                &Context::new(),
                &analysis,
                cands,
            )
        }
    }

    /// 洗濯 + き: 洗濯機 が辞書にある。器（接尾辞）は連接で上限、機（名詞）は 0 だった
    fn washing_machine() -> (Env, Vec<Candidate>) {
        let mut env = Env::new();
        env.add("せんたく", "洗濯", COMMON_NOUN, 3000);
        env.add("せんたくき", "洗濯機", COMMON_NOUN, 4000);
        let cands = vec![
            env.cand("き", "器", SUFFIX, SUFFIX_LEFT),
            env.cand("き", "機", COMMON_NOUN, NOUN_LEFT),
        ];
        (env, cands)
    }

    #[test]
    fn a_compound_in_the_dictionary_raises_the_candidate_to_the_cap_plus_the_bonus() {
        let (env, mut cands) = washing_machine();
        let d = env.run("洗濯", COMMON_NOUN, &mut cands);
        assert_eq!(d, vec![-CAP, -CAP - BONUS]);
    }

    #[test]
    fn a_suffix_candidate_that_forms_the_compound_gets_the_cap_plus_the_bonus_not_more() {
        // 自動 + か: 化 が接尾辞でも、合計は上限 + 加点（-12000 にはならない）
        let mut env = Env::new();
        env.add("じどう", "自動", COMMON_NOUN, 3000);
        env.add("じどうか", "自動化", COMMON_NOUN, 4000);
        let mut cands = vec![
            env.cand("か", "家", SUFFIX, SUFFIX_LEFT),
            env.cand("か", "化", SUFFIX, SUFFIX_LEFT),
        ];
        let d = env.run("自動", COMMON_NOUN, &mut cands);
        assert_eq!(d, vec![-CAP, -CAP - BONUS]);
    }

    #[test]
    fn the_connection_delta_is_raised_to_the_cap_whatever_it_was() {
        // 当たった候補は、連接の差分が弱い（-2000）、正（+6000）でも、上限まで引き上げてから加点する
        let mut env = Env::new();
        env.add("せんたく", "洗濯", COMMON_NOUN, 3000);
        env.add("せんたくき", "洗濯機", COMMON_NOUN, 4000);
        env.add("せんたくし", "洗濯紙", COMMON_NOUN, 4000);
        let mut cands = vec![
            env.cand("し", "紙", SUFFIX, BAD_LEFT),
            env.cand("き", "機", SUFFIX, WEAK_LEFT),
            env.cand("き", "木", SUFFIX, WEAK_LEFT),
        ];
        // 洗濯紙 と 洗濯機 は同じコスト。先に見つかった 洗濯紙 の候補だけ（同じ lemma でないので 機 は外れる）
        let d = env.run("洗濯", COMMON_NOUN, &mut cands);
        assert_eq!(d, vec![-CAP - BONUS, -2000, -2000]);
        // 紙 を外すと 機 が当たる（弱い連接 -2000 から引き上げる）
        let mut env = Env::new();
        env.add("せんたく", "洗濯", COMMON_NOUN, 3000);
        env.add("せんたくき", "洗濯機", COMMON_NOUN, 4000);
        let mut cands = vec![env.cand("き", "機", SUFFIX, WEAK_LEFT)];
        assert_eq!(env.run("洗濯", COMMON_NOUN, &mut cands), vec![-CAP - BONUS]);
    }

    /// 最適 + かい: 解（名詞）と 海（名詞的な接尾辞）。最適解 が辞書にある
    fn optimal_solution(left_pos: &str) -> (Env, Vec<Candidate>, &str) {
        let mut env = Env::new();
        env.add("さいてき", "最適", left_pos, 3000);
        env.add("さいてきかい", "最適解", COMMON_NOUN, 4000);
        let cands = vec![
            env.cand("かい", "海", NOUN_SUFFIX, SUFFIX_LEFT),
            env.cand("かい", "解", COMMON_NOUN, NOUN_LEFT),
        ];
        (env, cands, "最適")
    }

    #[test]
    fn a_noun_like_suffix_is_not_amplified_but_a_compound_still_raises_the_candidate() {
        // 複合語が当たらなければ 0（増幅しない）。最適解 が当たる解は、上限 + 加点
        let (env, mut cands, left) = optimal_solution(ADJECTIVAL_GENERAL);
        let d = env.run(left, ADJECTIVAL_GENERAL, &mut cands);
        assert_eq!(d, vec![0, -CAP - BONUS]);
        // 名詞 + 名詞的な接尾辞（複合語なし）も 0
        let env = Env::new();
        let mut cands = vec![env.cand("か", "家", NOUN_SUFFIX, SUFFIX_LEFT)];
        assert_eq!(env.run("文字列", COMMON_NOUN, &mut cands), vec![0]);
        // 複合語が当たった名詞的な接尾辞は、今までどおり上限 + 加点
        let mut env = Env::new();
        env.add("じどう", "自動", COMMON_NOUN, 3000);
        env.add("じどうか", "自動化", COMMON_NOUN, 4000);
        let mut cands = vec![env.cand("か", "化", NOUN_SUFFIX, SUFFIX_LEFT)];
        assert_eq!(env.run("自動", COMMON_NOUN, &mut cands), vec![-CAP - BONUS]);
    }

    #[test]
    fn counter_suffixes_and_other_suffixes_are_still_amplified() {
        let env = Env::new();
        let mut cands = vec![
            env.cand("さい", "歳", COUNTER_SUFFIX, SUFFIX_LEFT),
            env.cand("てき", "的", SUFFIX, SUFFIX_LEFT),
        ];
        assert_eq!(env.run("五", NUMERAL, &mut cands), vec![-CAP, -CAP]);
    }

    #[test]
    fn adjectival_general_and_tari_left_words_count_for_compounds() {
        for pos in [ADJECTIVAL_GENERAL, ADJECTIVAL_TARI] {
            let (env, mut cands, left) = optimal_solution(pos);
            let d = env.run(left, pos, &mut cands);
            assert_eq!(d, vec![0, -CAP - BONUS], "{pos}");
        }
    }

    #[test]
    fn an_auxiliary_stem_adjectival_left_word_does_not_count() {
        // そう、よう、みたい（形状詞,助動詞語幹）は複合語の左の語にしない（ADR 0029 §2）
        let (env, mut cands, left) = optimal_solution(ADJECTIVAL_STEM);
        let d = env.run(left, ADJECTIVAL_STEM, &mut cands);
        assert_eq!(d, vec![0, 0]);
    }

    #[test]
    fn nothing_when_the_left_word_is_not_a_noun() {
        // できる + かぎり → 限り（できる限り）のような誤りを防ぐ（ADR 0007 §3.1 の条件 1）
        let (env, mut cands) = washing_machine();
        let d = env.run("洗濯", PARTICLE, &mut cands);
        assert_eq!(d, vec![-CAP, 0]);
    }

    #[test]
    fn a_proper_noun_left_word_counts_as_a_noun() {
        let mut env = Env::new();
        env.add("きょうと", "京都", PLACE, 3000);
        env.add("きょうとし", "京都市", PLACE, 4000);
        let mut cands = vec![
            env.cand("し", "史", COMMON_NOUN, NOUN_LEFT),
            env.cand("し", "市", SUFFIX, SUFFIX_LEFT),
        ];
        let d = env.run("京都", PLACE, &mut cands);
        assert_eq!(d, vec![0, -CAP - BONUS]);
    }

    #[test]
    fn candidates_of_several_words_unknown_words_or_not_from_the_dictionary_are_left_alone() {
        let (env, _) = washing_machine();
        let ki = env.cand("き", "機", COMMON_NOUN, NOUN_LEFT);
        // 複数の語（機 + 機）。最初の語で調べることもしない（ADR 0007 §5）
        let mut two = ki.clone();
        two.segments.push(ki.segments[0].clone());
        two.surface = "機機".to_owned();
        // 未知語
        let mut unknown = ki.clone();
        unknown.segments[0].word = None;
        // 辞書でない候補（ひらがな）
        let mut hiragana = ki.clone();
        hiragana.origin = CandidateOrigin::Hiragana;
        let mut cands = vec![two, unknown, hiragana, ki];
        let d = env.run("洗濯", COMMON_NOUN, &mut cands);
        assert_eq!(d, vec![0, 0, 0, -CAP - BONUS]);
    }

    #[test]
    fn a_multi_character_single_word_candidate_is_covered() {
        // 電話 + ばんごう → 番号（1語なら長さは問わない。ADR 0007 §5）
        let mut env = Env::new();
        env.add("でんわ", "電話", COMMON_NOUN, 3000);
        env.add("でんわばんごう", "電話番号", COMMON_NOUN, 4000);
        let mut cands = vec![env.cand("ばんごう", "番号", COMMON_NOUN, NOUN_LEFT)];
        let d = env.run("電話", COMMON_NOUN, &mut cands);
        assert_eq!(d, vec![-CAP - BONUS]);
    }

    #[test]
    fn person_names_and_other_system_proper_nouns_are_not_compounds() {
        // 犬 + と → 都（犬都。名詞,固有名詞,人名,名）。システム辞書の 固有名詞,一般 も数えない（条件 4）
        let mut env = Env::new();
        env.add("いぬ", "犬", COMMON_NOUN, 3000);
        env.add("いぬと", "犬都", PERSON_GIVEN, 4000);
        env.add("いぬし", "犬氏", PROPER, 4000);
        let mut cands = vec![
            env.cand("と", "と", PARTICLE, SUFFIX_LEFT),
            env.cand("と", "都", COMMON_NOUN, NOUN_LEFT),
            env.cand("し", "氏", COMMON_NOUN, NOUN_LEFT),
        ];
        let d = env.run("犬", COMMON_NOUN, &mut cands);
        assert_eq!(d, vec![-CAP, 0, 0]);
    }

    #[test]
    fn place_names_are_compounds() {
        let mut env = Env::new();
        env.add("おおさか", "大阪", PLACE, 3000);
        env.add("おおさかふ", "大阪府", PLACE, 4000);
        let mut cands = vec![
            env.cand("ふ", "付", SUFFIX, SUFFIX_LEFT),
            env.cand("ふ", "府", COMMON_NOUN, NOUN_LEFT),
        ];
        let d = env.run("大阪", PLACE, &mut cands);
        assert_eq!(d, vec![-CAP, -CAP - BONUS]);
    }

    #[test]
    fn excluded_entries_are_not_compounds() {
        let mut env = Env::new();
        env.add("せんたく", "洗濯", COMMON_NOUN, 3000);
        let ex = env.add("せんたくき", "洗濯機", COMMON_NOUN, 4000);
        env.system.set_penalty(ex, IME_EXCLUDED);
        let mut cands = vec![
            env.cand("き", "器", SUFFIX, SUFFIX_LEFT),
            env.cand("き", "機", COMMON_NOUN, NOUN_LEFT),
        ];
        assert_eq!(env.run("洗濯", COMMON_NOUN, &mut cands), vec![-CAP, 0]);
        // 同じ表記の項目が別にあれば、そちらで当たる
        let (mut env, mut cands) = washing_machine();
        let ex = env.add("せんたくき", "洗濯機", COMMON_NOUN, 100);
        env.system.set_penalty(ex, IME_EXCLUDED);
        assert_eq!(
            env.run("洗濯", COMMON_NOUN, &mut cands),
            vec![-CAP, -CAP - BONUS]
        );
    }

    #[test]
    fn the_compound_reading_must_be_the_left_reading_plus_the_candidate_reading() {
        // 私 + たち で 私立（しりつ）は当たらない（条件 5）
        let mut env = Env::new();
        env.add("わたし", "私", COMMON_NOUN, 3000);
        env.add("しりつ", "私立", COMMON_NOUN, 4000);
        let mut cands = vec![
            env.cand("たち", "たち", SUFFIX, SUFFIX_LEFT),
            env.cand("たち", "立", COMMON_NOUN, NOUN_LEFT),
        ];
        let d = env.run("私", COMMON_NOUN, &mut cands);
        assert_eq!(d, vec![-CAP, 0]);
    }

    #[test]
    fn any_reading_of_the_left_surface_will_do() {
        // 日本 は にっぽん と解析されても、にほん の読みの項目があれば 日本語（にほんご）が当たる
        let mut env = Env::new();
        env.add("にっぽん", "日本", COMMON_NOUN, 3000);
        env.add("にほん", "日本", COMMON_NOUN, 3500);
        env.add("にほんご", "日本語", COMMON_NOUN, 4000);
        let mut cands = vec![
            env.cand("ご", "御", SUFFIX, SUFFIX_LEFT),
            env.cand("ご", "語", COMMON_NOUN, NOUN_LEFT),
        ];
        let d = env.run("日本", COMMON_NOUN, &mut cands);
        assert_eq!(d, vec![-CAP, -CAP - BONUS]);
    }

    #[test]
    fn nothing_when_the_left_surface_has_no_entry() {
        let mut env = Env::new();
        env.add("せんたくき", "洗濯機", COMMON_NOUN, 4000);
        let mut cands = vec![env.cand("き", "機", COMMON_NOUN, NOUN_LEFT)];
        let d = env.run("洗濯", COMMON_NOUN, &mut cands);
        assert_eq!(d, vec![0]);
    }

    #[test]
    fn only_candidates_of_the_cheapest_compound_get_the_bonus() {
        // 年 + かん: 年間、年官 が当たる。cost + penalty がいちばん安い 年間 の 間 だけ
        let mut env = Env::new();
        env.add("ねん", "年", COMMON_NOUN, 3000);
        env.add("ねんかん", "年官", COMMON_NOUN, 9000);
        env.add("ねんかん", "年間", COMMON_NOUN, 4000);
        let mut cands = vec![
            env.cand("かん", "官", SUFFIX, SUFFIX_LEFT),
            env.cand("かん", "間", COMMON_NOUN, NOUN_LEFT),
        ];
        let d = env.run("年", COMMON_NOUN, &mut cands);
        assert_eq!(d, vec![-CAP, -CAP - BONUS]);
    }

    #[test]
    fn the_penalty_counts_in_the_cost_of_the_compound() {
        let mut env = Env::new();
        env.add("ねん", "年", COMMON_NOUN, 3000);
        env.add("ねんかん", "年官", COMMON_NOUN, 9000);
        let id = env.add("ねんかん", "年間", COMMON_NOUN, 4000);
        env.system.set_penalty(id, 6000);
        let mut cands = vec![
            env.cand("かん", "官", SUFFIX, SUFFIX_LEFT),
            env.cand("かん", "間", COMMON_NOUN, NOUN_LEFT),
        ];
        let d = env.run("年", COMMON_NOUN, &mut cands);
        assert_eq!(d, vec![-CAP - BONUS, 0]);
    }

    #[test]
    fn candidates_of_compounds_with_the_same_lemma_all_get_the_bonus() {
        // 子供 + たち: 子供たち と 子供達 は同じ lemma。表記の好みは別の Ranker と経路のコストに任せる
        let mut env = Env::new();
        env.add("こども", "子供", COMMON_NOUN, 3000);
        let a = env.add("こどもたち", "子供たち", COMMON_NOUN, 5000);
        env.system.set_lemma(a, "子供達");
        env.add("こどもたち", "子供達", COMMON_NOUN, 4000);
        let mut cands = vec![
            env.cand("たち", "たち", SUFFIX, SUFFIX_LEFT),
            env.cand("たち", "達", SUFFIX, SUFFIX_LEFT),
        ];
        let d = env.run("子供", COMMON_NOUN, &mut cands);
        assert_eq!(d, vec![-CAP - BONUS, -CAP - BONUS]);
    }

    #[test]
    fn user_dictionary_proper_nouns_count_but_person_names_do_not() {
        // ユーザー辞書の「固有名詞」「組織」（名詞,固有名詞,一般）は当たる。「人名」「姓」「名」は当たらない（§12.1 Q4）
        let mut env = Env::new();
        env.add("いかり", "錨", COMMON_NOUN, 3000);
        env.add_user("いかりばん", "錨版", PROPER);
        env.add_user("いかりし", "錨氏", PERSON_GENERAL);
        env.add_user("いかりさ", "錨佐", PERSON_SURNAME);
        env.add_user("いかりこ", "錨子", PERSON_GIVEN);
        let mut cands = vec![
            env.cand("ばん", "版", COMMON_NOUN, NOUN_LEFT),
            env.cand("し", "氏", COMMON_NOUN, NOUN_LEFT),
            env.cand("さ", "佐", COMMON_NOUN, NOUN_LEFT),
            env.cand("こ", "子", COMMON_NOUN, NOUN_LEFT),
        ];
        let d = env.run("錨", COMMON_NOUN, &mut cands);
        assert_eq!(d, vec![-CAP - BONUS, 0, 0, 0]);
    }

    #[test]
    fn a_user_dictionary_common_noun_with_a_user_left_word_works() {
        // 製品名 + 版 を登録すると、製品名 + ばん → 版。左の語がユーザー辞書の語でもよい（ADR 0007 §6.4）
        let mut env = Env::new();
        env.add_user("ぷろだくと", "プロダクト", PROPER);
        env.add_user("ぷろだくとばん", "プロダクト版", COMMON_NOUN);
        let mut cands = vec![
            env.cand("ばん", "板", SUFFIX, SUFFIX_LEFT),
            env.cand("ばん", "版", COMMON_NOUN, NOUN_LEFT),
        ];
        let d = env.run("プロダクト", PROPER, &mut cands);
        assert_eq!(d, vec![-CAP, -CAP - BONUS]);
    }

    #[test]
    fn zero_compound_bonus_keeps_the_connection_only_behavior() {
        let (env, mut cands) = washing_machine();
        let analysis = env.analysis("洗濯", COMMON_NOUN);
        let cfg = LeftContextConfig {
            compound_bonus: 0,
            ..LeftContextConfig::default()
        };
        let d = env.deltas(cfg, &Context::new(), &analysis, &mut cands);
        assert_eq!(d, vec![-CAP, 0]);
    }

    #[test]
    fn nothing_when_secure_or_at_the_start_of_a_sentence() {
        let (env, mut cands) = washing_machine();
        let mut secure = Context::new();
        secure.secure = true;
        let analysis = env.analysis("洗濯", COMMON_NOUN);
        let d = env.deltas(LeftContextConfig::default(), &secure, &analysis, &mut cands);
        assert_eq!(d, vec![0, 0]);
        let (env, mut cands) = washing_machine();
        let mut start = env.analysis("洗濯", COMMON_NOUN);
        start.bos_right_id = env.grammar.bos_eos_id();
        let d = env.deltas(
            LeftContextConfig::default(),
            &Context::new(),
            &start,
            &mut cands,
        );
        assert_eq!(d, vec![0, 0]);
    }

    #[test]
    fn compound_bonus_is_read_from_the_config() {
        let kv: BTreeMap<String, String> = [("left_context.compound_bonus", "500")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let cfg = LeftContextConfig::from_kv(&kv).unwrap();
        assert_eq!(cfg.compound_bonus, 500);
        assert_eq!(cfg.weight, LeftContextConfig::default().weight);
        assert_eq!(LeftContextConfig::default().compound_bonus, 3000);
    }

    /// 三 + じ: 次（接尾辞,助数詞）と 時（名詞,助数詞可能）。どちらも連接は接尾辞と同じ -4895 の想定
    fn counters() -> (Env, Vec<Candidate>) {
        let env = Env::new();
        let cands = vec![
            env.cand("じ", "次", COUNTER_SUFFIX, SUFFIX_LEFT),
            env.cand("じ", "時", COUNTER_NOUN, SUFFIX_LEFT),
            env.cand("じ", "字", COMMON_NOUN, SUFFIX_LEFT),
        ];
        (env, cands)
    }

    #[test]
    fn after_a_numeral_a_counter_noun_is_amplified_like_a_suffix() {
        let (env, mut cands) = counters();
        let d = env.run("三", NUMERAL, &mut cands);
        // 助数詞の名詞（時）も接尾辞（次）と同じ上限。助数詞でない名詞（字）は 0 のまま
        assert_eq!(d, vec![-CAP, -CAP, 0]);
    }

    #[test]
    fn after_a_non_numeral_a_counter_noun_is_not_amplified() {
        let (env, mut cands) = counters();
        let d = env.run("日本語", COMMON_NOUN, &mut cands);
        assert_eq!(d, vec![-CAP, 0, 0]);
    }

    #[test]
    fn the_numeral_counter_rule_can_be_turned_off() {
        let (env, mut cands) = counters();
        let analysis = env.analysis("三", NUMERAL);
        let cfg = LeftContextConfig {
            numeral_counter: false,
            ..LeftContextConfig::default()
        };
        let d = env.deltas(cfg, &Context::new(), &analysis, &mut cands);
        assert_eq!(d, vec![-CAP, 0, 0]);
    }

    #[test]
    fn numeral_counter_config_key_is_read() {
        let kv: BTreeMap<String, String> = [("left_context.numeral_counter", "0")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        assert!(!LeftContextConfig::from_kv(&kv).unwrap().numeral_counter);
        assert!(LeftContextConfig::default().numeral_counter);
    }
}
