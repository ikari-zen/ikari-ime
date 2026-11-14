//! 開発者向けの 1 行（`developer.lm_status`。ADR 0061 §6.2）。LM の判定・時間・装置を 1 行の文にする。
//!
//! **判断の置き場所: Rust**。何を言うか、どう丸めるかは shell-core が決め、OS の画面は描くだけにする。
//! 文言は案（実機で見てから直す）。読み・候補・文脈は出さない（NFR-19）。

use crate::menu::EngineLmState;
use crate::types::{LmDeviceInfo, LmReportInfo, LmVerdictInfo};

const MICROS_PER_MILLI: u32 = 1_000;

const PREFIX_USED: &str = "LM";
const PREFIX_NONE: &str = "LM なし:";

const TEXT_APPLIED: &str = "間に合った";
const TEXT_APPLIED_LATE: &str = "間に合った（遅れて届いた結果）";
const TEXT_TIMED_OUT: &str = "時間切れ";
const TEXT_NOT_SCORED: &str = "採点なし";
const TEXT_NO_PREFETCH: &str = "先行計算なし";
const TEXT_SECURE: &str = "秘密の入力";
const TEXT_LOADING: &str = "読み込み中";
const TEXT_DISABLED: &str = "無効";
const TEXT_FAILED: &str = "失敗";
const TEXT_UNAVAILABLE: &str = "使えない";

const DEVICE_GPU: &str = "GPU";
const DEVICE_CPU: &str = "CPU";

/// 候補窓の下に出す 1 行。判定が不明（`Unspecified`）なら空（行を出さない）
pub(super) fn lm_status_line(report: &LmReportInfo) -> String {
    let device = device_text(report.device);
    let with_device = |text: String| {
        if device.is_empty() {
            text
        } else {
            format!("{text} {device}")
        }
    };
    match report.verdict {
        LmVerdictInfo::Unspecified => String::new(),
        LmVerdictInfo::Applied => {
            let state = if report.late_hits > 0 {
                TEXT_APPLIED_LATE
            } else {
                TEXT_APPLIED
            };
            with_device(format!(
                "{PREFIX_USED} {state} {}ms / 締め切り {}ms（推論 {}ms）",
                millis(report.wait_micros),
                millis(report.lm_deadline_micros),
                millis(report.infer_micros),
            ))
        }
        LmVerdictInfo::TimedOut => with_device(format!(
            "{PREFIX_USED} {TEXT_TIMED_OUT} {}ms / 締め切り {}ms",
            millis(report.wait_micros),
            millis(report.lm_deadline_micros),
        )),
        LmVerdictInfo::NotScored => with_device(format!("{PREFIX_USED} {TEXT_NOT_SCORED}")),
        LmVerdictInfo::PrefetchLate => format!(
            "{PREFIX_NONE} 先行計算が Space から {}ms に間に合わず",
            millis(report.convert_wait_micros)
        ),
        LmVerdictInfo::NoPrefetch => format!("{PREFIX_NONE} {TEXT_NO_PREFETCH}"),
        LmVerdictInfo::Secure => format!("{PREFIX_NONE} {TEXT_SECURE}"),
        LmVerdictInfo::Unavailable => {
            let reason = match report.lm_state {
                Some(EngineLmState::Loading) => TEXT_LOADING,
                Some(EngineLmState::Disabled) => TEXT_DISABLED,
                Some(EngineLmState::Failed) => TEXT_FAILED,
                Some(EngineLmState::Ready | EngineLmState::Unspecified) | None => TEXT_UNAVAILABLE,
            };
            format!("{PREFIX_NONE} {reason}")
        }
    }
}

/// us を ms に四捨五入する
fn millis(micros: u32) -> u32 {
    micros.saturating_add(MICROS_PER_MILLI / 2) / MICROS_PER_MILLI
}

fn device_text(device: LmDeviceInfo) -> &'static str {
    match device {
        LmDeviceInfo::Gpu => DEVICE_GPU,
        LmDeviceInfo::Cpu => DEVICE_CPU,
        LmDeviceInfo::Unspecified => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(verdict: LmVerdictInfo) -> LmReportInfo {
        LmReportInfo {
            verdict,
            wait_micros: 42_000,
            infer_micros: 38_000,
            lm_deadline_micros: 300_000,
            convert_wait_micros: 30_000,
            device: LmDeviceInfo::Gpu,
            ..LmReportInfo::default()
        }
    }

    #[test]
    fn rounds_to_milliseconds() {
        assert_eq!(millis(499), 0);
        assert_eq!(millis(500), 1);
        assert_eq!(millis(u32::MAX), 4_294_967);
    }

    #[test]
    fn unspecified_verdict_and_unknown_device_are_quiet() {
        assert_eq!(lm_status_line(&report(LmVerdictInfo::Unspecified)), "");
        let mut r = report(LmVerdictInfo::NotScored);
        r.device = LmDeviceInfo::Unspecified;
        assert_eq!(lm_status_line(&r), "LM 採点なし");
    }

    #[test]
    fn every_verdict_has_its_sentence() {
        let line = |verdict| lm_status_line(&report(verdict));
        assert_eq!(
            line(LmVerdictInfo::Applied),
            "LM 間に合った 42ms / 締め切り 300ms（推論 38ms） GPU"
        );
        assert_eq!(
            line(LmVerdictInfo::TimedOut),
            "LM 時間切れ 42ms / 締め切り 300ms GPU"
        );
        assert_eq!(
            line(LmVerdictInfo::PrefetchLate),
            "LM なし: 先行計算が Space から 30ms に間に合わず"
        );
        assert_eq!(line(LmVerdictInfo::NoPrefetch), "LM なし: 先行計算なし");
        assert_eq!(line(LmVerdictInfo::Secure), "LM なし: 秘密の入力");
        // 状態の言い方は、読み込み中・無効・失敗で分かれる
        let mut r = report(LmVerdictInfo::Unavailable);
        r.lm_state = Some(EngineLmState::Loading);
        assert_eq!(lm_status_line(&r), "LM なし: 読み込み中");
        r.lm_state = None;
        assert_eq!(lm_status_line(&r), "LM なし: 使えない");
    }

    #[test]
    fn late_result_is_said() {
        let mut r = report(LmVerdictInfo::Applied);
        r.late_hits = 1;
        assert!(lm_status_line(&r).contains("遅れて届いた結果"));
    }
}
