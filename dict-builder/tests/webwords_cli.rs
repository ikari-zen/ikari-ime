//! `ikari-dict-builder webwords`（ADR 0055 の契約のコミット）。中身は未実装で、サブコマンドと引数の形だけを固定する。

use std::process::Command;

fn run(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ikari-dict-builder"))
        .args(args)
        .output()
        .expect("run ikari-dict-builder");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn collect_is_registered_and_not_implemented() {
    let (ok, stderr) = run(&[
        "webwords",
        "collect",
        "--pages-articles",
        "a.bz2",
        "--out",
        "o.tsv",
    ]);
    assert!(!ok);
    assert!(stderr.contains("未実装"), "{stderr}");
}

#[test]
fn classify_is_registered_and_not_implemented() {
    let (ok, stderr) = run(&[
        "webwords",
        "classify",
        "--collected",
        "c.tsv",
        "--page-sql",
        "p",
        "--redirect-sql",
        "r",
        "--page-props-sql",
        "pp",
        "--categorylinks-sql",
        "cl",
        "--linktarget-sql",
        "lt",
        "--lex",
        "l.csv",
        "--source",
        "jawiki-20261001",
        "--sha256",
        "00",
        "--articles",
        "1",
        "--sudachi",
        "s",
        "--out",
        "o.tsv",
    ]);
    assert!(!ok);
    assert!(stderr.contains("未実装"), "{stderr}");
}

#[test]
fn lex_is_registered_and_reads_its_inputs() {
    let (ok, stderr) = run(&[
        "webwords",
        "lex",
        "--list",
        "w.tsv",
        "--lex",
        "l.csv",
        "--out",
        "o.csv",
        "--proper-offset",
        "proper.place=-2000",
    ]);
    assert!(!ok);
    // 実装済み: 引数は通り、入力の一覧が開けなくて失敗する
    assert!(stderr.contains("開けません"), "{stderr}");
}

#[test]
fn lex_rejects_a_common_offset() {
    let (ok, stderr) = run(&[
        "webwords",
        "lex",
        "--list",
        "w.tsv",
        "--lex",
        "l.csv",
        "--out",
        "o.csv",
        "--proper-offset",
        "common=1",
    ]);
    assert!(!ok);
    assert!(!stderr.contains("未実装"), "{stderr}");
}
