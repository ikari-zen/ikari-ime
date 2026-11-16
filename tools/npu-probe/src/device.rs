//! 測る装置の名前（ADR 0075 §4.3）。

use std::fmt;
use std::str::FromStr;

/// OpenVINO の装置
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OvTarget {
    Cpu,
    Gpu,
    Npu,
}

impl OvTarget {
    pub const ALL: [OvTarget; 3] = [OvTarget::Cpu, OvTarget::Gpu, OvTarget::Npu];

    /// OpenVINO の装置名
    pub fn ov_name(self) -> &'static str {
        match self {
            OvTarget::Cpu => "CPU",
            OvTarget::Gpu => "GPU",
            OvTarget::Npu => "NPU",
        }
    }

    fn short(self) -> &'static str {
        match self {
            OvTarget::Cpu => "cpu",
            OvTarget::Gpu => "gpu",
            OvTarget::Npu => "npu",
        }
    }
}

/// IR の重みの精度
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Precision {
    Fp16,
    Int8,
}

impl Precision {
    pub const ALL: [Precision; 2] = [Precision::Fp16, Precision::Int8];

    /// 名前（`models\ir\<名前>\` のフォルダ名を兼ねる）
    pub fn name(self) -> &'static str {
        match self {
            Precision::Fp16 => "fp16",
            Precision::Int8 => "int8",
        }
    }
}

/// 測る装置
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Device {
    /// 今の `nll`（llama.cpp、CPU）。基準
    LlamaCpu,
    /// 今の `nll`（llama.cpp、Vulkan）
    LlamaVulkan,
    /// OpenVINO を直に使う（IR、形を固定）
    Ov(OvTarget, Precision),
    /// llama.cpp の OpenVINO バックエンド（副。このツールでは作っていない）
    LlamaOvNpu,
}

impl Device {
    /// 既定で測る順（基準を先に）
    pub fn all() -> Vec<Device> {
        let mut out = vec![Device::LlamaCpu, Device::LlamaVulkan];
        for t in OvTarget::ALL {
            for p in Precision::ALL {
                out.push(Device::Ov(t, p));
            }
        }
        out.push(Device::LlamaOvNpu);
        out
    }

    pub fn is_ov(self) -> bool {
        matches!(self, Device::Ov(..))
    }
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Device::LlamaCpu => f.write_str("llama-cpu"),
            Device::LlamaVulkan => f.write_str("llama-vulkan"),
            Device::Ov(t, p) => write!(f, "ov-{}-{}", t.short(), p.name()),
            Device::LlamaOvNpu => f.write_str("llama-ov-npu"),
        }
    }
}

impl FromStr for Device {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Device::all()
            .into_iter()
            .find(|d| d.to_string() == s)
            .ok_or_else(|| format!("知らない装置: {s}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for d in Device::all() {
            assert_eq!(d.to_string().parse::<Device>(), Ok(d));
        }
        assert_eq!(
            Device::Ov(OvTarget::Npu, Precision::Int8).to_string(),
            "ov-npu-int8"
        );
        assert!("ov-tpu-fp16".parse::<Device>().is_err());
    }
}
