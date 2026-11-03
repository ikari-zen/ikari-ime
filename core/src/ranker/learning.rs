//! 文脈付きの学習の加点（ADR 0002 §8.4）。secure のときは加点しない。
//!
//! 現在の文脈から作った特徴量 f ごとに、その特徴量の下での選択の割合
//! `share = n(読み, f, 候補) / (N(読み, f) + prior_count)` を求め、`Σ_f w_f × share × base_bonus` を加点する。
//! n と N は層の重み（usage 1.0 / initial 0.4）と経過日数の減衰（半減期 90 日）をかけた回数。
//! `prior_count`（既定 1.0）は、観測が少ないうちは share を 1 にしないための平滑化で、
//! ADR 0002 §8.4 の `n/N` からの小さな逸脱（これがないと層の重みと減衰が share に効かない）。
//! 「最後に選んだものを 1 位にする」学習ではない。`Unconditional` の重みは低い（0.2）ので、別の文脈
//! （`Prev`、`Bag`、`Topic` が一致しない）での選択はほとんど効かない。ただし、今の文脈の特徴量に
//! その読みの記録が 1 つもないときだけ、`Unconditional` の重みを 1.0 にする（back-off。ADR 0013）。
//! 上限（`max_total`）は phrase の加点と word の加点に別々にかける。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use super::names;
use super::{RankInput, Ranker, RankerConfigError};
use crate::analysis::AnalyzedContext;
use crate::candidate::{Candidate, CandidateOrigin};
use crate::pos::{is_content_word, same_word_role};
use crate::user::features::UNCONDITIONAL_WEIGHT;
use crate::user::{
    FeatureKind, Layer, LearningRecord, LearningStore, Unit, extract_named, rewriter_key,
    today_unix_day,
};
use crate::vectors::WordVectors;

/// `from_kv` が見るキーの接頭辞（ranker-config は Ranker をまたいで共有されるため、他のキーは無視する）
pub const CONFIG_KEY_PREFIX: &str = "learning.";

/// back-off のときの `Unconditional` の重みの既定（ADR 0013 §9 Q2）
const DEFAULT_BACKOFF_WEIGHT: f32 = 1.0;

/// 役割が違う対抗と競うときの word の係数の既定。1.0 は弱めない（ADR 0044。値は掃引で決める）
const DEFAULT_WORD_ROLE_MISMATCH_FACTOR: f32 = 1.0;

#[derive(Clone, Debug, PartialEq)]
pub struct LearningConfig {
    /// `Σ_f w_f * share * base_bonus`
    pub base_bonus: i32,
    pub usage_weight: f32,
    pub initial_weight: f32,
    /// `word` 記録（セグメント単位）の一致の係数
    pub word_factor: f32,
    /// word の加点を受けるセグメントが、同じ区間の「いちばん手前の対抗」（違う表記）と品詞1・活用形で
    /// 役割が違うときに、word の share にかける係数（0.0〜1.0。1.0 で弱めない。ADR 0044）。
    /// phrase の加点には効かない
    pub word_role_mismatch_factor: f32,
    pub half_life_days: u32,
    /// 1候補あたり、phrase の加点と word の加点それぞれの上限（ADR 0013 F2）
    pub max_total: i32,
    /// `Unconditional`（文脈によらない、その読みでの選択の割合）の重み
    pub unconditional_weight: f32,
    /// 今の文脈の特徴量に、その読みの記録が 1 つもないときの `Unconditional` の重み（ADR 0013 案 C）。
    /// `unconditional_weight` と同じ値にすると back-off をしない
    pub unconditional_backoff_weight: f32,
    /// 経過日数の基準日（UNIX 日）。None なら現在の日。テスト用
    pub today: Option<u32>,
    /// `share = n / (N + prior_count)` の平滑化。観測が少ないほど、層の重みと経過日数による減衰が
    /// 加点の強さに効く（比だけにすると、1 通りしか選ばれていない記録は減衰しても share が 1 のままになる）
    pub prior_count: f32,
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            base_bonus: 3000,
            usage_weight: 1.0,
            initial_weight: 0.4,
            word_factor: 0.5,
            word_role_mismatch_factor: DEFAULT_WORD_ROLE_MISMATCH_FACTOR,
            half_life_days: 90,
            max_total: 6000,
            unconditional_weight: UNCONDITIONAL_WEIGHT,
            unconditional_backoff_weight: DEFAULT_BACKOFF_WEIGHT,
            today: None,
            prior_count: 1.0,
        }
    }
}

fn config_error(key: &str, message: impl Into<String>) -> RankerConfigError {
    RankerConfigError {
        key: key.to_string(),
        message: message.into(),
    }
}

fn parse_value<T: std::str::FromStr>(key: &str, value: &str) -> Result<T, RankerConfigError> {
    value
        .trim()
        .parse()
        .map_err(|_| config_error(key, format!("値 `{value}` を数値として読めません")))
}

impl LearningConfig {
    /// `from_kv` が見るキーの接頭辞
    pub const KEY_PREFIX: &'static str = CONFIG_KEY_PREFIX;

    /// `learning.<name>` のキーだけを見る（`base_bonus`、`usage_weight`、`initial_weight`、
    /// `word_factor`、`word_role_mismatch_factor`、`half_life_days`、`max_total`、`prior_count`、`unconditional_weight`、`unconditional_backoff_weight`、`today`）。他の接頭辞のキーは無視する。
    /// `learning.` の下の未知のキーや、読めない値はエラー。
    pub fn from_kv(kv: &BTreeMap<String, String>) -> Result<Self, RankerConfigError> {
        let mut cfg = Self::default();
        for (key, value) in kv {
            let Some(name) = key.strip_prefix(CONFIG_KEY_PREFIX) else {
                continue;
            };
            match name {
                "base_bonus" => cfg.base_bonus = parse_value(key, value)?,
                "usage_weight" => cfg.usage_weight = parse_value(key, value)?,
                "initial_weight" => cfg.initial_weight = parse_value(key, value)?,
                "word_factor" => cfg.word_factor = parse_value(key, value)?,
                "word_role_mismatch_factor" => {
                    let factor: f32 = parse_value(key, value)?;
                    if !(0.0..=1.0).contains(&factor) {
                        return Err(config_error(key, "0.0〜1.0 の範囲で指定してください"));
                    }
                    cfg.word_role_mismatch_factor = factor;
                }
                "half_life_days" => cfg.half_life_days = parse_value(key, value)?,
                "max_total" => cfg.max_total = parse_value(key, value)?,
                "today" => cfg.today = Some(parse_value(key, value)?),
                "prior_count" => cfg.prior_count = parse_value(key, value)?,
                "unconditional_weight" => cfg.unconditional_weight = parse_value(key, value)?,
                "unconditional_backoff_weight" => {
                    cfg.unconditional_backoff_weight = parse_value(key, value)?;
                }
                _ => return Err(config_error(key, "未知のキーです")),
            }
        }
        Ok(cfg)
    }

    fn layer_weight(&self, layer: Layer) -> f32 {
        match layer {
            Layer::Usage => self.usage_weight,
            Layer::Initial => self.initial_weight,
        }
    }

    /// 層の重み × 経過日数による減衰（半減期）の後の回数。
    fn effective_count(&self, r: &LearningRecord, today: u32) -> f32 {
        let age = today.saturating_sub(r.last_day) as f32;
        let half_life = self.half_life_days.max(1) as f32;
        r.count as f32 * self.layer_weight(r.layer) * 0.5f32.powf(age / half_life)
    }
}

pub struct LearningRanker {
    store: Arc<LearningStore>,
    vectors: Option<Arc<dyn WordVectors>>,
    cfg: LearningConfig,
}

impl LearningRanker {
    pub fn new(
        store: Arc<LearningStore>,
        vectors: Option<Arc<dyn WordVectors>>,
        cfg: LearningConfig,
    ) -> Self {
        Self {
            store,
            vectors,
            cfg,
        }
    }
}

/// 1 つの読み・1 つの単位についての、特徴量ごとの重み付き回数。
#[derive(Default)]
struct ShareTable {
    /// 特徴量 -> 全表記の回数の合計
    totals: HashMap<u64, f32>,
    /// (特徴量, 表記) -> 回数
    by_surface: HashMap<(u64, String), f32>,
}

impl ShareTable {
    fn build(
        hits: &[LearningRecord],
        unit: Unit,
        weights: &HashMap<u64, f32>,
        cfg: &LearningConfig,
        today: u32,
    ) -> Self {
        let mut t = Self::default();
        for r in hits
            .iter()
            .filter(|r| r.unit == unit && weights.contains_key(&r.feature))
        {
            let e = cfg.effective_count(r, today);
            *t.totals.entry(r.feature).or_default() += e;
            *t.by_surface
                .entry((r.feature, r.surface.clone()))
                .or_default() += e;
        }
        t
    }

    /// 今の文脈の特徴量（`Unconditional` 以外）の記録が 1 つでもあるか
    fn has_context_record(&self, unconditional: &HashSet<u64>) -> bool {
        self.totals.keys().any(|f| !unconditional.contains(f))
    }

    /// `Σ_f w_f × n(f, surface) / (N(f) + prior)` を、文脈の特徴量の分と `Unconditional` の分に分けて返す。
    /// `Unconditional` の重みは `unconditional_weight`（この表の back-off で決めた値）を使う。
    fn weighted_share(
        &self,
        surface: &str,
        weights: &HashMap<u64, f32>,
        unconditional: &HashSet<u64>,
        unconditional_weight: f32,
        prior: f32,
    ) -> Share {
        let mut share = Share::default();
        for (&feature, &w) in weights {
            let Some(&total) = self.totals.get(&feature) else {
                continue;
            };
            if let Some(&n) = self.by_surface.get(&(feature, surface.to_string())) {
                let ratio = n / (total + prior.max(0.0)).max(f32::EPSILON);
                if unconditional.contains(&feature) {
                    share.unconditional += unconditional_weight * ratio;
                } else {
                    share.context += w * ratio;
                }
            }
        }
        share
    }
}

/// share の合計の内訳
#[derive(Clone, Copy, Default)]
struct Share {
    /// 文脈の特徴量（Prev、Bag、Topic など）の分
    context: f32,
    /// `Unconditional` の分
    unconditional: f32,
}

impl Share {
    fn scaled(self, factor: f32) -> Self {
        Self {
            context: self.context * factor,
            unconditional: self.unconditional * factor,
        }
    }

    fn add(&mut self, other: Self) {
        self.context += other.context;
        self.unconditional += other.unconditional;
    }
}

/// 入力の読みの上の区間（バイトの位置。`start..end`）
type Span = (usize, usize);

/// 1 つの区間に現れるセグメントの、表記と品詞ID
#[derive(Clone, Copy)]
struct SpanSeg<'a> {
    surface: &'a str,
    pos_id: u16,
}

/// 1 つの区間について、候補の並びで最初の表記と、それと違う最初の表記（ADR 0044 §2-2）
#[derive(Clone, Copy)]
struct SpanRivals<'a> {
    first: SpanSeg<'a>,
    first_other: Option<SpanSeg<'a>>,
}

impl<'a> SpanRivals<'a> {
    /// 並びの順に呼ぶ
    fn observe(&mut self, seg: SpanSeg<'a>) {
        if self.first_other.is_none() && seg.surface != self.first.surface {
            self.first_other = Some(seg);
        }
    }

    /// `surface` と違う、並びで最初の表記
    fn rival_of(&self, surface: &str) -> Option<SpanSeg<'a>> {
        if self.first.surface != surface {
            Some(self.first)
        } else {
            self.first_other
        }
    }
}

/// 区間ごとの対抗の表。Rewriter の候補は入れない（`variants` と同じ）。候補の数 × セグメントの数で作る
fn span_rivals(cands: &[Candidate]) -> HashMap<Span, SpanRivals<'_>> {
    let mut table: HashMap<Span, SpanRivals<'_>> = HashMap::new();
    for cand in cands {
        if matches!(cand.origin, CandidateOrigin::Rewriter { .. }) {
            continue;
        }
        let mut start = 0;
        for seg in &cand.segments {
            let end = start + seg.reading.len();
            let sg = SpanSeg {
                surface: &seg.surface,
                pos_id: seg.pos_id,
            };
            table
                .entry((start, end))
                .and_modify(|r| r.observe(sg))
                .or_insert(SpanRivals {
                    first: sg,
                    first_other: None,
                });
            start = end;
        }
    }
    table
}

/// word の share にかける、役割の違いによる係数。品詞名が両方あり `same_word_role` が偽なら `factor`、それ以外は 1.0
fn word_role_factor(own: Option<&str>, rival: Option<&str>, factor: f32) -> f32 {
    match (own, rival) {
        (Some(a), Some(b)) if !same_word_role(a, b) => factor,
        _ => 1.0,
    }
}

/// 候補ごとの加点（正の値）
struct Bonuses {
    /// 全体の加点（`rank` が引く値）
    total: Vec<i32>,
    /// 文脈の特徴量だけの加点（同じく phrase と word に別々に上限をかけた値）
    context_only: Vec<i32>,
}

impl LearningRanker {
    fn bonuses(&self, input: &RankInput<'_>, cands: &[Candidate]) -> Bonuses {
        let mut out = Bonuses {
            total: vec![0; cands.len()],
            context_only: vec![0; cands.len()],
        };
        if input.context.secure || cands.is_empty() || self.store.is_empty() {
            return out;
        }
        // 借りた左文脈（ほかのアプリの文）は、学習の文脈の特徴量にしない（ADR 0015 Q7）。
        // 記録のほうも同じく「文脈なし」で残すので、引き当てが食い違わない。
        let no_context = AnalyzedContext::default();
        let analysis = if input.context.left_borrowed {
            &no_context
        } else {
            input.analysis
        };
        let feats = extract_named(
            analysis,
            self.store.grammar(),
            self.vectors.as_deref(),
            self.store.salt(),
        );
        let mut weights: HashMap<u64, f32> = HashMap::new();
        let mut unconditional: HashSet<u64> = HashSet::new();
        for f in &feats.features {
            *weights.entry(f.hash).or_default() += f.weight;
            if f.kind == FeatureKind::Unconditional {
                unconditional.insert(f.hash);
            }
        }
        if weights.is_empty() {
            return out;
        }
        let today = self.cfg.today.unwrap_or_else(today_unix_day);
        // 表ごとに、今の文脈の記録がなければ back-off の重み、あれば通常の重み
        let uncond_weight_of = |table: &ShareTable| {
            if table.has_context_record(&unconditional) {
                self.cfg.unconditional_weight
            } else {
                self.cfg.unconditional_backoff_weight
            }
        };

        let phrase_hits = self.store.lookup(input.reading);
        let phrase_table =
            ShareTable::build(&phrase_hits, Unit::Phrase, &weights, &self.cfg, today);
        let phrase_uncond_weight = uncond_weight_of(&phrase_table);
        let mut word_tables: HashMap<String, (ShareTable, f32)> = HashMap::new();
        let input_word_table =
            ShareTable::build(&phrase_hits, Unit::Word, &weights, &self.cfg, today);
        let input_word_weight = uncond_weight_of(&input_word_table);
        word_tables.insert(
            input.reading.to_string(),
            (input_word_table, input_word_weight),
        );

        // 候補の間で表記が分かれているセグメント（同じ読みに 2 通り以上の表記がある）だけに word の加点をする。
        // どの候補にも同じ表記で現れるセグメントは、順位を決めず、上限だけを歪めるため。
        let mut variants: HashMap<String, HashSet<String>> = HashMap::new();
        for cand in cands.iter() {
            if matches!(cand.origin, CandidateOrigin::Rewriter { .. }) {
                continue;
            }
            for seg in &cand.segments {
                variants
                    .entry(seg.reading.clone())
                    .or_default()
                    .insert(seg.surface.clone());
            }
        }

        // 役割が違う対抗と競うときの係数（1.0 のときは表を作らず、今と同じ動きにする）
        let rivals = (self.cfg.word_role_mismatch_factor < 1.0).then(|| span_rivals(cands));

        let base = self.cfg.base_bonus as f32;
        let cap = i64::from(self.cfg.max_total.max(0));
        // share -> 加点（四捨五入して 0..=cap）
        let capped = |share: f32| ((share * base).round() as i64).clamp(0, cap);
        for (i, cand) in cands.iter().enumerate() {
            let (phrase_key, is_rewriter) = match cand.origin {
                CandidateOrigin::Rewriter { name, format } => (rewriter_key(name, format), true),
                _ => (cand.surface.clone(), false),
            };
            let phrase = phrase_table.weighted_share(
                &phrase_key,
                &weights,
                &unconditional,
                phrase_uncond_weight,
                self.cfg.prior_count,
            );
            let mut word = Share::default();
            if !is_rewriter {
                let mut seg_start = 0;
                for seg in &cand.segments {
                    let span = (seg_start, seg_start + seg.reading.len());
                    seg_start = span.1;
                    let own_pos = input.grammar.pos_name(seg.pos_id);
                    let content = own_pos.is_some_and(is_content_word);
                    let contested = variants.get(&seg.reading).is_some_and(|v| v.len() >= 2);
                    if !content || !contested {
                        continue;
                    }
                    let (table, uncond_weight) =
                        word_tables.entry(seg.reading.clone()).or_insert_with(|| {
                            let hits = self.store.lookup(&seg.reading);
                            let table =
                                ShareTable::build(&hits, Unit::Word, &weights, &self.cfg, today);
                            let w = uncond_weight_of(&table);
                            (table, w)
                        });
                    let role_factor = rivals
                        .as_ref()
                        .and_then(|r| r.get(&span))
                        .and_then(|r| r.rival_of(&seg.surface))
                        .map_or(1.0, |rival| {
                            word_role_factor(
                                own_pos,
                                input.grammar.pos_name(rival.pos_id),
                                self.cfg.word_role_mismatch_factor,
                            )
                        });
                    word.add(
                        table
                            .weighted_share(
                                &seg.surface,
                                &weights,
                                &unconditional,
                                *uncond_weight,
                                self.cfg.prior_count,
                            )
                            .scaled(self.cfg.word_factor * role_factor),
                    );
                }
            }
            out.total[i] = (capped(phrase.context + phrase.unconditional)
                + capped(word.context + word.unconditional)) as i32;
            out.context_only[i] = (capped(phrase.context) + capped(word.context)) as i32;
        }
        out
    }
}

impl Ranker for LearningRanker {
    fn rank(&self, input: &RankInput<'_>, cands: &mut [Candidate]) {
        let bonuses = self.bonuses(input, cands).total;
        for (cand, bonus) in cands.iter_mut().zip(bonuses) {
            cand.score = cand.score.saturating_sub(bonus);
        }
    }

    fn context_free_bonus(&self, input: &RankInput<'_>, cands: &[Candidate]) -> Option<Vec<i32>> {
        // 文脈の特徴量だけで説明できる分は、文脈の証拠のある加点なので Guard に渡さない（ADR 0013 F1）
        let b = self.bonuses(input, cands);
        Some(
            b.total
                .iter()
                .zip(&b.context_only)
                .map(|(all, ctx)| (all - ctx).max(0))
                .collect(),
        )
    }

    fn name(&self) -> &str {
        names::LEARNING
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::AnalyzedContext;
    use crate::candidate::Segment;
    use crate::context::Context;
    use crate::converter::CommitEvent;
    use crate::dict::{DenseGrammar, InMemoryDictionary, UnknownParams, WordId};
    use crate::user::features::test_support::{analysis, token};
    use crate::user::{ContextFeatures, LayerSel, SegmentKey, extract};

    const READING: &str = "はじめてしった";
    const TODAY: u32 = 20_000;
    const WORK_VERB: &str = "動詞,一般,*,*,五段-ラ行,終止形-一般";

    struct Fixture {
        grammar: Arc<DenseGrammar>,
        dict: InMemoryDictionary,
        adv: u16,
        verb: u16,
        noun: u16,
        particle: u16,
    }

    fn fixture() -> Fixture {
        let mut grammar = DenseGrammar::new(
            2,
            2,
            0,
            UnknownParams {
                left_id: 1,
                right_id: 1,
                pos_id: 0,
                cost_per_char: 1,
            },
        );
        let adv = grammar.add_pos("副詞,*,*,*,*,*");
        let verb = grammar.add_pos(WORK_VERB);
        let noun = grammar.add_pos("名詞,普通名詞,一般,*,*,*");
        let particle = grammar.add_pos("助詞,格助詞,*,*,*,*");
        Fixture {
            grammar: Arc::new(grammar),
            dict: InMemoryDictionary::new(0),
            adv,
            verb,
            noun,
            particle,
        }
    }

    fn seg(surface: &str, reading: &str, pos_id: u16) -> Segment {
        Segment {
            reading: reading.to_string(),
            surface: surface.to_string(),
            word: Some(WordId { dict: 0, index: 0 }),
            left_id: 1,
            right_id: 1,
            pos_id,
        }
    }

    fn cand(segs: Vec<Segment>, base: i32) -> Candidate {
        Candidate {
            surface: segs.iter().map(|s| s.surface.as_str()).collect(),
            segments: segs,
            base_cost: base,
            score: base,
            origin: CandidateOrigin::Dictionary,
        }
    }

    /// 初めて知った（副詞 + 動詞）と 始めて知った（動詞 + 動詞）。ベースは 初めて が 2000 有利
    fn pool(f: &Fixture) -> Vec<Candidate> {
        vec![
            cand(
                vec![
                    seg("初めて", "はじめて", f.adv),
                    seg("知った", "しった", f.verb),
                ],
                1000,
            ),
            cand(
                vec![
                    seg("始めて", "はじめて", f.verb),
                    seg("知った", "しった", f.verb),
                ],
                3000,
            ),
        ]
    }

    fn work_context(f: &Fixture) -> AnalyzedContext {
        analysis(vec![
            token("作業", true, f.noun, None),
            token("を", false, f.particle, None),
        ])
    }

    fn story_context(f: &Fixture) -> AnalyzedContext {
        analysis(vec![
            token("その", false, f.particle, None),
            token("話", true, f.noun, None),
            token("は", false, f.particle, None),
        ])
    }

    fn open_store(dir: &tempfile::TempDir, f: &Fixture) -> Arc<LearningStore> {
        let store = LearningStore::open(&dir.path().join("learning.tsv")).unwrap();
        store.attach_grammar(f.grammar.clone());
        Arc::new(store)
    }

    fn commit(c: &Candidate, ctx: AnalyzedContext) -> CommitEvent {
        CommitEvent {
            reading: READING.to_string(),
            candidate: c.clone(),
            analysis: Arc::new(ctx),
        }
    }

    fn rank(
        ranker: &LearningRanker,
        f: &Fixture,
        analysis: &AnalyzedContext,
        secure: bool,
        mut cands: Vec<Candidate>,
    ) -> Vec<Candidate> {
        let ctx = Context::new().with_secure(secure);
        let input = RankInput {
            reading: READING,
            context: &ctx,
            grammar: &*f.grammar,
            analysis,
            dict: &f.dict,
            deadline: None,
        };
        ranker.rank(&input, &mut cands);
        cands.sort_by_key(|c| c.score);
        cands
    }

    fn top(cands: &[Candidate]) -> &str {
        &cands[0].surface
    }

    fn score_of(cands: &[Candidate], surface: &str) -> i32 {
        cands.iter().find(|c| c.surface == surface).unwrap().score
    }

    fn cfg() -> LearningConfig {
        LearningConfig {
            today: Some(TODAY),
            ..LearningConfig::default()
        }
    }

    fn no_backoff_cfg() -> LearningConfig {
        LearningConfig {
            unconditional_backoff_weight: LearningConfig::default().unconditional_weight,
            ..cfg()
        }
    }

    /// `始めて知った` に付く加点（ベース 3000 からの差）
    fn learned_bonus(
        store: &Arc<LearningStore>,
        f: &Fixture,
        cfg: LearningConfig,
        analysis: &AnalyzedContext,
    ) -> i32 {
        let ranker = LearningRanker::new(Arc::clone(store), None, cfg);
        let r = rank(&ranker, f, analysis, false, pool(f));
        3000 - score_of(&r, "始めて知った")
    }

    fn store_with_work_commits(
        f: &Fixture,
        dir: &tempfile::TempDir,
        n: usize,
    ) -> Arc<LearningStore> {
        let store = open_store(dir, f);
        let p = pool(f);
        for _ in 0..n {
            store.record(&commit(&p[1], work_context(f)), None, TODAY);
        }
        store
    }

    #[test]
    fn backoff_raises_unconditional_only_when_the_context_has_no_record() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_work_commits(&f, &dir, 1);
        let strong = LearningConfig {
            unconditional_weight: 1.0,
            unconditional_backoff_weight: 1.0,
            ..cfg()
        };

        // 記録のない文脈（その話は）: back-off で Unconditional の重みが 1.0 になる
        let story = story_context(&f);
        let backoff = learned_bonus(&store, &f, cfg(), &story);
        assert_eq!(backoff, learned_bonus(&store, &f, strong.clone(), &story));
        assert!(backoff > learned_bonus(&store, &f, no_backoff_cfg(), &story) * 3);
        // 左文脈が空でも同じ（Unconditional の特徴量しかない）
        let empty = AnalyzedContext::default();
        assert_eq!(
            learned_bonus(&store, &f, cfg(), &empty),
            learned_bonus(&store, &f, strong, &empty)
        );

        // 記録のある文脈（同じ文脈）: 0.2 のまま
        let work = work_context(&f);
        assert_eq!(
            learned_bonus(&store, &f, cfg(), &work),
            learned_bonus(&store, &f, no_backoff_cfg(), &work)
        );
    }

    fn rank_with(
        ranker: &LearningRanker,
        f: &Fixture,
        analysis: &AnalyzedContext,
        borrowed: bool,
    ) -> Vec<Candidate> {
        let ctx = Context::new().with_left_borrowed(borrowed);
        let input = RankInput {
            reading: READING,
            context: &ctx,
            grammar: &*f.grammar,
            analysis,
            dict: &f.dict,
            deadline: None,
        };
        let mut cands = pool(f);
        ranker.rank(&input, &mut cands);
        cands.sort_by_key(|c| c.score);
        cands
    }

    #[test]
    fn borrowed_left_context_is_ranked_as_no_context() {
        // ADR 0015 Q7: 借りた文脈は、引き当てで「文脈なし」（左文脈が空のときと同じ）になる
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_work_commits(&f, &dir, 3);
        let ranker = LearningRanker::new(Arc::clone(&store), None, cfg());
        let empty = rank_with(&ranker, &f, &AnalyzedContext::default(), false);
        let scores = |r: &[Candidate]| -> Vec<(String, i32)> {
            r.iter().map(|c| (c.surface.clone(), c.score)).collect()
        };
        // 記録と同じ文脈（作業を）でも、借りたものなら空の文脈と同じ点
        let borrowed_same = rank_with(&ranker, &f, &work_context(&f), true);
        assert_eq!(scores(&borrowed_same), scores(&empty));
        let borrowed_other = rank_with(&ranker, &f, &story_context(&f), true);
        assert_eq!(scores(&borrowed_other), scores(&empty));
        // 借りていなければ、文脈の効きは今までどおり（同じ文脈のほうが加点が大きい）
        let own = rank_with(&ranker, &f, &work_context(&f), false);
        assert_eq!(
            scores(&own),
            scores(&rank(&ranker, &f, &work_context(&f), false, pool(&f)))
        );
        assert!(score_of(&own, "始めて知った") < score_of(&empty, "始めて知った"));
    }

    #[test]
    fn borrowed_context_gets_the_backoff_weight() {
        // 文脈なしの記録がなければ back-off の重み（ADR 0013）。借りた文脈でも同じ
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_work_commits(&f, &dir, 1);
        let ranker = LearningRanker::new(Arc::clone(&store), None, cfg());
        let strong = LearningRanker::new(
            store,
            None,
            LearningConfig {
                unconditional_weight: 1.0,
                unconditional_backoff_weight: 1.0,
                ..cfg()
            },
        );
        let a = rank_with(&ranker, &f, &work_context(&f), true);
        let b = rank_with(&strong, &f, &work_context(&f), true);
        assert_eq!(score_of(&a, "始めて知った"), score_of(&b, "始めて知った"));
    }

    #[test]
    fn recording_without_context_leaves_only_unconditional_rows() {
        // 借りた文脈の確定は、Converter が空の解析で CommitEvent を作る。その行は Unconditional（hash 0）だけ
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(&dir, &f);
        let p = pool(&f);
        store.record(&commit(&p[1], AnalyzedContext::default()), None, TODAY);
        let rows = store.records();
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|r| r.feature == 0));
    }

    #[test]
    fn backoff_is_off_when_the_weights_are_equal_and_other_readings_do_not_count() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_work_commits(&f, &dir, 1);
        // 別の読みの記録は、この読みの back-off の判断に使わない（今の文脈と一致する記録があっても）
        let single = cand(vec![seg("別", "べつのよみ", f.verb)], 3000);
        let mut other = commit(&single, story_context(&f));
        other.reading = "べつのよみ".to_string();
        store.record(&other, None, TODAY);
        let story = story_context(&f);
        assert!(
            learned_bonus(&store, &f, cfg(), &story)
                > learned_bonus(&store, &f, no_backoff_cfg(), &story)
        );
    }

    #[test]
    fn learned_surface_survives_the_guard_value_after_the_cap() {
        // F1: 同じ文脈で何度も選んで上限に届いた候補は、文脈によらない部分が 0（Guard で戻されない）
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_work_commits(&f, &dir, 30);
        let capped = LearningConfig {
            max_total: 500,
            ..cfg()
        };
        let ranker = LearningRanker::new(Arc::clone(&store), None, capped);
        let ctx = Context::new();
        let analysis = work_context(&f);
        let input = RankInput {
            reading: READING,
            context: &ctx,
            grammar: &*f.grammar,
            analysis: &analysis,
            dict: &f.dict,
            deadline: None,
        };
        let free = ranker.context_free_bonus(&input, &pool(&f)).unwrap();
        assert_eq!(free, vec![0, 0]);
        // 別の文脈では、文脈によらない加点がそのまま Guard に渡る
        let story = story_context(&f);
        let input = RankInput {
            analysis: &story,
            ..input
        };
        let free = ranker.context_free_bonus(&input, &pool(&f)).unwrap();
        assert!(free[1] > 0);
    }

    #[test]
    fn cap_is_applied_to_phrase_and_word_separately() {
        // F2: 上限は phrase と word に別々にかかる（合計は 2 倍まで）
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_work_commits(&f, &dir, 30);
        let capped = LearningConfig {
            max_total: 500,
            ..cfg()
        };
        assert_eq!(
            learned_bonus(&store, &f, capped, &work_context(&f)),
            1000,
            "phrase 500 + word 500"
        );
    }

    #[test]
    fn a02_learning_does_not_override_other_contexts() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(&dir, &f);
        // back-off（ADR 0013）なしの重み。別の文脈で記録がなければ back-off がかかる（下の別のテスト）
        let ranker = LearningRanker::new(Arc::clone(&store), None, no_backoff_cfg());
        let p = pool(&f);

        // 学習前: ベースの順位どおり
        let before = rank(&ranker, &f, &story_context(&f), false, p.clone());
        assert_eq!(top(&before), "初めて知った");

        // 作業の文脈で 始めて知った を 3 回選ぶ
        for _ in 0..3 {
            store.record(&commit(&p[1], work_context(&f)), None, TODAY);
        }
        // 同じ文脈では、学習した方が勝つ
        let same = rank(&ranker, &f, &work_context(&f), false, p.clone());
        assert_eq!(top(&same), "始めて知った");
        // 別の文脈（その話は）では、初めて知った のまま（a-02）
        let other = rank(&ranker, &f, &story_context(&f), false, p.clone());
        assert_eq!(top(&other), "初めて知った");
        // 文脈なしでも元の順位
        let none = rank(&ranker, &f, &AnalyzedContext::default(), false, p.clone());
        assert_eq!(top(&none), "初めて知った");
        // 別の文脈での加点は、文脈の一致した場合よりずっと小さい
        let bonus = |r: &[Candidate]| 3000 - score_of(r, "始めて知った");
        assert!(bonus(&other) > 0, "弱い無条件の加点はある");
        assert!(
            bonus(&other) * 3 < bonus(&same),
            "{} vs {}",
            bonus(&other),
            bonus(&same)
        );
    }

    #[test]
    fn context_free_bonus_is_only_the_unconditional_part() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(&dir, &f);
        let ranker = LearningRanker::new(Arc::clone(&store), None, cfg());
        let p = pool(&f);
        for _ in 0..3 {
            store.record(&commit(&p[1], work_context(&f)), None, TODAY);
        }
        let ctx = Context::new();
        let free_in = |analysis: &AnalyzedContext| {
            let input = RankInput {
                reading: READING,
                context: &ctx,
                grammar: &*f.grammar,
                analysis,
                dict: &f.dict,
                deadline: None,
            };
            ranker.context_free_bonus(&input, &p).unwrap()
        };
        // 同じ文脈: 全体の加点の一部だけが「文脈に依らない」部分
        let same = free_in(&work_context(&f));
        let total = rank(&ranker, &f, &work_context(&f), false, p.clone());
        let total_bonus = 3000 - score_of(&total, "始めて知った");
        assert!(
            same[1] > 0 && same[1] * 3 < total_bonus,
            "{same:?} {total_bonus}"
        );
        assert_eq!(same[0], 0, "選ばれていない候補には付かない");
        // 別の文脈: 全体の加点は文脈に依らない部分だけ（同じ値）
        let other_ctx = story_context(&f);
        let other = free_in(&other_ctx);
        let all = rank(&ranker, &f, &other_ctx, false, p.clone());
        assert_eq!(other[1], 3000 - score_of(&all, "始めて知った"));
        // secure は 0
        let secure_ctx = Context::new().with_secure(true);
        let input = RankInput {
            reading: READING,
            context: &secure_ctx,
            grammar: &*f.grammar,
            analysis: &other_ctx,
            dict: &f.dict,
            deadline: None,
        };
        assert_eq!(ranker.context_free_bonus(&input, &p).unwrap(), vec![0, 0]);
    }

    #[test]
    fn learning_survives_a_save_and_reload() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(&dir, &f);
        let p = pool(&f);
        for _ in 0..3 {
            store.record(&commit(&p[1], work_context(&f)), None, TODAY);
        }
        store.save().unwrap();
        let reloaded = open_store(&dir, &f);
        assert_eq!(reloaded.salt(), store.salt());
        let ranker = LearningRanker::new(reloaded, None, cfg());
        let r = rank(&ranker, &f, &work_context(&f), false, p);
        assert_eq!(top(&r), "始めて知った");
    }

    #[test]
    fn secure_context_gets_no_bonus() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(&dir, &f);
        let ranker = LearningRanker::new(Arc::clone(&store), None, cfg());
        let p = pool(&f);
        for _ in 0..3 {
            store.record(&commit(&p[1], AnalyzedContext::default()), None, TODAY);
        }
        let normal = rank(&ranker, &f, &AnalyzedContext::default(), false, p.clone());
        assert!(normal.iter().any(|c| c.score < c.base_cost));
        let secure = rank(&ranker, &f, &AnalyzedContext::default(), true, p);
        assert!(secure.iter().all(|c| c.score == c.base_cost));
    }

    #[test]
    fn other_readings_are_untouched() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(&dir, &f);
        let p = pool(&f);
        let single = cand(vec![seg("始めて", "べつのよみ", f.verb)], 3000);
        let mut other = commit(&single, work_context(&f));
        other.reading = "べつのよみ".to_string();
        store.record(&other, None, TODAY);
        let ranker = LearningRanker::new(store, None, cfg());
        let r = rank(&ranker, &f, &work_context(&f), false, p);
        assert!(r.iter().all(|c| c.score == c.base_cost));
    }

    #[test]
    fn initial_layer_is_weaker_and_resettable_and_removable_by_source() {
        let f = fixture();
        let p = pool(&f);
        let ctx = work_context(&f);

        let dir = tempfile::tempdir().unwrap();
        let store = open_store(&dir, &f);
        let ranker = LearningRanker::new(Arc::clone(&store), None, cfg());
        let feats: ContextFeatures = extract(&ctx, None, store.salt());
        let key = SegmentKey {
            surface: "始めて".to_string(),
            reading: "はじめて".to_string(),
            pos_name: WORK_VERB.to_string(),
        };
        store.add_initial(7, "はじめて", std::slice::from_ref(&key), &feats, TODAY);

        let usage_dir = tempfile::tempdir().unwrap();
        let usage_store = open_store(&usage_dir, &f);
        let usage_ranker = LearningRanker::new(Arc::clone(&usage_store), None, cfg());
        usage_store.record(&commit(&p[1], work_context(&f)), None, TODAY);

        let initial_score = score_of(&rank(&ranker, &f, &ctx, false, p.clone()), "始めて知った");
        let usage_score = score_of(
            &rank(&usage_ranker, &f, &ctx, false, p.clone()),
            "始めて知った",
        );
        assert!(initial_score < 3000, "取り込みの記録も効く");
        assert!(usage_score < initial_score, "usage の方が強い");

        let now =
            |s: &LearningRanker| score_of(&rank(s, &f, &ctx, false, p.clone()), "始めて知った");
        store.remove_source(7);
        assert_eq!(now(&ranker), 3000);
        store.add_initial(7, "はじめて", std::slice::from_ref(&key), &feats, TODAY);
        store.record(&commit(&p[1], work_context(&f)), None, TODAY);
        store.reset(LayerSel::Usage);
        assert!(now(&ranker) < 3000, "usage のリセットで initial は残る");
        store.reset(LayerSel::Initial);
        assert_eq!(now(&ranker), 3000);
    }

    #[test]
    fn bonus_decays_with_age_and_is_capped() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(&dir, &f);
        let p = pool(&f);
        for _ in 0..3 {
            store.record(&commit(&p[1], work_context(&f)), None, TODAY);
        }
        let bonus_at = |today: u32, cfg: LearningConfig| {
            let c = LearningConfig {
                today: Some(today),
                ..cfg
            };
            let r = rank(
                &LearningRanker::new(Arc::clone(&store), None, c),
                &f,
                &work_context(&f),
                false,
                p.clone(),
            );
            3000 - score_of(&r, "始めて知った")
        };
        let weak = LearningConfig {
            base_bonus: 300,
            ..LearningConfig::default()
        };
        let fresh = bonus_at(TODAY, weak.clone());
        let aged = bonus_at(TODAY + 90, weak.clone());
        let ancient = bonus_at(TODAY + 900, weak);
        assert!(fresh > 0 && aged > 0 && ancient < aged);
        assert!(aged < fresh, "{fresh} {aged}");
        let capped = bonus_at(
            TODAY,
            LearningConfig {
                max_total: 500,
                ..LearningConfig::default()
            },
        );
        // 上限は phrase と word の加点にそれぞれかかる
        assert_eq!(capped, 500 * 2);
    }

    #[test]
    fn rewriter_candidates_are_learned_by_format() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(&dir, &f);
        let mk = |surface: &str, format: u16| Candidate {
            surface: surface.to_string(),
            segments: vec![seg(surface, "きょう", f.noun)],
            base_cost: 1000,
            score: 1000,
            origin: CandidateOrigin::Rewriter {
                name: "date",
                format,
            },
        };
        let ctx = work_context(&f);
        let mut ev = commit(&mk("2026年10月11日", 2), ctx.clone());
        ev.reading = "きょう".to_string();
        for _ in 0..3 {
            store.record(&ev, None, TODAY);
        }
        // 日付が変わって表記が違っても、書式 2 が上がる
        let mut cands = vec![mk("2026/10/12", 1), mk("2026年10月12日", 2)];
        let ranker = LearningRanker::new(store, None, cfg());
        let c = Context::new();
        let input = RankInput {
            reading: "きょう",
            context: &c,
            grammar: &*f.grammar,
            analysis: &ctx,
            dict: &f.dict,
            deadline: None,
        };
        ranker.rank(&input, &mut cands);
        assert_eq!(cands[0].score, 1000);
        assert!(cands[1].score < 1000);
    }

    #[test]
    fn from_kv_reads_only_learning_keys() {
        let mut kv = BTreeMap::new();
        kv.insert("learning.base_bonus".to_string(), "1500".to_string());
        kv.insert("learning.today".to_string(), "20001".to_string());
        kv.insert(
            "learning.unconditional_backoff_weight".to_string(),
            "0.6".to_string(),
        );
        kv.insert("profile.k".to_string(), "9".to_string());
        kv.insert("unrelated".to_string(), "x".to_string());
        let c = LearningConfig::from_kv(&kv).unwrap();
        assert_eq!(c.base_bonus, 1500);
        assert_eq!(c.today, Some(20_001));
        assert_eq!(c.unconditional_backoff_weight, 0.6);
        assert_eq!(c.unconditional_weight, 0.2);
        assert_eq!(c.max_total, LearningConfig::default().max_total);

        kv.insert("learning.nope".to_string(), "1".to_string());
        assert_eq!(
            LearningConfig::from_kv(&kv).unwrap_err().key,
            "learning.nope"
        );
        kv.remove("learning.nope");
        kv.insert("learning.max_total".to_string(), "many".to_string());
        assert!(LearningConfig::from_kv(&kv).is_err());
        assert_eq!(
            LearningConfig::from_kv(&BTreeMap::new()).unwrap(),
            LearningConfig::default()
        );
    }

    fn mismatch_cfg(factor: f32) -> LearningConfig {
        LearningConfig {
            word_role_mismatch_factor: factor,
            ..cfg()
        }
    }

    #[test]
    fn mismatch_factor_one_is_the_same_as_before() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_work_commits(&f, &dir, 3);
        let ctx = work_context(&f);
        assert_eq!(LearningConfig::default().word_role_mismatch_factor, 1.0);
        let base = learned_bonus(&store, &f, cfg(), &ctx);
        assert!(base > 0);
        assert_eq!(learned_bonus(&store, &f, mismatch_cfg(1.0), &ctx), base);
    }

    #[test]
    fn mismatch_factor_weakens_only_the_word_bonus_against_a_different_role() {
        // 始めて（動詞）は、同じ区間のいちばん手前の対抗 初めて（副詞）と役割が違う
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_work_commits(&f, &dir, 3);
        let ctx = work_context(&f);
        let full = learned_bonus(&store, &f, cfg(), &ctx);
        let half = learned_bonus(&store, &f, mismatch_cfg(0.5), &ctx);
        let zero = learned_bonus(&store, &f, mismatch_cfg(0.0), &ctx);
        let no_word = LearningConfig {
            word_factor: 0.0,
            ..cfg()
        };
        assert!(zero < half && half < full, "{zero} {half} {full}");
        // 係数 0 は word の学習を切ったのと同じ（phrase の加点は残る）
        assert_eq!(zero, learned_bonus(&store, &f, no_word, &ctx));
        assert!(zero > 0);
    }

    #[test]
    fn mismatch_factor_is_not_applied_against_the_same_role() {
        // 初めて/始めて をどちらも動詞にすると、役割が同じなので弱めない
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(&dir, &f);
        let mut p = pool(&f);
        p[0].segments[0].pos_id = f.verb;
        for _ in 0..3 {
            store.record(&commit(&p[1], work_context(&f)), None, TODAY);
        }
        let bonus = |cfg: LearningConfig| {
            let ranker = LearningRanker::new(Arc::clone(&store), None, cfg);
            let r = rank(&ranker, &f, &work_context(&f), false, p.clone());
            3000 - score_of(&r, "始めて知った")
        };
        assert_eq!(bonus(mismatch_cfg(0.0)), bonus(cfg()));
    }

    #[test]
    fn mismatch_factor_is_not_applied_without_a_rival_in_the_span() {
        // 同じ区間に違う表記がなければ、対抗はない
        let f = fixture();
        let only = cand(
            vec![seg("始", "はじ", f.noun), seg("知った", "しった", f.verb)],
            0,
        );
        let rivals = span_rivals(std::slice::from_ref(&only));
        assert!(rivals.values().all(|r| r.first_other.is_none()));
        assert!(
            rivals
                .values()
                .all(|r| r.rival_of(r.first.surface).is_none())
        );
    }

    #[test]
    fn rival_is_the_first_different_surface_in_slice_order() {
        let f = fixture();
        let noun = |s: &str| cand(vec![seg(s, "し", f.noun)], 0);
        let verb = |s: &str| cand(vec![seg(s, "し", f.verb)], 0);
        let cands = vec![noun("死"), verb("為"), noun("氏"), noun("死")];
        let rivals = span_rivals(&cands);
        let r = rivals[&(0, "し".len())];
        // 死 の対抗は、最初の違う表記 為（動詞）。氏 と 為 の対抗は、いちばん手前の 死
        assert_eq!(r.rival_of("死").unwrap().surface, "為");
        assert_eq!(r.rival_of("氏").unwrap().surface, "死");
        assert_eq!(r.rival_of("為").unwrap().surface, "死");
        assert_eq!(r.rival_of("死").unwrap().pos_id, f.verb);
    }

    #[test]
    fn rewriter_candidates_are_not_rivals() {
        let f = fixture();
        let mut rewriter = cand(vec![seg("1", "し", f.noun)], 0);
        rewriter.origin = CandidateOrigin::Rewriter {
            name: "x",
            format: 0,
        };
        let cands = vec![rewriter, cand(vec![seg("死", "し", f.noun)], 0)];
        let rivals = span_rivals(&cands);
        let r = rivals[&(0, "し".len())];
        assert_eq!(r.first.surface, "死");
        assert!(r.first_other.is_none());
    }

    #[test]
    fn role_factor_depends_on_the_pos_names() {
        const NOUN: &str = "名詞,普通名詞,一般,*,*,*";
        assert_eq!(word_role_factor(Some(WORK_VERB), Some(NOUN), 0.25), 0.25);
        assert_eq!(word_role_factor(Some(NOUN), Some(NOUN), 0.25), 1.0);
        assert_eq!(word_role_factor(Some(NOUN), None, 0.25), 1.0);
        assert_eq!(word_role_factor(None, Some(NOUN), 0.25), 1.0);
    }

    #[test]
    fn mismatch_factor_must_be_in_range() {
        for (value, ok) in [
            ("0", true),
            ("0.5", true),
            ("1.0", true),
            ("1.5", false),
            ("-0.1", false),
        ] {
            let mut kv = BTreeMap::new();
            kv.insert(
                "learning.word_role_mismatch_factor".to_string(),
                value.to_string(),
            );
            assert_eq!(LearningConfig::from_kv(&kv).is_ok(), ok, "{value}");
        }
    }
}
