pub mod a_webview;
pub mod b_gdi;
pub mod c_tsf;
mod common;

use crate::Args;

pub fn run(args: &Args) -> i32 {
    common::print_memory("startup");
    let result = match args.method {
        'a' => match a_webview::run(args) {
            Ok(()) => Ok(()),
            Err(a_webview::Unavailable::RuntimeMissing(e)) => {
                println!("RESULT A WebView2 runtime NOT FOUND ({e}). Falling back to B.");
                b_gdi::run(args)
            }
            Err(a_webview::Unavailable::Other(e)) => Err(format!("A failed: {e}")),
        },
        'b' => b_gdi::run(args),
        'c' => c_tsf::run(args),
        _ => Err("method must be a, b or c".to_string()),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}
