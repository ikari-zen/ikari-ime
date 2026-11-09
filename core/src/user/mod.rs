//! ユーザーデータ（学習、プロファイル、ユーザー辞書。ADR 0002 §8〜§10）。

mod dictionary;
mod dir;
mod error;
pub(crate) mod features;
mod ingest;
mod learning;
mod profile;
mod transfer;
mod util;

pub use dictionary::{
    IT_PACK_ID, MIN_REP_SHARE_PERMILLE, MozcParse, PACK_LATIN_OFFSET, PACK_SCHEMA, PackEntry,
    PackForm, PosCheckReport, PosOption, SkippedLine, USER_DICT_SCHEMA, UserDictEntry,
    UserDictionary, VocabularyPack, builtin_it_pack, check_pos_table, pos_options, verb_conj_types,
};
pub use dir::{
    INGEST_CANDIDATES_FILE_NAME, INGEST_SOURCES_FILE_NAME, LEARNING_FILE_NAME, PROFILE_FILE_NAME,
    USER_DICT_FILE_NAME, USER_DIR_ENV, UserDir, UserDirEnv,
};
pub use error::UserDataError;
pub use features::{ContextFeatures, Feature, FeatureKind, extract, extract_named};
pub use ingest::{IngestCandidate, IngestCandidates, IngestSource, IngestSources, format_utc};
pub use learning::{
    Layer, LayerSel, LearningRecord, LearningStore, MAX_LEARNING_ROWS, RecordPolicy, SegmentKey,
    Unit, rewriter_key,
};
pub use profile::{PROFILE_SCHEMA, Profile, ProfileDomain, ProfileWeight};
pub use transfer::{
    EXPORT_SCHEMA, ExportIssue, ExportIssueKind, ImportSummary, check_export, export_text,
    import_text, validate_export,
};
pub use util::{
    PRIVATE_DIR_MODE, PRIVATE_FILE_MODE, create_private_dir_all, create_private_new, fnv1a64,
    fnv1a64_continue, normalize_reading, tighten_files, today_unix_day,
};

pub use crate::ranker::LearningRanker;
