//! 取得（`ikari-data.exe`）の起動、進み具合、取り消しの C の ABI（ADR 0076 §3.1、§8.1。ヘッダは
//! `include/ikari_settings_core.h`）。画面は、ボタンから `start`、タイマーで `poll`、取り消しで `cancel`、
//! 終わったら `free` を呼ぶだけ。文は核が作る。
//!
//! - 文字列は UTF-8 のポインタと長さ（呼び出しの間だけ借りる）。`msg` と `text` は
//!   `ikari_settings_core_buf_free` で返す。
//! - どの入口も panic を外へ出さない。
//!
//! 版で中身を替える（ADR 0080 §7.1）。口（名前、引数、値）は両方の版で同じ（画面を版ごとに分けない）。
//! - 通常版（feature `online`）: `online`。`ikari-data.exe` を起こす。
//! - オフライン版: `offline`。何も起こさず、取得の核も `fetch_os` も持たない。始める口は `ERR_FAILED` と文を返し、
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
    /// 0 取得中、1 取得できた、2 失敗、3 取り消した（`fetch::Phase::code`）
    pub phase: i32,
    /// 受けたバイト
    pub received: u64,
    /// 全体のバイト（`has_total` が 0 なら使わない）
    pub total: u64,
    pub has_total: u8,
}
