//! `cargo xtask check`（ADR 0043 §2-3・段 X1。レーン L-D）。`scripts/check.sh` の移し先。
//! 記録の書式（`ikari-check/<SHA>` に `CHECK_EXIT=`・`SCOPE=` の 2 行、最後に `CHECK_EXIT=<コード>` を出す）を保つ
//! （`.claude/hooks/guard-pr-merge.sh` が読む。`scripts/check-record.sh` と同じ置き場所と挙動）。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::XtaskError;
use crate::data::consts::LibConsts;
use crate::data::run::{
    Failure, capture, cargo, log, repo_root, run as exec, succeeds_quietly, warn,
};

mod docs;
mod frameworks;
mod record;

const SWIFT_TEST_TIMEOUT_ENV: &str = "IKARI_SWIFT_TEST_TIMEOUT";
/// 暫定値（計測なし。通常は数分）。swift test が終わらずに固まることがあったため（check.sh から引き継ぐ）
const SWIFT_TEST_TIMEOUT_SECS: u64 = 900;
const SWIFT_POLL: Duration = Duration::from_millis(500);
const EDGE_MODEL_FILE: &str = "kanakanji-edge-33m-Q8_0.gguf";

/// fmt、clippy、cargo test、swift test（macOS のみ）、文書とスクリプトの検査。結果を記録する（コミット前に流す門）
#[derive(Debug, clap::Args)]
pub struct Args {
    /// ikari-lm / ikari-engine-ffi / ikari-stack を feature つきで clippy と test（cmake が要る）
    #[arg(long)]
    lm: bool,
    /// 実データの #[ignore] テスト（辞書とモデルを使う。--lm を含む）
    #[arg(long)]
    real: bool,
    /// 性能の #[ignore] テスト（release）
    #[arg(long)]
    perf: bool,
    /// cargo xtask eval snapshot --compare <基準>（振る舞いが変わっていないことの確認）
    #[arg(long, value_name = "基準のディレクトリ")]
    eval: Option<PathBuf>,
    /// 文書とスクリプトの検査だけ（数秒。記録しない）
    #[arg(long)]
    docs_only: bool,
    /// swift test を飛ばす（macOS 以外では常に飛ばす）
    #[arg(long)]
    no_swift: bool,
}

struct Plan {
    lm: bool,
    real: bool,
    perf: bool,
    eval: Option<PathBuf>,
    docs_only: bool,
    swift: bool,
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    let root = repo_root();
    let swift = !args.no_swift && cfg!(target_os = "macos");
    if !args.no_swift && !swift {
        warn("macOS ではないので swift test を飛ばします（SCOPE=no-swift）");
    }
    let plan = Plan {
        lm: args.lm || args.real,
        real: args.real,
        perf: args.perf,
        eval: args.eval,
        docs_only: args.docs_only,
        swift,
    };
    let result = steps(&root, &plan);
    let code = match &result {
        Ok(()) => 0,
        Err(failure) => {
            eprintln!("error: {}", failure.message);
            failure.code
        }
    };
    // --docs-only は一部の検査しか流さないので記録しない
    if !plan.docs_only {
        let scope = if plan.swift { "full" } else { "no-swift" };
        record::write(&root, code, scope);
    }
    println!("CHECK_EXIT={code}");
    if code == 0 {
        Ok(())
    } else {
        std::process::exit(code)
    }
}

fn steps(root: &Path, plan: &Plan) -> Result<(), Failure> {
    let consts = LibConsts::load(root).map_err(|e| Failure::new(e.to_string()))?;
    docs::links(root)?;
    docs::model_name(root, &consts)?;
    docs::identity(root)?;
    docs::leaks(root)?;
    docs::adr_status(root)?;
    docs::shell_syntax(root)?;
    for script in ["scripts/test-diag-report.sh", "scripts/test-hooks.sh"] {
        docs::run_script(root, script)?;
    }
    if plan.docs_only {
        log("完了（--docs-only）");
        return Ok(());
    }

    audit(root)?;
    rust_base(root)?;
    if plan.lm {
        rust_lm(root)?;
    }
    if plan.swift {
        // 古い XCFramework のまま swift test が通ったり落ちたりしないよう、先に鮮度を確かめる（#148）
        frameworks::verify(root)?;
        swift_test(root)?;
    }
    if plan.real {
        real(root, &consts)?;
    }
    if plan.perf {
        perf(root)?;
    }
    if let Some(base) = &plan.eval {
        log(&format!("eval の比較: {}", base.display()));
        // 自分自身（cargo xtask eval snapshot）を呼ぶ。シェルを挟まない
        let exe = std::env::current_exe()
            .map_err(|e| Failure::new(format!("xtask の実行ファイルの場所を取れません: {e}")))?;
        let mut command = Command::new(exe);
        command
            .current_dir(root)
            .args(["eval", "snapshot", "--compare"])
            .arg(base);
        exec(&mut command)?;
    }
    log("完了");
    Ok(())
}

fn audit(root: &Path) -> Result<(), Failure> {
    log("依存パッケージの脆弱性（cargo audit。RustSec のデータベースで Cargo.lock を調べる）");
    if !succeeds_quietly(cargo(root).args(["audit", "--version"])) {
        warn("cargo-audit がありません。検査を飛ばします（cargo install cargo-audit --locked）");
        return Ok(());
    }
    exec(cargo(root).args(["audit", "-q"])).map_err(|_| {
        Failure::new(
            "cargo audit が脆弱性を見つけたか、データベースの取得に失敗しました（cargo audit で確かめてください）",
        )
    })
}

fn rust_base(root: &Path) -> Result<(), Failure> {
    log("cargo fmt --check");
    exec(cargo(root).args(["fmt", "--all", "--", "--check"]))?;
    log("cargo clippy --workspace --all-targets -D warnings");
    exec(cargo(root).args([
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ]))?;
    log("cargo test --workspace");
    exec(cargo(root).args(["test", "--workspace"]))
}

fn rust_lm(root: &Path) -> Result<(), Failure> {
    log("LM つきの clippy と test（llama.cpp をビルドする。cmake が要る）");
    if !succeeds_quietly(Command::new("cmake").arg("--version")) {
        return Err(Failure::new("cmake がありません（brew install cmake）"));
    }
    let clippy = [
        ("ikari-lm", "metal"),
        ("ikari-stack", "lm-metal"),
        ("ikari-engine-ffi", "lm-metal"),
        ("ikari-eval", "lm-metal"),
    ];
    for (package, feature) in clippy {
        exec(cargo(root).args([
            "clippy",
            "-p",
            package,
            "--features",
            feature,
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ]))?;
    }
    for (package, feature) in &clippy[..3] {
        exec(cargo(root).args(["test", "-p", package, "--features", feature]))?;
    }
    Ok(())
}

fn swift_timeout() -> u64 {
    std::env::var(SWIFT_TEST_TIMEOUT_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(SWIFT_TEST_TIMEOUT_SECS)
}

/// 子プロセスの木を止める（swift test は子を作る）。
fn kill_tree(pid: u32) {
    if let Some(children) = capture(Command::new("pgrep").args(["-P", &pid.to_string()])) {
        for child in children.split_whitespace().filter_map(|s| s.parse().ok()) {
            kill_tree(child);
        }
    }
    let _ = Command::new("kill").arg(pid.to_string()).status();
}

/// swift test が固まることがあるので、時間を見て、超えたら子ごと止めて失敗にする。
fn swift_test(root: &Path) -> Result<(), Failure> {
    log("swift test");
    let timeout = swift_timeout();
    let mut child = Command::new("swift")
        .current_dir(root)
        .args([
            "test",
            "--manifest-cache",
            "none",
            "--package-path",
            "macos",
        ])
        .spawn()
        .map_err(|e| Failure::new(format!("swift を起動できません: {e}")))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(Failure {
                        code: status.code().unwrap_or(1),
                        message: format!("swift test が失敗しました（{status}）"),
                    })
                };
            }
            Ok(None) => {}
            Err(e) => return Err(Failure::new(format!("swift test の状態を読めません: {e}"))),
        }
        if started.elapsed() >= Duration::from_secs(timeout) {
            kill_tree(child.id());
            let _ = child.wait();
            return Err(Failure::new(format!(
                "swift test が {timeout} 秒で終わらなかったので止めた（固まった疑い）"
            )));
        }
        std::thread::sleep(SWIFT_POLL);
    }
}

fn env_path(name: &str, default: PathBuf) -> PathBuf {
    std::env::var_os(name).map_or(default, PathBuf::from)
}

/// 実データのテスト。前提（辞書、モデル）がなければ、黙って成功せずここで失敗させる
fn real(root: &Path, consts: &LibConsts) -> Result<(), Failure> {
    let zenz_file = consts
        .get("ZENZ_MODEL_FILE")
        .map_err(|e| Failure::new(e.to_string()))?;
    let dict = env_path("IKARI_DICT_DIR", root.join("data/build/dict"));
    let zenz = env_path(
        "IKARI_ZENZ_MODEL",
        root.join("data/raw/zenz").join(zenz_file),
    );
    let edge = env_path(
        "IKARI_EDGE_MODEL",
        root.join("data/raw/kanakanji-edge").join(EDGE_MODEL_FILE),
    );
    let required = [
        dict.join("grammar.bin"),
        dict.join("system.dict"),
        dict.join("vectors.bin"),
        zenz.clone(),
        edge.clone(),
    ];
    for file in &required {
        if !file.is_file() {
            return Err(Failure::new(format!(
                "{} がありません（scripts/setup.sh --fetch で辞書とモデルを用意する。kanakanji-edge は data/raw/kanakanji-edge/ に手で置く）",
                file.display()
            )));
        }
    }
    log("実データのテスト（--ignored）");
    let tests: [&[&str]; 9] = [
        &[
            "test",
            "-p",
            "ikari-core",
            "--test",
            "dict_real",
            "--",
            "--ignored",
        ],
        &[
            "test",
            "--release",
            "-p",
            "ikari-core",
            "--test",
            "segment_real",
            "--",
            "--ignored",
        ],
        &[
            "test",
            "--release",
            "-p",
            "ikari-core",
            "--test",
            "left_context_real",
            "--",
            "--ignored",
        ],
        &[
            "test",
            "-p",
            "ikari-eval",
            "--test",
            "flags",
            "--",
            "--ignored",
        ],
        &[
            "test",
            "--release",
            "-p",
            "ikari-eval",
            "--test",
            "it_pack_real",
            "--",
            "--ignored",
        ],
        &[
            "test",
            "--release",
            "-p",
            "ikari-eval",
            "--test",
            "rare_homophones_real",
            "--",
            "--ignored",
        ],
        &[
            "test",
            "-p",
            "ikari-engine",
            "--features",
            "lm-metal",
            "--",
            "--ignored",
        ],
        &[
            "test",
            "-p",
            "ikari-lm",
            "--features",
            "metal",
            "--test",
            "real_model",
            "--test",
            "real_model_edge",
            "--",
            "--ignored",
        ],
        &[
            "test",
            "-p",
            "ikari-eval",
            "--features",
            "lm-metal",
            "--test",
            "scenario_lm",
            "--",
            "--ignored",
        ],
    ];
    for test in tests {
        exec(
            cargo(root)
                .args(test)
                .env("IKARI_DICT_DIR", &dict)
                .env("IKARI_ZENZ_MODEL", &zenz)
                .env("IKARI_EDGE_MODEL", &edge),
        )?;
    }
    Ok(())
}

fn perf(root: &Path) -> Result<(), Failure> {
    log("性能のテスト（release、--ignored）");
    exec(cargo(root).args([
        "test",
        "--release",
        "-p",
        "ikari-core",
        "--",
        "--ignored",
        "--nocapture",
        "perf",
    ]))?;
    // エンジン内の文節と再変換（p99 ≤ 2ms、LM なし）。実辞書が要る
    let dict = env_path("IKARI_DICT_DIR", root.join("data/build/dict"));
    if !dict.join("system.dict").is_file() {
        return Err(Failure::new(format!(
            "{}/system.dict がありません（scripts/setup.sh --fetch で辞書を用意する）",
            dict.display()
        )));
    }
    exec(
        cargo(root)
            .args([
                "test",
                "--release",
                "-p",
                "ikari-engine",
                "--",
                "--ignored",
                "--nocapture",
                "segment_perf",
            ])
            .env("IKARI_DICT_DIR", &dict),
    )
}
