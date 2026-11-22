//! `cargo xtask scan-product --stage <フォルダ>`: インストーラに入れる前の、配置の形（ADR 0067 §3.1）に並べたフォルダの検査。
//! インストーラ（`windows/installer/ikari.iss`）は、このフォルダの中身をそのまま入れるので、入れる物の全部にここで検査を効かせる。
//!   - あるべきファイルがあること（エンジン、LM の子と llama の DLL、Vulkan のバックエンド、候補窓、設定画面、取得の子、TIP とリソース、helper、ライセンス）
//!   - ikari の Rust の製品（エンジン、helper、取得の子、TIP、候補窓と設定画面の核）に、`scan-product` の検査の全部を効かせる
//!   - LM の子は、開発用の文字列と測定ツールの印だけを見る（llama.cpp と動的な C ランタイムは入っていて正しい。ADR 0067 §4.4・§4.5）
//!   - 直下の exe は決まった名前だけ（評価ツールや開発用の exe を入れない）。直下の DLL に ikari の DLL を置かない
//!     （`ikari.iss` は直下の `*.dll` をすべてインストール先の直下に入れる）。`tip\` には TIP の DLL とリソースだけ
//!   - 測定ツール（`tools/npu-probe`。ADR 0075 §4）の名前のファイルが、どこにも無いこと
//!
//! 他社の部品（llama と ggml の DLL、.NET と Windows App SDK の一式）の中身は見ない（開発用の語の検査は ikari の口のためのもので、
//! 他社の DLL には当てはまらない。たとえば .NET の実行環境は `OutputDebugString` を呼ぶ）。

use std::path::{Path, PathBuf};

use super::{check_bytes, find_dev_strings, find_probe};

/// `scan-product` の検査の全部を効かせる、ikari の Rust の製品（配置の形の中の相対パス。区切りは `/`）
const PRODUCT_FILES: &[&str] = &[
    "ikari-engine-host.exe",
    "ikari-setup-helper.exe",
    // 取得の子（ADR 0076。設定画面の核が、インストール先の直下のこれを起動する。windows/settings/src/fetch_os.rs）
    DATA_EXE,
    "tip/ikari_tip.dll",
    "candidate/ikari_candidate.dll",
    "settings/ikari_settings_core.dll",
];
/// 取得の子
const DATA_EXE: &str = "ikari-data.exe";
/// LM の子。開発用の文字列と測定ツールの印だけを見る
const LM_CHILD: &str = "ikari-lm-host.exe";
/// あるべきもの（中身はここでは見ない）
const REQUIRED_FILES: &[&str] = &[
    "candidate/ikari-candidate.exe",
    "settings/ikari-settings.exe",
    "LICENSE",
    "NOTICE",
    "THIRD_PARTY_NOTICES.md",
    "THIRD_PARTY_LICENSES_RUST.md",
];
/// あるべきフォルダ（TIP が DLL の隣の `resources` を読む。windows/tip/src/deploy.rs）
const REQUIRED_DIRS: &[&str] = &["tip/resources/keymap"];
/// 1 つ以上あるべきもの（フォルダ、名前の前置き、名前の後置き）。LM の子は llama の DLL と、Vulkan のバックエンドを読む（ADR 0059）
const REQUIRED_PATTERNS: &[(&str, &str, &str)] =
    &[("", "llama", ".dll"), ("backends", "ggml-vulkan", ".dll")];
/// 直下に置いてよい exe
const TOP_LEVEL_EXES: &[&str] = &[
    "ikari-engine-host.exe",
    "ikari-setup-helper.exe",
    DATA_EXE,
    LM_CHILD,
];
/// 直下の DLL に使ってはいけない名前の前置き（ikari の DLL は、それぞれのフォルダに置く）
const TOP_LEVEL_DLL_FORBIDDEN_PREFIX: &str = "ikari";
/// `tip\` の直下に置いてよいもの
const TIP_DIR: &str = "tip";
const TIP_ENTRIES: &[&str] = &["ikari_tip.dll", "resources"];
/// どこにも置いてはいけない名前（大文字小文字を区別しない部分の一致）。測定ツール（ADR 0075 §4）
const FORBIDDEN_NAME_PARTS: &[&str] = &["npu-probe", "npu_probe", "openvino"];

/// 配置の形のフォルダの問題の文を返す（空なら合格）
pub fn check_stage(stage: &Path) -> Result<Vec<String>, String> {
    if !stage.is_dir() {
        return Err(format!("{} はフォルダではありません", stage.display()));
    }
    let mut problems = Vec::new();
    for relative in PRODUCT_FILES {
        let path = at(stage, relative);
        match std::fs::read(&path) {
            Ok(bytes) => problems.extend(check_bytes(&path, &bytes)),
            Err(e) => problems.push(format!("{}: ありません（{e}）", path.display())),
        }
    }
    let lm = at(stage, LM_CHILD);
    match std::fs::read(&lm) {
        Ok(bytes) => problems.extend(check_lm_child(&lm, &bytes)),
        Err(e) => problems.push(format!("{}: ありません（{e}）", lm.display())),
    }
    for relative in REQUIRED_FILES {
        let path = at(stage, relative);
        if !path.is_file() {
            problems.push(format!("{}: ありません", path.display()));
        }
    }
    for relative in REQUIRED_DIRS {
        let path = at(stage, relative);
        if !path.is_dir() {
            problems.push(format!("{}: フォルダがありません", path.display()));
        }
    }
    for (dir, prefix, suffix) in REQUIRED_PATTERNS {
        let names = file_names(&at(stage, dir));
        if !names.iter().any(|n| matches(n, prefix, suffix)) {
            problems.push(format!(
                "{}: {prefix}*{suffix} がありません",
                at(stage, dir).display()
            ));
        }
    }
    problems.extend(top_level_problems(stage));
    problems.extend(tip_dir_problems(stage));
    problems.extend(forbidden_names(stage));
    Ok(problems)
}

/// LM の子の検査（開発用の文字列と測定ツールの印）
fn check_lm_child(path: &Path, bytes: &[u8]) -> Vec<String> {
    let dev = find_dev_strings(bytes).into_iter().map(|word| {
        format!(
            "{}: 開発用の文字列 `{word}` が入っています（LM の子を feature `dev-knobs` つきで作っています）",
            path.display()
        )
    });
    let probe = find_probe(bytes).into_iter().map(|word| {
        format!(
            "{}: 測定ツールの印 `{word}` が入っています（tools/npu-probe は製品に入れない。ADR 0075 §4）",
            path.display()
        )
    });
    dev.chain(probe).collect()
}

/// 直下の exe と DLL の名前
fn top_level_problems(stage: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    for name in file_names(stage) {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".exe") && !TOP_LEVEL_EXES.contains(&lower.as_str()) {
            problems.push(format!(
                "{}: 直下に置いてよい exe ではありません（{}）",
                stage.join(&name).display(),
                TOP_LEVEL_EXES.join("、")
            ));
        }
        if lower.ends_with(".dll") && lower.starts_with(TOP_LEVEL_DLL_FORBIDDEN_PREFIX) {
            problems.push(format!(
                "{}: ikari の DLL は直下に置かない（インストール先の直下に入ってしまう）",
                stage.join(&name).display()
            ));
        }
    }
    problems
}

/// `tip\` には、TIP の DLL とリソースだけ
fn tip_dir_problems(stage: &Path) -> Vec<String> {
    let dir = stage.join(TIP_DIR);
    entry_names(&dir)
        .into_iter()
        .filter(|name| !TIP_ENTRIES.contains(&name.as_str()))
        .map(|name| {
            format!(
                "{}: tip には {} だけを置く",
                dir.join(name).display(),
                TIP_ENTRIES.join("、")
            )
        })
        .collect()
}

/// どこにも置いてはいけない名前のファイルとフォルダ
fn forbidden_names(stage: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    let mut pending = vec![stage.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let lower = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if let Some(part) = FORBIDDEN_NAME_PARTS.iter().find(|p| lower.contains(*p)) {
                problems.push(format!(
                    "{}: 測定ツールの部品（名前に `{part}`）は製品に入れない（ADR 0075 §4）",
                    path.display()
                ));
            }
            if path.is_dir() {
                pending.push(path);
            }
        }
    }
    problems.sort();
    problems
}

/// 配置の形の中の相対パス（区切りは `/`）を、OS のパスにする
fn at(stage: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(stage.to_path_buf(), |path, part| path.join(part))
}

/// フォルダの直下のファイルの名前（無ければ空）
fn file_names(dir: &Path) -> Vec<String> {
    names(dir, true)
}

/// フォルダの直下の項目（ファイルとフォルダ）の名前（無ければ空）
fn entry_names(dir: &Path) -> Vec<String> {
    names(dir, false)
}

fn names(dir: &Path, files_only: bool) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| !files_only || e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// 名前が前置きと後置きに合うか（大文字小文字を区別しない）
fn matches(name: &str, prefix: &str, suffix: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with(prefix) && lower.ends_with(suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAN: &[u8] = b"MZ\x90\x00 KERNEL32.dll ntdll.dll";

    fn write(stage: &Path, relative: &str, bytes: &[u8]) {
        let path = at(stage, relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// 合格する配置の形
    fn good_stage() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let stage = dir.path();
        for relative in PRODUCT_FILES.iter().chain(REQUIRED_FILES) {
            write(stage, relative, CLEAN);
        }
        // LM の子は llama.cpp と動的な C ランタイムを持ってよい
        write(stage, LM_CHILD, b"MZ llama.dll VCRUNTIME140.dll");
        write(stage, "llama.dll", b"llama_model_load ggml_backend");
        write(stage, "ggml-base.dll", CLEAN);
        write(stage, "backends/ggml-vulkan.dll", CLEAN);
        write(stage, "backends/ggml-cpu-haswell.dll", CLEAN);
        write(stage, "tip/resources/keymap/ms-ime.tsv", b"a\tb\n");
        write(stage, "candidate/coreclr.dll", b"OutputDebugStringW");
        dir
    }

    fn problems(stage: &Path) -> Vec<String> {
        check_stage(stage).unwrap()
    }

    #[test]
    fn a_good_stage_passes() {
        let dir = good_stage();
        assert_eq!(problems(dir.path()), Vec::<String>::new());
    }

    #[test]
    fn a_missing_stage_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(check_stage(&dir.path().join("none")).is_err());
    }

    #[test]
    fn missing_files_are_reported() {
        let dir = good_stage();
        std::fs::remove_file(at(dir.path(), "settings/ikari-settings.exe")).unwrap();
        std::fs::remove_file(at(dir.path(), "tip/ikari_tip.dll")).unwrap();
        std::fs::remove_file(at(dir.path(), "backends/ggml-vulkan.dll")).unwrap();
        std::fs::remove_dir_all(at(dir.path(), "tip/resources")).unwrap();
        let found = problems(dir.path());
        assert_eq!(found.len(), 4, "{found:?}");
    }

    #[test]
    fn product_files_get_every_check() {
        let dir = good_stage();
        write(dir.path(), "ikari-setup-helper.exe", b"MZ msvcp140.dll");
        write(
            dir.path(),
            "candidate/ikari_candidate.dll",
            b"IKARI_USER_DIR",
        );
        let found = problems(dir.path());
        assert_eq!(found.len(), 2, "{found:?}");
    }

    #[test]
    fn the_lm_child_must_not_have_dev_knobs() {
        let dir = good_stage();
        write(dir.path(), LM_CHILD, b"MZ llama.dll IKARI_LM_GPU");
        let found = problems(dir.path());
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("IKARI_LM_GPU"));
    }

    #[test]
    fn dev_tools_and_stray_dlls_are_rejected() {
        let dir = good_stage();
        write(dir.path(), "ikari-eval.exe", CLEAN);
        write(dir.path(), "ikari_tip.dll", CLEAN);
        write(dir.path(), "tip/ikari_tip.pdb", CLEAN);
        write(dir.path(), "backends/openvino_c.dll", CLEAN);
        let found = problems(dir.path());
        assert_eq!(found.len(), 4, "{found:?}");
    }

    #[test]
    fn relative_paths_use_the_os_separator() {
        assert_eq!(
            at(Path::new("s"), "tip/ikari_tip.dll"),
            Path::new("s").join("tip").join("ikari_tip.dll")
        );
    }
}
