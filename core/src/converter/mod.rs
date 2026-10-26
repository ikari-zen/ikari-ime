//! Converter 本体。公開シグネチャは契約（ADR 0001 §4.5）、アルゴリズムは §4.6。

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use ikari_dict_format::reading::normalize_key;

use crate::analysis::{AnalyzedContext, ContextAnalyzer, NullAnalyzer};
use crate::context::Context;
use crate::dict::{Dictionary, Grammar};
use crate::lattice;
use crate::nbest::NBest;
use crate::ranker::{CostRanker, Ranker};
use crate::rewriter::{Clock, Rewriter, SystemClock};

mod conversion;
mod katakana_like;
mod options;
mod reconvert;
#[cfg(test)]
mod reconvert_tests;
mod segment;
#[cfg(test)]
mod segment_tests;
#[cfg(test)]
mod tests;

pub use conversion::{CommitEvent, Conversion, Page};
pub use katakana_like::KatakanaLikeRule;
pub use options::{
    AdverbNounRule, ConvertOptions, LmBudget, NounSuffixRule, NounSuruRule, NumeralCounterRule,
    NumeralCoverRule, VolitionalRule,
};
pub use segment::{
    SegmentChoice, SegmentCommit, SegmentError, SegmentLayout, SegmentSpan, Segmented,
    SegmentedOrigin,
};

use options::MAX_PAGE_SIZE;

#[derive(Debug)]
pub enum ConvertError {
    GrammarMismatch { dict: u64, grammar: u64 },
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConvertError::GrammarMismatch { dict, grammar } => write!(
                f,
                "grammar mismatch: dictionary expects grammar_id={dict:#018x}, grammar has {grammar:#018x}"
            ),
        }
    }
}

impl std::error::Error for ConvertError {}

/// 変換器。辞書・文法・Ranker は Arc で共有し、`Converter` 自体は `Send + Sync`。
pub struct Converter {
    dict: Arc<dyn Dictionary>,
    grammar: Arc<dyn Grammar>,
    ranker: Arc<dyn Ranker>,
    opts: ConvertOptions,
    analyzer: Arc<dyn ContextAnalyzer>,
    rewriters: Vec<Arc<dyn Rewriter>>,
    clock: Arc<dyn Clock>,
    /// ラティスの辺の補正（`opts.adverb_noun`、`opts.noun_suru` と品詞の表。ADR 0008、0016）
    edge_rules: lattice::EdgeRules,
}

/// `Converter::builder` の組み立て役。`build` で `grammar_id` を照合する。
pub struct ConverterBuilder {
    dict: Arc<dyn Dictionary>,
    grammar: Arc<dyn Grammar>,
    ranker: Arc<dyn Ranker>,
    analyzer: Arc<dyn ContextAnalyzer>,
    rewriters: Vec<Arc<dyn Rewriter>>,
    clock: Arc<dyn Clock>,
    opts: ConvertOptions,
}

impl ConverterBuilder {
    /// 既定は CostRanker
    pub fn ranker(mut self, r: Arc<dyn Ranker>) -> Self {
        self.ranker = r;
        self
    }

    /// 既定は NullAnalyzer
    pub fn analyzer(mut self, a: Arc<dyn ContextAnalyzer>) -> Self {
        self.analyzer = a;
        self
    }

    /// 既定は空
    pub fn rewriters(mut self, r: Vec<Arc<dyn Rewriter>>) -> Self {
        self.rewriters = r;
        self
    }

    /// 既定は SystemClock
    pub fn clock(mut self, c: Arc<dyn Clock>) -> Self {
        self.clock = c;
        self
    }

    pub fn options(mut self, o: ConvertOptions) -> Self {
        self.opts = o;
        self
    }

    pub fn build(self) -> Result<Converter, ConvertError> {
        Converter::from_builder(self)
    }
}

impl Converter {
    /// 辞書と文法の `grammar_id` が違えば `GrammarMismatch`。
    /// `page_size` は 1..=9、`pool_size` と `chunk_size` は 1 以上に丸める（0 だと進まなくなるため）。
    pub fn new(
        dict: Arc<dyn Dictionary>,
        grammar: Arc<dyn Grammar>,
        ranker: Arc<dyn Ranker>,
        opts: ConvertOptions,
    ) -> Result<Self, ConvertError> {
        Self::builder(dict, grammar)
            .ranker(ranker)
            .options(opts)
            .build()
    }

    /// 組み立て。解析器、Rewriter、時計は既定（NullAnalyzer、なし、SystemClock）。
    pub fn builder(dict: Arc<dyn Dictionary>, grammar: Arc<dyn Grammar>) -> ConverterBuilder {
        ConverterBuilder {
            dict,
            grammar,
            ranker: Arc::new(CostRanker),
            analyzer: Arc::new(NullAnalyzer),
            rewriters: Vec::new(),
            clock: Arc::new(SystemClock),
            opts: ConvertOptions::default(),
        }
    }

    fn from_builder(b: ConverterBuilder) -> Result<Self, ConvertError> {
        let ConverterBuilder {
            dict,
            grammar,
            ranker,
            analyzer,
            rewriters,
            clock,
            opts,
        } = b;
        let (d, g) = (dict.grammar_id(), grammar.grammar_id());
        if d != g {
            return Err(ConvertError::GrammarMismatch {
                dict: d,
                grammar: g,
            });
        }
        let opts = ConvertOptions {
            page_size: opts.page_size.clamp(1, MAX_PAGE_SIZE),
            pool_size: opts.pool_size.max(1),
            chunk_size: opts.chunk_size.max(1),
            max_astar_pops: opts.max_astar_pops,
            rewriter_anchor: opts.rewriter_anchor,
            lm_budget: opts.lm_budget,
            adverb_noun: opts.adverb_noun,
            noun_suru: opts.noun_suru,
            noun_suffix: opts.noun_suffix,
            numeral_cover: opts.numeral_cover,
            volitional: opts.volitional,
            numeral_counter: opts.numeral_counter,
            katakana_like: opts.katakana_like,
        };
        let edge_rules = lattice::EdgeRules::new_with_volitional(
            &*grammar,
            opts.adverb_noun,
            opts.noun_suru,
            opts.noun_suffix,
            opts.numeral_cover,
            opts.volitional,
            opts.numeral_counter,
        );
        Ok(Self {
            dict,
            grammar,
            ranker,
            opts,
            analyzer,
            rewriters,
            clock,
            edge_rules,
        })
    }

    /// reading は normalize_key を通す。空なら候補0件の Conversion を返す。
    ///
    /// 流れ（ADR 0002 §6.2）: 文脈の sanitize -> 左文脈の解析（secure なら解析しない）->
    /// 解析結果の BOS でラティス構築 -> 辞書の候補 -> Rewriter -> かな -> Ranker。
    pub fn convert(&self, reading: &str, ctx: &Context) -> Conversion {
        let key = normalize_key(reading);
        let deadline = Instant::now() + self.opts.lm_budget.for_reading(&key);
        self.convert_until(reading, ctx, Some(deadline))
    }

    /// `convert` の締め切り指定版（ADR 0004 §3.4）。この呼び出しの中の `rank` はすべて同じ `deadline` を使う。
    /// `None` は待たない（LM を使わない）。
    pub fn convert_until(
        &self,
        reading: &str,
        ctx: &Context,
        deadline: Option<Instant>,
    ) -> Conversion {
        let key = normalize_key(reading);
        let ctx = sanitize_context(ctx);
        let analysis = self.analyze(&ctx);
        let lattice = lattice::build_with_starts(
            &key,
            &*self.dict,
            &*self.grammar,
            &analysis.bos_options(),
            &self.edge_rules,
        );
        let nbest = NBest::new(&lattice, &*self.grammar);
        let mut conv = Conversion {
            dict: Arc::clone(&self.dict),
            grammar: Arc::clone(&self.grammar),
            ranker: Arc::clone(&self.ranker),
            rewriters: self.rewriters.clone(),
            clock: Arc::clone(&self.clock),
            opts: self.opts.clone(),
            ctx,
            analysis,
            lattice,
            nbest,
            generated: Vec::new(),
            seen: HashSet::new(),
            exhausted: false,
            deadline,
        };
        if key.is_empty() {
            conv.exhausted = true;
        } else {
            conv.build_first_pool();
        }
        conv
    }

    /// 左文脈を解析する。secure（`ctx` は sanitize 済み）なら解析せず、BOS は `bos_eos_id`。
    /// トークンのない結果（NullAnalyzer など）の BOS も `bos_eos_id` に揃える
    /// （`bos_eos_id != 0` の文法でも、フェーズ0と同じ挙動になるように）。
    fn analyze(&self, ctx: &Context) -> Arc<AnalyzedContext> {
        let bos_eos = self.grammar.bos_eos_id();
        if ctx.secure {
            return Arc::new(AnalyzedContext {
                bos_right_id: bos_eos,
                ..AnalyzedContext::default()
            });
        }
        let analysis = self.analyzer.analyze(&ctx.left_text, &ctx.history);
        if analysis.tokens.is_empty() && analysis.bos_right_id != bos_eos {
            return Arc::new(AnalyzedContext {
                bos_right_id: bos_eos,
                ..(*analysis).clone()
            });
        }
        analysis
    }
}

/// secure（パスワード欄）なら文脈をすべて空にする（NFR-18）。secure のフラグ自体は残す。
fn sanitize_context(ctx: &Context) -> Context {
    if ctx.secure {
        Context::new().with_secure(true)
    } else {
        ctx.clone()
    }
}
