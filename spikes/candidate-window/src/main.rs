//! 候補窓の方式のスパイク（ADR 0040 §2.4、issue #319）。捨てるコード。main に入れない。
//! 使い方: `candidate-window-spike <a|b|c> [--delay-ms N] [--updates N] [--tick-ms N] [--hold-ms N]`

#[cfg_attr(not(windows), allow(dead_code))]
mod model;
#[cfg_attr(not(windows), allow(dead_code))]
mod stats;
#[cfg(windows)]
mod win;

#[cfg_attr(not(windows), allow(dead_code))]
pub struct Args {
    pub method: char,
    /// 起動してから窓を出すまでの待ち。この間に対象のアプリ（Notepad など）を前面にする
    pub delay_ms: u64,
    /// 選択を動かす回数（更新の時間の標本数）
    pub updates: usize,
    /// 更新の間隔
    pub tick_ms: u64,
    /// 更新が終わってから窓を残す時間（目で見る）
    pub hold_ms: u64,
}

const DEFAULT_DELAY_MS: u64 = 3000;
const DEFAULT_UPDATES: usize = 50;
const DEFAULT_TICK_MS: u64 = 100;
const DEFAULT_HOLD_MS: u64 = 5000;

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let method = match it.next().as_deref() {
        Some("a") => 'a',
        Some("b") => 'b',
        Some("c") => 'c',
        _ => return Err("usage: candidate-window-spike <a|b|c> [--delay-ms N] [--updates N] [--tick-ms N] [--hold-ms N]".into()),
    };
    let mut args = Args {
        method,
        delay_ms: DEFAULT_DELAY_MS,
        updates: DEFAULT_UPDATES,
        tick_ms: DEFAULT_TICK_MS,
        hold_ms: DEFAULT_HOLD_MS,
    };
    while let Some(flag) = it.next() {
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        let n: u64 = value
            .parse()
            .map_err(|_| format!("{flag}: not a number: {value}"))?;
        match flag.as_str() {
            "--delay-ms" => args.delay_ms = n,
            "--updates" => args.updates = n as usize,
            "--tick-ms" => args.tick_ms = n,
            "--hold-ms" => args.hold_ms = n,
            _ => return Err(format!("unknown flag: {flag}")),
        }
    }
    Ok(args)
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    #[cfg(windows)]
    std::process::exit(win::run(&args));
    #[cfg(not(windows))]
    {
        let _ = (&args, model::html(&model::Panel::fixed()));
        eprintln!("This spike runs on Windows only.");
        std::process::exit(1);
    }
}
