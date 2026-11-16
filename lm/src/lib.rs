//! ikari-lm: zenz（GGUF）による話題スコア（ADR 0002 §13.2）。
//!
//! - [`ZenzRanker`]: `ikari_core::Ranker` の実装。モデルは専用のワーカースレッドが持ち、
//!   `rank()` は `budget`（既定 300ms。実際の待ちは変換1回の締め切りで決まる。NFR-04）だけ待って、間に合わなければ何もせずに戻る。
//!   アイドルが `idle_unload`（既定 300 秒。NFR-11）続くとモデルを解放する。
//! - [`LmScorer`]: 推論の口。ワーカー、時間切れ、解放はこの trait のモックでテストする。
//! - [`host`]: 推論を別プロセス `ikari-lm-host` に頼む口（ADR 0060 Q4）。子が落ちても固まっても、
//!   `ZenzRanker` は結果なし（辞書だけの順位）で戻る。子の起動、締め切り、回収、連続失敗での断念を持つ。
//! - llama.cpp を使う実装（`LlamaScorer`）は feature `llama`（Metal は `metal`）の後ろに置く。
//!   cmake と Xcode のコマンドラインツールが必要で、既定のビルドには入れない。

pub mod cancel;
mod config;
mod error;
pub mod host;
mod placement;
mod prompt;
mod ranker;
mod scorer;
pub mod trace;

#[cfg(feature = "llama")]
mod llama;

pub use cancel::CancelToken;
pub use config::{ModelKind, ZenzConfig};
pub use error::LmError;
#[cfg(feature = "dev-knobs")]
pub use placement::GPU_ENV;
pub use placement::{GpuState, devices, gpu_state, log_note};
pub use prompt::{PromptInput, build_prompt, build_prompt_for, tail_chars};
pub use ranker::{LmStats, LmTimings, ZenzRanker};
pub use scorer::{LmDevice, LmLoader, LmScorer};
pub use trace::LmTrace;

#[cfg(feature = "llama")]
pub use llama::{LlamaScorer, ModelInfo, probe_devices};
