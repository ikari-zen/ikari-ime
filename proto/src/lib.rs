//! ikari-proto: エンジンと殻の間のメッセージ（`proto/ikari/engine/v1/engine.proto`。ADR 0004 §5.2）。

/// スキーマの版。`Envelope.schema_version` に入れる。
pub const SCHEMA_VERSION: u32 = 1;

pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/ikari.engine.v1.rs"));
}
