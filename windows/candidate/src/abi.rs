//! 画面（C#。`LibraryImport`）が呼ぶ C の ABI（ADR 0062 §7.3）。ヘッダは `include/ikari_candidate.h`。**宣言だけ。中身はスタブ**。
//!
//! - 受け渡しは、数と、proto のバイト列（`RenderState`）だけ。Rust の型を境界に出さない（`shell-ffi` と同じ作法）。
//! - 「描く状態が変わった」の知らせは、核のスレッドから呼ばれる。C# は自分の UI のスレッド（`DispatcherQueue`）に戻して描く。
//! - 命令の状態（最後だけ、持ち主、切れたら隠す）と、パイプの受け手は、L2 が実装する。

use std::ffi::c_void;

use crate::metrics::{PanelRows, panel_height_dip};
use crate::placement::horizontal_origin;
use ikari_shell_core::panel::Rect;

/// ABI の版。互換を壊す変更で上げる。
pub const ABI_VERSION: u32 = 1;

/// 成功
pub const IKARI_CANDIDATE_OK: i32 = 0;
/// まだ実装されていない（スタブ）
pub const IKARI_CANDIDATE_NOT_IMPLEMENTED: i32 = -1;

/// 「描く状態が変わった」の知らせ。`state` は `RenderState`（proto）のバイト列で、**この呼び出しの間だけ有効**（借用。
/// C# は中でコピーするか、デコードして戻る）。核のスレッドから呼ばれる。
pub type RenderStateCallback = extern "C" fn(state: *const u8, len: usize, user_data: *mut c_void);

/// 核の持ち物（C からは不透明なポインタ）。
pub struct CandidateCore {
    _private: (),
}

#[unsafe(no_mangle)]
pub extern "C" fn ikari_candidate_abi_version() -> u32 {
    ABI_VERSION
}

/// 核を起動する（パイプの受け手を始める）。スタブ（null を返す）。
/// 戻り値は `ikari_candidate_stop` で必ず解放する。
#[unsafe(no_mangle)]
pub extern "C" fn ikari_candidate_start(
    _callback: RenderStateCallback,
    _user_data: *mut c_void,
) -> *mut CandidateCore {
    std::ptr::null_mut()
}

/// 核を止めて解放する。null は何もしない。スタブ。
#[unsafe(no_mangle)]
pub extern "C" fn ikari_candidate_stop(_core: *mut CandidateCore) -> i32 {
    IKARI_CANDIDATE_NOT_IMPLEMENTED
}

/// パネル全体の高さ（DIP）。`metrics::panel_height_dip` の C 版。
#[unsafe(no_mangle)]
pub extern "C" fn ikari_candidate_panel_height_dip(
    candidate_rows: u32,
    has_annotation: bool,
    has_developer_note: bool,
) -> f64 {
    panel_height_dip(PanelRows {
        candidate_rows,
        has_annotation,
        has_developer_note,
    })
}

/// 窓の左端の x（物理のピクセル）。`placement::horizontal_origin` の C 版。
#[unsafe(no_mangle)]
pub extern "C" fn ikari_candidate_horizontal_origin(
    anchor_x: f64,
    panel_width: f64,
    work_x: f64,
    work_width: f64,
) -> f64 {
    let anchor = Rect::new(anchor_x, 0.0, 0.0, 0.0);
    let work_area = Rect::new(work_x, 0.0, work_width, 0.0);
    horizontal_origin(&anchor, panel_width, &work_area)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stubs_are_inert() {
        assert_eq!(ikari_candidate_abi_version(), ABI_VERSION);
        assert_eq!(
            ikari_candidate_stop(std::ptr::null_mut()),
            IKARI_CANDIDATE_NOT_IMPLEMENTED
        );
    }
}
