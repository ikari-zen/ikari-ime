//! 設定画面の「LM が CPU で動いている」案内（#425）。出すかどうかと文を決める（OS 非依存。判断は Rust に置く）。
//!
//! 出すのは、LM がオンで、読み込みが済み（`Ready`）、実行装置が CPU と分かっているときだけ。
//! LM オフ、GPU、読み込み中、失敗、装置が分からない（古いエンジン、読まれていない）ときは出さない。
//! 数字（ミリ秒）は出さない。締め切り（`engine.lm_deadline_ms`）の既定は変えない（ユーザー決定 2026-11-15）。

use crate::menu::EngineLmState;
use crate::types::LmDeviceInfo;

/// LM が CPU で動いているときの案内の文。Windows の設定画面（W4-s3）も同じ文を出す
pub const LM_CPU_NOTICE: &str =
    "GPU が使えないため、言語モデルは CPU で動いています。GPU があると、変換がより快適になります。";

/// 案内を出すなら、その文。`lm_enabled` は設定 `engine.lm` の今の値、`state` と `device` はエンジンの `Hello` の返事
#[must_use]
pub fn lm_cpu_notice(
    lm_enabled: bool,
    state: EngineLmState,
    device: LmDeviceInfo,
) -> Option<&'static str> {
    (lm_enabled && state == EngineLmState::Ready && device == LmDeviceInfo::Cpu)
        .then_some(LM_CPU_NOTICE)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATES: [EngineLmState; 5] = [
        EngineLmState::Unspecified,
        EngineLmState::Disabled,
        EngineLmState::Loading,
        EngineLmState::Ready,
        EngineLmState::Failed,
    ];
    const DEVICES: [LmDeviceInfo; 3] = [
        LmDeviceInfo::Unspecified,
        LmDeviceInfo::Cpu,
        LmDeviceInfo::Gpu,
    ];

    #[test]
    fn shows_when_enabled_ready_and_on_the_cpu() {
        assert_eq!(
            lm_cpu_notice(true, EngineLmState::Ready, LmDeviceInfo::Cpu),
            Some(LM_CPU_NOTICE)
        );
    }

    #[test]
    fn hidden_on_the_gpu() {
        assert_eq!(
            lm_cpu_notice(true, EngineLmState::Ready, LmDeviceInfo::Gpu),
            None
        );
    }

    #[test]
    fn hidden_when_the_lm_is_off() {
        // 設定がオフ（エンジンの返事が古い・オンのまま CPU と言っていても）
        assert_eq!(
            lm_cpu_notice(false, EngineLmState::Ready, LmDeviceInfo::Cpu),
            None
        );
        assert_eq!(
            lm_cpu_notice(true, EngineLmState::Disabled, LmDeviceInfo::Cpu),
            None
        );
    }

    #[test]
    fn hidden_unless_ready_or_the_device_is_unknown() {
        for state in STATES {
            for device in DEVICES {
                let want = state == EngineLmState::Ready && device == LmDeviceInfo::Cpu;
                assert_eq!(
                    lm_cpu_notice(true, state, device).is_some(),
                    want,
                    "{state:?} {device:?}"
                );
            }
        }
    }

    #[test]
    fn the_text_has_no_numbers() {
        assert!(!LM_CPU_NOTICE.chars().any(|c| c.is_ascii_digit()));
    }
}
