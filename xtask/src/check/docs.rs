//! check の「文書とスクリプトの検査」（`scripts/check.sh` の check_links ほか）。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::data::consts::LibConsts;
use crate::data::run::{Failure, capture, git, has_bash, log, run, succeeds_quietly, warn};

const ENGINE_SOURCE: &str = "engine/src/lib.rs";
const IDENTITY_SWIFT: &str = "macos/Sources/IkariXPC/Identity.swift";
const GEN_IDENTITY: &str = "macos/scripts/gen-identity.sh";
const ADR_DIR: &str = "docs/adr";
const GITLEAKS_CONFIGS: [&str; 2] = [".gitleaks.toml", ".gitleaks.local.toml"];
/// リンク検査から外すディレクトリ（リポジトリ直下からの相対）
const LINK_SKIP_TOP: [&str; 4] = ["target", "data", ".claude", ".git"];
const LINK_SKIP_ANY: &str = ".build";
const SHELL_SCRIPT_DIRS: [&str; 4] = ["scripts", "macos/scripts", "eval/scripts", ".claude/hooks"];

/// root 以下の *.md を並べて返す（target、data、.claude、.git の直下と、どこにある .build は見ない）。
fn markdown_files(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, top: bool, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if (top && LINK_SKIP_TOP.contains(&name.as_str())) || name == LINK_SKIP_ANY {
                    continue;
                }
                walk(&path, false, out);
            } else if kind.is_file() && name.ends_with(".md") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(root, true, &mut files);
    files.sort();
    files
}

/// 1 行の中の `](…)` の中身を取り出す。
fn link_targets(line: &str) -> Vec<&str> {
    let mut targets = Vec::new();
    let mut rest = line;
    while let Some(pos) = rest.find("](") {
        let after = &rest[pos + 2..];
        match after.find(')') {
            Some(end) => {
                targets.push(&after[..end]);
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    targets
}

/// 検査する相対パス（外部リンク、アンカーだけのものは None）。`[text](path "title")` と `<path>` に対応。
fn link_path(target: &str) -> Option<&str> {
    let target = target.split(' ').next().unwrap_or("");
    let target = target.strip_prefix('<').unwrap_or(target);
    let target = target.strip_suffix('>').unwrap_or(target);
    if target.is_empty()
        || target.starts_with('#')
        || target.contains("://")
        || target.starts_with("mailto:")
    {
        return None;
    }
    let path = target.split('#').next().unwrap_or("");
    (!path.is_empty()).then_some(path)
}

/// Markdown の相対リンクが指す先が存在するか（アンカー # 以降と、http(s) などの外部リンクは見ない）
pub fn links(root: &Path) -> Result<(), Failure> {
    log("文書の相対リンク");
    let mut bad = false;
    for file in markdown_files(root) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let dir = file.parent().unwrap_or(root);
        for line in text.lines() {
            for target in link_targets(line) {
                let Some(path) = link_path(target) else {
                    continue;
                };
                if !dir.join(path).exists() {
                    let shown = file.strip_prefix(root).unwrap_or(&file);
                    eprintln!("broken link: {} -> {target}", shown.display());
                    bad = true;
                }
            }
        }
    }
    if bad {
        Err(Failure::new("切れた相対リンクがあります"))
    } else {
        Ok(())
    }
}

/// モデルのファイル名が、言語をまたいで一致していること（scripts/lib.sh と engine/src/lib.rs）
pub fn model_name(root: &Path, consts: &LibConsts) -> Result<(), Failure> {
    log("モデルのファイル名の一致");
    let name = consts
        .get("ZENZ_MODEL_FILE")
        .map_err(|e| Failure::new(e.to_string()))?;
    let source = std::fs::read_to_string(root.join(ENGINE_SOURCE))
        .map_err(|e| Failure::new(format!("{ENGINE_SOURCE} を読めません: {e}")))?;
    if source.contains(&format!("MODEL_FILE_NAME: &str = \"{name}\"")) {
        Ok(())
    } else {
        Err(Failure::new(format!(
            "{ENGINE_SOURCE} の MODEL_FILE_NAME が scripts/lib.sh の ZENZ_MODEL_FILE（{name}）と違います"
        )))
    }
}

/// ADR の本文の「状態」の語（`- 状態: 採用（…）` の「採用」）
fn adr_body_status(text: &str) -> String {
    let line = text
        .lines()
        .find(|l| l.starts_with("- 状態:"))
        .unwrap_or("");
    let body = line.trim_start_matches("- 状態:").trim_start_matches(' ');
    let body = body.split('（').next().unwrap_or("");
    body.split(' ').next().unwrap_or("").to_owned()
}

/// 索引（README の `| [NNNN](…) | 題 | 状態 | … |` の行）の状態。一致する行が複数あれば改行でつなぐ。
fn adr_index_status(readme: &str, number: &str) -> String {
    let prefix = format!("| [{number}]");
    readme
        .lines()
        .filter(|l| l.starts_with(&prefix))
        .map(|l| l.split('|').nth(3).unwrap_or("").replace(' ', ""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// ADR の索引の状態が、各 ADR の本文の「状態」の語と一致していること
pub fn adr_status(root: &Path) -> Result<(), Failure> {
    log("ADR の索引の状態");
    let dir = root.join(ADR_DIR);
    let readme = std::fs::read_to_string(dir.join("README.md")).unwrap_or_default();
    let mut adrs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| Failure::new(format!("{ADR_DIR} を読めません: {e}")))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            name.ends_with(".md")
                && name.len() > 5
                && name.as_bytes()[..4].iter().all(u8::is_ascii_digit)
                && name.as_bytes()[4] == b'-'
        })
        .collect();
    adrs.sort();
    let mut bad = false;
    for adr in adrs {
        let name = adr
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let number = &name[..4];
        let text = std::fs::read_to_string(&adr).unwrap_or_default();
        let body = adr_body_status(&text);
        let index = adr_index_status(&readme, number);
        if body.is_empty() || body != index {
            eprintln!("ADR {number}: 本文の状態「{body}」と索引「{index}」が違います");
            bad = true;
        }
    }
    if bad {
        Err(Failure::new("ADR の索引と本文の状態が食い違っています"))
    } else {
        Ok(())
    }
}

/// 識別子の頭とチームID が、lib.sh と Identity.swift で一致していること（ADR 0036 §3.1）。gen-identity.sh が bash なので、bash があるときだけ
pub fn identity(root: &Path) -> Result<(), Failure> {
    log("Identity.swift が lib.sh の既定値から生成したものと同じか");
    if !has_bash() {
        warn("bash がありません。Identity.swift の検査を飛ばします");
        return Ok(());
    }
    // 環境変数の上書きを外して、既定値で生成し直す
    let generated = capture(
        Command::new("bash")
            .current_dir(root)
            .args([GEN_IDENTITY, "--stdout"])
            .env_remove("IKARI_ID_PREFIX")
            .env_remove("TEAM_ID"),
    );
    let current = std::fs::read(root.join(IDENTITY_SWIFT)).unwrap_or_default();
    match generated {
        Some(text) if text.as_bytes() == current.as_slice() => Ok(()),
        _ => Err(Failure::new(format!(
            "{IDENTITY_SWIFT} が lib.sh の既定値と違います。macos/scripts/gen-identity.sh を流してください"
        ))),
    }
}

/// 秘密と個人情報の検査（gitleaks。コミットの作者のメールも）
pub fn leaks(root: &Path) -> Result<(), Failure> {
    log("秘密と個人情報の検査（gitleaks。コミットの作者のメールも）");
    // コミットの作者・署名者のメールは noreply だけ（gitleaks は作者欄を見ないので、ここで調べる）
    let emails = capture(git(root).args(["log", "--all", "--format=%ae%n%ce"])).unwrap_or_default();
    let mut bad: Vec<&str> = emails.lines().filter(|l| !l.contains("noreply")).collect();
    bad.sort_unstable();
    bad.dedup();
    if !bad.is_empty() {
        return Err(Failure::new(format!(
            "noreply 以外のメールがコミットの作者欄にあります（GitHub の設定で Keep my email addresses private をオンに）: {}",
            bad.join(" ")
        )));
    }
    if !succeeds_quietly(Command::new("gitleaks").arg("version")) {
        warn("gitleaks がありません。検査を飛ばします（brew install gitleaks）");
        return Ok(());
    }
    for conf in GITLEAKS_CONFIGS {
        if !root.join(conf).is_file() {
            continue;
        }
        let scan = |extra: &[&str]| {
            succeeds_quietly(
                Command::new("gitleaks")
                    .current_dir(root)
                    .args(["git", ".", "--config", conf])
                    .args(extra)
                    .args(["--no-banner", "--redact"]),
            )
        };
        if !scan(&[]) {
            return Err(Failure::new(format!(
                "gitleaks が履歴に漏れを見つけました（{conf}）。gitleaks git . --config {conf} --redact で確かめてください"
            )));
        }
        if !scan(&["--staged"]) {
            return Err(Failure::new(format!(
                "gitleaks がステージ済みの変更に漏れを見つけました（{conf}）"
            )));
        }
    }
    Ok(())
}

/// シェルスクリプトの構文（bash -n）。bash がなければ飛ばす
pub fn shell_syntax(root: &Path) -> Result<(), Failure> {
    log("シェルスクリプトの構文");
    if !has_bash() {
        warn("bash がありません。シェルスクリプトの構文検査を飛ばします");
        return Ok(());
    }
    for dir in SHELL_SCRIPT_DIRS {
        let mut scripts: Vec<PathBuf> = std::fs::read_dir(root.join(dir))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "sh"))
            .collect();
        scripts.sort();
        for script in scripts {
            let ok = Command::new("bash")
                .arg("-n")
                .arg(&script)
                .stdin(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if !ok {
                let shown = script.strip_prefix(root).unwrap_or(&script);
                return Err(Failure::new(format!(
                    "{} に構文エラーがあります",
                    shown.display()
                )));
            }
        }
    }
    Ok(())
}

/// リポジトリのシェルスクリプトを bash で流す（テスト用。bash がなければ飛ばす）
pub fn run_script(root: &Path, script: &str) -> Result<(), Failure> {
    if !has_bash() {
        warn(&format!("bash がありません。{script} を飛ばします"));
        return Ok(());
    }
    run(Command::new("bash").current_dir(root).arg(script))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_targets_and_paths() {
        let line = "see [a](x/y.md#h) and [b](https://e.com) [c](<z.md> \"t\") [d](#top)";
        let paths: Vec<_> = link_targets(line)
            .into_iter()
            .filter_map(link_path)
            .collect();
        assert_eq!(paths, ["x/y.md", "z.md"]);
    }

    #[test]
    fn adr_status_words() {
        assert_eq!(
            adr_body_status("# t\n- 状態: 採用（2026-10-01 変更）\n"),
            "採用"
        );
        assert_eq!(adr_body_status("- 状態:  提案 \n"), "提案");
        assert_eq!(adr_body_status("none"), "");
        let readme = "| [0042](0042-x.md) | 題 | 採用 | 概要 |\n";
        assert_eq!(adr_index_status(readme, "0042"), "採用");
        assert_eq!(adr_index_status(readme, "0001"), "");
    }
}
