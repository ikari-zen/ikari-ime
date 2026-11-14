//! ikari-lm-host の試作。stdin から 1 行 1 要求（`プロンプト TAB 候補 TAB 候補 …`）、stdout に 1 行 1 結果
//! （空白区切りの NLL。失敗は `ERR メッセージ`）。起動して読み込みが終わると `READY` を 1 行出す。
//! 採点は `ikari_lm::LlamaScorer` をそのまま使う。OS の隔離は作らない（計測のみ）。

use std::io::{BufRead, Write};
use std::path::PathBuf;

use ikari_lm::{LlamaScorer, LmScorer};

fn main() {
    let Some(model) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: ikari-lm-host <model.gguf>");
        std::process::exit(2);
    };
    let mut scorer = match LlamaScorer::load(&model) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("load failed: {e:?}");
            std::process::exit(1);
        }
    };
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = writeln!(out, "READY");
    let _ = out.flush();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let mut parts = line.split('\t');
        let prompt = parts.next().unwrap_or("");
        let cands: Vec<String> = parts.map(str::to_owned).collect();
        let reply = match scorer.nll(prompt, &cands) {
            Ok(v) => v
                .iter()
                .map(|x| format!("{x:?}"))
                .collect::<Vec<_>>()
                .join(" "),
            Err(e) => format!("ERR {e:?}").replace('\n', " "),
        };
        if writeln!(out, "{reply}").is_err() || out.flush().is_err() {
            break;
        }
    }
}
