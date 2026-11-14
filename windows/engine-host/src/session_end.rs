//! ログオフ・シャットダウンの通知（`WM_QUERYENDSESSION`、`WM_ENDSESSION`）への反応の判断（OS に依存しない。
//! Mac でテストする）。ウィンドウとメッセージの受け取りは `endsession.rs`（`cfg(windows)`）。
//!
//! 判断の置き場所: Rust。ウィンドウの手続きは、メッセージの番号を渡して、返す値と「止めるか」と診断の注記を受け取るだけ。
//! 注記は固定の語と数だけ（ユーザーの文字は入らない）。

/// `WM_QUERYENDSESSION`（0x0011）
pub const WM_QUERYENDSESSION: u32 = 0x0011;
/// `WM_ENDSESSION`（0x0016）
pub const WM_ENDSESSION: u32 = 0x0016;
/// lParam: アプリを閉じるよう求められた（再起動のための更新など）
pub const ENDSESSION_CLOSEAPP: u32 = 0x0000_0001;
/// lParam: セッションが終わる途中で、アプリは止められない
pub const ENDSESSION_CRITICAL: u32 = 0x4000_0000;
/// lParam: ユーザーがログオフする
pub const ENDSESSION_LOGOFF: u32 = 0x8000_0000;

/// `WM_QUERYENDSESSION` への返事。終わってよい（TRUE）
pub const ALLOW_END: isize = 1;
/// 処理したメッセージ（`WM_ENDSESSION`）への返事
pub const HANDLED: isize = 0;

/// ウィンドウの手続きがすること
#[derive(Debug, PartialEq, Eq)]
pub struct Reaction {
    /// ウィンドウの手続きが返す値
    pub result: isize,
    /// 止める合図を送り、後始末が終わるまで待つか
    pub stop: bool,
    /// `engine.log` に書く注記（固定の語と数だけ）
    pub note: String,
}

/// 終了の通知なら反応を返す。それ以外のメッセージは `None`（既定の処理に任せる）。
///
/// `WM_ENDSESSION` は、wParam が 0 なら終了が取りやめになったので、止めない。
pub fn react(message: u32, wparam: usize, lparam: isize) -> Option<Reaction> {
    // 上位ビットを含む 32 ビットの値として見る（32 ビット環境では LPARAM が負になりうる）
    let flags = lparam as u32;
    let flag = |bit: u32| u8::from(flags & bit != 0);
    let reasons = format!(
        "logoff={} closeapp={} critical={}",
        flag(ENDSESSION_LOGOFF),
        flag(ENDSESSION_CLOSEAPP),
        flag(ENDSESSION_CRITICAL)
    );
    match message {
        WM_QUERYENDSESSION => Some(Reaction {
            result: ALLOW_END,
            stop: false,
            note: format!("msg=queryendsession {reasons}"),
        }),
        WM_ENDSESSION => {
            let ending = wparam != 0;
            Some(Reaction {
                result: HANDLED,
                stop: ending,
                note: format!("msg=endsession ending={} {reasons}", u8::from(ending)),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_allows_the_end_without_stopping() {
        let r = react(WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF as isize).unwrap();
        assert_eq!(r.result, ALLOW_END);
        assert!(!r.stop);
        assert_eq!(r.note, "msg=queryendsession logoff=1 closeapp=0 critical=0");
    }

    #[test]
    fn end_session_stops_only_when_it_is_really_ending() {
        let r = react(WM_ENDSESSION, 1, ENDSESSION_LOGOFF as isize).unwrap();
        assert!(r.stop);
        assert_eq!(r.result, HANDLED);
        assert_eq!(
            r.note,
            "msg=endsession ending=1 logoff=1 closeapp=0 critical=0"
        );

        let cancelled = react(WM_ENDSESSION, 0, 0).unwrap();
        assert!(!cancelled.stop);
        assert_eq!(
            cancelled.note,
            "msg=endsession ending=0 logoff=0 closeapp=0 critical=0"
        );
    }

    #[test]
    fn reads_every_reason_flag() {
        let all = (ENDSESSION_LOGOFF | ENDSESSION_CLOSEAPP | ENDSESSION_CRITICAL) as isize;
        let r = react(WM_ENDSESSION, 1, all).unwrap();
        assert!(r.note.ends_with("logoff=1 closeapp=1 critical=1"));
        // シャットダウン（どの印もない）
        let r = react(WM_ENDSESSION, 1, 0).unwrap();
        assert!(r.note.ends_with("logoff=0 closeapp=0 critical=0"));
    }

    #[test]
    fn other_messages_are_left_to_the_default() {
        assert_eq!(react(0x0001, 1, 0), None);
    }
}
