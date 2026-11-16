//! 候補窓の位置の材料を OS から取る（ADR 0062 §6）。判断は持たない（どれを使うか、いつ取り直すかは
//! `candidate_view`）。ここは、OS の値を物理の画面座標（左上が原点、y は下向き）にそろえて返すだけ。
//!
//! - アンカー: 合成（なければ選択）の**先頭の 1 文字**の `ITfContextView::GetTextExt`。1 文字で答えなければ
//!   長さ 0 も試す（ADR 0062 §6.1）。返った矩形は、アプリの窓（`ITfContextView::GetWnd`）の DPI の扱いの座標なので、
//!   `LogicalToPhysicalPointForPerMonitorDPI` で物理の座標に直す（DPI に対応したアプリでは変わらない。§6.3）。
//! - 画面: 作業領域（`GetMonitorInfoW` の `rcWork`）と倍率（`GetDpiForMonitor` の実効の DPI / 96）。アプリのスレッドの
//!   DPI の扱いに引きずられないよう、読む間だけスレッドを Per-Monitor V2 にして、戻す。
//! - 最後に取れた正常なアンカーは、このプロセス（= アプリ）で 1 つ持つ（ADR 0062 §6.1）。
//!
//! 推測（実機で確かめる。`docs/windows-real-device-check.md`）: DPI に対応しないアプリで `GetTextExt` が論理の座標を
//! 返し、上の変換で物理の座標になる。

use std::sync::{Mutex, PoisonError};

use ikari_shell_core::panel::{Point, Rect};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    MonitorFromRect,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor,
    LogicalToPhysicalPointForPerMonitorDPI, MDT_EFFECTIVE_DPI, SetThreadDpiAwarenessContext,
};
use windows::Win32::UI::TextServices::{ITfComposition, ITfContext, ITfRange, TF_ANCHOR_START};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
use windows_core::BOOL;

use super::edit::default_selection;
use crate::candidate_view::{Desktop, Screen, rect_from_corners};

/// 倍率の基準の DPI（100%）
const BASE_DPI: f64 = 96.0;
/// アンカーにする文字の数（合成の先頭の 1 文字）
const ANCHOR_CHARS: i32 = 1;

/// クライアントから取ったアンカー（物理の画面座標）と、`GetTextExt` の `clipped`（診断用）。
#[derive(Debug, Clone, Copy)]
pub struct ClientAnchor {
    pub rect: Rect,
    pub clipped: bool,
}

/// 合成（なければ選択）の先頭の 1 文字の矩形。取れなければ `None`。
pub fn client_anchor(
    ec: u32,
    context: &ITfContext,
    composition: Option<&ITfComposition>,
) -> Option<ClientAnchor> {
    let range = match composition {
        // SAFETY: 合成は生きている（呼ぶ側が持つ）
        Some(composition) => unsafe { composition.GetRange() }.ok()?,
        None => default_selection(ec, context)?,
    };
    // SAFETY: 文書は生きている
    let view = unsafe { context.GetActiveView() }.ok()?;
    // 窓のない文書（`GetWnd` が失敗する、または null）は、変換せずに使う
    // SAFETY: 同上
    let hwnd = unsafe { view.GetWnd() }.ok().filter(|h| !h.is_invalid());
    let start = collapsed_start(ec, &range)?;
    let probes = [first_chars(ec, &start), Some(start)];
    probes.into_iter().flatten().find_map(|probe| {
        let mut rect = RECT::default();
        let mut clipped = BOOL::default();
        // SAFETY: `rect` と `clipped` は書ける場所。`ec` はこのセッションのもの
        unsafe { view.GetTextExt(ec, &probe, &mut rect, &mut clipped) }.ok()?;
        Some(ClientAnchor {
            rect: to_physical(hwnd, rect),
            clipped: clipped.as_bool(),
        })
    })
}

/// 範囲の先頭に縮めた複製。
fn collapsed_start(ec: u32, range: &ITfRange) -> Option<ITfRange> {
    // SAFETY: `ec` はこのセッションのもの。`range` は生きている
    let copy = unsafe { range.Clone() }.ok()?;
    // SAFETY: 同上
    unsafe { copy.Collapse(ec, TF_ANCHOR_START) }.ok()?;
    Some(copy)
}

/// `start`（長さ 0）から 1 文字ぶん伸ばした範囲。伸ばせなければ（合成が空）`None`。
fn first_chars(ec: u32, start: &ITfRange) -> Option<ITfRange> {
    // SAFETY: `ec` はこのセッションのもの。`start` は生きている
    let range = unsafe { start.Clone() }.ok()?;
    let mut moved = 0i32;
    // SAFETY: `moved` は書ける場所。止める条件はなし（文書の終わりで止まる）
    unsafe { range.ShiftEnd(ec, ANCHOR_CHARS, &mut moved, std::ptr::null()) }.ok()?;
    (moved == ANCHOR_CHARS).then_some(range)
}

/// アプリの窓の座標を物理の画面座標に直す。直せなければ、そのまま使う。
fn to_physical(hwnd: Option<HWND>, rect: RECT) -> Rect {
    let convert = |x: i32, y: i32| {
        let raw = POINT { x, y };
        let point = hwnd
            .and_then(|hwnd| {
                let mut converted = raw;
                // SAFETY: `converted` は書ける場所。`hwnd` は TSF が返したアプリの窓（閉じていれば失敗が返るだけ）
                unsafe { LogicalToPhysicalPointForPerMonitorDPI(Some(hwnd), &mut converted) }
                    .as_bool()
                    .then_some(converted)
            })
            .unwrap_or(raw);
        Point {
            x: f64::from(point.x),
            y: f64::from(point.y),
        }
    };
    rect_from_corners(
        convert(rect.left, rect.top),
        convert(rect.right, rect.bottom),
    )
}

fn rect_of(rect: &RECT) -> Rect {
    rect_from_corners(
        Point {
            x: f64::from(rect.left),
            y: f64::from(rect.top),
        },
        Point {
            x: f64::from(rect.right),
            y: f64::from(rect.bottom),
        },
    )
}

/// 四捨五入して `i32` に収める（画面座標の範囲の外は端に寄せる）。
fn to_i32(value: f64) -> i32 {
    value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

/// 読む間だけ、このスレッドの DPI の扱いを Per-Monitor V2 にする（落とすと元に戻す）。
/// 古い OS などで変えられなければ、何もしない（アプリの扱いのまま読む）。
struct PerMonitorScope(DPI_AWARENESS_CONTEXT);

impl PerMonitorScope {
    fn enter() -> Self {
        // SAFETY: 値を渡すだけ。失敗なら無効な値が返る
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
    }
}

impl Drop for PerMonitorScope {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: `enter` で返った、元の扱いに戻すだけ
            unsafe { SetThreadDpiAwarenessContext(self.0) };
        }
    }
}

/// 画面 1 枚の作業領域と倍率（Per-Monitor V2 の中で呼ぶ）。
fn screen_of_monitor(monitor: HMONITOR) -> Option<Screen> {
    if monitor.is_invalid() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` は書ける場所で、`cbSize` を埋めてある
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return None;
    }
    let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
    // SAFETY: 書ける場所を 2 つ渡す
    unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }.ok()?;
    if dpi_x == 0 {
        return None;
    }
    Some(Screen {
        work_area: rect_of(&info.rcWork),
        scale: f64::from(dpi_x) / BASE_DPI,
    })
}

/// このプロセスで最後に取れた正常なアンカー
static LAST_KNOWN: Mutex<Option<Rect>> = Mutex::new(None);

/// OS の画面の情報（`candidate_view::Desktop` の Windows の実装）。
pub struct Win32Desktop;

impl Desktop for Win32Desktop {
    fn screen_of(&self, rect: &Rect) -> Option<Screen> {
        let _scope = PerMonitorScope::enter();
        let rect = RECT {
            left: to_i32(rect.min_x()),
            top: to_i32(rect.min_y()),
            right: to_i32(rect.max_x()),
            bottom: to_i32(rect.max_y()),
        };
        // SAFETY: `rect` は読める場所
        screen_of_monitor(unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST) })
    }

    fn cursor_screen(&self) -> Option<Screen> {
        let _scope = PerMonitorScope::enter();
        let mut point = POINT::default();
        // SAFETY: `point` は書ける場所
        unsafe { GetCursorPos(&mut point) }.ok()?;
        // SAFETY: 値を渡すだけ
        screen_of_monitor(unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) })
    }

    fn last_known(&self) -> Option<Rect> {
        *LAST_KNOWN.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn remember(&self, anchor: Rect) {
        *LAST_KNOWN.lock().unwrap_or_else(PoisonError::into_inner) = Some(anchor);
    }
}
