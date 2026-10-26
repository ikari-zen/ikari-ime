//! 文脈の読み取りの量と時間の値（ADR 0015 §10 の決定）。
//!
//! 契約コミットで値だけを宣言する。使うのはレーン A（`session.rs`）で、ここの型と名前は変えない。
//! 殻や core は参照しない（エンジンの中だけの値）。

use std::time::Duration;

/// 休止した（`EndSession` を受けた）入力欄の履歴を残す時間（Q2。D1）
pub const PAUSED_SESSION_TTL: Duration = Duration::from_secs(30 * 60);
/// 同じ入力欄の履歴の量（表記の合計の文字数。Q3、D4。解析の窓 `MAX_CONTEXT_CHARS` と同じ）
pub const HISTORY_MAX_CHARS: usize = 2000;
/// アプリをまたぐ「最近の確定」を残す時間（Q4。D5）
pub const RECENT_COMMITS_TTL: Duration = Duration::from_secs(10 * 60);
/// アプリをまたぐ「最近の確定」の量（表記の合計の文字数。Q4。D5）
pub const RECENT_COMMITS_MAX_CHARS: usize = 2000;
/// 同じ入力欄に文脈が何もないときに、ほかのアプリの確定を左文脈として借りる窓（Q5、X5。D5）
pub const BORROW_WINDOW: Duration = Duration::from_secs(2 * 60);

/// 上の値をまとめた型。テストで短い時間に差し替えるためにある（本番は [`ContextLimits::DEFAULT`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextLimits {
    pub paused_session_ttl: Duration,
    pub history_max_chars: usize,
    pub recent_commits_ttl: Duration,
    pub recent_commits_max_chars: usize,
    pub borrow_window: Duration,
}

impl ContextLimits {
    pub const DEFAULT: Self = Self {
        paused_session_ttl: PAUSED_SESSION_TTL,
        history_max_chars: HISTORY_MAX_CHARS,
        recent_commits_ttl: RECENT_COMMITS_TTL,
        recent_commits_max_chars: RECENT_COMMITS_MAX_CHARS,
        borrow_window: BORROW_WINDOW,
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
        assert_eq!(d.recent_commits_ttl, Duration::from_secs(600));
        assert_eq!(d.recent_commits_max_chars, 2000);
        assert_eq!(d.borrow_window, Duration::from_secs(120));
        assert!(d.borrow_window <= d.recent_commits_ttl);
    }
}
