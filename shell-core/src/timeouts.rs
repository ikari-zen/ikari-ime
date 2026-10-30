//! 変換 1 回の待ち時間（ADR 0004 §3.4、§7.5）。今の `LmBudget.swift`（Swift の型名は `EngineTimeouts`）。
//! エンジン側の `LmBudget` と同じ式。

use std::time::Duration;

pub const BASE: Duration = Duration::from_millis(30);
pub const PER_CHARACTER: Duration = Duration::from_millis(6);
pub const MAXIMUM: Duration = Duration::from_millis(300);
/// 殻が `lm_budget` に足す IPC の余裕。
pub const IPC_MARGIN: Duration = Duration::from_millis(40);
/// 管理の要求の待ちの上限（ADR 0005 §8.3）。インポートとエクスポート以外。
pub const ADMIN: Duration = Duration::from_secs(3);
/// インポートとエクスポートの待ちの上限。
pub const ADMIN_TRANSFER: Duration = Duration::from_secs(30);
/// 住所の問い合わせの待ちの上限。値は暫定（実測が要る。ADR 0032 §9.1）。
pub const POSTAL_LOOKUP: Duration = Duration::from_secs(1);

/// `min(300ms, max(30ms, 6ms × 読みの文字数))`。
pub fn lm_budget(reading_character_count: usize) -> Duration {
    let count = u32::try_from(reading_character_count).unwrap_or(u32::MAX);
    let scaled = PER_CHARACTER.saturating_mul(count);
    scaled.max(BASE).min(MAXIMUM)
}

/// `Convert` と `Page` を同期で待つ上限 = `lm_budget(読み) + 40ms`。
pub fn convert_timeout(reading_character_count: usize) -> Duration {
    lm_budget(reading_character_count) + IPC_MARGIN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_follows_base_per_character_and_cap() {
        assert_eq!(lm_budget(0), Duration::from_millis(30));
        assert_eq!(lm_budget(3), Duration::from_millis(30));
        assert_eq!(lm_budget(7), Duration::from_millis(42));
        assert_eq!(lm_budget(500), Duration::from_millis(300));
        assert_eq!(lm_budget(usize::MAX), Duration::from_millis(300));
    }

    #[test]
    fn admin_timeouts_follow_the_phase3a_contract() {
        assert_eq!(ADMIN, Duration::from_secs(3));
        assert_eq!(ADMIN_TRANSFER, Duration::from_secs(30));
    }

    #[test]
    fn convert_timeout_adds_the_ipc_margin() {
        assert_eq!(convert_timeout(3), Duration::from_millis(70));
    }
}
