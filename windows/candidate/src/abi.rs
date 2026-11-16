//! 画面（C#。`LibraryImport`）が呼ぶ C の ABI（ADR 0062 §7.3）。ヘッダは `include/ikari_candidate.h`（同じに保つ）。
//!
//! - 受け渡しは、数と、proto のバイト列（`RenderState`）だけ。Rust の型を境界に出さない（`shell-ffi` と同じ作法）。
//! - 「描く状態が変わった」の知らせは、核のスレッドから呼ばれる。C# は自分の UI のスレッド（`DispatcherQueue`）に戻して描く。
//!   知らせの中で、核の関数（`ikari_candidate_stop` など）を呼ばないこと。
//! - `ikari_candidate_start` は、Windows では、このユーザーとセッションの候補窓のパイプ（`pipe::candidate_pipe_name`）で
//!   受け付けを始める。高い権限で動いているときは始めない（ADR 0068 §5）。ほかの OS では始めない（null）。

use std::ffi::c_void;

use prost::Message;

use crate::command::RenderState;
use crate::metrics::{PanelRows, panel_height_dip};
use crate::placement::{horizontal_origin, vertical_origin};
use crate::receiver::{RenderSink, Running};
use ikari_shell_core::panel::{PanelSide, Rect};

/// ABI の版。互換を壊す変更で上げる。
pub const ABI_VERSION: u32 = 1;

/// 成功
pub const IKARI_CANDIDATE_OK: i32 = 0;
/// まだ実装されていない（スタブ。今は使わない。ヘッダとの互換のために残す）
pub const IKARI_CANDIDATE_NOT_IMPLEMENTED: i32 = -1;
/// 止めるときに受け付けのスレッドが誤りで終わっていた
pub const IKARI_CANDIDATE_ERROR: i32 = -2;

/// `ikari_candidate_vertical_origin` の `side` の値（proto の `PanelSide` と同じ番号）
pub const IKARI_CANDIDATE_SIDE_BELOW: i32 = 1;
pub const IKARI_CANDIDATE_SIDE_ABOVE: i32 = 2;

/// 「描く状態が変わった」の知らせ。`state` は `RenderState`（proto）のバイト列で、**この呼び出しの間だけ有効**（借用。
/// C# は中でコピーするか、デコードして戻る）。核のスレッドから呼ばれる。
pub type RenderStateCallback = extern "C" fn(state: *const u8, len: usize, user_data: *mut c_void);

/// 核の持ち物（C からは不透明なポインタ）。
pub struct CandidateCore {
    running: Running,
}

/// C# が渡した知らせの先。`user_data` は C# が持つもので、核は中を見ずに返すだけ。
struct Callback {
    f: RenderStateCallback,
    user_data: *mut c_void,
}

// SAFETY: `user_data` は C# が渡した不透明な値で、核は読み書きせず、知らせのときにそのまま返すだけ。
// どのスレッドから呼んでよいかは、ABI の約束（核のスレッドから呼ぶ）として C# が受け持つ。
unsafe impl Send for Callback {}
// SAFETY: 同上（核は `user_data` に触れない）。
unsafe impl Sync for Callback {}

impl Callback {
    fn call(&self, state: &RenderState) {
        let bytes = state.encode_to_vec();
        (self.f)(bytes.as_ptr(), bytes.len(), self.user_data);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn ikari_candidate_abi_version() -> u32 {
    ABI_VERSION
}

/// 核を起動する（パイプの受け手を始める）。失敗は null。戻り値は `ikari_candidate_stop` で必ず解放する。
#[unsafe(no_mangle)]
pub extern "C" fn ikari_candidate_start(
    callback: RenderStateCallback,
    user_data: *mut c_void,
) -> *mut CandidateCore {
    let callback = Callback {
        f: callback,
        user_data,
    };
    let sink: RenderSink = std::sync::Arc::new(move |state: &RenderState| callback.call(state));
    match start_platform(sink) {
        Some(running) => Box::into_raw(Box::new(CandidateCore { running })),
        None => std::ptr::null_mut(),
    }
}

#[cfg(windows)]
fn start_platform(sink: RenderSink) -> Option<Running> {
    use ikari_pipe::elevation;
    use ikari_pipe::os::windows::{
        NamedPipeListener, current_integrity_rid, current_session_id, current_user_sid,
    };
    // 高い権限では動かない（ADR 0068 §5）
    elevation::check_self(current_integrity_rid()).ok()?;
    let name =
        ikari_pipe::candidate_pipe_name(&current_user_sid().ok()?, current_session_id().ok()?)
            .ok()?;
    let listener = NamedPipeListener::bind(&name).ok()?;
    let closer = Box::new(listener.closer());
    crate::receiver::spawn(listener, Some(closer), sink, None).ok()
}

#[cfg(not(windows))]
fn start_platform(_sink: RenderSink) -> Option<Running> {
    None
}

/// 核を止めて解放する。null は何もしない（`IKARI_CANDIDATE_OK`）。
///
/// # Safety
/// `core` は null か、`ikari_candidate_start` が返してまだ止めていないもの。同じ値を 2 回渡さない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_candidate_stop(core: *mut CandidateCore) -> i32 {
    if core.is_null() {
        return IKARI_CANDIDATE_OK;
    }
    // SAFETY: `core` は `ikari_candidate_start` が `Box::into_raw` で返したもので、C# はこれを 1 回だけ渡す（ABI の約束）。
    let core = unsafe { Box::from_raw(core) };
    match core.running.stop() {
        Ok(()) => IKARI_CANDIDATE_OK,
        Err(_) => IKARI_CANDIDATE_ERROR,
    }
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

/// 窓の上端の y（物理のピクセル。y は下向き）。`placement::vertical_origin` の C 版。
/// `side` は `IKARI_CANDIDATE_SIDE_*`（知らない値は下）。
#[unsafe(no_mangle)]
pub extern "C" fn ikari_candidate_vertical_origin(
    anchor_y: f64,
    anchor_height: f64,
    panel_height: f64,
    side: i32,
    work_y: f64,
    work_height: f64,
) -> f64 {
    let anchor = Rect::new(0.0, anchor_y, 0.0, anchor_height);
    let work_area = Rect::new(0.0, work_y, 0.0, work_height);
    let side = if side == IKARI_CANDIDATE_SIDE_ABOVE {
        PanelSide::Above
    } else {
        PanelSide::Below
    };
    vertical_origin(&anchor, panel_height, side, &work_area)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikari_proto::shell::v1::PanelSide as PbSide;

    #[test]
    fn version_and_null_stop() {
        assert_eq!(ikari_candidate_abi_version(), ABI_VERSION);
        assert_eq!(
            // SAFETY: null は何もしない
            unsafe { ikari_candidate_stop(std::ptr::null_mut()) },
            IKARI_CANDIDATE_OK
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn start_is_null_outside_windows() {
        extern "C" fn ignore(_state: *const u8, _len: usize, _user_data: *mut c_void) {}
        assert!(ikari_candidate_start(ignore, std::ptr::null_mut()).is_null());
    }

    #[test]
    fn side_constants_match_the_proto() {
        assert_eq!(IKARI_CANDIDATE_SIDE_BELOW, PbSide::Below as i32);
        assert_eq!(IKARI_CANDIDATE_SIDE_ABOVE, PbSide::Above as i32);
    }

    #[test]
    fn c_functions_match_the_rust_ones() {
        assert_eq!(
            ikari_candidate_panel_height_dip(3, true, false),
            panel_height_dip(PanelRows {
                candidate_rows: 3,
                has_annotation: true,
                has_developer_note: false
            })
        );
        assert_eq!(
            ikari_candidate_horizontal_origin(1850.0, 200.0, 0.0, 1920.0),
            1720.0
        );
        assert_eq!(
            ikari_candidate_vertical_origin(
                500.0,
                20.0,
                100.0,
                IKARI_CANDIDATE_SIDE_ABOVE,
                0.0,
                1040.0
            ),
            400.0
        );
        assert_eq!(
            ikari_candidate_vertical_origin(500.0, 20.0, 100.0, 0, 0.0, 1040.0),
            520.0
        );
    }

    #[test]
    fn callback_receives_decodable_bytes() {
        use std::sync::Mutex;
        static SEEN: Mutex<Vec<u8>> = Mutex::new(Vec::new());
        extern "C" fn record(state: *const u8, len: usize, _user_data: *mut c_void) {
            // SAFETY: 核は、この呼び出しの間生きている `len` バイトを渡す
            let bytes = unsafe { std::slice::from_raw_parts(state, len) };
            *SEEN.lock().unwrap() = bytes.to_vec();
        }
        let cb = Callback {
            f: record,
            user_data: std::ptr::null_mut(),
        };
        let state = RenderState {
            visible: true,
            revision: 7,
            ..Default::default()
        };
        cb.call(&state);
        let back = RenderState::decode(SEEN.lock().unwrap().as_slice()).unwrap();
        assert_eq!(back, state);
    }
}
