//! Windows の候補窓（`ikari-candidate.exe`）の核（ADR 0062 §7。**契約の骨組み。中身はスタブ**で、レーン L2 が実装する）。
//!
//! - `command`: TIP → 候補窓の命令（`proto/ikari/candidate/v1/candidate.proto`）の型の入口
//! - `metrics`: 行の高さ（DIP）。**行数と行の種類だけで決まる固定の関数**。TIP と候補窓が同じ関数を使う
//! - `placement`: 候補窓の横の位置（`shell-core` の `panel` と同じ規則）
//! - `abi`: 画面（C#）が呼ぶ C の ABI（ヘッダは `include/ikari_candidate.h`）
//!
//! 判断の置き場所: Rust（この crate）。画面（C#）は、核が渡す `RenderState` を描くだけ（ADR 0042、ADR 0062 §3）。
//! 純粋な部分は `cfg(windows)` なしで、Mac でテストが回る。

#![deny(unsafe_code)]

pub mod command;
pub mod metrics;
pub mod placement;

#[allow(unsafe_code)]
pub mod abi;
