use std::sync::Arc;

use crate::error::LmError;

/// 言語モデルによる候補の採点。ワーカースレッドの中だけで使う（`Send` は要らない）。
pub trait LmScorer {
    /// `prompt` の続きとして各候補を出力する確率の負の対数（nat。EOS を含む）。
    /// 返す長さは `candidates` と同じ。評価できない候補（長すぎるなど）は `f32::INFINITY`。
    fn nll(&mut self, prompt: &str, candidates: &[String]) -> Result<Vec<f32>, LmError>;
}

/// モデルを読み込む関数。ワーカースレッドの中で、必要なときに（読み直しも）呼ばれる。
pub type LmLoader = Arc<dyn Fn() -> Result<Box<dyn LmScorer>, LmError> + Send + Sync>;
