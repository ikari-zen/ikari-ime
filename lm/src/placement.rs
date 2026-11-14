//! LM をどの装置（CPU / GPU）に載せたかの記録（ADR 0059 §2.3・§2.4）。
//!
//! llama.cpp を持たないビルドでも型は使える（記録は空のまま）。エンジンが `lm_load` のログに足すための
//! 固定の語だけを持つ（モデルのパスやエラーの文面は入れない。NFR-19）。

use std::sync::Mutex;

/// GPU を使う設定か（`engine.lm_gpu`）の環境変数による上書き（開発用）。`1` でオン、`0` でオフ。
/// それ以外の値、または未設定なら設定に従う。feature `vulkan` のときだけ効く
pub const GPU_ENV: &str = "IKARI_LM_GPU";

/// GPU をどう扱ったか（ログの `gpu=` の語）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuState {
    /// まだ読み込んでいない
    Unknown,
    /// GPU に載せた
    On,
    /// 設定（`engine.lm_gpu = off`）で CPU だけ
    OffSetting,
    /// 環境変数（`IKARI_LM_GPU=0`）で CPU だけ
    OffEnv,
    /// GPU のデバイスが無いので CPU（GPU のバックエンドが読めない、`vulkan-1.dll` が無い、を含む）
    NoGpu,
    /// GPU での読み込みに失敗したので CPU で読み直した
    FallbackLoadFailed,
    /// 先に GPU の読み込みに失敗したので、この起動の間は CPU だけ
    OffAfterFailure,
    /// このビルドに GPU の経路がない（CPU だけのビルド）
    NotBuilt,
}

impl GpuState {
    pub fn word(self) -> &'static str {
        match self {
            GpuState::Unknown => "unknown",
            GpuState::On => "on",
            GpuState::OffSetting => "off_setting",
            GpuState::OffEnv => "off_env",
            GpuState::NoGpu => "no_gpu",
            GpuState::FallbackLoadFailed => "fallback_load_failed",
            GpuState::OffAfterFailure => "off_after_failure",
            GpuState::NotBuilt => "not_built",
        }
    }
}

#[derive(Default)]
struct Record {
    /// 登録されたデバイスの要約（`cpu,vulkan:IntegratedGpu`）
    devices: Option<String>,
    gpu: Option<GpuState>,
}

static RECORD: Mutex<Record> = Mutex::new(Record {
    devices: None,
    gpu: None,
});

fn with_record<T>(f: impl FnOnce(&mut Record) -> T) -> T {
    // 毒された Mutex でも、中身は単純な値なのでそのまま使う
    let mut guard = RECORD.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

#[cfg_attr(not(feature = "llama"), allow(dead_code))]
pub(crate) fn set_devices(summary: String) {
    with_record(|r| r.devices = Some(summary));
}

#[cfg_attr(not(feature = "llama"), allow(dead_code))]
pub(crate) fn set_gpu(state: GpuState) {
    with_record(|r| r.gpu = Some(state));
}

/// 登録されたデバイスの要約（LM を一度でも読もうとしたあとだけ Some）
pub fn devices() -> Option<String> {
    with_record(|r| r.devices.clone())
}

/// GPU の扱い（読もうとしたあとだけ `Unknown` 以外）
pub fn gpu_state() -> GpuState {
    with_record(|r| r.gpu.unwrap_or(GpuState::Unknown))
}

/// `lm_load` のログに足す固定の語（`gpu=on devices=cpu,vulkan:IntegratedGpu`）。何も記録がなければ None
pub fn log_note() -> Option<String> {
    with_record(|r| match (&r.devices, r.gpu) {
        (None, None) => None,
        (devices, gpu) => Some(format!(
            "gpu={} devices={}",
            gpu.unwrap_or(GpuState::Unknown).word(),
            devices.as_deref().unwrap_or("unknown")
        )),
    })
}

/// 環境変数の値から、設定を上書きした結果を返す。`Some(false)` は環境変数でオフ
#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
pub fn env_override(value: Option<&str>) -> Option<bool> {
    match value {
        Some("1") => Some(true),
        Some("0") => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_reads_only_zero_and_one() {
        assert_eq!(env_override(Some("1")), Some(true));
        assert_eq!(env_override(Some("0")), Some(false));
        assert_eq!(env_override(Some("on")), None);
        assert_eq!(env_override(Some("")), None);
        assert_eq!(env_override(None), None);
    }

    #[test]
    fn gpu_words_are_fixed_and_distinct() {
        let all = [
            GpuState::Unknown,
            GpuState::On,
            GpuState::OffSetting,
            GpuState::OffEnv,
            GpuState::NoGpu,
            GpuState::FallbackLoadFailed,
            GpuState::OffAfterFailure,
            GpuState::NotBuilt,
        ];
        let mut words: Vec<&str> = all.iter().map(|s| s.word()).collect();
        words.sort_unstable();
        words.dedup();
        assert_eq!(words.len(), all.len());
    }
}
