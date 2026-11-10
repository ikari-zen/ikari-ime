//! リソース（辞書・表・モデル）の束の契約（ADR 0058 §4〜§7、#246-1）。
//!
//! 型と検証だけを持つ。入れ替えの実装（見回り、rename、起動時の回復）はエンジン側（#246-2）。
//! OS に依存しない。エンジン、xtask、後の `ikari-data` が共有する。
//!
//! - `manifest`: manifest.json の型と読み書き（schema の版つき）
//! - `verify`: 束の検証（形、大きさ、sha256、形式の版、既知のモデル、古い版）
//! - `models`: アプリに埋めた既知のモデルの表
//! - `journal`: 入れ替えの手順の記録（`inbox/.journal.json`）
//! - `sha256`: ストリームでの sha256

pub mod error;
pub mod journal;
pub mod manifest;
pub mod models;
pub mod sha256;
pub mod verify;

pub use error::ResourceError;
pub use journal::{JOURNAL_FILE_NAME, Journal, JournalOp, JournalStage};
pub use manifest::{
    Component, ComponentKind, Created, FileEntry, Freshness, MANIFEST_FILE_NAME, MANIFEST_SCHEMA,
    Manifest, Origin,
};
pub use models::{KNOWN_MODELS, KnownModel, ZENZ_SMALL_FILE_NAME, ZENZ_SMALL_SHA256};
pub use sha256::{sha256_file, sha256_reader};
pub use verify::{
    SupportedFormats, TABLE_FORMAT_VERSION, VerifiedBundle, VerifyOptions, verify_bundle,
};
