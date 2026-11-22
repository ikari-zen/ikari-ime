//! 取得（`ikari-data`）の起動、進み具合、取り消しの口（ヘッダの `ikari_settings_fetch_*`。ADR 0079 §3.6）。
//! 名前と引数の並びは Windows の `fetch_abi.rs` と同じ。失敗の値は `IKARI_SHELL_ERR_*`、文の入れ物は `IkariShellBuf`。
//!
//! 版で中身を替える（ADR 0080 §7.1）。口（名前、引数、値）は両方の版で同じにして、Swift の殻を版ごとに分けない。
//! - 通常版（feature `online`）: `online`。`ikari-data` を起こす。
//! - オフライン版: `offline`。何も起こさず、取得の核も持たない。始める口は `IKARI_SHELL_ERR_FAILED` と文を返し、
//!   ハンドルを作らない（画面は `ikari_settings_edition_capabilities` でボタンを出さないので、ふつうは呼ばれない）。

#[cfg(not(feature = "online"))]
mod offline;
#[cfg(feature = "online")]
mod online;

#[cfg(not(feature = "online"))]
pub use offline::*;
#[cfg(feature = "online")]
pub use online::*;

/// `ikari_settings_fetch_start` の `kind`: 表のモデル（`a` が ID）
pub const IKARI_SETTINGS_FETCH_PRESET: i32 = 1;
/// 自由指定の URL（`a` が URL。`format` が必要。`sha256` と `name` は空でよい）
pub const IKARI_SETTINGS_FETCH_URL: i32 = 2;

/// `ikari_settings_fetch_poll` が書く様子
#[repr(C)]
pub struct IkariSettingsFetchStatus {
    /// 0 取得中、1 取得できた、2 失敗、3 取り消した
    pub phase: i32,
    /// 受けたバイト
    pub received: u64,
    /// 全体のバイト（`has_total` が 0 なら使わない）
    pub total: u64,
    pub has_total: u8,
}
