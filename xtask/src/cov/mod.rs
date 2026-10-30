//! `cargo xtask cov`（issue #158）。テストの行カバレッジを、決まった条件で測り、crate ごとの表と低いファイルを出す。
//!
//! - 道具は `cargo-llvm-cov`（無ければ入れ方を案内して止まる。自動では入れない）
//! - 条件: `--workspace`、除外は [`IGNORE_FILENAME_REGEX`]（xtask、テスト、ベンチ、build.rs、生成コード）。Swift は対象外
//! - 目標値もゲートも持たない。数字を見てユーザーが決める（`--fail-under` は任意で、既定は無効）
//! - 変換のロジックの crate は [`summary::CONVERSION_CRATES`]（1 か所）。`--group conversion` で絞る
//! - 機能つき（`--lm`: llama.cpp、cmake が要る）と実データ（`--real`: `check --real` と同じテスト。辞書とモデルが要る）は任意

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::XtaskError;
use crate::check::{real_inputs, real_tests};
use crate::data::consts::LibConsts;
use crate::data::run::{Failure, cargo, git, log, repo_root, run as exec, succeeds_quietly, warn};

mod summary;

use summary::{Group, LowCriteria};

/// 測る対象から外すファイル名の正規表現（2026-10-31 に手で測った条件。issue #158）。
const IGNORE_FILENAME_REGEX: &str =
    r"(xtask|/tests/|tests\.rs|benches|build\.rs|generated|\.pb\.rs)";
/// `--lm` で有効にする機能（`check --lm` と同じ組。macOS の Metal）。
const LM_FEATURES: &str = "ikari-lm/metal,ikari-stack/lm-metal,ikari-engine/lm-metal,ikari-engine-ffi/lm-metal,ikari-eval/lm-metal";
/// 低いファイルの既定の目安（行数がこれ以上で、カバレッジがこれ未満）
const DEFAULT_LOW_MIN_LINES: u64 = 60;
const DEFAULT_LOW_BELOW_PERCENT: f64 = 70.0;
/// 記録のファイルの先頭の見出し（別のファイルを上書きしないための印）
const RECORD_TITLE: &str = "# カバレッジの記録";

/// テストの行カバレッジを測り、crate ごとの表と低いファイルを出す（cargo-llvm-cov が要る。`scripts/check.sh` には入れない）
#[derive(Debug, clap::Args)]
pub struct Args {
    /// 表に出す対象（既定は全体。全体のときも変換のロジックの小計の行は出る）
    #[arg(long, value_enum, default_value = "all")]
    group: Group,
    /// 機能つき（llama.cpp。Metal）でも測る。cmake が要る。LM の実際の経路は `--real` と合わせないと通らない
    #[arg(long)]
    lm: bool,
    /// 実データの #[ignore] テスト（`check --real` と同じもの）も流して含める（辞書とモデルを読むだけ。`--lm` を含む）
    #[arg(long)]
    real: bool,
    /// 低いファイルの目安: この行数以上
    #[arg(long, value_name = "行数", default_value_t = DEFAULT_LOW_MIN_LINES)]
    low_min_lines: u64,
    /// 低いファイルの目安: このカバレッジ（%）未満
    #[arg(long, value_name = "%", default_value_t = DEFAULT_LOW_BELOW_PERCENT)]
    low_below: f64,
    /// 結果を Markdown で書く（例: eval/reports/coverage-YYYY-MM-DD.md）。この印のあるファイル以外は上書きしない
    #[arg(long, value_name = "ファイル")]
    out: Option<PathBuf>,
    /// 対象の合計がこの値（%）未満なら失敗にする。既定は無効（目標値は決めていない）
    #[arg(long, value_name = "%")]
    fail_under: Option<f64>,
    /// 測らずに、保存した `cargo llvm-cov report --json --summary-only` の出力から表を作る
    #[arg(long, value_name = "ファイル")]
    from_json: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    run_inner(&args).map_err(|failure| XtaskError(failure.message))
}

fn run_inner(args: &Args) -> Result<(), Failure> {
    let root = repo_root();
    let lm = args.lm || args.real;
    let (json, failed_real) = match &args.from_json {
        Some(path) => (
            std::fs::read_to_string(path)
                .map_err(|e| Failure::new(format!("{}: {e}", path.display())))?,
            Vec::new(),
        ),
        None => measure(&root, lm, args.real)?,
    };
    let files = summary::parse(&json, &root.to_string_lossy()).map_err(Failure::new)?;
    if files.is_empty() {
        return Err(Failure::new("測れたファイルがありません"));
    }
    let criteria = LowCriteria {
        min_lines: args.low_min_lines,
        below_percent: args.low_below,
    };
    let tables = summary::render(&files, args.group, criteria);
    println!("\n{tables}");

    if let Some(out) = &args.out {
        let header = header(&root, args, lm, &failed_real);
        write_record(out, &format!("{header}\n{tables}"))?;
        log(&format!("記録: {}", out.display()));
    }
    if let Some(threshold) = args.fail_under {
        let (lines, covered) = summary::total(&files, args.group);
        let percent = summary::percent(covered, lines);
        if percent < threshold {
            return Err(Failure::new(format!(
                "カバレッジ {percent:.1}% が --fail-under {threshold}% を下回りました"
            )));
        }
    }
    Ok(())
}

/// 測って、JSON（要約だけ）と、失敗した実データのテストの名前を返す。
fn measure(root: &Path, lm: bool, real: bool) -> Result<(String, Vec<String>), Failure> {
    ensure_tools()?;
    // 前提がなければ、長いテストを流す前に止める
    let real_paths = if real {
        let consts = LibConsts::load(root).map_err(|e| Failure::new(e.to_string()))?;
        Some(real_inputs(root, &consts)?)
    } else {
        None
    };

    log("前回の計測の結果を消す（cargo llvm-cov clean --workspace）");
    exec(cargo(root).args(["llvm-cov", "clean", "--workspace"]))?;

    log("cargo llvm-cov（--workspace。xtask は除く）");
    let mut base = cargo(root);
    base.args([
        "llvm-cov",
        "--no-report",
        "--workspace",
        "--exclude",
        "xtask",
    ]);
    if lm {
        base.args(["--features", LM_FEATURES]);
    }
    exec(&mut base)?;

    let mut failed_real = Vec::new();
    if let Some(inputs) = real_paths {
        log("実データのテスト（--ignored。check --real と同じ）");
        for test in real_tests() {
            // `cargo test <args>` を `cargo llvm-cov --no-report <args>` に読み替える
            let mut command = cargo(root);
            command.args(["llvm-cov", "--no-report"]).args(&test[1..]);
            command
                .env("IKARI_DICT_DIR", &inputs.dict)
                .env("IKARI_ZENZ_MODEL", &inputs.zenz)
                .env("IKARI_EDGE_MODEL", &inputs.edge);
            // 測定が目的なので、テストの失敗では止めない（時間の上限を測るテストは、計装と負荷で落ちうる。
            // 失敗したテストの実行分もカバレッジに入る）。合否は `check --real` が見る。失敗は警告と記録に残す
            if exec(&mut command).is_err() {
                let name = test[1..].join(" ");
                warn(&format!(
                    "実データのテストが失敗しました（測定は続けます）: {name}"
                ));
                failed_real.push(name);
            }
        }
    }

    log("集計（cargo llvm-cov report --json --summary-only）");
    let output = cargo(root)
        .args([
            "llvm-cov",
            "report",
            "--json",
            "--summary-only",
            "--ignore-filename-regex",
            IGNORE_FILENAME_REGEX,
        ])
        .output()
        .map_err(|e| Failure::new(format!("cargo llvm-cov report を起動できません: {e}")))?;
    if !output.status.success() {
        return Err(Failure::new(format!(
            "cargo llvm-cov report が失敗しました\n{}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        failed_real,
    ))
}

/// cargo-llvm-cov と llvm-tools-preview があるか。無ければ入れ方を案内して止まる（自動では入れない）。
fn ensure_tools() -> Result<(), Failure> {
    let mut missing = Vec::new();
    if !succeeds_quietly(Command::new("cargo").args(["llvm-cov", "--version"])) {
        missing.push("cargo install cargo-llvm-cov --locked");
    }
    if !llvm_tools_installed() {
        missing.push("rustup component add llvm-tools-preview");
    }
    if missing.is_empty() {
        return Ok(());
    }
    Err(Failure::new(format!(
        "カバレッジの測定に要る道具がありません。次を実行してから、もう一度流してください（自動では入れません）\n  {}",
        missing.join("\n  ")
    )))
}

/// rustup があるときだけ llvm-tools の有無を見る（rustup がなければ、cargo-llvm-cov に任せる）。
fn llvm_tools_installed() -> bool {
    let Ok(output) = Command::new("rustup")
        .args(["component", "list", "--installed"])
        .output()
    else {
        return true;
    };
    !output.status.success() || String::from_utf8_lossy(&output.stdout).contains("llvm-tools")
}

/// 記録の先頭（いつ、どの版で、どの条件で測ったか）。
fn header(root: &Path, args: &Args, lm: bool, failed_real: &[String]) -> String {
    let sha =
        git_output(root, &["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| "不明".into());
    let dirty = git_output(root, &["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    let date = jiff::Zoned::now().strftime("%Y-%m-%d").to_string();
    let source = match &args.from_json {
        Some(path) => format!("保存した JSON（{}）から作成", path.display()),
        None => "`cargo xtask cov` で測定".into(),
    };
    format!(
        "{RECORD_TITLE}\n\n\
         - 日付: {date}\n\
         - コミット: {sha}{}\n\
         - 測り方: {source}。`cargo llvm-cov --workspace`（xtask を除く）、集計の除外 `{IGNORE_FILENAME_REGEX}`、行カバレッジ\n\
         - 機能つき（llama.cpp, Metal）: {}\n\
         - 実データ（check --real のテスト）: {}\n\
         - 対象: {}\n{}",
        if dirty {
            "（作業ツリーに未コミットの変更あり）"
        } else {
            ""
        },
        if lm { "含む" } else { "含まない" },
        if args.real { "含む" } else { "含まない" },
        match args.group {
            Group::All => "全体",
            Group::Conversion => "変換のロジックのみ",
        },
        if failed_real.is_empty() {
            String::new()
        } else {
            format!(
                "- 失敗した実データのテスト（測定は続け、実行した分は数えている）: {}\n",
                failed_real.join("、")
            )
        },
    )
}

fn git_output(root: &Path, args: &[&str]) -> Option<String> {
    let output = git(root).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// 記録を書く。別の内容のファイルは壊さない。
fn write_record(path: &Path, text: &str) -> Result<(), Failure> {
    if let Ok(existing) = std::fs::read_to_string(path)
        && !existing.starts_with(RECORD_TITLE)
    {
        return Err(Failure::new(format!(
            "{} は cov の記録ではないので上書きしません",
            path.display()
        )));
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|e| Failure::new(format!("{}: {e}", dir.display())))?;
    }
    std::fs::write(path, text).map_err(|e| Failure::new(format!("{}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_record_refuses_foreign_files() {
        let dir = tempfile::tempdir().unwrap();
        let foreign = dir.path().join("other.md");
        std::fs::write(&foreign, "# 別の文書\n").unwrap();
        assert!(write_record(&foreign, "x").is_err());
        assert_eq!(std::fs::read_to_string(&foreign).unwrap(), "# 別の文書\n");

        let own = dir.path().join("sub/cov.md");
        write_record(&own, &format!("{RECORD_TITLE}\n1")).unwrap();
        write_record(&own, &format!("{RECORD_TITLE}\n2")).unwrap();
        assert!(std::fs::read_to_string(&own).unwrap().ends_with('2'));
    }

    #[test]
    fn ignore_regex_matches_the_agreed_conditions() {
        // 正規表現エンジンは入れていないので、文字列そのものを固定して条件の変更に気づけるようにする
        assert_eq!(
            IGNORE_FILENAME_REGEX,
            r"(xtask|/tests/|tests\.rs|benches|build\.rs|generated|\.pb\.rs)"
        );
    }

    #[test]
    fn cli_parses_defaults() {
        use clap::Parser;
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: Args,
        }
        let cli = Cli::parse_from(["cov"]);
        assert_eq!(cli.args.low_min_lines, 60);
        assert_eq!(cli.args.low_below, 70.0);
        assert!(cli.args.fail_under.is_none());
        assert!(!cli.args.lm && !cli.args.real);
    }
}
