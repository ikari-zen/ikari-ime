use std::sync::Arc;

use crate::error::LmError;

/// モデルが動く装置（ADR 0061 §4.2）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LmDevice {
    /// モデルが読まれていない（まだ決まっていない）
    #[default]
    Unknown,
    Cpu,
    Gpu,
}

/// 言語モデルによる候補の採点。ワーカースレッドの中だけで使う（`Send` は要らない）。
pub trait LmScorer {
    /// `prompt` の続きとして各候補を出力する確率の負の対数（nat。EOS を含む）。
    /// 返す長さは `candidates` と同じ。評価できない候補（長すぎるなど）は `f32::INFINITY`。
    fn nll(&mut self, prompt: &str, candidates: &[String]) -> Result<Vec<f32>, LmError>;

    /// モデルが動いている装置（ADR 0061 D3）。既定は `Cpu`。
    fn device(&self) -> LmDevice {
        LmDevice::Cpu
    }
}

/// モデルを読み込む関数。ワーカースレッドの中で、必要なときに（読み直しも）呼ばれる。
pub type LmLoader = Arc<dyn Fn() -> Result<Box<dyn LmScorer>, LmError> + Send + Sync>;
