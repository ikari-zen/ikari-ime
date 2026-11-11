//! `cargo xtask eval snapshot`（ADR 0043 段 X4）。リファクタの前後で振る舞いが変わっていないことを確かめるための snapshot
//! （docs/refactor-plan-2026-10.md §6.1、§7.1）。`eval/scripts/snapshot.sh` の移し先。出力ディレクトリの形、画面の出力、終了コードは
//! シェルと同じにしてある。
//!
//! 取る: `snapshot <出力ディレクトリ> [--lm <gguf>] [--ajimee] [--smoke] [--no-ref]`。
//! 取って基準と比べる: `snapshot --compare <基準のディレクトリ> [<出力ディレクトリ>]`
//! （`--lm` / `--ajimee` / `--smoke` は基準の meta.env と同じものが既定）。`<出力ディレクトリ>` が取得済み（COMPLETE がある）なら、
//! 取り直さずに比べるだけ。
//!
//! 内容（出力ディレクトリの中）:
//! - `meta.env`: コミット、日付、構成、使ったバイナリ
//! - `dict.sha256`: 辞書（data/build/dict）の各ファイルの sha256
//! - `dict-build.txt`: 辞書を作り直すコマンドと、入力の sha256
//! - `eval/<名前>.{tsv,md,cmd}`: ケース別の結果（--out-cases）、レポート、実行したコマンド
//! - `scenario/<名前>.md`: 学習のシナリオのレポート
//! - `ref/<名前>.{tsv,md}`: LM あり構成を既定の締め切りで流した参考値（比較しない）
//! - `cli/golden.txt`: eval/scripts/cli-golden.tsv の入力に対する ikari の出力
//! - `smoke.txt`: --smoke のときだけ。macos/scripts/smoke.sh --signing team --installed の出力
//! - `summary.tsv`: 構成ごとの 1 位正解数、warm p99、LM の時間切れ、取り直した回数
//!
//! 一致の判定（§7.1）: --out-cases の列 1〜7（id kind context reading expected top1 rank）がバイト単位で一致、シナリオのレポートと
//! CLI の出力が一致、辞書の sha256 が一致。smoke は PASS/FAIL/SKIP の行の集合（括弧の中の pid や時間は除く）で比べる。
//! warm p99 は基準の 1.1 倍を超えたら警告だけ出す（ミリ秒未満の差は揺れるので、絶対差 0.05ms 以下は無視する）。
//! LM ありの構成は 2000ms の締め切りで流し、時間切れが 0 になるまで最大 3 回流し直す。0 にならなかった構成は INVALID にして、
//! 比較の失敗として数える。
//!
//! 環境変数: `IKARI_SNAPSHOT_FLAGS`（ikari-eval と ikari に足す引数。基準を語彙集なしで取るときは --no-packs。ADR 0012）、
//! `IKARI_DICT_DIR`（既定 data/build/dict）、`IKARI_EVAL_BIN` / `IKARI_BIN`（バイナリを指定。指定すると build しない）、
//! `IKARI_SNAPSHOT_TARGET_DIR`（LM つきのバイナリの置き場所。既定 target/snapshot-bin）。
//! 終了コード: 0 = 取得できた（比較なら差分 0 かつ有効）、1 = 差分または INVALID がある、2 = 実行できなかった。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::Instant;

use super::textdiff::{changed_lines, normal_diff};
use crate::XtaskError;
use crate::data::consts::LibConsts;
use crate::data::run::{Failure, capture, git, repo_root};
use crate::data::sha256::sha256_file;

const LM_BUDGET_MS: u32 = 2000;
const LM_DEADLINE_MS: u32 = 2000;
const LM_MAX_ATTEMPTS: u32 = 3;
/// LM ありは 1 回が重いので、warm の繰り返しを減らす（p99 は参考値。順位には影響しない）
const LM_REPEAT: u32 = 3;
const P99_RATIO_LIMIT: f64 = 1.1;
const P99_ABS_FLOOR_MS: f64 = 0.05;
const DIFF_SHOW_LINES: usize = 40;
const CLI_NUM: u32 = 12;
const GOLDEN_TSV: &str = "eval/scripts/cli-golden.tsv";
const AJIMEE_JSON: &str = "data/raw/ajimee/evaluation_items.json";
const SCENARIOS: [&str; 7] = [
    "a-02",
    "a-03",
    "a-04",
    "b23-naoru",
    "b23-susumeru",
    "b23-same-context",
    "it-pack",
];
const CHIVE_TXT: &str = "data/raw/chive/chive-1.3-mc90/chive-1.3-mc90.txt";
const SUDACHI_DIR: &str = "data/raw/sudachi-20260723";
const CHIVE_POLICY: &str = "dict-builder/policy/chive.tsv";
/// `cut -f` の指定（id kind context reading expected top1 rank）
const CASE_COLUMNS: usize = 7;
const CASE_COLUMNS_LABEL: &str = "1-7";
const DEFAULT_DICT_DIR: &str = "data/build/dict";
const DEFAULT_SNAPSHOT_DIR: &str = "target/refactor-snapshot";
const DEFAULT_LM_TARGET_DIR: &str = "target/snapshot-bin";
const DATE_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%z";
const DIR_STAMP_FORMAT: &str = "%Y%m%d-%H%M%S";
const SMOKE_SCRIPT: &str = "macos/scripts/smoke.sh";
const SET_HOMOPHONES: &str = "eval/sets/homophones.tsv";
const SET_EXT: &str = "eval/sets/homophones-ext.tsv";
const SET_HIRAGANA: &str = "eval/sets/hiragana.tsv";
const SET_KATAKANA: &str = "eval/sets/katakana-suffix.tsv";
const SET_SEGMENTS: &str = "eval/sets/segments.tsv";
const SET_VERBS: &str = "eval/sets/verbs.tsv";
/// 辞書の次の評価セット（名前、ファイル）。`ranker` は full。dev / test をまとめて流す
const FULL_SETS: [(&str, &str); 12] = [
    ("context-compound-full", "eval/sets/context-compound.tsv"),
    ("adverbs-full", "eval/sets/adverbs.tsv"),
    ("verbs-full", SET_VERBS),
    ("compound-affix-full", "eval/sets/compound-affix.tsv"),
    ("new-words-full", "eval/sets/new-words.tsv"),
    ("rare-homophones-full", "eval/sets/rare-homophones.tsv"),
    ("formal-noun-split-full", "eval/sets/formal-noun-split.tsv"),
    ("counters-full", "eval/sets/counters.tsv"),
    ("toshiyou-full", "eval/sets/toshiyou.tsv"),
    ("sahen-shiyou-full", "eval/sets/sahen-shiyou.tsv"),
    ("path-loses-full", "eval/sets/path-loses.tsv"),
    ("issue185-full", "eval/sets/issue185.tsv"),
];

/// 左文脈の持ち越し（ADR 0046）の派生セット（C1〜C5 と手書きの carry-chat）。`ranker` は full。
/// `--carry-fill 0`（既定）と、足す設定（`CARRY_ON_ARGS`）の両方で流す
const CARRY_SETS: [(&str, &str); 6] = [
    ("carry-empty", "eval/sets/carry-empty.tsv"),
    ("carry-overlap", "eval/sets/carry-overlap.tsv"),
    ("carry-unrelated", "eval/sets/carry-unrelated.tsv"),
    ("carry-mismatch", "eval/sets/carry-mismatch.tsv"),
    (
        "carry-empty-unrelated",
        "eval/sets/carry-empty-unrelated.tsv",
    ),
    ("carry-chat", "eval/sets/carry-chat.tsv"),
];
/// 足す設定の評価に付ける引数（スナップショットの安定のための固定値。既定の値の提案ではない）
const CARRY_ON_ARGS: [&str; 4] = ["--carry-fill", "40", "--carry-joint", "period"];
/// 足す設定で流すシナリオ（`--carry-fill 40` が要る。ADR 0046 §7.1）
const CARRY_SCENARIO: &str = "c182-carryover";

/// リファクタの前後で振る舞いが変わっていないことを確かめるための snapshot を取る、または基準と比べる
#[derive(Debug, clap::Args)]
pub struct Args {
    /// 出力ディレクトリ（--compare のときの既定: target/refactor-snapshot/<コミット>-<日時>）
    out: Option<PathBuf>,
    /// LM のモデル（gguf）。LM ありの構成も取る
    #[arg(long)]
    lm: Option<String>,
    /// AJIMEE の評価も取る（eval/scripts/fetch_ajimee.sh で取得しておく）
    #[arg(long)]
    ajimee: bool,
    /// インストール済みのアプリで smoke も流す（学習ファイルのロックを取るので、使っている ikari が一時的に変換できなくなる）
    #[arg(long)]
    smoke: bool,
    /// LM ありの参考値（既定の締め切り）を取らない
    #[arg(long)]
    no_ref: bool,
    /// 取って、この基準のディレクトリと比べる
    #[arg(long, value_name = "基準のディレクトリ")]
    compare: Option<PathBuf>,
}

/// 実行できなかった（終了コード 2）。メッセージは `snapshot: ` を付けて標準エラーに出す。
fn die(message: impl Into<String>) -> Failure {
    Failure {
        code: 2,
        message: message.into(),
    }
}

fn log(message: &str) {
    eprintln!("snapshot: {message}");
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    // シェルは root に cd してから動く（相対パスの引数も root からになる）
    let root = repo_root();
    let result = std::env::set_current_dir(&root)
        .map_err(|e| die(format!("{} に移れません: {e}", root.display())))
        .and_then(|()| Snapshot::new(&root, args)?.run());
    match result {
        Ok(true) => Ok(()),
        Ok(false) => std::process::exit(1),
        Err(failure) => {
            eprintln!("snapshot: {}", failure.message);
            std::process::exit(failure.code)
        }
    }
}

struct Snapshot {
    root: PathBuf,
    out: PathBuf,
    compare_base: Option<PathBuf>,
    dict_dir: PathBuf,
    extra_flags: Vec<String>,
    eval: PathBuf,
    cli: PathBuf,
    lm: String,
    ajimee: bool,
    smoke: bool,
    reference: bool,
    fail_count: u32,
}

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// meta.env の `NAME=値` の値（最初の一致）。
fn meta_value(meta: &str, name: &str) -> String {
    let prefix = format!("{name}=");
    meta.lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .unwrap_or_default()
        .to_owned()
}

/// golden の TSV から (context, reading) を取り出す。コメント、タブのない行、読みが空の行は飛ばす。
/// context が空の行がある（タブで分ける）。
fn golden_cases(text: &str) -> Vec<(&str, &str)> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('\t'))
        .filter(|(_, reading)| !reading.is_empty())
        .collect()
}

fn read_text(path: &Path) -> String {
    // BOM つきでも読む（ADR 0040 §2.9）
    ikari_textfile::read_to_string(path).unwrap_or_default()
}

fn head_commit(root: &Path) -> Result<String, Failure> {
    capture(git(root).args(["rev-parse", "--short=7", "HEAD"]))
        .map(|s| s.trim().to_owned())
        .ok_or_else(|| die("git rev-parse が失敗しました"))
}

fn now(format: &str) -> String {
    jiff::Zoned::now().strftime(format).to_string()
}

impl Snapshot {
    fn new(root: &Path, args: Args) -> Result<Self, Failure> {
        let mut lm = args.lm.clone().unwrap_or_default();
        let mut ajimee = args.ajimee;
        let mut smoke = args.smoke;
        let out = if let Some(base) = &args.compare {
            if !base.join("COMPLETE").is_file() {
                return Err(die(format!(
                    "基準が取得済みではありません（COMPLETE がありません）: {}",
                    base.display()
                )));
            }
            // 基準と同じ構成で取る（明示した指定を優先する）
            let meta = read_text(&base.join("meta.env"));
            if args.lm.is_none() {
                lm = meta_value(&meta, "LM");
            }
            if !args.ajimee {
                ajimee = meta_value(&meta, "AJIMEE") == "1";
            }
            if !args.smoke {
                smoke = meta_value(&meta, "SMOKE") == "1";
            }
            args.out.clone().unwrap_or_else(|| {
                let commit = head_commit(root).unwrap_or_default();
                PathBuf::from(format!(
                    "{DEFAULT_SNAPSHOT_DIR}/{commit}-{}",
                    now(DIR_STAMP_FORMAT)
                ))
            })
        } else {
            args.out.clone().ok_or_else(|| {
                die("使い方: snapshot.sh <出力ディレクトリ> [--lm <gguf>] [--ajimee] [--smoke] | snapshot.sh --compare <基準> [<出力>]")
            })?
        };
        let dict_dir = std::env::var_os("IKARI_DICT_DIR")
            .filter(|v| !v.is_empty())
            .map_or_else(|| PathBuf::from(DEFAULT_DICT_DIR), PathBuf::from);
        let extra_flags = std::env::var("IKARI_SNAPSHOT_FLAGS")
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        Ok(Self {
            root: root.to_path_buf(),
            out,
            compare_base: args.compare,
            dict_dir,
            extra_flags,
            eval: PathBuf::new(),
            cli: PathBuf::new(),
            lm,
            ajimee,
            smoke,
            reference: !args.no_ref,
            fail_count: 0,
        })
    }

    /// 比較なら、差分 0 かつ有効のとき true。
    fn run(&mut self) -> Result<bool, Failure> {
        if self.out.join("COMPLETE").is_file() && self.compare_base.is_some() {
            log(&format!("取得済みの {} を使う", self.out.display()));
            let env_eval = std::env::var("IKARI_EVAL_BIN").unwrap_or_default();
            self.eval = if env_eval.is_empty() {
                PathBuf::from(meta_value(
                    &read_text(&self.out.join("meta.env")),
                    "EVAL_BIN",
                ))
            } else {
                PathBuf::from(env_eval)
            };
        } else {
            self.take_snapshot()?;
        }
        match self.compare_base.clone() {
            Some(base) => {
                self.compare_snapshots(&base);
                Ok(self.fail_count == 0)
            }
            None => Ok(true),
        }
    }

    // ---- 取得 -----------------------------------------------------------------

    fn prepare_binaries(&mut self) -> Result<(), Failure> {
        let env_eval = std::env::var("IKARI_EVAL_BIN").unwrap_or_default();
        let env_cli = std::env::var("IKARI_BIN").unwrap_or_default();
        if !env_eval.is_empty() && !env_cli.is_empty() {
            self.eval = PathBuf::from(env_eval);
            self.cli = PathBuf::from(env_cli);
            return Ok(());
        }
        let mut build = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        build.current_dir(&self.root).args(["build", "--release"]);
        let bin_dir = if self.lm.is_empty() {
            log("build (release)");
            build.args(["-p", "ikari-eval", "-p", "ikari-cli"]);
            self.root.join("target")
        } else {
            let tdir = std::env::var_os("IKARI_SNAPSHOT_TARGET_DIR")
                .filter(|v| !v.is_empty())
                .map_or_else(|| self.root.join(DEFAULT_LM_TARGET_DIR), PathBuf::from);
            log(&format!("build (release, lm-metal) -> {}", tdir.display()));
            build.env("CARGO_TARGET_DIR", &tdir).args([
                "-p",
                "ikari-eval",
                "-p",
                "ikari-cli",
                "--features",
                "lm-metal",
            ]);
            tdir
        };
        let status = build
            .status()
            .map_err(|e| die(format!("cargo を起動できません: {e}")))?;
        if !status.success() {
            return Err(Failure {
                code: status.code().unwrap_or(1),
                message: "cargo build が失敗しました".to_owned(),
            });
        }
        self.eval = bin_dir.join("release").join(exe("ikari-eval"));
        self.cli = bin_dir.join("release").join(exe("ikari"));
        Ok(())
    }

    fn eval_dir(&self) -> PathBuf {
        self.out.join("eval")
    }

    fn add_summary(&self, fields: [&str; 8]) -> Result<(), Failure> {
        append(
            &self.out.join("summary.tsv"),
            &format!("{}\n", fields.join("\t")),
        )
    }

    /// ikari-eval の共通の前半（`--dict d --ranker r` と追加の引数）。
    fn base_args(&self, ranker: &str) -> Vec<String> {
        let mut args = vec![
            "--dict".to_owned(),
            self.dict_dir.display().to_string(),
            "--ranker".to_owned(),
            ranker.to_owned(),
        ];
        args.extend(self.extra_flags.iter().cloned());
        args
    }

    fn eval_command(&self, args: &[String]) -> Command {
        let mut command = Command::new(&self.eval);
        command.args(args);
        command
    }

    /// 標準出力を md に、標準エラーを err（None なら捨てる）に書いて実行する。
    fn run_redirected(
        &self,
        args: &[String],
        stdout: &Path,
        stderr: Option<&Path>,
    ) -> Result<ExitStatus, Failure> {
        let mut command = self.eval_command(args);
        command.stdout(create(stdout)?);
        command.stderr(match stderr {
            Some(path) => Stdio::from(create(path)?),
            None => Stdio::null(),
        });
        command
            .status()
            .map_err(|e| die(format!("{} を起動できません: {e}", self.eval.display())))
    }

    /// `run_eval <名前> <ranker> <lm> <ikari-eval の追加引数...>`
    fn run_eval(
        &self,
        name: &str,
        ranker: &str,
        use_lm: bool,
        extra: &[&str],
    ) -> Result<(), Failure> {
        let md = self.eval_dir().join(format!("{name}.md"));
        let tsv = self.eval_dir().join(format!("{name}.tsv"));
        let err = self.eval_dir().join(format!("{name}.err"));
        let mut base = self.base_args(ranker);
        if use_lm {
            base.extend(
                [
                    "--lm",
                    &self.lm,
                    "--lm-budget-ms",
                    &LM_BUDGET_MS.to_string(),
                    "--lm-deadline-ms",
                    &LM_DEADLINE_MS.to_string(),
                    "--repeat",
                    &LM_REPEAT.to_string(),
                ]
                .map(str::to_owned),
            );
        }
        base.extend(extra.iter().map(|s| (*s).to_owned()));
        base.extend(["--out-cases".to_owned(), tsv.display().to_string()]);

        let mut attempts = 0u32;
        let mut valid = "ok".to_owned();
        let mut timeouts = String::new();
        let mut calls = String::new();
        loop {
            attempts += 1;
            let mut cmd_line = String::new();
            for word in std::iter::once(self.eval.display().to_string()).chain(base.iter().cloned())
            {
                cmd_line.push_str(&shell_quote(&word));
                cmd_line.push(' ');
            }
            cmd_line.push('\n');
            write(&self.eval_dir().join(format!("{name}.cmd")), &cmd_line)?;
            let status = self.run_redirected(&base, &md, Some(&err))?;
            if !status.success() {
                log(&format!(
                    "warn: {name} の終了コードが 0 ではありません（基準を割る構成は正常。.err を見る）"
                ));
            }
            if !use_lm {
                break;
            }
            let text = read_text(&md);
            let counts = lm_counts(&text);
            calls = counts.as_ref().map(|c| c.0.clone()).unwrap_or_default();
            timeouts = counts.map(|c| c.1).unwrap_or_default();
            if timeouts.is_empty() {
                valid = "NO-LM-LINE".to_owned();
                break;
            }
            if timeouts.parse::<u64>().unwrap_or(0) == 0 {
                break;
            }
            if attempts >= LM_MAX_ATTEMPTS {
                valid = format!("INVALID(timeouts={timeouts})");
                break;
            }
            log(&format!(
                "{name}: LM の時間切れ {timeouts} 件。流し直す（{attempts}/{LM_MAX_ATTEMPTS}）"
            ));
        }
        if std::fs::metadata(&err).is_ok_and(|m| m.len() == 0) {
            let _ = std::fs::remove_file(&err);
        }
        let text = read_text(&md);
        let (cases, top1, p99) = (
            report_cases(&text),
            report_top1(&text),
            report_warm_p99(&text),
        );
        let dash = |s: &str| {
            if s.is_empty() {
                "-".to_owned()
            } else {
                s.to_owned()
            }
        };
        self.add_summary([
            name,
            &cases,
            &top1,
            &p99,
            &dash(&calls),
            &dash(&timeouts),
            &attempts.to_string(),
            &valid,
        ])?;
        log(&format!(
            "{name}: {top1} / {cases} warm p99={p99}ms lm_timeouts={} attempts={attempts} {valid}",
            dash(&timeouts)
        ));
        Ok(())
    }

    /// 既定の締め切りでの参考値（比較しない）
    fn run_ref(&self, name: &str, ranker: &str, extra: &[&str]) -> Result<(), Failure> {
        let md = self.out.join("ref").join(format!("{name}.md"));
        let mut args = self.base_args(ranker);
        args.extend(["--lm", &self.lm, "--repeat", &LM_REPEAT.to_string()].map(str::to_owned));
        args.extend(extra.iter().map(|s| (*s).to_owned()));
        args.extend([
            "--out-cases".to_owned(),
            self.out
                .join("ref")
                .join(format!("{name}.tsv"))
                .display()
                .to_string(),
        ]);
        let _ = self.run_redirected(&args, &md, None)?;
        let text = read_text(&md);
        log(&format!(
            "ref {name}: {} lm_timeouts={}",
            report_top1(&text),
            lm_counts(&text).map(|c| c.1).unwrap_or_default()
        ));
        Ok(())
    }

    fn run_scenario(
        &self,
        name: &str,
        file: &str,
        use_lm: bool,
        extra: &[&str],
    ) -> Result<(), Failure> {
        let md = self.out.join("scenario").join(format!("{name}.md"));
        let mut args = self.base_args("full");
        args.extend(["--scenario".to_owned(), file.to_owned()]);
        args.extend(extra.iter().map(|s| (*s).to_owned()));
        if use_lm {
            args.extend(
                [
                    "--lm",
                    &self.lm,
                    "--lm-budget-ms",
                    &LM_BUDGET_MS.to_string(),
                    "--lm-deadline-ms",
                    &LM_DEADLINE_MS.to_string(),
                ]
                .map(str::to_owned),
            );
        }
        let _ = self.run_redirected(&args, &md, None)?;
        let failures: Vec<String> = read_text(&md)
            .lines()
            .filter_map(scenario_failures)
            .map(|n| format!("失敗 {n}"))
            .collect();
        log(&format!("scenario {name}: {}", failures.join("\n")));
        Ok(())
    }

    fn record_dict(&self, consts: &LibConsts) -> Result<(), Failure> {
        let mut sums = String::new();
        for file in consts.array("DICT_FILES").map_err(|e| die(e.to_string()))? {
            let path = self.dict_dir.join(file);
            if !path.is_file() {
                return Err(die(format!("辞書がありません: {}", path.display())));
            }
            let sha = sha256_file(&path).map_err(|e| die(format!("{}: {e}", path.display())))?;
            sums.push_str(&format!("{sha}  {file}\n"));
        }
        write(&self.out.join("dict.sha256"), &sums)?;

        let tree = |dir: &str| {
            capture(git(&self.root).args(["rev-parse", &format!("HEAD:{dir}")]))
                .map(|s| s.trim().to_owned())
                .unwrap_or_default()
        };
        let head = capture(git(&self.root).args(["rev-parse", "HEAD"]))
            .map(|s| s.trim().to_owned())
            .unwrap_or_default();
        let mut text = String::new();
        text.push_str("# 辞書を作り直して、dict.sha256 と比べる（ビルドは決定的。docs/refactor-plan-2026-10.md §7.1）\n");
        text.push_str(&format!(
            "# builder: HEAD:dict-builder={} HEAD:dict-format={}\n",
            tree("dict-builder"),
            tree("dict-format")
        ));
        text.push_str(&format!("# commit: {head}\n\n"));
        text.push_str(
            "## コマンド（リポジトリのルートで。出力先は data/build/dict.new などに変える）\n",
        );
        text.push_str(&format!("cargo run --release -p ikari-dict-builder -- --policy {CHIVE_POLICY} --chive-vocab {CHIVE_TXT} --lex {SUDACHI_DIR}/small_lex.csv --lex {SUDACHI_DIR}/core_lex.csv --notcore-lex {SUDACHI_DIR}/notcore_lex.csv --matrix {SUDACHI_DIR}/matrix.def --label sudachi-full-<版> --out <OUT>\n"));
        text.push_str(&format!("cargo run --release -p ikari-dict-builder -- vectors --chive {CHIVE_TXT} --dict <OUT> --out <OUT>/vectors.bin --lex {SUDACHI_DIR}/small_lex.csv --lex {SUDACHI_DIR}/core_lex.csv --lex {SUDACHI_DIR}/notcore_lex.csv\n"));
        text.push_str("cargo run --release -p ikari-dict-builder -- --verify <OUT>\n\n");
        text.push_str("## 入力（sha256  サイズ  パス）\n");
        let inputs = [
            CHIVE_POLICY.to_owned(),
            CHIVE_TXT.to_owned(),
            format!("{SUDACHI_DIR}/small_lex.csv"),
            format!("{SUDACHI_DIR}/core_lex.csv"),
            format!("{SUDACHI_DIR}/notcore_lex.csv"),
            format!("{SUDACHI_DIR}/matrix.def"),
        ];
        for input in inputs {
            let path = Path::new(&input);
            if path.is_file() {
                let sha = sha256_file(path).map_err(|e| die(format!("{input}: {e}")))?;
                let size = std::fs::metadata(path).map_or(0, |m| m.len());
                text.push_str(&format!("{sha}  {size}  {input}\n"));
            } else {
                text.push_str(&format!("MISSING  -  {input}\n"));
            }
        }
        write(&self.out.join("dict-build.txt"), &text)
    }

    fn run_cli_golden(&self) -> Result<(), Failure> {
        let golden = self.root.join(GOLDEN_TSV);
        if !golden.is_file() {
            return Err(die(format!("golden がありません: {}", golden.display())));
        }
        let text = read_text(&golden);
        let tmp = std::env::temp_dir().join(format!("ikari-snapshot-{}", std::process::id()));
        let golden_out = self.out.join("cli").join("golden.txt");
        write(&golden_out, "")?;
        let user_dir = tmp.join("user");
        for (context, reading) in golden_cases(&text) {
            let _ = std::fs::remove_dir_all(&user_dir);
            std::fs::create_dir_all(&user_dir)
                .map_err(|e| die(format!("{} を作れません: {e}", user_dir.display())))?;
            append(
                &golden_out,
                &format!("### context={context} reading={reading}\n"),
            )?;
            // 2>&1 と同じ（標準出力と標準エラーを同じファイルに追記する）
            let file = OpenOptions::new()
                .append(true)
                .open(&golden_out)
                .map_err(|e| die(format!("{}: {e}", golden_out.display())))?;
            let file2 = file
                .try_clone()
                .map_err(|e| die(format!("{}: {e}", golden_out.display())))?;
            let mut command = Command::new(&self.cli);
            command
                .arg("--dict")
                .arg(&self.dict_dir)
                .arg("--user-dir")
                .arg(&user_dir)
                .args(&self.extra_flags)
                .args([
                    "--format",
                    "tsv",
                    "-n",
                    &CLI_NUM.to_string(),
                    "--context",
                    context,
                    reading,
                ])
                .stdout(Stdio::from(file))
                .stderr(Stdio::from(file2));
            let status = command
                .status()
                .map_err(|e| die(format!("{} を起動できません: {e}", self.cli.display())))?;
            if !status.success() {
                append(&golden_out, &format!("(exit {})\n", exit_code(status)))?;
            }
        }
        let _ = std::fs::remove_dir_all(&tmp);
        let count = read_text(&golden_out)
            .lines()
            .filter(|l| l.starts_with("### "))
            .count();
        log(&format!("cli golden: {count} 入力"));
        Ok(())
    }

    fn take_snapshot(&mut self) -> Result<(), Failure> {
        if self.out.join("COMPLETE").exists() {
            return Err(die(format!(
                "取得済みです（上書きしません）: {}",
                self.out.display()
            )));
        }
        if !self.dict_dir.join("system.dict").is_file() {
            return Err(die(format!(
                "辞書がありません: {}",
                self.dict_dir.display()
            )));
        }
        if !self.lm.is_empty() && !Path::new(&self.lm).is_file() {
            return Err(die(format!("LM のモデルがありません: {}", self.lm)));
        }
        if self.ajimee && !Path::new(AJIMEE_JSON).is_file() {
            return Err(die(format!(
                "AJIMEE がありません: {AJIMEE_JSON}（eval/scripts/fetch_ajimee.sh）"
            )));
        }
        let consts = LibConsts::load(&self.root).map_err(|e| die(e.to_string()))?;
        for sub in ["eval", "scenario", "cli", "ref"] {
            let dir = self.out.join(sub);
            std::fs::create_dir_all(&dir)
                .map_err(|e| die(format!("{} を作れません: {e}", dir.display())))?;
        }
        write(&self.out.join("summary.tsv"), "")?;
        let started = Instant::now();
        self.prepare_binaries()?;

        let head = capture(git(&self.root).args(["rev-parse", "HEAD"]))
            .map(|s| s.trim().to_owned())
            .ok_or_else(|| die("git rev-parse が失敗しました"))?;
        let dirty = capture(git(&self.root).args(["status", "--porcelain"]))
            .map_or(0, |s| s.lines().count());
        let mut meta = format!(
            "COMMIT={head}\nDIRTY_FILES={dirty}\nDATE={}\nLM={}\nAJIMEE={}\nSMOKE={}\nDICT_DIR={}\nEVAL_BIN={}\nCLI_BIN={}\nLM_BUDGET_MS={LM_BUDGET_MS}\nLM_DEADLINE_MS={LM_DEADLINE_MS}\nLM_REPEAT={LM_REPEAT}\n",
            now(DATE_FORMAT),
            self.lm,
            u8::from(self.ajimee),
            u8::from(self.smoke),
            self.dict_dir.display(),
            self.eval.display(),
            self.cli.display(),
        );
        if !self.lm.is_empty() {
            let sha =
                sha256_file(Path::new(&self.lm)).map_err(|e| die(format!("{}: {e}", self.lm)))?;
            meta.push_str(&format!("LM_SHA256={sha}\n"));
        }
        write(&self.out.join("meta.env"), &meta)?;

        self.record_dict(&consts)?;

        let has_lm = !self.lm.is_empty();
        // 1. homophones
        self.run_eval("homophones-cost", "cost", false, &["--set", SET_HOMOPHONES])?;
        self.run_eval(
            "homophones-topic",
            "topic",
            false,
            &["--set", SET_HOMOPHONES],
        )?;
        self.run_eval("homophones-full", "full", false, &["--set", SET_HOMOPHONES])?;
        // 2. ext（dev / test）
        for split in ["dev", "test"] {
            self.run_eval(
                &format!("ext-{split}-topic"),
                "topic",
                false,
                &["--set", SET_EXT, "--split", split],
            )?;
            self.run_eval(
                &format!("ext-{split}-full"),
                "full",
                false,
                &["--set", SET_EXT, "--split", split],
            )?;
        }
        // 3. hiragana
        self.run_eval("hiragana-topic", "topic", false, &["--set", SET_HIRAGANA])?;
        // 3b. katakana-suffix（接尾の漢字とカタカナ。dev / test をまとめて）
        self.run_eval("katakana-topic", "topic", false, &["--set", SET_KATAKANA])?;
        self.run_eval("katakana-full", "full", false, &["--set", SET_KATAKANA])?;
        // 3d〜3m. 辞書の追加や補正の副作用を見るセット（dev / test をまとめて。ADR は各セットの README と eval/sets/REVIEW.md）
        for (name, set) in FULL_SETS {
            self.run_eval(name, "full", false, &["--set", set])?;
        }
        // 3n. 左文脈の持ち越し（ADR 0046）。足さない（既定）と、足す設定の両方
        for (name, set) in CARRY_SETS {
            self.run_eval(name, "full", false, &["--set", set])?;
            let mut on = vec!["--set", set];
            on.extend(CARRY_ON_ARGS);
            self.run_eval(&format!("{name}-fill"), "full", false, &on)?;
        }
        // 3c. segments（文節の区切り。ケース別の --out-cases がないので、レポートを比べる）
        let mut seg = vec!["segments".to_owned()];
        seg.extend(self.base_args("full"));
        seg.extend([
            "--set".to_owned(),
            SET_SEGMENTS.to_owned(),
            "--split".to_owned(),
            "all".to_owned(),
        ]);
        let seg_md = self.eval_dir().join("segments.md");
        let status = self.run_redirected(&seg, &seg_md, None)?;
        if !status.success() {
            log("warn: segments の終了コードが 0 ではありません");
        }
        log(&format!(
            "segments: {}",
            segments_summary(&read_text(&seg_md))
        ));
        // 4. AJIMEE
        if self.ajimee {
            self.run_eval(
                "ajimee-topic",
                "topic",
                false,
                &["--set", AJIMEE_JSON, "--set-format", "ajimee"],
            )?;
        }
        // 5. LM あり
        if has_lm {
            self.run_eval(
                "homophones-full-lm",
                "full",
                true,
                &["--set", SET_HOMOPHONES],
            )?;
            for split in ["dev", "test"] {
                self.run_eval(
                    &format!("ext-{split}-topic-lm"),
                    "topic",
                    true,
                    &["--set", SET_EXT, "--split", split],
                )?;
            }
            self.run_eval("hiragana-topic-lm", "topic", true, &["--set", SET_HIRAGANA])?;
            self.run_eval("verbs-full-lm", "full", true, &["--set", SET_VERBS])?;
            if self.ajimee {
                self.run_eval(
                    "ajimee-topic-lm",
                    "topic",
                    true,
                    &["--set", AJIMEE_JSON, "--set-format", "ajimee"],
                )?;
            }
        }
        // 6. シナリオ
        for scenario in SCENARIOS {
            let file = format!("eval/scenarios/{scenario}.tsv");
            self.run_scenario(scenario, &file, false, &[])?;
            if has_lm {
                self.run_scenario(&format!("{scenario}-lm"), &file, true, &[])?;
            }
        }
        let carry_file = format!("eval/scenarios/{CARRY_SCENARIO}.tsv");
        self.run_scenario(CARRY_SCENARIO, &carry_file, false, &CARRY_ON_ARGS)?;
        if has_lm {
            self.run_scenario(
                &format!("{CARRY_SCENARIO}-lm"),
                &carry_file,
                true,
                &CARRY_ON_ARGS,
            )?;
        }
        // 7. CLI
        self.run_cli_golden()?;
        // 8. 既定の締め切りでの参考値
        if has_lm && self.reference {
            self.run_ref("homophones-full-lm", "full", &["--set", SET_HOMOPHONES])?;
            for split in ["dev", "test"] {
                self.run_ref(
                    &format!("ext-{split}-topic-lm"),
                    "topic",
                    &["--set", SET_EXT, "--split", split],
                )?;
            }
            self.run_ref("hiragana-topic-lm", "topic", &["--set", SET_HIRAGANA])?;
            if self.ajimee {
                self.run_ref(
                    "ajimee-topic-lm",
                    "topic",
                    &["--set", AJIMEE_JSON, "--set-format", "ajimee"],
                )?;
            }
        }
        // 9. smoke
        if self.smoke {
            log("smoke (インストール済みのアプリを使う。再インストールはしない)");
            let smoke_out = self.out.join("smoke.txt");
            let file = create(&smoke_out)?;
            let file2 = file.try_clone().map_err(|e| die(e.to_string()))?;
            let status = Command::new("bash")
                .current_dir(&self.root)
                .args([SMOKE_SCRIPT, "--signing", "team", "--installed"])
                .stdout(Stdio::from(file))
                .stderr(Stdio::from(file2))
                .status();
            let code = status.map_or(127, exit_code);
            append(&smoke_out, &format!("(exit {code})\n"))?;
            log(&format!("smoke: exit {code}"));
        }
        let _ = std::fs::remove_dir(self.out.join("ref"));

        let elapsed = started.elapsed().as_secs();
        append(
            &self.out.join("meta.env"),
            &format!("ELAPSED_SECONDS={elapsed}\n"),
        )?;
        write(
            &self.out.join("COMPLETE"),
            &format!("{}\n", now(DATE_FORMAT)),
        )?;
        log(&format!("完了: {}（{elapsed} 秒）", self.out.display()));
        Ok(())
    }

    // ---- 比較 -----------------------------------------------------------------

    fn note_fail(&mut self) {
        self.fail_count += 1;
    }

    /// 2 つのファイルを比べて、先頭だけ見せる。差分があれば false
    fn show_diff(&self, label: &str, a: &Path, b: &Path) -> bool {
        let (ta, tb) = (read_text(a), read_text(b));
        if std::fs::read(a)
            .ok()
            .is_some_and(|x| Some(x) == std::fs::read(b).ok())
        {
            println!("  {label:<28} 一致");
            return true;
        }
        let diff = normal_diff(&ta, &tb);
        println!("  {label:<28} 差分あり（{} 行）", changed_lines(&diff));
        print_head(&diff);
        false
    }

    fn compare_cases(&mut self, name: &str, base: &Path, other: &Path) {
        let a = base.join("eval").join(format!("{name}.tsv"));
        let b = other.join("eval").join(format!("{name}.tsv"));
        if !a.is_file() || !b.is_file() {
            println!("  {name:<28} ファイルがない");
            self.note_fail();
            return;
        }
        let (ca, cb) = (cut_columns(&read_text(&a)), cut_columns(&read_text(&b)));
        if ca == cb {
            println!("  {name:<28} 一致（{} 行）", ca.lines().count());
        } else {
            println!("  {name:<28} 差分あり");
            // `ikari-eval compare` の出力（標準エラーも）の先頭と、列 1〜7 の差分の先頭
            if let Ok(output) = self
                .eval_command(&[
                    "compare".to_owned(),
                    a.display().to_string(),
                    b.display().to_string(),
                ])
                .output()
            {
                let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&output.stderr));
                print_head(&text);
            }
            print_head(&normal_diff(&ca, &cb));
            self.note_fail();
        }
    }

    fn compare_snapshots(&mut self, base: &Path) {
        let other = self.out.clone();
        let commit7 = |dir: &Path| {
            meta_value(&read_text(&dir.join("meta.env")), "COMMIT")
                .chars()
                .take(7)
                .collect::<String>()
        };
        println!("== 比較: 基準 {} ({})", base.display(), commit7(base));
        println!(
            "        今回 {} ({}, 未コミットの変更 {} 件)",
            other.display(),
            commit7(&other),
            meta_value(&read_text(&other.join("meta.env")), "DIRTY_FILES")
        );

        println!("-- 辞書の sha256");
        if !self.show_diff(
            "dict.sha256",
            &base.join("dict.sha256"),
            &other.join("dict.sha256"),
        ) {
            self.note_fail();
        }

        println!("-- eval（--out-cases の列 {CASE_COLUMNS_LABEL}）");
        let base_summary = read_text(&base.join("summary.tsv"));
        let other_summary = read_text(&other.join("summary.tsv"));
        let names: Vec<&str> = base_summary
            .lines()
            .filter_map(|l| l.split('\t').next())
            .collect();
        for name in &names {
            self.compare_cases(name, base, &other);
        }
        // 基準にない構成が今回にだけあるのは、構成が違うということ
        for name in other_summary.lines().filter_map(|l| l.split('\t').next()) {
            if !base_summary
                .lines()
                .any(|l| l.split('\t').next() == Some(name))
            {
                println!("  {name:<28} 基準にない構成");
                self.note_fail();
            }
        }

        println!("-- segments（区切りのレポート）");
        if !self.show_diff(
            "eval/segments.md",
            &base.join("eval/segments.md"),
            &other.join("eval/segments.md"),
        ) {
            self.note_fail();
        }

        println!("-- LM の有効性（時間切れ 0）");
        let mut bad = false;
        for (summary, who) in [(&other_summary, "今回"), (&base_summary, "基準")] {
            for line in summary.lines() {
                let fields: Vec<&str> = line.split('\t').collect();
                let valid = fields.get(7).copied().unwrap_or_default();
                if valid != "ok" {
                    println!("  {:<28} {valid}（{who}）", fields[0]);
                    bad = true;
                }
            }
        }
        if bad {
            self.note_fail();
        } else {
            println!("  すべて有効");
        }

        println!("-- シナリオ");
        let mut scenarios: Vec<String> = std::fs::read_dir(base.join("scenario"))
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        scenarios.sort();
        for name in scenarios {
            let theirs = other.join("scenario").join(&name);
            if theirs.is_file() {
                if !self.show_diff(&name, &base.join("scenario").join(&name), &theirs) {
                    self.note_fail();
                }
            } else {
                println!("  {name:<28} 今回にない");
                self.note_fail();
            }
        }

        println!("-- CLI golden");
        if !self.show_diff(
            "cli/golden.txt",
            &base.join("cli/golden.txt"),
            &other.join("cli/golden.txt"),
        ) {
            self.note_fail();
        }

        println!("-- smoke");
        let (smoke_a, smoke_b) = (base.join("smoke.txt"), other.join("smoke.txt"));
        if smoke_a.is_file() && smoke_b.is_file() {
            let (pa, pb) = (
                smoke_lines(&read_text(&smoke_a)),
                smoke_lines(&read_text(&smoke_b)),
            );
            if pa == pb {
                println!("  PASS/FAIL の行の集合: 一致");
            } else {
                println!("  PASS/FAIL の行の集合: 差分あり");
                let diff = normal_diff(
                    &format!("{}\n", pa.join("\n")),
                    &format!("{}\n", pb.join("\n")),
                );
                for line in diff.lines() {
                    println!("      {line}");
                }
                self.note_fail();
            }
        } else {
            println!("  どちらかに smoke.txt がない（比較しない）");
        }

        println!(
            "-- 速さ（warm p99。基準の {P99_RATIO_LIMIT} 倍超かつ {P99_ABS_FLOOR_MS}ms 超の悪化だけ警告。失敗にはしない）"
        );
        let mut warned = false;
        for name in &names {
            let (pa, pb) = (
                summary_field(&base_summary, name, 3),
                summary_field(&other_summary, name, 3),
            );
            if pa.is_empty() || pb.is_empty() {
                continue;
            }
            let (a, b) = (awk_number(&pa), awk_number(&pb));
            if b > a * P99_RATIO_LIMIT && b - a > P99_ABS_FLOOR_MS {
                println!("  警告 {name:<24} {pa} ms -> {pb} ms");
                warned = true;
            }
        }
        if !warned {
            println!("  悪化なし");
        }

        println!();
        if self.fail_count == 0 {
            println!("結果: 差分 0（一致）");
        } else {
            println!("結果: 差分または無効が {} 件", self.fail_count);
        }
    }
}

// ---- 部品 -----------------------------------------------------------------

fn create(path: &Path) -> Result<File, Failure> {
    File::create(path).map_err(|e| die(format!("{} を作れません: {e}", path.display())))
}

fn write(path: &Path, content: &str) -> Result<(), Failure> {
    std::fs::write(path, content).map_err(|e| die(format!("{} に書けません: {e}", path.display())))
}

fn append(path: &Path, content: &str) -> Result<(), Failure> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(content.as_bytes()))
        .map_err(|e| die(format!("{} に書けません: {e}", path.display())))
}

/// シェルの終了コード（シグナルで終わったら 128 + シグナル番号）。
fn exit_code(status: ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    status.code().unwrap_or(1)
}

/// bash の `printf '%q'` に近い引用（安全な文字はそのまま、ほかは 1 文字ずつバックスラッシュ、空は ''）。
fn shell_quote(word: &str) -> String {
    const SAFE: &str = "_@%+=:,./-";
    if word.is_empty() {
        return "''".to_owned();
    }
    word.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || SAFE.contains(c) || !c.is_ascii() {
                c.to_string()
            } else {
                format!("\\{c}")
            }
        })
        .collect()
}

/// `head -n 40 | sed 's/^/      /'`
fn print_head(text: &str) {
    for line in text.lines().take(DIFF_SHOW_LINES) {
        println!("      {line}");
    }
}

/// `cut -f1-7`（各行の先頭 7 列）
fn cut_columns(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let cols: Vec<&str> = line
            .splitn(CASE_COLUMNS + 1, '\t')
            .take(CASE_COLUMNS)
            .collect();
        out.push_str(&cols.join("\t"));
        out.push('\n');
    }
    out
}

fn summary_field(summary: &str, name: &str, column: usize) -> String {
    summary
        .lines()
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|fields| fields.first() == Some(&name))
        .and_then(|fields| fields.get(column).map(|s| (*s).to_owned()))
        .unwrap_or_default()
}

/// awk の文字列から数への変換（先頭の数だけ読む。読めなければ 0）。
fn awk_number(text: &str) -> f64 {
    let end = text
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    (0..=end)
        .rev()
        .find_map(|n| text[..n].parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// smoke.txt の PASS/FAIL/SKIP と (exit の行。最初の「  (」から後ろを除いて、並べ替える。
fn smoke_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| {
            l.starts_with("PASS")
                || l.starts_with("FAIL")
                || l.starts_with("SKIP")
                || l.starts_with("(exit")
        })
        .map(|l| l.split("  (").next().unwrap_or(l).to_owned())
        .collect();
    lines.sort();
    lines
}

// レポート（Markdown）から値を取り出す。`| 全体 | <件数> | <1位> | ...` のような表の行を、`|` で分けた番号（1 始まり）で読む
fn table_field(md: &str, label: &str, field: usize) -> Option<String> {
    md.lines().find_map(|line| {
        let fields: Vec<&str> = line.split('|').collect();
        (fields.get(1)?.trim_matches(' ') == label).then(|| {
            fields
                .get(field - 1)
                .map(|f| f.replace(' ', ""))
                .unwrap_or_default()
        })
    })
}

fn report_top1(md: &str) -> String {
    table_field(md, "全体", 4).unwrap_or_default()
}

fn report_cases(md: &str) -> String {
    table_field(md, "全体", 3).unwrap_or_default()
}

fn report_warm_p99(md: &str) -> String {
    table_field(md, "warm", 5).unwrap_or_default()
}

fn segments_summary(md: &str) -> String {
    match (table_field(md, "全体", 4), table_field(md, "全体", 7)) {
        (Some(exact), Some(f1)) => format!("完全一致 {exact} F1 {f1}"),
        _ => String::new(),
    }
}

/// `- LM: 呼び出し <n> / 時間切れ <m> ...` の行（最初の 1 行）から (呼び出し, 時間切れ)。
fn lm_counts(md: &str) -> Option<(String, String)> {
    md.lines().find_map(|line| {
        let rest = line.strip_prefix("- LM: 呼び出し ")?;
        let calls: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let rest = rest[calls.len()..].strip_prefix(" / 時間切れ ")?;
        let timeouts: String = rest.chars().take_while(char::is_ascii_digit).collect();
        rest[timeouts.len()..]
            .starts_with(' ')
            .then_some((calls, timeouts))
    })
}

/// `- steps: ... / 失敗: <n>` の行の n。
fn scenario_failures(line: &str) -> Option<String> {
    let rest = line.strip_prefix("- steps: ")?;
    let (_, count) = rest.rsplit_once(" / 失敗: ")?;
    count
        .chars()
        .all(|c| c.is_ascii_digit())
        .then(|| count.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = "| 区分 | 件数 | 1位 | 3位以内 | 平均 | x |\n|---|---|---|---|---|---|\n| 全体 | 120 | 100 | 0.90 | 0.5 | 0.8 |\n| warm | 1 | 2 | 0.123 |\n- LM: 呼び出し 12 / 時間切れ 3 (budget)\n";

    #[test]
    fn golden_cases_skip_comments_and_blank_readings() {
        let text = "# c\n前\tよみ\n\tだけ\ntab なし\n前\t\n";
        assert_eq!(golden_cases(text), vec![("前", "よみ"), ("", "だけ")]);
    }

    #[test]
    fn golden_cases_read_lf_crlf_and_bom_files() {
        // 直す前は CRLF のファイルだと読みの末尾に CR が残っていた
        for (v, text) in ikari_textfile::testing::read_variants("# c\n前\tよみ\n\tだけ\n") {
            // BOM は read_text が外す。ここでは本文だけを渡す
            let body = ikari_textfile::strip_bom(&text);
            assert_eq!(
                golden_cases(body),
                vec![("前", "よみ"), ("", "だけ")],
                "{v:?}"
            );
        }
    }

    #[test]
    fn read_text_strips_the_bom() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("golden.tsv");
        std::fs::write(&path, "\u{feff}前\tよみ\r\n").unwrap();
        assert_eq!(golden_cases(&read_text(&path)), vec![("前", "よみ")]);
    }

    #[test]
    fn reads_report_fields_like_awk() {
        assert_eq!(report_cases(REPORT), "120");
        assert_eq!(report_top1(REPORT), "100");
        assert_eq!(report_warm_p99(REPORT), "0.123");
        assert_eq!(segments_summary(REPORT), "完全一致 100 F1 0.8");
        assert_eq!(report_top1("no table"), "");
    }

    #[test]
    fn reads_lm_counts_and_scenario_failures() {
        assert_eq!(lm_counts(REPORT), Some(("12".to_owned(), "3".to_owned())));
        assert_eq!(lm_counts("- LM: 呼び出し 12 / 時間切れ 3"), None);
        assert_eq!(
            scenario_failures("- steps: 10 / 失敗: 2"),
            Some("2".to_owned())
        );
        assert_eq!(scenario_failures("- steps: 10 / 失敗: x"), None);
        assert_eq!(scenario_failures("steps: 10 / 失敗: 2"), None);
    }

    #[test]
    fn quotes_like_printf_q() {
        assert_eq!(shell_quote("--dict"), "--dict");
        assert_eq!(shell_quote("a b"), "a\\ b");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("data/x.tsv"), "data/x.tsv");
    }

    #[test]
    fn cut_keeps_first_seven_columns() {
        assert_eq!(
            cut_columns("1\t2\t3\t4\t5\t6\t7\t8\t9\n"),
            "1\t2\t3\t4\t5\t6\t7\n"
        );
        assert_eq!(cut_columns("a\tb\n"), "a\tb\n");
    }

    #[test]
    fn awk_number_reads_prefix() {
        assert_eq!(awk_number("0.25"), 0.25);
        assert_eq!(awk_number("abc"), 0.0);
        assert_eq!(awk_number("1.5ms"), 1.5);
    }

    #[test]
    fn smoke_lines_drop_the_parenthesis_and_sort() {
        let text = "PASS b  (pid 1)\nnoise\nFAIL a\n(exit 0)\n";
        assert_eq!(smoke_lines(text), ["(exit 0)", "FAIL a", "PASS b"]);
    }

    #[test]
    fn summary_field_picks_the_row() {
        let summary = "a\t1\t2\t3\nb\t4\t5\t6\n";
        assert_eq!(summary_field(summary, "b", 3), "6");
        assert_eq!(summary_field(summary, "c", 3), "");
    }

    #[test]
    fn full_sets_are_in_the_shell_order() {
        let names: Vec<&str> = FULL_SETS.iter().map(|(name, _)| *name).collect();
        assert_eq!(names.first(), Some(&"context-compound-full"));
        assert_eq!(names.last(), Some(&"issue185-full"));
    }
}
