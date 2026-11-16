//! 設定画面とエンジンのつながりの状態と、その文（ADR 0068 §7、Q3。Windows の設定画面の状態の欄）。
//!
//! 受け口を作れない・相手を確認できないことは、打っている最中には知らせず、設定画面の状態の欄だけに出す。
//! 文はここ 1 か所に置く（画面は出すだけ）。理由は種類だけで、パスや番号は入れない。

/// つながりの状態
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkStatus {
    /// つながっている
    Connected,
    /// エンジンが動いていない（設定の口が無い）
    EngineNotRunning,
    /// 相手がインストール先のエンジンだと確かめられない
    PeerNotVerified,
    /// 版が合わない
    VersionMismatch,
    /// 設定画面が高い権限で動いている
    Elevated,
    /// つながりが切れた、または返事が来ない
    Unavailable,
}

impl LinkStatus {
    /// 状態の欄に出す文
    pub fn text(self) -> &'static str {
        match self {
            Self::Connected => "エンジンにつながっています",
            Self::EngineNotRunning => {
                "エンジンにつながりません。理由: エンジンが動いていません。ikari で文字を打つと起動します"
            }
            Self::PeerNotVerified => "エンジンにつながりません。理由: 相手を確認できません",
            Self::VersionMismatch => {
                "エンジンにつながりません。理由: エンジンと設定画面の版が合いません"
            }
            Self::Elevated => {
                "管理者として動いているため、エンジンにつなぎません。ふつうの権限で開き直してください"
            }
            Self::Unavailable => "エンジンにつながりません。理由: 返事がありません",
        }
    }

    /// ABI とログに使う番号（変えない）
    pub fn code(self) -> i32 {
        match self {
            Self::Connected => 0,
            Self::EngineNotRunning => 1,
            Self::PeerNotVerified => 2,
            Self::VersionMismatch => 3,
            Self::Elevated => 4,
            Self::Unavailable => 5,
        }
    }

    /// `code` の逆。知らない番号は `None`
    pub fn from_code(code: i32) -> Option<Self> {
        ALL.into_iter().find(|s| s.code() == code)
    }
}

const ALL: [LinkStatus; 6] = [
    LinkStatus::Connected,
    LinkStatus::EngineNotRunning,
    LinkStatus::PeerNotVerified,
    LinkStatus::VersionMismatch,
    LinkStatus::Elevated,
    LinkStatus::Unavailable,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_and_texts_are_distinct() {
        let mut texts = std::collections::HashSet::new();
        for status in ALL {
            assert_eq!(LinkStatus::from_code(status.code()), Some(status));
            assert!(texts.insert(status.text()));
        }
        assert_eq!(LinkStatus::from_code(99), None);
    }

    #[test]
    fn refusals_name_the_reason_kind_only() {
        assert!(LinkStatus::PeerNotVerified.text().contains("理由"));
        assert!(!LinkStatus::PeerNotVerified.text().contains('\\'));
    }
}
