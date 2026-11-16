//! 高い権限のプロセスを断る判定（ADR 0067 §4.3、ADR 0068 §2 の 5）。**OS に依存しない**（Mac でテストする）。
//!
//! OS から整合性のレベルを取るのは OS 層（`os::windows`）で、ここは取れた値（取れなければ `None`）から、
//! 続けてよいか・つなぎを受けてよいかだけを決める。取れないときは断る（安全な側）。
//!
//! - 起動: 高い権限（整合性が高以上）で起動された engine-host などは、何もせずに終わる（`check_self`）。
//! - 受け付け: つないできた相手の整合性が高以上なら、何も読まずに切る（`check_peer`）。
//!   自分が高い権限のときだけは、相手が高くても断らない（製品の engine-host はそもそも高い権限で動かない。
//!   これは、管理者権限で流れるテストの実行環境のため）。

/// 整合性のレベルの「高い」（`SECURITY_MANDATORY_HIGH_RID`）。これ以上が高い権限
pub const HIGH_INTEGRITY_RID: u32 = 0x3000;

/// 断った理由（ログに出す固定の語になる）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// 自分が高い権限で起動された
    SelfElevated,
    /// 自分の整合性のレベルが取れない
    SelfUnknown,
    /// 相手が高い権限
    PeerElevated,
    /// 相手の整合性のレベルが取れない（高い権限のプロセスのトークンは、ふつうの権限からは開けない）
    PeerUnknown,
}

impl Refusal {
    /// ログに出す固定の語
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SelfElevated => "self-elevated",
            Self::SelfUnknown => "self-integrity-unknown",
            Self::PeerElevated => "peer-elevated",
            Self::PeerUnknown => "peer-integrity-unknown",
        }
    }
}

fn is_elevated(rid: u32) -> bool {
    rid >= HIGH_INTEGRITY_RID
}

/// 自分のトークンの整合性のレベルから、起動を続けてよいかを決める。
pub fn check_self(own_rid: Option<u32>) -> Result<(), Refusal> {
    match own_rid {
        None => Err(Refusal::SelfUnknown),
        Some(rid) if is_elevated(rid) => Err(Refusal::SelfElevated),
        Some(_) => Ok(()),
    }
}

/// つないできた相手を受けてよいかを決める。
pub fn check_peer(own_rid: Option<u32>, peer_rid: Option<u32>) -> Result<(), Refusal> {
    let own = own_rid.ok_or(Refusal::SelfUnknown)?;
    if is_elevated(own) {
        return Ok(());
    }
    match peer_rid {
        None => Err(Refusal::PeerUnknown),
        Some(rid) if is_elevated(rid) => Err(Refusal::PeerElevated),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOW: u32 = 0x1000;
    const MEDIUM: u32 = 0x2000;
    const MEDIUM_PLUS: u32 = 0x2100;
    const HIGH: u32 = 0x3000;
    const SYSTEM: u32 = 0x4000;

    #[test]
    fn self_table() {
        let cases: &[(&str, Option<u32>, Result<(), Refusal>)] = &[
            ("low", Some(LOW), Ok(())),
            ("medium", Some(MEDIUM), Ok(())),
            ("medium plus", Some(MEDIUM_PLUS), Ok(())),
            ("just below high", Some(HIGH - 1), Ok(())),
            ("high", Some(HIGH), Err(Refusal::SelfElevated)),
            ("system", Some(SYSTEM), Err(Refusal::SelfElevated)),
            ("unknown", None, Err(Refusal::SelfUnknown)),
        ];
        for (name, rid, expected) in cases {
            assert_eq!(check_self(*rid), *expected, "{name}");
        }
    }

    #[test]
    fn peer_table() {
        type Case = (&'static str, Option<u32>, Option<u32>, Result<(), Refusal>);
        let cases: &[Case] = &[
            ("medium to medium", Some(MEDIUM), Some(MEDIUM), Ok(())),
            ("medium to low", Some(MEDIUM), Some(LOW), Ok(())),
            (
                "medium to high",
                Some(MEDIUM),
                Some(HIGH),
                Err(Refusal::PeerElevated),
            ),
            (
                "medium to system",
                Some(MEDIUM),
                Some(SYSTEM),
                Err(Refusal::PeerElevated),
            ),
            (
                "peer unreadable",
                Some(MEDIUM),
                None,
                Err(Refusal::PeerUnknown),
            ),
            (
                "self unreadable",
                None,
                Some(MEDIUM),
                Err(Refusal::SelfUnknown),
            ),
            ("both unreadable", None, None, Err(Refusal::SelfUnknown)),
            // 自分が高い権限のときは、相手で断らない（テストの実行環境のため）
            ("high to high", Some(HIGH), Some(HIGH), Ok(())),
        ];
        for (name, own, peer, expected) in cases {
            assert_eq!(check_peer(*own, *peer), *expected, "{name}");
        }
    }

    #[test]
    fn refusal_texts_are_distinct() {
        let all = [
            Refusal::SelfElevated,
            Refusal::SelfUnknown,
            Refusal::PeerElevated,
            Refusal::PeerUnknown,
        ];
        let texts: std::collections::HashSet<_> = all.iter().map(|r| r.as_str()).collect();
        assert_eq!(texts.len(), all.len());
    }
}
