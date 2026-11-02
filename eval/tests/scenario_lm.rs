//! 学習のシナリオ（a-02、a-03、a-04、b23-*）を、実辞書と実モデル（LM あり）で流す（`#[ignore]`）。
//! ADR 0003 §6-11: LM が学習の結果を覆さない。spec の基本原則: 学習が現在の文脈を覆さない。
//!
//! ```text
//! IKARI_DICT_DIR=data/build/dict IKARI_ZENZ_MODEL=data/raw/zenz/zenz-v3.2-small-Q5_K_M.gguf \
//!   CARGO_TARGET_DIR=target/lm cargo test -p ikari-eval --features lm-metal --release \
//!   --test scenario_lm -- --ignored --nocapture
//! ```
//!
//! どちらかの環境変数が未設定なら、テストは失敗する（`#[ignore]` を明示して流したときに黙って成功させない）。
#![cfg(feature = "lm")]

use std::path::{Path, PathBuf};
use std::process::Command;

const DICT_ENV: &str = "IKARI_DICT_DIR";
const MODEL_ENV: &str = "IKARI_ZENZ_MODEL";
const EXIT_OK: i32 = 0;
/// 先行計算が済んでいる状況（エンジンの Prefetch は最大 300ms）を再現する締め切り
const DEADLINE_MS: &str = "300";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("eval/ has a parent")
        .to_path_buf()
}

fn env_path(name: &str) -> PathBuf {
    let p = std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} is not set (required by #[ignore] tests)"));
    assert!(p.exists(), "{name}={} does not exist", p.display());
    p
}

fn run_scenario(scenario: &str) {
    run_scenario_with(scenario, &[]);
}

/// `extra` は ikari-eval に足す引数（`--carry-fill` など）
fn run_scenario_with(scenario: &str, extra: &[&str]) {
    let (dict, model) = (env_path(DICT_ENV), env_path(MODEL_ENV));
    let file = workspace_root().join("eval/scenarios").join(scenario);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ikari-eval"));
    cmd.arg("--dict")
        .arg(&dict)
        .args(["--ranker", "full", "--lm"])
        .arg(&model)
        .args([
            "--lm-budget-ms",
            DEADLINE_MS,
            "--lm-deadline-ms",
            DEADLINE_MS,
        ])
        .args(extra)
        .arg("--scenario")
        .arg(&file);
    if let Some(config) = config {
        cmd.arg("--ranker-config")
            .arg(workspace_root().join("eval/scenarios").join(config));
    }
    let out = cmd.output().expect("run ikari-eval");
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!("{stdout}");
    assert_eq!(
        out.status.code(),
        Some(EXIT_OK),
        "{scenario} failed with LM: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("失敗: 0"), "{stdout}");
}

#[test]
#[ignore = "requires the real dictionary and a zenz GGUF model (IKARI_DICT_DIR, IKARI_ZENZ_MODEL)"]
fn a02_holds_with_the_lm() {
    run_scenario("a-02.tsv");
}

#[test]
#[ignore = "requires the real dictionary and a zenz GGUF model (IKARI_DICT_DIR, IKARI_ZENZ_MODEL)"]
fn a03_learning_does_not_override_the_current_context_with_the_lm() {
    run_scenario("a-03.tsv");
}

#[test]
#[ignore = "requires the real dictionary and a zenz GGUF model (IKARI_DICT_DIR, IKARI_ZENZ_MODEL)"]
fn a04_context_free_learning_does_not_override_the_current_context_with_the_lm() {
    run_scenario("a-04.tsv");
}

#[test]
#[ignore = "requires the real dictionary and a zenz GGUF model (IKARI_DICT_DIR, IKARI_ZENZ_MODEL)"]
fn b23_naoru_learning_without_context_with_the_lm() {
    run_scenario("b23-naoru.tsv");
}

#[test]
#[ignore = "requires the real dictionary and a zenz GGUF model (IKARI_DICT_DIR, IKARI_ZENZ_MODEL)"]
fn b23_susumeru_learning_without_context_with_the_lm() {
    run_scenario("b23-susumeru.tsv");
}

#[test]
#[ignore = "requires the real dictionary and a zenz GGUF model (IKARI_DICT_DIR, IKARI_ZENZ_MODEL)"]
fn b23_same_context_keeps_the_learned_surface_with_the_lm() {
    run_scenario("b23-same-context.tsv");
}

/// ADR 0046: 入力欄の左が足りないとき、同じアプリの確定履歴で補うと 品詞を が 1 位になる（`--carry-fill 40`）
#[test]
#[ignore = "requires the real dictionary and a zenz GGUF model (IKARI_DICT_DIR, IKARI_ZENZ_MODEL)"]
fn c182_carryover_puts_the_history_into_the_left_context_with_the_lm() {
    run_scenario_with("c182-carryover.tsv", &["--carry-fill", "40"]);
}

#[test]
#[ignore = "requires the real dictionary and a zenz GGUF model (IKARI_DICT_DIR, IKARI_ZENZ_MODEL)"]
fn issue177_recent_choice_wins_within_the_window_with_the_lm() {
    let conf = workspace_root().join("eval/scenarios/issue177-recent.conf");
    run_scenario_with(
        "issue177-recent.tsv",
        &["--ranker-config", conf.to_str().unwrap()],
    );
}
