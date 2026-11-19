//! モデルの記述（profile）。型と表は `ikari-resource` にある（ADR 0060 §3、§5。表は 1 つ: `MODEL_PRESETS`）。
//! ここは lm の口として再公開するだけ。

pub use ikari_resource::{
    DEFAULT_LIMITS, LmLimits, ModelProfile, PromptFormat, ZENZ_V3_2_SMALL_PROFILE, profile_by_id,
};

/// zenz-v3.2 small の profile（`ZenzConfig` の既定の元）
pub const ZENZ_V3_2_SMALL: ModelProfile = ZENZ_V3_2_SMALL_PROFILE;
