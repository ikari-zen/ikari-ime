//! `cargo xtask eval rank-within`（ADR 0043 段 X4）。期待が N 位以内かを調べる。`eval/scripts/rank-within.sh` の移し先。
//!
//! ikari-eval の評価セットには N 位以内の書式がないので、ケース別の結果（--out-cases）を読んで判定する
//! （brand-katakana.tsv の回帰用。#223）。tags に `known` がある行は、N 位以内でなくても失敗に数えない（今は届かない既知の行）。
//! known なのに N 位以内に入った行は「印を外す」と出す。tags に `within-N` がある行は、その行だけ N を上書きする
//! （postal.tsv のように行ごとに位が違うセット用。#203）。
//! 終了コード: 0 = 失敗なし、1 = known でない行が N 位以内に入っていない。

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::data::run::{Failure, repo_root};

const DEFAULT_LIMIT: &str = "2";
const KNOWN_TAG: &str = "known";
const WITHIN_TAG_PREFIX: &str = "within-";

/// 期待が N 位以内かを調べる（ikari-eval の追加の引数は N のあとに続けて渡す）
#[derive(Debug, clap::Args)]
pub struct Args {
    /// 評価セット（.tsv）
    #[arg(allow_hyphen_values = true)]
    set_file: PathBuf,
    /// 何位以内か（既定 2）
    #[arg(allow_hyphen_values = true)]
    limit: Option<String>,
    /// ikari-eval に足す引数
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    eval_args: Vec<String>,
}

/// 判定の結果。
#[derive(Debug, Default, PartialEq, Eq)]
struct Verdict {
    /// 標準出力に出す行（印を外す、失敗）
    lines: Vec<String>,
    total: u32,
    pass: u32,
    skipped: u32,
    fail: u32,
}

/// awk の `x + 0`（先頭の数だけ読む。読めなければ 0）。
fn awk_number(text: &str) -> f64 {
    let end = text
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_digit() || matches!(c, '.' | '-' | '+'))
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    (0..=end)
        .rev()
        .find_map(|n| text[..n].parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// tags の `within-<数字>` の数字（最初の一致）。
fn within_limit(tags: &str) -> Option<f64> {
    tags.split(',').find_map(|tag| {
        let digits = tag.strip_prefix(WITHIN_TAG_PREFIX)?;
        (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
            .then(|| awk_number(digits))
    })
}

fn judge(cases_tsv: &str, limit: f64) -> Verdict {
    let mut verdict = Verdict::default();
    let mut lines = cases_tsv.lines();
    let columns: HashMap<&str, usize> = lines
        .next()
        .unwrap_or_default()
        .split('\t')
        .enumerate()
        .map(|(i, name)| (name, i))
        .collect();
    for line in lines {
        let fields: Vec<&str> = line.split('\t').collect();
        let get = |name: &str| {
            columns
                .get(name)
                .and_then(|i| fields.get(*i))
                .copied()
                .unwrap_or_default()
        };
        let tags = get("tags");
        let lim = within_limit(tags).unwrap_or(limit);
        let rank = get("rank");
        let ok = !rank.is_empty() && awk_number(rank) <= lim;
        let known = tags.split(',').any(|tag| tag == KNOWN_TAG);
        let id = fields.first().copied().unwrap_or_default();
        verdict.total += 1;
        if ok {
            verdict.pass += 1;
            if known {
                verdict.lines.push(format!(
                    "印を外す: {id} {} {} (rank {rank})",
                    get("reading"),
                    get("expected")
                ));
            }
        } else if known {
            verdict.skipped += 1;
        } else {
            verdict.fail += 1;
            verdict.lines.push(format!(
                "失敗: {id} {} {} top1={} (rank {rank})",
                get("reading"),
                get("expected"),
                get("top1")
            ));
        }
    }
    verdict
}

pub fn run(args: Args) -> Result<(), Failure> {
    let root = repo_root();
    let limit_text = args.limit.unwrap_or_else(|| DEFAULT_LIMIT.to_owned());
    let out = std::env::temp_dir().join(format!("ikari-rank-within-{}.tsv", std::process::id()));
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command
        .current_dir(&root)
        .args(["run", "--release", "-q", "-p", "ikari-eval", "--"])
        .args(["--ranker", "full", "--repeat", "1"])
        .args(&args.eval_args)
        .arg("--set")
        .arg(&args.set_file)
        .arg("--out-cases")
        .arg(&out)
        .stdout(Stdio::null());
    let status = command.status();
    let result = match status {
        Ok(status) if status.success() => std::fs::read_to_string(&out)
            .map_err(|e| Failure::new(format!("{} を読めません: {e}", out.display()))),
        Ok(status) => Err(Failure {
            code: status.code().unwrap_or(1),
            message: format!("ikari-eval が失敗しました（{status}）"),
        }),
        Err(e) => Err(Failure::new(format!("cargo を起動できません: {e}"))),
    };
    let _ = std::fs::remove_file(&out);
    let verdict = judge(&result?, awk_number(&limit_text));
    for line in &verdict.lines {
        println!("{line}");
    }
    println!(
        "{} 件中 {} 件が {} 位以内（既知の未達 {}、失敗 {}）",
        verdict.total,
        verdict.pass,
        awk_number(&limit_text) as i64,
        verdict.skipped,
        verdict.fail
    );
    if verdict.fail > 0 {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "id\tkind\tcontext\treading\texpected\ttop1\trank\ttags";

    fn tsv(rows: &[&str]) -> String {
        format!("{HEADER}\n{}\n", rows.join("\n"))
    }

    #[test]
    fn within_n_known_and_failures() {
        let text = tsv(&[
            "a\tk\t\tよみ1\t期待1\tx\t1\t",
            "b\tk\t\tよみ2\t期待2\ty\t3\t",
            "c\tk\t\tよみ3\t期待3\tz\t3\twithin-3",
            "d\tk\t\tよみ4\t期待4\tw\t\tknown",
            "e\tk\t\tよみ5\t期待5\tv\t1\tknown",
        ]);
        let v = judge(&text, 2.0);
        assert_eq!(v.total, 5);
        assert_eq!(v.pass, 3);
        assert_eq!(v.skipped, 1);
        assert_eq!(v.fail, 1);
        assert_eq!(
            v.lines,
            [
                "失敗: b よみ2 期待2 top1=y (rank 3)",
                "印を外す: e よみ5 期待5 (rank 1)"
            ]
        );
    }

    #[test]
    fn within_tag_must_be_exact() {
        assert_eq!(within_limit("a,within-4,b"), Some(4.0));
        assert_eq!(within_limit("within-"), None);
        assert_eq!(within_limit("xwithin-3"), None);
        assert_eq!(within_limit("within-3x"), None);
    }

    #[test]
    fn empty_input_has_no_cases() {
        assert_eq!(judge("", 2.0), Verdict::default());
    }
}
