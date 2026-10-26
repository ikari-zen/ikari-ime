//! ADR 0001 §10.3: fixture の辞書で `ikari-eval` を実行し、レポートの節を確かめる。
//! レーンBの辞書ビルド（`ikari_dict_builder::build`）と `testdata/sudachi-mini/` が必要。

use std::path::{Path, PathBuf};
use std::process::Command;

use ikari_dict_builder::{BuildConfig, build};

const UNK_POS: &str = "名詞,普通名詞,一般,*,*,*";
const UNK_COST: i16 = 10_000;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("eval/ has a parent")
        .to_path_buf()
}

#[test]
fn report_has_accuracy_and_latency_sections() {
    let root = workspace_root();
    let fixture = root.join("testdata/sudachi-mini");
    let tmp = tempfile::tempdir().unwrap();
    let dict_dir = tmp.path().join("dict");

    build(&BuildConfig {
        lex: vec![fixture.join("lex.csv")],
        matrix: fixture.join("matrix.def"),
        out_dir: dict_dir.clone(),
        unk_pos: UNK_POS.to_owned(),
        unk_cost: UNK_COST,
        label: "sudachi-mini".to_owned(),
    })
    .expect("build fixture dictionary");

    let out_cases = tmp.path().join("cases.tsv");
    let output = Command::new(env!("CARGO_BIN_EXE_ikari-eval"))
        .arg("--dict")
        .arg(&dict_dir)
        .arg("--set")
        .arg(root.join("eval/sets/homophones.tsv"))
        .args(["--repeat", "2"])
        .arg("--out-cases")
        .arg(&out_cases)
        .output()
        .expect("run ikari-eval");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = String::from_utf8(output.stdout).unwrap();
    assert!(report.contains("正解率"), "{report}");
    assert!(report.contains("応答時間"), "{report}");
    assert!(report.contains("(14 cases)"), "{report}");
    let cases = std::fs::read_to_string(out_cases).unwrap();
    assert_eq!(cases.lines().count(), 15);
}
