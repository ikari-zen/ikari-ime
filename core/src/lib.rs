//! ikari-core: 変換エンジン（辞書、ラティス、Ranker、入力）。ADR 0001 §4、ADR 0002。

pub mod analysis;
pub mod candidate;
pub mod context;
pub mod converter;
pub mod dict;
pub mod emoji;
pub mod pos;
pub mod postal;
pub mod ranker;
pub mod rewriter;
pub mod user;
pub mod vectors;

mod lattice;
mod nbest;

pub use analysis::{
    AnalyzedContext, CONTEXT_WINDOW_CHARS, ContextAnalyzer, ContextToken, LatticeContextAnalyzer,
    NullAnalyzer,
};
pub use candidate::{Candidate, CandidateOrigin, Segment};
pub use context::{Commit, Context};
pub use converter::{
    AdverbNounRule, CommitEvent, Conversion, ConvertError, ConvertOptions, Converter,
    ConverterBuilder, KatakanaLikeRule, LmBudget, NounSuffixRule, NounSuruRule, NumeralCounterRule,
    NumeralCoverRule, Page, SegmentChoice, SegmentCommit, SegmentError, SegmentLayout, SegmentSpan,
    Segmented, SegmentedOrigin, VolitionalRule,
};
pub use dict::{
    ConjRule, DenseGrammar, DictError, Dictionary, DictionarySet, Grammar, GrammarFile,
    IME_EXCLUDED, InMemoryDictionary, Lemma, LoadedDict, PosInfo, SystemDictionary, UnknownParams,
    VECTORS_FILE_NAME, WordEntry, WordId, open_dir,
};
pub use emoji::{EMOJI_TSV_FILE_NAME, EmojiError, EmojiTable};
pub use postal::{POSTAL_TSV_FILE_NAME, PostalError, PostalMatch, PostalTable};
pub use ranker::{
    CompositeRanker, CostRanker, DocumentCacheConfig, DocumentCacheRanker, KanaStyleConfig,
    KanaStyleParseError, KanaStyleRanker, KanaStyleTable, LearningConfig, LearningRanker,
    LeftContextConfig, LeftContextRanker, ProfileConfig, ProfileRanker, RankInput, Ranker,
    RankerConfigError, TopicConfig, TopicRanker,
};
pub use rewriter::{
    CalculatorRewriter, Clock, DateOptions, DateRewriter, EmoticonRewriter, FixedClock,
    LocalDateTime, NumberRewriter, PostalOptions, PostalRewriter, RewriteCandidate, RewriteInput,
    RewritePosition, Rewriter, SymbolRewriter, SystemClock, TableParseError, UnicodeOptions,
    UnicodeRewriter, WeekdayRewriter,
};
pub use user::{
    LayerSel, LearningStore, Profile, ProfileDomain, ProfileWeight, UserDataError, UserDictionary,
    UserDir,
};
pub use vectors::{WordVectors, open_vectors};
