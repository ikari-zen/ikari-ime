//! 方式 A: WebView2（tao + wry）。HTML と CSS で描く。
//! 実際の構成では TIP とは別の exe になる（このスパイクは単独の exe）。
//! 描き終わりは、JS の requestAnimationFrame を 2 回待ってから `window.ipc.postMessage` で知らせる（実測の近似）。

use super::common::*;
use crate::model::{html, html_height, Panel, HTML_WIDTH_PX};
use crate::stats::summarize;
use crate::Args;
use std::time::{Duration, Instant};
use tao::dpi::{LogicalSize, PhysicalPosition};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
use tao::window::WindowBuilder;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNOACTIVATE};
use wry::{WebContext, WebViewBuilder};

/// 方式 A が動かせなかった理由
pub enum Unavailable {
    /// WebView2 のランタイムが入っていない。呼び出し側は方式 B に切り替える（下の注）
    RuntimeMissing(String),
    Other(String),
}

/// WebView2 のランタイムの有無を確かめる。
/// `wry::webview_version()` は、ランタイム（Evergreen）のバージョンを `GetAvailableCoreWebView2BrowserVersionString` で調べる。
/// 無いときの扱い（案。製品での判断は ADR で決める）: インストーラが Evergreen Bootstrapper を同梱して入れる。
/// 入れられなかった／消されたときは、候補窓を方式 B（自前描画）に切り替える。このスパイクは後者を再現する
pub fn detect_runtime() -> Result<String, Unavailable> {
    wry::webview_version().map_err(|e| Unavailable::RuntimeMissing(e.to_string()))
}

enum Phase {
    WaitFirstPaint,
    Idle,
    WaitUpdatePaint,
    Hold,
}

pub fn run(args: &Args) -> Result<(), Unavailable> {
    let version = detect_runtime()?;
    println!("A: WebView2 runtime version = {version}");
    println!("A: waiting {} ms. Focus the target app now.", args.delay_ms);
    std::thread::sleep(Duration::from_millis(args.delay_ms));
    let before = foreground_snapshot();
    let t0 = qpc();
    let anchor = caret_anchor();
    println!(
        "A: anchor=({}, {}) source={}",
        anchor.x, anchor.y, anchor.source
    );
    let mut panel = Panel::fixed();

    let mut event_loop = EventLoopBuilder::<String>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_decorations(false)
        .with_always_on_top(true)
        .with_visible(false)
        .with_focused(false)
        .with_skip_taskbar(true)
        .with_inner_size(LogicalSize::new(
            HTML_WIDTH_PX,
            html_height(panel.reserved_rows),
        ))
        .with_position(PhysicalPosition::new(anchor.x, anchor.y))
        .build(&event_loop)
        .map_err(|e| Unavailable::Other(e.to_string()))?;
    let hwnd = HWND(window.hwnd() as *mut _);
    apply_noactivate_styles(hwnd);

    // 学習ファイルなどと同じ場所は使わない。スパイク専用の一時フォルダ
    let mut ctx = WebContext::new(Some(std::env::temp_dir().join("ikari-spike-webview2")));
    let webview = WebViewBuilder::new_with_web_context(&mut ctx)
        .with_html(html(&panel))
        .with_ipc_handler(move |req| {
            let _ = proxy.send_event(req.body().clone());
        })
        .build(&window)
        .map_err(|e| Unavailable::Other(e.to_string()))?;
    println!(
        "RESULT A webview_created={:.2}ms (show request -> WebView built)",
        qpc_ms(t0, qpc())
    );
    // SAFETY: 自分の窓。アクティブにしないで出す
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }

    let mut phase = Phase::WaitFirstPaint;
    let mut updates_left = args.updates;
    let mut samples: Vec<f64> = Vec::new();
    let mut t_update = 0i64;
    let tick = Duration::from_millis(args.tick_ms);
    let hold = Duration::from_millis(args.hold_ms);
    // 先頭の描画が来ないとき（JS が動かないなど）に終わるための期限
    let deadline = Instant::now() + Duration::from_secs(30);

    event_loop.run_return(|event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(msg) if msg.starts_with("painted:") => match phase {
                Phase::WaitFirstPaint => {
                    println!(
                        "RESULT A first_paint={:.2}ms (show request -> first frame painted in page)",
                        qpc_ms(t0, qpc())
                    );
                    if let Some(ms) = ms_since_process_creation() {
                        println!("RESULT A cold_start_total={ms:.1}ms (process creation -> now, includes the wait of --delay-ms)");
                    }
                    report_focus(&before, &foreground_snapshot());
                    print_memory("A after-first-paint");
                    phase = Phase::Idle;
                    *flow = ControlFlow::WaitUntil(Instant::now() + tick);
                }
                Phase::WaitUpdatePaint => {
                    samples.push(qpc_ms(t_update, qpc()));
                    phase = Phase::Idle;
                    *flow = ControlFlow::WaitUntil(Instant::now() + tick);
                }
                _ => {}
            },
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => match phase {
                Phase::Idle if updates_left > 0 => {
                    updates_left -= 1;
                    panel.advance();
                    t_update = qpc();
                    if webview.evaluate_script(&format!("setSelected({})", panel.selected)).is_err() {
                        println!("A: evaluate_script failed");
                        *flow = ControlFlow::Exit;
                    } else {
                        phase = Phase::WaitUpdatePaint;
                    }
                }
                Phase::Idle => {
                    summarize("A update (selection change -> frame painted in page)", &mut samples);
                    phase = Phase::Hold;
                    *flow = ControlFlow::WaitUntil(Instant::now() + hold);
                }
                Phase::Hold => *flow = ControlFlow::Exit,
                _ => {}
            },
            _ => {}
        }
        if matches!(phase, Phase::WaitFirstPaint | Phase::WaitUpdatePaint) && Instant::now() > deadline {
            println!("A: timeout waiting for paint message");
            *flow = ControlFlow::Exit;
        }
    });
    print_memory("A end");
    report_focus(&before, &foreground_snapshot());
    Ok(())
}
