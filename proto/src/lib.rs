//! ikari-proto: エンジンと殻の間のメッセージ（`proto/ikari/engine/v1/engine.proto`。ADR 0004 §5.2）と、
//! 殻の入口と shell-core の間のメッセージ（`proto/ikari/shell/v1/shell.proto`。ADR 0041 §4.4）。

/// スキーマの版。`Envelope.schema_version` に入れる。
pub const SCHEMA_VERSION: u32 = 1;

/// `ikari.engine.v1`
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/ikari.engine.v1.rs"));
}

/// `ikari.shell.v1`
pub mod shell {
    pub mod v1 {
        include!(concat!(env!("OUT_DIR"), "/ikari.shell.v1.rs"));
    }
}
