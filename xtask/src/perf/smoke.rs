//! `cargo xtask perf smoke`: P2 変換の往復（Space から 1 ページ目まで。XPC 込み）（ADR 0041 §6.3）。
//!
//! 入れてある ikari.app の `ikari-smoke`（`macos/scripts/smoke.sh --installed`）を流し、`latency` の行の p50・p99・max を読む。
//! **smoke は別のエンジンを起動して学習ファイルのロックを取るので、その間ユーザーの ikari は漢字に変換できなくなる。**
//! そのため `--allow-smoke`（ユーザーの了解を取ったとき）を付けたときだけ流す。LM あり・なしは `--label` で区別して 2 回流す。

use std::path::PathBuf;
use std::process::Command;

use super::record::{self, Collector, LoadWatch, err, repo_root};
use super::shell::print_table;
use crate::XtaskError;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// ユーザーの了解を取ったときだけ付ける（smoke の間、ユーザーの ikari は変換できなくなる）
    #[arg(long)]
    pub allow_smoke: bool,
    /// 指標の名前に入れる印（例: `lm`、`nolm`）。smoke の設定でエンジンの LM を切り替えて 2 回流すときに使う
    #[arg(long, default_value = "default")]
    pub label: String,
    /// 計測をくり返す回数
    #[arg(long, default_value_t = 5)]
    pub runs: usize,
    /// 結果の書き出し先（あれば、ほかの区間の指標を残して混ぜる）
    #[arg(long)]
    pub out: Option<PathBuf>,
}

/// smoke の `latency` の行（`PASS  latency  n=50 p50=1.23ms p99=4.56ms max=7.89ms failures=0`）から、ミリ秒の値を読む。
pub fn parse_latency(output: &str) -> Option<[f64; 3]> {
    let line = output
        .lines()
        .find(|line| line.contains("latency") && line.contains("p50="))?;
    let get = |key: &str| -> Option<f64> {
        let start = line.find(key)? + key.len();
        let rest = &line[start..];
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        rest[..end].parse().ok()
    };
    Some([get("p50=")?, get("p99=")?, get("max=")?])
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    if !args.allow_smoke {
        return Err(err(
            "perf smoke は smoke を流します。流す間、ユーザーの ikari は変換できなくなるので、ユーザーの了解を取って --allow-smoke を付けてください",
        ));
    }
    if args.runs == 0 {
        return Err(err("--runs は 1 以上"));
    }
    let mut collector = Collector::default();
    let mut watch = LoadWatch::start();
    for run in 1..=args.runs {
        watch.sample();
        eprintln!("perf smoke: {run}/{}", args.runs);
        let output = Command::new(repo_root().join("macos/scripts/smoke.sh"))
            .arg("--installed")
            .output()
            .map_err(|error| err(format!("smoke.sh: 起動できません: {error}")))?;
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.status.success() {
            return Err(err(format!(
                "smoke.sh が失敗しました（終了コード {:?}。2 は engine.lock が使用中: ABC などに切り替える）:\n{text}{}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let [p50, p99, max] = parse_latency(&text)
            .ok_or_else(|| err(format!("smoke の出力に latency の行がありません:\n{text}")))?;
        for (key, value) in [("p50_ms", p50), ("p99_ms", p99), ("max_ms", max)] {
            collector.push(format!("p2.{}.{key}", args.label), "ms", value);
        }
    }
    let metrics = collector.into_metrics();
    print_table(&metrics);
    if let Some(out) = &args.out {
        let mut section = watch.finish();
        section["measured_at"] = record::now_utc().into();
        section["runs"] = args.runs.into();
        section["label"] = args.label.clone().into();
        record::merge_into(out, &format!("smoke_{}", args.label), section, metrics)?;
        eprintln!("perf smoke: {} に書きました", out.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_latency_line() {
        let output =
            "PASS  hello\nPASS  latency  n=50 p50=1.23ms p99=4.56ms max=7.89ms failures=0\n";
        assert_eq!(parse_latency(output), Some([1.23, 4.56, 7.89]));
    }

    #[test]
    fn no_line_gives_none() {
        assert_eq!(parse_latency("PASS  hello"), None);
    }

    #[test]
    fn refuses_without_the_flag() {
        let args = Args {
            allow_smoke: false,
            label: "default".into(),
            runs: 1,
            out: None,
        };
        assert!(run(args).is_err());
    }
}
