//! Windows の設定画面（`ikari-settings.exe`）の核（ADR 0071 §2。段の計画は `windows/settings-ui/README.md`）。
//!
//! - `view`: 画面に出す行（一覧 + 今の値。部品、範囲、選択肢）
//! - `edit`: 画面の操作 → 設定の口に送る書き換え（`SettingsValues::plan` を通す）
//! - `peer`: 設定の口の相手（engine-host）の照合（ADR 0068 §4。S3 の設定画面の側）
//! - `abi`: 画面（C#）が呼ぶ C の ABI（ヘッダは `include/ikari_settings_core.h`）
//!
//! 判断の置き場所: Rust（この crate と、その下の `shell-core` の `settings`）。画面（C#）は、核が渡すものを描くことと、
//! OS の入口（窓、ファイルを選ぶ窓）だけ（ADR 0042、ADR 0071 §2）。純粋な部分は `cfg(windows)` なしで、Mac でテストが回る。
//!
//! 通信しない（NFR-17）。この crate は通信の手段を持たない。

#![deny(unsafe_code)]

pub mod edit;
pub mod peer;
pub mod view;

#[allow(unsafe_code)]
pub mod abi;
