//! U-a: WebView2（tao + wry）で HTML と CSS の設定画面を出す。捨てるスパイク（#322）。
//! 設定の変更は標準出力に `CHANGE key<TAB>value`、計測は `RESULT ...`。

#[path = "../model.rs"]
mod model;

#[cfg(windows)]
#[path = "../args.rs"]
mod args;
#[cfg(windows)]
#[path = "../html.rs"]
mod html;
#[cfg(windows)]
#[path = "../metrics.rs"]
mod metrics;

#[cfg(not(windows))]
fn main() {
    eprintln!("settings-webview は Windows でだけ動く（スパイク）");
    let _ = model::fixed_items();
}

#[cfg(windows)]
fn main() {
    use std::time::{Duration, Instant};
    use tao::dpi::LogicalSize;
    use tao::event::{Event, StartCause, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use wry::{WebContext, WebViewBuilder};

    let t_main = metrics::qpc();
    let args = args::Args::parse();
    let items = model::fixed_items();

    match wry::webview_version() {
        Ok(v) => println!("WebView2 runtime version = {v}"),
        Err(e) => {
            println!("RESULT WEBVIEW NOT FOUND: {e}");
            return;
        }
    }

    let event_loop = EventLoopBuilder::<String>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = match tao::window::WindowBuilder::new()
        .with_title("ikari-ime 設定（WebView2）")
        .with_inner_size(LogicalSize::new(760.0, 640.0))
        .build(&event_loop)
    {
        Ok(w) => w,
        Err(e) => {
            println!("window build failed: {e}");
            return;
        }
    };
    // ユーザーデータと混ざらない、スパイク専用の一時フォルダ
    let mut ctx = WebContext::new(Some(
        std::env::temp_dir().join("ikari-spike-settings-webview2"),
    ));
    let webview = match WebViewBuilder::new_with_web_context(&mut ctx)
        .with_html(html::page(&items))
        .with_ipc_handler(move |req| {
            let _ = proxy.send_event(req.body().clone());
        })
        .build(&window)
    {
        Ok(w) => w,
        Err(e) => {
            println!("webview build failed: {e}");
            return;
        }
    };
    println!(
        "RESULT A webview_created={:.1}ms (main -> WebView built)",
        metrics::qpc_ms(t_main, metrics::qpc())
    );

    let mut settle_at: Option<Instant> = None;
    event_loop.run(move |event, _, flow| {
        let _keep = &webview; // 窓が閉じるまで WebView を生かす
        *flow = match settle_at {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        };
        match event {
            Event::UserEvent(msg) => {
                if msg == "ready" {
                    metrics::report_first_paint("A", t_main);
                    settle_at = Some(Instant::now() + Duration::from_millis(args.settle_ms));
                } else if let Some(rest) = msg.strip_prefix("change\t") {
                    if let Some((k, v)) = rest.split_once('\t') {
                        model::report_change(k, v);
                    }
                }
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if settle_at.take().is_some() {
                    metrics::print_memory("A settled");
                    if args.exit_after_settle {
                        *flow = ControlFlow::Exit;
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                metrics::print_memory("A at-close");
                *flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}
