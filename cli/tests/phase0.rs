//! フェーズ0の統合テスト（ADR 0001 §10.3）。fixture 辞書を作って `ikari` を起動する。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ikari_dict_builder::{BuildConfig, build};

const UNK_POS: &str = "名詞,普通名詞,一般,*,*,*";
const UNK_COST: i16 = 10_000;
const CONTEXT: &str = "ユーザー辞書に単語を登録するときは";
const EXIT_USAGE: i32 = 2;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../testdata/sudachi-mini")
}

fn build_fixture_dict(out_dir: &Path) {
    let cfg = BuildConfig {
        lex: vec![fixture_dir().join("lex.csv")],
        matrix: fixture_dir().join("matrix.def"),
        out_dir: out_dir.to_path_buf(),
        unk_pos: UNK_POS.to_string(),
        unk_cost: UNK_COST,
        label: "phase0-test".to_string(),
    };
    build(&cfg).expect("build fixture dictionary");
}

fn run_ikari(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ikari"))
        .args(args)
        .output()
        .expect("spawn ikari")
}

#[test]
fn hinshi_candidates_include_hinshi_and_hinshi_variants() {
    let tmp = tempfile::tempdir().unwrap();
    build_fixture_dict(tmp.path());

    let out = run_ikari(&[
        "--dict",
        tmp.path().to_str().unwrap(),
        "--context",
        CONTEXT,
        "--format",
        "tsv",
        "-n",
        "10",
        "ひんし",
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("品詞"), "stdout: {stdout}");
    assert!(stdout.contains("瀕死"), "stdout: {stdout}");
}

#[test]
fn romaji_input_reports_pending_on_stderr() {
    let tmp = tempfile::tempdir().unwrap();
    build_fixture_dict(tmp.path());

    let out = run_ikari(&[
        "--dict",
        tmp.path().to_str().unwrap(),
        "--format",
        "tsv",
        "hinshik",
    ]);
    assert!(out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("未確定: k"), "stderr: {stderr}");
}

#[test]
fn missing_dictionary_exits_with_code_1() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("nothing-here");
    let out = run_ikari(&["--dict", missing.to_str().unwrap(), "ひんし"]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn invalid_arguments_exit_with_code_2() {
    assert_eq!(
        run_ikari(&["--page-size", "0", "a"]).status.code(),
        Some(EXIT_USAGE)
    );
    assert_eq!(
        run_ikari(&["--input", "bogus", "a"]).status.code(),
        Some(EXIT_USAGE)
    );
    assert_eq!(run_ikari(&[]).status.code(), Some(EXIT_USAGE));
}

#[test]
fn invalid_jis_notation_exits_with_code_2() {
    // 辞書を開く前に弾くので、辞書は不要
    let out = run_ikari(&["--input", "jis", "あ"]);
    assert_eq!(out.status.code(), Some(EXIT_USAGE));
    let out = run_ikari(&["--input", "jis", "t+"]);
    assert_eq!(out.status.code(), Some(EXIT_USAGE));
}

#[test]
fn invalid_romaji_table_is_an_error_before_opening_dictionary() {
    let tmp = tempfile::tempdir().unwrap();
    let table = tmp.path().join("bad.tsv");
    std::fs::write(&table, "ka\tか\nka\tこ\n").unwrap();
    let out = run_ikari(&["--romaji-table", table.to_str().unwrap(), "ka"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("line 2"), "stderr: {stderr}");
}
