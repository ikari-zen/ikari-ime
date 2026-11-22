//! 止める口（ADR 0068 §6.2）: 名前つきのイベントが合図されたら、エンジンを静かに終わらせる。
//!
//! 判断の置き場所: Rust。ここは OS に依存しない部分（名前と ACL の決定、合図から終了へのつなぎ）。
//! イベントを作って待つ OS 層は `os`（`cfg(windows)`）。終わらせる経路は、Ctrl+C などと同じ `StopHandle`
//! （受け付けを止め、学習を保存して終わる。新しい終了の仕組みは持たない）。
//!
//! - 口でできるのは「止める」だけ（R6）。データは運ばない。
//! - 合図を許すのは、自分のユーザー、SYSTEM、管理者だけ（`stop_event_sddl`）。権利は合図（`EVENT_MODIFY_STATE`）だけ。
//!   待つのは作ったときのハンドルなので、待つ権利は要らない。所有者は自分のユーザーで、中の整合性のラベルを明示する。
//! - 名前は自分のセッションの `Local\`。別のセッションの合図する側は `Session\<番号>\` で開く（`stop_event_open_name`）。
//! - 作れなかったとき（名前を先に取られていた、など）は、止める口が無いだけで、エンジンは動き続ける。
//!   待ちが壊れたときも、止めずに口を閉じるだけ（誤って止めない）。

use ikari_pipe::name::{PipeNameError, stop_event_create_name};

#[cfg(windows)]
use crate::run::StopHandle;

#[cfg(windows)]
#[allow(unsafe_code)]
mod os;

/// イベントに許す権利（SDDL の 16 進）: `EVENT_MODIFY_STATE`（0x2。合図する）だけ。
/// 待つ権利（`SYNCHRONIZE`）は、作る側が `CreateEventW` のハンドルで待つので要らない。削除・ACL の変更も許さない。
const SIGNAL_RIGHTS: &str = "0x2";
/// 所有者の指定の先頭（`O:<sid>`）
const OWNER_PREFIX: &str = "O:";
/// 中の整合性の明示のラベル（`pipe/src/os/mod.rs` の方針に揃える。それより低い整合性からの書き込みを拒む）
const MANDATORY_LABEL: &str = "S:(ML;;NW;;;ME)";
/// 親からの継承を受けない DACL の先頭
const DACL_PREFIX: &str = "D:P";
/// SYSTEM と、組み込みの Administrators（SDDL の別名）
const SYSTEM_ALIAS: &str = "SY";
const ADMINISTRATORS_ALIAS: &str = "BA";

/// 止める口を作れなかった理由（ログには固定の語だけを出す）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopPortError {
    /// ユーザーの SID またはセッション番号が取れない・使えない
    Identity,
    /// 名前を先に取られていた（作ったのが自分でない）
    NameTaken,
    /// イベントを作れない
    Create,
    /// 待つスレッドを起こせない
    Spawn,
}

impl StopPortError {
    pub fn as_str(self) -> &'static str {
        match self {
            StopPortError::Identity => "identity",
            StopPortError::NameTaken => "name-taken",
            StopPortError::Create => "create",
            StopPortError::Spawn => "spawn",
        }
    }
}

/// 名前（`Local\ikari-ime-stop-v1-<sid>-s<session>`）と、所有者・DACL・ラベルの SDDL。`sid` は `stop_event_create_name` の規則を通ったものだけ。
pub fn stop_event_spec(sid: &str, session_id: u32) -> Result<(String, String), PipeNameError> {
    let name = stop_event_create_name(sid, session_id)?;
    Ok((name, stop_event_sddl(sid)))
}

/// 自分のユーザー（`sid`）、SYSTEM、管理者だけに合図を許す。所有者は `sid`、ラベルは中。ほかの ACE は無いので、ほかのユーザー、
/// Everyone、匿名は開けない。`sid` は呼び出し側が検査済み（`stop_event_spec` を通す）。
fn stop_event_sddl(sid: &str) -> String {
    format!(
        "{OWNER_PREFIX}{sid}{DACL_PREFIX}(A;;{SIGNAL_RIGHTS};;;{sid})(A;;{SIGNAL_RIGHTS};;;{SYSTEM_ALIAS})(A;;{SIGNAL_RIGHTS};;;{ADMINISTRATORS_ALIAS}){MANDATORY_LABEL}"
    )
}

/// 待ちの結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    /// 合図された
    Signaled,
    /// 待てなくなった（OS の失敗）。止める合図ではない
    Failed,
}

/// 合図を待ち、合図されたときだけ `stop` を呼ぶ。待ちが壊れたときは何もせず `false`（エンジンは動き続ける）。
/// 止めたなら `true`。
pub fn watch(wait: impl FnOnce() -> WaitOutcome, stop: impl FnOnce()) -> bool {
    match wait() {
        WaitOutcome::Signaled => {
            stop();
            true
        }
        WaitOutcome::Failed => false,
    }
}

/// 止める口を開き、合図を待つスレッドを起こす（Windows だけ）。
/// `Err` のとき、エンジンは動き続けてよい（呼び出し側は、語だけをログに出す）。
#[cfg(windows)]
pub fn start(stop: StopHandle) -> Result<(), StopPortError> {
    os::start(stop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::StopWait;

    const SID: &str = "S-1-5-21-1004336348-1177238915-682003330-1001";

    #[test]
    fn name_and_sddl_follow_the_contract() {
        let (name, sddl) = stop_event_spec(SID, 2).unwrap();
        assert_eq!(name, format!(r"Local\ikari-ime-stop-v1-{SID}-s2"));
        assert_eq!(
            sddl,
            format!("O:{SID}D:P(A;;0x2;;;{SID})(A;;0x2;;;SY)(A;;0x2;;;BA)S:(ML;;NW;;;ME)")
        );
    }

    #[test]
    fn sddl_has_only_the_user_system_and_administrators() {
        let (_, sddl) = stop_event_spec(SID, 1).unwrap();
        assert_eq!(sddl.matches("(A;").count(), 3);
        for forbidden in ["WD", "AU", "AN", "BU", "IU", "GA", "(D;"] {
            assert!(!sddl.contains(forbidden), "{forbidden}");
        }
        // 待つ権利（SYNCHRONIZE）は付けない
        assert!(!sddl.contains("0x100000"));
        assert!(!sddl.contains("0x100002"));
    }

    #[test]
    fn sids_that_could_break_the_sddl_are_rejected() {
        for sid in ["", "S-", "1-5", "S-1)(A;;GA;;;WD", "S-1;5", "S-1 5"] {
            assert!(stop_event_spec(sid, 1).is_err(), "{sid:?}");
        }
    }

    #[test]
    fn signal_stops_the_host_through_the_same_path_as_ctrl_c() {
        let stop = StopWait::new();
        let handle = stop.handle();
        assert!(watch(|| WaitOutcome::Signaled, || handle.stop()));
        assert!(stop.try_recv_stop());
    }

    #[test]
    fn a_broken_wait_does_not_stop_the_host() {
        let stop = StopWait::new();
        let handle = stop.handle();
        assert!(!watch(|| WaitOutcome::Failed, || handle.stop()));
        assert!(!stop.try_recv_stop());
    }
}
