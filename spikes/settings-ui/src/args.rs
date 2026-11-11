//! 2 つの exe で共通のオプション。
//! `--settle-ms N`: 最初の描画のあと、メモリを測り直すまでの待ち（既定 3000）。
//! `--exit-after-settle`: 測り直したら自動で終わる（スクリプトで回すとき）。

pub struct Args {
    pub settle_ms: u64,
    pub exit_after_settle: bool,
}

impl Args {
    pub fn parse() -> Args {
        let mut a = Args {
            settle_ms: 3000,
            exit_after_settle: false,
        };
        let mut it = std::env::args().skip(1);
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--settle-ms" => {
                    if let Some(v) = it.next().and_then(|v| v.parse().ok()) {
                        a.settle_ms = v;
                    }
                }
                "--exit-after-settle" => a.exit_after_settle = true,
                other => eprintln!("unknown option: {other}"),
            }
        }
        a
    }
}
