//! 偽の lm-host（テスト用。モデル不要）。引数でふるまいを選ぶ。プロトコルは host.rs と同じ。
//! NLL は、候補が「良」なら 0.0、それ以外は 20.0。
//!
//! 引数: `ok` | `die-after=N`（N 件答えたあと、次の要求で何も返さず終了）|
//!       `hang-after=N`（N 件答えたあと、次の要求で応答を返さず眠る）| `die-on-start`（READY 前に終了）

use std::io::{BufRead, Write};

const HANG_SLEEP_SECS: u64 = 3600;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "ok".to_owned());
    if mode == "die-on-start" {
        std::process::exit(1);
    }
    let out = std::io::stdout();
    let mut out = out.lock();
    let _ = writeln!(out, "READY");
    let _ = out.flush();
    for (answered, line) in std::io::stdin().lock().lines().enumerate() {
        let Ok(line) = line else { break };
        if let Some(n) = mode.strip_prefix("die-after=")
            && answered >= n.parse().unwrap_or(0)
        {
            std::process::exit(9);
        }
        if let Some(n) = mode.strip_prefix("hang-after=")
            && answered >= n.parse().unwrap_or(0)
        {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(HANG_SLEEP_SECS));
            }
        }
        let reply = line
            .split('\t')
            .skip(1)
            .map(|c| if c == "良" { "0.0" } else { "20.0" })
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(out, "{reply}");
        let _ = out.flush();
    }
}
