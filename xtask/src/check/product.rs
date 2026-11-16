//! 製品のビルドの検査のうち、ソースと設定を読むだけでできるもの（ADR 0067 §4.4・§6）。
//!   - Windows（MSVC）で C ランタイムを静的に取り込む設定（`crt-static`）が、`.cargo/config.toml` と、
//!     `RUSTFLAGS` を設定する workflow（設定すると config の rustflags は無視される）にあること
//!   - `dev-knobs` を、CLI と評価の依存以外の `Cargo.toml` が付けていないこと（既定の feature にも入れない）
//!   - 開発用の文字列が、`dev-knobs` の feature の中（かテスト）にしか書かれていないこと
//!
//! 成果物（exe、DLL）の検査は `cargo xtask scan-product`（Windows の成果物ができたあと）。

use std::path::Path;

use crate::data::run::{Failure, git, log};
use crate::scan::DEV_STRINGS;

const CARGO_CONFIG: &str = ".cargo/config.toml";
const WORKFLOW_DIR: &str = ".github/workflows";
const CRT_STATIC: &str = "target-feature=+crt-static";
const FEATURE_NAME: &str = "dev-knobs";
/// `dev-knobs` を依存の feature に付けてよい `Cargo.toml`（開発の道具。製品に入らない）
const KNOB_USERS: &[&str] = &["cli/Cargo.toml", "eval/Cargo.toml"];
/// 開発用の文字列を調べるソースの置き場（先頭のディレクトリ）。xtask、CLI、評価、macOS の殻は対象外
const SOURCE_ROOTS: &[&str] = &[
    "core/",
    "lm/",
    "pipe/",
    "stack/",
    "engine/",
    "engine-ffi/",
    "shell-ffi/",
    "windows/",
];
/// ソースに書いた形で調べる語（`scan::DEV_STRINGS` のうち、引用符で書くもの）に足す、呼び出し
const EXTRA_SOURCE_WORDS: &[&str] = &["OutputDebugStringW("];
/// 空行をまたがず、直前のこの行数のうちに `dev-knobs` の条件があれば、囲まれているとみなす
const GATE_LOOKBACK: usize = 6;
const TEST_MODULE_MARK: &str = "#[cfg(test)]";

/// `.cargo/config.toml` と、`RUSTFLAGS` を設定する workflow に `crt-static` があること
pub fn crt_static(root: &Path) -> Result<(), Failure> {
    log("Windows の C ランタイムを静的に取り込む設定（crt-static。ADR 0067 §4.4）");
    let config = read(root, CARGO_CONFIG)?;
    let mut problems = Vec::new();
    if !config_has_crt_static(&config) {
        problems.push(format!(
            "{CARGO_CONFIG} の [target.x86_64-pc-windows-msvc] に rustflags = [\"-C\", \"{CRT_STATIC}\"] がありません"
        ));
    }
    for file in tracked(root, WORKFLOW_DIR)? {
        let text = read(root, &file)?;
        for line in workflow_rustflags_without_crt_static(&text) {
            problems.push(format!(
                "{file}: Windows の RUSTFLAGS に `{CRT_STATIC}` がありません（RUSTFLAGS を設定すると config の rustflags は無視される）: {line}"
            ));
        }
    }
    finish(problems)
}

/// `dev-knobs` の付け方（依存の feature と既定の feature）が、製品のビルドに入らない形であること
pub fn dev_knobs_wiring(root: &Path) -> Result<(), Failure> {
    log("dev-knobs が製品の依存と既定の feature に入っていないこと（ADR 0067 §6）");
    let mut problems = Vec::new();
    for file in tracked(root, "")? {
        if !file.ends_with("Cargo.toml") {
            continue;
        }
        let text = read(root, &file)?;
        let allowed = KNOB_USERS.contains(&file.as_str());
        for line in knob_wiring_problems(&text, allowed) {
            problems.push(format!("{file}: {line}"));
        }
    }
    finish(problems)
}

/// 開発用の文字列が、`dev-knobs` の feature の中（かテスト）にしか書かれていないこと
pub fn dev_strings_gated(root: &Path) -> Result<(), Failure> {
    log("開発用の文字列が dev-knobs の feature の中にだけあること（ADR 0067 §6）");
    let mut problems = Vec::new();
    for file in tracked(root, "")? {
        let is_source = file.ends_with(".rs")
            && SOURCE_ROOTS.iter().any(|r| file.starts_with(r))
            && !file.contains("/tests/")
            && !file.ends_with("/tests.rs");
        if !is_source {
            continue;
        }
        let text = read(root, &file)?;
        for (line_no, word) in ungated_words(&text) {
            problems.push(format!(
                "{file}:{line_no}: `{word}` が `dev-knobs` の feature の外にあります（製品のビルドに入ります）"
            ));
        }
    }
    finish(problems)
}

/// LM の子の起動の入口（`host_loader(`）を呼んでよいソース。ここ（`ikari-stack`。エンジンと engine-host が通る）の
/// 1 か所だけにして、引数・環境・権限の方針を 1 か所に保つ（ADR 0072）。`lm/` の中（定義、テスト）は対象外
const LM_SPAWN_ENTRY: &str = "stack/src/lm.rs";
const LM_SPAWN_CALL: &str = "host_loader(";

/// LM の子の起動の入口が、`LM_SPAWN_ENTRY` の 1 か所だけであること
pub fn lm_spawn_single(root: &Path) -> Result<(), Failure> {
    log("LM の子の起動が 1 か所だけを通ること（ADR 0072）");
    let mut problems = Vec::new();
    for file in tracked(root, "")? {
        let is_source = file.ends_with(".rs")
            && SOURCE_ROOTS.iter().any(|r| file.starts_with(r))
            && !file.starts_with("lm/")
            && file != LM_SPAWN_ENTRY
            && !file.contains("/tests/")
            && !file.ends_with("/tests.rs");
        if !is_source {
            continue;
        }
        let text = read(root, &file)?;
        if let Some(line_no) = spawn_calls_outside_tests(&text).first() {
            problems.push(format!(
                "{file}:{line_no}: `{LM_SPAWN_CALL}` を呼んでいます（LM の子の起動は {LM_SPAWN_ENTRY} だけを通す）"
            ));
        }
    }
    finish(problems)
}

/// テストの前（`#[cfg(test)]` より前）で、コメントでない行に `host_loader(` がある行番号
fn spawn_calls_outside_tests(text: &str) -> Vec<usize> {
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with(TEST_MODULE_MARK) {
            break;
        }
        if !trimmed.starts_with("//") && trimmed.contains(LM_SPAWN_CALL) {
            found.push(index + 1);
        }
    }
    found
}

fn finish(problems: Vec<String>) -> Result<(), Failure> {
    if problems.is_empty() {
        Ok(())
    } else {
        Err(Failure::new(problems.join("\n")))
    }
}

fn read(root: &Path, relative: &str) -> Result<String, Failure> {
    std::fs::read_to_string(root.join(relative))
        .map_err(|e| Failure::new(format!("{relative} を読めません: {e}")))
}

/// git が管理するファイル（と、まだ add していないもの）のうち、`prefix` の下のもの
fn tracked(root: &Path, prefix: &str) -> Result<Vec<String>, Failure> {
    let output = git(root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
        ])
        .arg(if prefix.is_empty() { "." } else { prefix })
        .output()
        .map_err(|e| Failure::new(format!("git ls-files を起動できません: {e}")))?;
    if !output.status.success() {
        return Err(Failure::new("git ls-files が失敗しました"));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect())
}

fn config_has_crt_static(config: &str) -> bool {
    let mut in_section = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line == "[target.x86_64-pc-windows-msvc]";
        } else if in_section && !line.starts_with('#') && line.contains(CRT_STATIC) {
            return true;
        }
    }
    false
}

/// Windows のビルド機（`USERPROFILE` を使う行）で `RUSTFLAGS` を設定していて、`crt-static` が無い行
fn workflow_rustflags_without_crt_static(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .filter(|l| {
            l.contains("RUSTFLAGS=") && l.contains("USERPROFILE") && !l.contains(CRT_STATIC)
        })
        .map(str::to_owned)
        .collect()
}

/// `Cargo.toml` の 1 ファイルの、`dev-knobs` の付け方の問題（行の文）。`allowed` は依存に付けてよいファイル
fn knob_wiring_problems(text: &str, allowed: bool) -> Vec<String> {
    let mut problems = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') || !line.contains(FEATURE_NAME) {
            continue;
        }
        if line.starts_with("default") {
            problems.push(format!(
                "既定の feature に `{FEATURE_NAME}` があります: {line}"
            ));
        } else if line.contains("features") && !allowed {
            problems.push(format!(
                "依存に `{FEATURE_NAME}` を付けています（付けてよいのは {} だけ）: {line}",
                KNOB_USERS.join("、")
            ));
        }
    }
    problems
}

/// 調べる語（ソースに書く形）
fn source_words() -> Vec<String> {
    DEV_STRINGS
        .iter()
        // 語の一部が普通の文に出る（"ikari-tip: " の接頭辞）ものと、呼び出しの名前で調べるものは、別に扱う
        .filter(|w| !w.starts_with("OutputDebugString") && !w.ends_with(": "))
        .map(|w| format!("\"{w}\""))
        .chain(EXTRA_SOURCE_WORDS.iter().map(|w| (*w).to_owned()))
        .collect()
}

/// ソース 1 ファイルのうち、`dev-knobs` の外（かテストの外）にある開発用の語（行番号と語）
fn ungated_words(text: &str) -> Vec<(usize, String)> {
    let words = source_words();
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with(TEST_MODULE_MARK) {
            // この先はテスト（ファイルの末尾に置く流儀）
            break;
        }
        if trimmed.starts_with("//") {
            continue;
        }
        let Some(word) = words.iter().find(|w| line.contains(w.as_str())) else {
            continue;
        };
        if !is_gated(&lines, index) {
            found.push((index + 1, word.clone()));
        }
    }
    found
}

/// `lines[index]` が、`dev-knobs` の条件の直下か、その条件がついた最上位の `mod` の中にあるか
fn is_gated(lines: &[&str], index: usize) -> bool {
    let start = index.saturating_sub(GATE_LOOKBACK);
    // 空行までの、直前の数行（属性と宣言のかたまり）に条件があるか
    let block = lines[start..index]
        .iter()
        .rev()
        .take_while(|l| !l.trim().is_empty());
    if block.into_iter().any(|l| l.contains(FEATURE_NAME)) {
        return true;
    }
    // 直近の最上位（字下げなし）の宣言が `mod` で、その直前の行が条件
    let top = (0..=index).rev().find(|&i| {
        let l = lines[i];
        !l.is_empty() && !l.starts_with([' ', '\t', '}', '/', '#', ')'])
    });
    top.is_some_and(|i| {
        lines[i].trim_start().starts_with("mod ") && i > 0 && lines[i - 1].contains(FEATURE_NAME)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_needs_crt_static_in_the_windows_section() {
        let ok = "[alias]\nx = \"y\"\n\n[target.x86_64-pc-windows-msvc]\nrustflags = [\"-C\", \"target-feature=+crt-static\"]\n";
        assert!(config_has_crt_static(ok));
        let wrong_section = "[alias]\nrustflags = [\"-C\", \"target-feature=+crt-static\"]\n";
        assert!(!config_has_crt_static(wrong_section));
        let commented = "[target.x86_64-pc-windows-msvc]\n# target-feature=+crt-static\n";
        assert!(!config_has_crt_static(commented));
    }

    #[test]
    fn only_windows_rustflags_lines_need_crt_static() {
        let text = "run: echo \"RUSTFLAGS=--remap-path-prefix=${HOME}=/c\" >> x\n\
                    \"RUSTFLAGS=--remap $($env:USERPROFILE)\\.cargo=/cargo\" | Out\n\
                    \"RUSTFLAGS=--remap $($env:USERPROFILE) -C target-feature=+crt-static\" | Out\n";
        let bad = workflow_rustflags_without_crt_static(text);
        assert_eq!(bad.len(), 1);
        assert!(bad[0].contains("USERPROFILE") && !bad[0].contains("crt-static"));
    }

    #[test]
    fn knob_wiring() {
        let manifest = "[features]\ndev-knobs = [\"a/dev-knobs\"]\n[dependencies]\nikari-core = { workspace = true, features = [\"dev-knobs\"] }\n";
        assert!(knob_wiring_problems(manifest, true).is_empty());
        assert_eq!(knob_wiring_problems(manifest, false).len(), 1);
        let default = "[features]\ndefault = [\"dev-knobs\"]\n";
        assert_eq!(knob_wiring_problems(default, true).len(), 1);
    }

    #[test]
    fn dev_strings_must_be_gated() {
        let text = "#[cfg(feature = \"dev-knobs\")]\nconst A: &str = \"IKARI_USER_DIR\";\n\nconst B: &str = \"IKARI_LM_GPU\";\n";
        let found = ungated_words(text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 4);
    }

    #[test]
    fn lm_spawn_calls_are_found_outside_tests_only() {
        let text = "// host_loader( の説明\nfn f() { let _ = host_loader(spec); }\n#[cfg(test)]\nmod t { fn g() { host_loader(s); } }\n";
        assert_eq!(spawn_calls_outside_tests(text), vec![2]);
    }

    #[test]
    fn module_gate_covers_the_whole_module() {
        let text = "#[cfg(feature = \"dev-knobs\")]\nmod on {\n    use x;\n\n    fn f() {\n        write(\"x\");\n        unsafe { OutputDebugStringW(&t) };\n    }\n}\n\nfn g() { OutputDebugStringW(&t); }\n";
        let found = ungated_words(text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 11);
    }

    #[test]
    fn comments_and_tests_are_skipped() {
        let text = "// \"IKARI_USER_DIR\" の説明\nfn f() {}\n#[cfg(test)]\nmod tests { const X: &str = \"IKARI_USER_DIR\"; }\n";
        assert!(ungated_words(text).is_empty());
    }
}
