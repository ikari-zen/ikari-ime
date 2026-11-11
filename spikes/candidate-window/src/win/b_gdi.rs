//! 方式 B: 自前描画。GDI（軽さ優先。Direct2D / DirectWrite は未実装）。
//! 窓はこのプロセスの中。実際の TIP では DLL の中に置く形か別 exe にする形になる（このスパイクは単独の exe）。

use super::common::*;
use crate::model::Panel;
use crate::stats::summarize;
use crate::Args;
use std::cell::RefCell;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush,
    DeleteDC, DeleteObject, EndPaint, FillRect, GdiFlush, GetDC, GetTextExtentPoint32W,
    InvalidateRect, ReleaseDC, SelectObject, SetBkMode, SetTextColor, TextOutW, UpdateWindow,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, HFONT, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    KillTimer, LoadCursorW, PostQuitMessage, RegisterClassW, SetTimer, SetWindowPos, ShowWindow,
    TranslateMessage, CS_DROPSHADOW, HWND_TOPMOST, IDC_ARROW, MA_NOACTIVATE, MSG, SWP_NOACTIVATE,
    SW_SHOWNOACTIVATE, WM_DESTROY, WM_MOUSEACTIVATE, WM_PAINT, WM_TIMER, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const TIMER_TICK: usize = 1;
const TIMER_END: usize = 2;
const FONT_PX_AT_96DPI: i32 = 16;
const PAD_PX_AT_96DPI: i32 = 8;
const ROW_PX_AT_96DPI: i32 = 28;
const GAP_PX_AT_96DPI: i32 = 12;
const COLOR_BG: u32 = rgb(0xff, 0xff, 0xff);
const COLOR_SEL: u32 = rgb(0xcf, 0xe3, 0xff);
const COLOR_TEXT: u32 = rgb(0x20, 0x20, 0x20);
const COLOR_DIM: u32 = rgb(0x80, 0x80, 0x80);

const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}

struct State {
    panel: Panel,
    font: HFONT,
    scale: f32,
    num_w: i32,
    width: i32,
    height: i32,
    t0: i64,
    first_paint: Option<i64>,
    samples: Vec<f64>,
    updates_left: usize,
    hold_ms: u32,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn px(v: i32, scale: f32) -> i32 {
    (v as f32 * scale).round() as i32
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// 方式 B を動かす。`Err` は準備に失敗したとき
pub fn run(args: &Args) -> Result<(), String> {
    // SAFETY: プロセスの最初に 1 回。失敗しても（すでに設定済みなど）続けられる
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    println!("B: waiting {} ms. Focus the target app now.", args.delay_ms);
    std::thread::sleep(std::time::Duration::from_millis(args.delay_ms));
    let before = foreground_snapshot();
    let t0 = qpc();
    let anchor = caret_anchor();
    println!(
        "B: anchor=({}, {}) source={}",
        anchor.x, anchor.y, anchor.source
    );
    let panel = Panel::fixed();

    // SAFETY: Win32 の窓の作成。ポインタはこの関数の間は有効
    unsafe {
        let hinst = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = w!("IkariSpikeCandidateB");
        let wc = WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).map_err(|e| e.to_string())?,
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class,
            PCWSTR::null(),
            WS_POPUP,
            anchor.x,
            anchor.y,
            10,
            10,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .map_err(|e| e.to_string())?;

        // 窓の置かれたモニタの DPI で、字と寸法を決める
        let dpi = GetDpiForWindow(hwnd).max(96);
        let scale = dpi as f32 / 96.0;
        println!("B: dpi={dpi} scale={scale}");
        let font = CreateFontW(
            -px(FONT_PX_AT_96DPI, scale),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Yu Gothic UI"),
        );
        let (num_w, width, height) = measure(hwnd, font, &panel, scale);
        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                panel,
                font,
                scale,
                num_w,
                width,
                height,
                t0,
                first_paint: None,
                samples: Vec::new(),
                updates_left: args.updates,
                hold_ms: args.hold_ms as u32,
            })
        });
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            anchor.x,
            anchor.y,
            width,
            height,
            SWP_NOACTIVATE,
        );
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = UpdateWindow(hwnd); // 同期で WM_PAINT を回す
        let t_done = qpc();
        let first = STATE.with(|s| s.borrow().as_ref().and_then(|st| st.first_paint));
        println!(
            "RESULT B first_paint={:.2}ms (show request -> first WM_PAINT done), call_return={:.2}ms",
            qpc_ms(t0, first.unwrap_or(t_done)),
            qpc_ms(t0, t_done)
        );
        if let Some(ms) = ms_since_process_creation() {
            println!("RESULT B cold_start_total={ms:.1}ms (process creation -> now, includes the wait of --delay-ms)");
        }
        report_focus(&before, &foreground_snapshot());
        print_memory("B after-first-paint");

        SetTimer(Some(hwnd), TIMER_TICK, args.tick_ms as u32, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    print_memory("B end");
    report_focus(&before, &foreground_snapshot());
    Ok(())
}

/// 番号の幅、窓の幅、高さ（行数は予約した数で固定。ページの替わりで大きさを変えない）
unsafe fn measure(hwnd: HWND, font: HFONT, panel: &Panel, scale: f32) -> (i32, i32, i32) {
    let dc = GetDC(Some(hwnd));
    let old = SelectObject(dc, font.into());
    let text_w = |s: &str| -> i32 {
        let v = utf16(s);
        let mut sz = SIZE::default();
        let _ = GetTextExtentPoint32W(dc, &v, &mut sz);
        sz.cx
    };
    let gap = px(GAP_PX_AT_96DPI, scale);
    let pad = px(PAD_PX_AT_96DPI, scale);
    let num_w = text_w("0") + gap / 2;
    let mut body = 0;
    for c in &panel.candidates {
        body = body.max(text_w(c.surface) + gap + text_w(c.annotation) + gap + text_w(c.label));
    }
    let footer_w = text_w(&panel.footer());
    let width = pad * 2 + num_w + body.max(footer_w);
    let height = px(ROW_PX_AT_96DPI, scale) * (panel.reserved_rows as i32 + 1) + 2;
    SelectObject(dc, old);
    ReleaseDC(Some(hwnd), dc);
    (num_w, width, height)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_TIMER => {
            on_timer(hwnd, wp.0);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn on_timer(hwnd: HWND, id: usize) {
    if id == TIMER_END {
        let _ = KillTimer(Some(hwnd), TIMER_END);
        let _ = DestroyWindow(hwnd);
        return;
    }
    let t_start = qpc();
    let finished = STATE.with(|s| {
        let mut b = s.borrow_mut();
        let st = b.as_mut().expect("state");
        if st.updates_left == 0 {
            return true;
        }
        st.updates_left -= 1;
        st.panel.advance();
        false
    });
    if finished {
        let _ = KillTimer(Some(hwnd), TIMER_TICK);
        STATE.with(|s| {
            let mut b = s.borrow_mut();
            let st = b.as_mut().expect("state");
            summarize(
                "B update (selection change -> repaint done)",
                &mut st.samples,
            );
            SetTimer(Some(hwnd), TIMER_END, st.hold_ms, None);
        });
        return;
    }
    let _ = InvalidateRect(Some(hwnd), None, false);
    let _ = UpdateWindow(hwnd);
    let t_end = qpc();
    STATE.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            st.samples.push(qpc_ms(t_start, t_end));
        }
    });
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    STATE.with(|s| {
        let mut b = s.borrow_mut();
        let Some(st) = b.as_mut() else { return };
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        // ちらつきを避けるため、裏のビットマップに描いてから写す
        let mem = CreateCompatibleDC(Some(hdc));
        let bmp = CreateCompatibleBitmap(hdc, w, h);
        let old_bmp = SelectObject(mem, bmp.into());
        let old_font = SelectObject(mem, st.font.into());
        let fill = |color: u32, r: &RECT| {
            let br = CreateSolidBrush(COLORREF(color));
            FillRect(mem, r, br);
            let _ = DeleteObject(br.into());
        };
        fill(COLOR_BG, &rc);
        SetBkMode(mem, TRANSPARENT);
        let pad = px(PAD_PX_AT_96DPI, st.scale);
        let gap = px(GAP_PX_AT_96DPI, st.scale);
        let row_h = px(ROW_PX_AT_96DPI, st.scale);
        let text_y = (row_h - px(FONT_PX_AT_96DPI, st.scale)) / 2;
        let text_w = |s: &str| -> i32 {
            let v = utf16(s);
            let mut sz = SIZE::default();
            let _ = GetTextExtentPoint32W(mem, &v, &mut sz);
            sz.cx
        };
        let draw = |x: i32, y: i32, s: &str, color: u32| {
            SetTextColor(mem, COLORREF(color));
            let _ = TextOutW(mem, x, y, &utf16(s));
        };
        for (i, (n, item, selected)) in st.panel.rows().into_iter().enumerate() {
            let y = i as i32 * row_h;
            if selected {
                fill(
                    COLOR_SEL,
                    &RECT {
                        left: 0,
                        top: y,
                        right: w,
                        bottom: y + row_h,
                    },
                );
            }
            draw(pad, y + text_y, &n.to_string(), COLOR_DIM);
            let sx = pad + st.num_w;
            draw(sx, y + text_y, item.surface, COLOR_TEXT);
            if selected && !item.annotation.is_empty() {
                draw(
                    sx + text_w(item.surface) + gap,
                    y + text_y,
                    item.annotation,
                    COLOR_DIM,
                );
            }
            if !item.label.is_empty() {
                draw(
                    w - pad - text_w(item.label),
                    y + text_y,
                    item.label,
                    COLOR_DIM,
                );
            }
        }
        let fy = st.panel.reserved_rows as i32 * row_h;
        draw(pad, fy + text_y, &st.panel.footer(), COLOR_DIM);
        let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
        let _ = GdiFlush();
        SelectObject(mem, old_font);
        SelectObject(mem, old_bmp);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        if st.first_paint.is_none() {
            st.first_paint = Some(qpc());
        }
        let _ = (st.width, st.height, st.t0);
    });
    let _ = EndPaint(hwnd, &ps);
}
