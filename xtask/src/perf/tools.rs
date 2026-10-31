//! `perf profile` と `perf alloc` が共有する部品: 外部の道具の有無の確認、測り手のビルドと実行ファイルの場所、報告の見出し。
//!
//! どれもシェルを使わず、`Command` に引数の列を渡す（Windows でも同じに動く）。

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use super::record::{self, capture, err, repo_root};
use crate::XtaskError;

/// 殻の測り手のパッケージ名と bench の名前（`shell-core/benches/shell_perf.rs`）
const SHELL_PACKAGE: &str = "ikari-shell-core";
const SHELL_BENCH: &str = "shell_perf";
/// プロファイルと確保の計測に要る、関数名と行（`debug = 1`）を、bench のビルドだけに足す環境変数
const BENCH_DEBUG_ENV: (&str, &str) = ("CARGO_PROFILE_BENCH_DEBUG", "1");
/// 結果のファイル（プロファイル、dhat）を置く場所（リポジトリの `target/` の下。git に入らない）
const OUTPUT_DIR: [&str; 2] = ["target", "perf-tools"];
/// 負荷平均がこれ以上のとき、報告に注意を書く（ほかの処理と競っている）
pub const QUIET_LOAD_AVERAGE: f64 = 4.0;

/// `program --version` が動くか確かめる。動かなければ、入れ方を案内して止まる（自動では入れない）。
pub fn require_tool(program: &str, install_hint: &str) -> Result<(), XtaskError> {
    match Command::new(program).arg("--version").output() {
        Ok(output) if output.status.success() => Ok(()),
        _ => Err(err(format!(
            "{program} が見つかりません。次の手順で入れてから、もう一度流してください（自動では入れません）:\n  {install_hint}"
        ))),
    }
}

/// 結果のファイルを置く場所（なければ作る）。
pub fn output_dir() -> Result<PathBuf, XtaskError> {
    let dir = OUTPUT_DIR
        .iter()
        .fold(repo_root().to_path_buf(), |path, part| path.join(part));
    std::fs::create_dir_all(&dir)
        .map_err(|error| err(format!("{} を作れません: {error}", dir.display())))?;
    Ok(dir)
}

/// 殻の測り手（`shell_perf`）を、関数名と行が残る形でビルドし、実行ファイルの場所を返す。
/// `features` は `ikari-shell-core` の feature（確保の計測は `dhat-heap`）。
pub fn build_shell_bench(features: &[&str]) -> Result<PathBuf, XtaskError> {
    let mut command = Command::new("cargo");
    command
        .current_dir(repo_root())
        .env(BENCH_DEBUG_ENV.0, BENCH_DEBUG_ENV.1)
        .args(["bench", "-p", SHELL_PACKAGE, "--bench", SHELL_BENCH])
        .args(["--no-run", "--message-format=json"]);
    if !features.is_empty() {
        command.args(["--features", &features.join(",")]);
    }
    let stdout = capture(&mut command, "cargo bench --no-run")?;
    executable_from_cargo_json(&stdout, SHELL_BENCH).ok_or_else(|| {
        err(format!(
            "cargo の出力に {SHELL_BENCH} の実行ファイルが見つかりません"
        ))
    })
}

/// `cargo --message-format=json` の出力から、`bench_name` の実行ファイルを取り出す。
pub fn executable_from_cargo_json(stdout: &str, bench_name: &str) -> Option<PathBuf> {
    stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .filter(|message| message["target"]["name"] == bench_name)
        .find_map(|message| message["executable"].as_str().map(PathBuf::from))
}

/// 測るときの環境の 1 行（日時、ブランチ、SHA、負荷平均）。記録に必ず入れる。
pub fn environment_lines() -> Vec<String> {
    let git = record::git_state();
    let load = record::load_average();
    let mut lines = vec![
        format!("- 測定日時（UTC）: {}", record::now_utc()),
        format!(
            "- コミット: {} （{}。作業ツリーの変更: {}）",
            git["sha"].as_str().unwrap_or("不明"),
            git["branch"].as_str().unwrap_or("不明"),
            if git["dirty"].as_bool().unwrap_or(false) {
                "あり"
            } else {
                "なし"
            }
        ),
        format!(
            "- 負荷平均（1 分。測定の前）: {}",
            load.map_or("不明".to_owned(), |value| format!("{value:.2}"))
        ),
    ];
    if load.is_some_and(|value| value >= QUIET_LOAD_AVERAGE) {
        lines.push(format!(
            "- 注意: 負荷平均が {QUIET_LOAD_AVERAGE} 以上。ほかの処理と競っているので、時間の割合は目安"
        ));
    }
    lines
}

/// 報告に書くパス。リポジトリの中なら相対にする（ホームのパスとユーザー名を記録に残さない。区切りは `/` にそろえる）。
pub fn repo_relative(path: &Path) -> String {
    let relative = path.strip_prefix(repo_root()).unwrap_or(path);
    relative.to_string_lossy().replace('\\', "/")
}

/// 長い関数名（ジェネリクスの展開）を表に収める。
pub fn shorten(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let head: String = text.chars().take(limit.saturating_sub(1)).collect();
    format!("{head}…")
}

/// 報告を標準出力に出し、`out` があればファイルにも書く。
pub fn emit(report: &str, out: Option<&Path>) -> Result<(), XtaskError> {
    print!("{report}");
    if let Some(path) = out {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| err(format!("{} を作れません: {error}", parent.display())))?;
        }
        std::fs::write(path, report)
            .map_err(|error| err(format!("{} に書けません: {error}", path.display())))?;
        eprintln!("{} に書きました", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_bench_executable() {
        let stdout = r#"{"reason":"compiler-artifact","target":{"name":"other"},"executable":"/t/other"}
{"reason":"compiler-artifact","target":{"name":"shell_perf"},"executable":null}
not json
{"reason":"compiler-artifact","target":{"name":"shell_perf"},"executable":"/t/shell_perf-abc"}
{"reason":"build-finished","success":true}"#;
        assert_eq!(
            executable_from_cargo_json(stdout, "shell_perf"),
            Some(PathBuf::from("/t/shell_perf-abc"))
        );
        assert_eq!(executable_from_cargo_json(stdout, "missing"), None);
    }

    #[test]
    fn report_paths_are_relative_to_the_repo() {
        let inside = repo_root().join("target").join("x.json");
        assert_eq!(repo_relative(&inside), "target/x.json");
        assert_eq!(repo_relative(Path::new("/elsewhere/y")), "/elsewhere/y");
    }

    #[test]
    fn shortens_long_names() {
        assert_eq!(shorten("abc", 5), "abc");
        assert_eq!(shorten("abcdefgh", 5), "abcd…");
    }

    #[test]
    fn missing_tool_gives_install_hint() {
        let error = require_tool("ikari-no-such-tool", "cargo install x").unwrap_err();
        assert!(error.to_string().contains("cargo install x"));
    }
}
