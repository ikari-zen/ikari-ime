//! Windows の候補窓（`ikari-candidate.exe`）の核（ADR 0062 §7。README.md に段階と範囲）。
//!
//! - `command`: TIP → 候補窓の命令（`proto/ikari/candidate/v1/candidate.proto`）の型の入口
//! - `metrics`: 行の高さ（DIP）。**行数と行の種類だけで決まる固定の関数**。TIP と候補窓が同じ関数を使う
//! - `placement`: 候補窓の横と縦の位置（`shell-core` の `panel` と同じ規則。縦は TIP の `side` に従う）
//! - `limits`: 受けたものの上限（行の数、1 行の長さ、文字の種類、座標と倍率。ADR 0068 §5）
//! - `state`: 命令の状態（`seq` と世代、持ち主の接続、切れたら隠す、並べる順）。`RenderState` を作る
//! - `receiver`: パイプの受け手（`pipe::Server`。候補窓の口の上限）
//! - `abi`: 画面（C#）が呼ぶ C の ABI（ヘッダは `include/ikari_candidate.h`）
//!
//! 判断の置き場所: Rust（この crate）。画面（C#）は、核が渡す `RenderState` を描くだけ（ADR 0042、ADR 0062 §3）。
//! 純粋な部分は `cfg(windows)` なしで、Mac でテストが回る。

#![deny(unsafe_code)]

pub mod command;
pub mod limits;
pub mod metrics;
pub mod placement;
pub mod receiver;
pub mod state;

#[allow(unsafe_code)]
pub mod abi;
