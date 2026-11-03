//! 文脈の読み取りの量と時間の値（ADR 0015 §10 の決定）。
//!
//! 殻や core は参照しない（エンジンの中だけの値）。

use std::time::Duration;

use ikari_core::{CarryJoint, CarryParams};

/// 休止した（`EndSession` を受けた）入力欄の履歴を残す時間（Q2。D1）
pub const PAUSED_SESSION_TTL: Duration = Duration::from_secs(30 * 60);
/// 同じ入力欄の履歴の量（表記の合計の文字数。Q3、D4。解析の窓 `MAX_CONTEXT_CHARS` と同じ）
pub const HISTORY_MAX_CHARS: usize = 2000;
/// 左が足りないときに同じセッションの履歴から足して目標にする文字数（ADR 0046 E1。0 なら足さない。
/// 契約の既定。掃引で替える。利用者の設定ではない）
pub const CARRY_FILL_CHARS: usize = 0;
/// 足した履歴と左のあいだの区切り（ADR 0046 E3。契約の既定。掃引で替える）
pub const CARRY_JOINT: CarryJoint = CarryJoint::None;

/// 上の値をまとめた型。テストで短い時間に差し替えるためにある（本番は [`ContextLimits::DEFAULT`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextLimits {
    pub paused_session_ttl: Duration,
    pub history_max_chars: usize,
    pub carry: CarryParams,
}

impl ContextLimits {
    pub const DEFAULT: Self = Self {
        paused_session_ttl: PAUSED_SESSION_TTL,
        history_max_chars: HISTORY_MAX_CHARS,
        carry: CarryParams {
            fill_chars: CARRY_FILL_CHARS,
            joint: CARRY_JOINT,
        },
    };
}

impl Default for ContextLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_adr_0015() {
        let d = ContextLimits::default();
        assert_eq!(d.paused_session_ttl, Duration::from_secs(1800));
        assert_eq!(d.history_max_chars, crate::MAX_CONTEXT_CHARS);
        assert_eq!(d.carry.fill_chars, 0);
        assert_eq!(d.carry.joint, CarryJoint::None);
    }
}
