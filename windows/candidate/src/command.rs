//! TIP → 候補窓の命令の型（`proto/ikari/candidate/v1/candidate.proto`）の入口。
//!
//! 型は proto から生成したもの（`ikari-proto`）をそのまま使う（二重に作らない）。枠は `pipe` の Post（返事なし）。
//! 内容の型（`CandidatePanelModel`、`Rect`、`PanelSide`）は `ikari.shell.v1` のもの。

pub use ikari_proto::candidate::v1::{
    CandidateCommand, Hello, Hide, RenderState, Show, Update, candidate_command,
};
pub use ikari_proto::shell::v1::{CandidatePanelModel, PanelSide, Rect};

/// `Hello.schema_version` に入れる版。互換を壊す変更で上げる。
pub const CANDIDATE_SCHEMA_VERSION: u32 = 1;

/// `Hide.generation` のこの値は「世代を問わず隠す」（TIP が外れるとき、フォーカスが外れたとき）。
pub const ANY_GENERATION: u64 = 0;
