//! リソース（辞書・表・モデル）の束の契約（ADR 0058 §4〜§7、#246-1）。
//!
//! 型と検証だけを持つ。入れ替えの実装（見回り、rename、起動時の回復）はエンジン側（#246-2）。
//! OS に依存しない。エンジン、xtask、後の `ikari-data` が共有する。
//!
//! - `manifest`: manifest.json の型と読み書き（schema の版つき）
//! - `verify`: 束の検証（形、大きさ、sha256、形式の版、既知のモデル、古い版）
//! - `models`: アプリに埋めた既知のモデルの表
//! - `model_list`: 入っている自由指定のモデルの一覧（`models/MANIFEST.json`。D-4）
//! - `platform`: この OS の製品で自由指定のモデルを出すか（エンジンと画面の出し分けが同じ値を読む）
//! - `journal`: 入れ替えの手順の記録（`inbox/.journal.json`）
//! - `sha256`: ストリームでの sha256
//! - `stage`: 取得・取り込みしたモデルの束を inbox に作る（ADR 0076）
//! - `fetch_report`: `ikari-data` の標準出力の行（ADR 0076 §8）
//! - `app_version`: アプリの版の確認の形と比べ方（ADR 0076 §6、§18.2）

pub mod app_version;
pub mod error;
pub mod fetch_report;
pub mod journal;
pub mod manifest;
pub mod model_list;
pub mod models;
pub mod platform;
pub mod profile;
pub mod sha256;
pub mod stage;
pub mod verify;

pub use error::ResourceError;
pub use journal::{JOURNAL_FILE_NAME, Journal, JournalOp, JournalStage};
pub use manifest::{
    Component, ComponentKind, Created, FileEntry, Freshness, MANIFEST_FILE_NAME, MANIFEST_SCHEMA,
    Manifest, Origin,
};
pub use model_list::{
    MAX_DISPLAY_NAME_CHARS, MODEL_FILE_EXTENSION, MODEL_LIST_FILE_NAME, MODEL_MAX_SIZE, ModelList,
    USER_MODEL_HEX_DIGITS, USER_MODEL_PREFIX, UserModelEntry, UserModelId, sanitize_display_name,
};
pub use models::{
    KNOWN_MODELS, KnownModel, MODEL_PRESETS, ModelPreset, ZENZ_SMALL_FILE_NAME, ZENZ_SMALL_SHA256,
    ZENZ_SMALL_SIZE, ZENZ_SMALL_URL, preset_by_id,
};
pub use profile::{
    DEFAULT_LIMITS, LmLimits, ModelProfile, PromptFormat, ZENZ_V3_2_SMALL_PROFILE, profile_by_id,
};
pub use sha256::{sha256_file, sha256_reader};
pub use stage::{
    Expected, MODEL_COMPONENT, PARTIAL_SUFFIX, Source, StageError, StagePlan, StagedBundle,
    stage_model_bundle,
};
pub use verify::{
    SupportedFormats, TABLE_FORMAT_VERSION, VerifiedBundle, VerifyOptions, verify_bundle,
    verify_model_file, verify_model_open, verify_open_expected,
};
