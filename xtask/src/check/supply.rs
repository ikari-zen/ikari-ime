//! 配る物の供給の検査（ADR 0069 §4.1、ADR 0066 §6.2 の層 1、ADR 0076 §9）。どちらもファイルと `cargo tree` を読むだけで、
//! 道具もネットワークも要らない。
//!   - 配る対象（Windows、macOS）の依存に、通信の手段（HTTP・TLS・ソケットの crate）が無いこと。
//!     例外は取得する部品 `ikari-data`（ADR 0076）の下だけ（`ONLY_UNDER_DATA`）。
//!   - `.github/workflows` の Action が、すべてコミットの SHA で固定されていること

use std::collections::BTreeSet;
use std::path::Path;

use crate::data::run::{Failure, capture, cargo, log};

const WORKFLOW_DIR: &str = ".github/workflows";
const SHA_LEN: usize = 40;

/// 配る対象（`deny.toml` の `[graph] targets` と同じ）。`Cargo.lock` は対象の OS を区別せず、`native-tls` の Linux 向けの
/// 依存（`openssl` など）も載るので、対象ごとの `cargo tree --target` で見る（ADR 0076 §9）
const DIST_TARGETS: &[&str] = &["aarch64-apple-darwin", "x86_64-pc-windows-msvc"];

/// 取得する部品の crate。通信の手段は、この crate の下にしか置けない
const DATA_CRATE: &str = "ikari-data";

/// 製品に入れない crate（HTTP・TLS・ソケット・非同期の実行環境、根の証明書の埋め込み、暗号の実装）。
/// `deny.toml` の `bans.deny` と同じ一覧にする。
const NET_CRATES: &[&str] = &[
    "reqwest",
    "ureq",
    "hyper",
    "hyper-util",
    "h2",
    "isahc",
    "attohttpc",
    "surf",
    "curl",
    "curl-sys",
    "tiny_http",
    "rustls",
    "native-tls",
    "openssl",
    "openssl-sys",
    "schannel",
    "security-framework",
    "webpki-roots",
    "webpki-root-certs",
    "ring",
    "tokio",
    "mio",
    "socket2",
    "async-std",
    "smol",
    "tungstenite",
    "tokio-tungstenite",
    "quinn",
    "hickory-resolver",
    "trust-dns-resolver",
    "axum",
    "warp",
    "actix-web",
];
/// 上の一覧のうち、`ikari-data` の下（このワークスペースの crate で、これに依存するのが `ikari-data` だけ）なら許すもの。
/// OS の TLS への橋（ADR 0076 §9）。`deny.toml` の `wrappers` と同じ
const ONLY_UNDER_DATA: &[&str] = &["ureq", "native-tls", "schannel", "security-framework"];

/// `cargo tree --prefix none --format "{p}"` の出力にある crate の名前
fn tree_names(out: &str) -> BTreeSet<&str> {
    out.lines()
        .filter_map(|l| l.split_whitespace().next())
        .collect()
}

/// `cargo tree --prefix depth --invert …` の出力で、このワークスペースの crate（パス依存。`name v0.1.0 (/path)`）の名前。
/// 深さ 0（逆向きの起点の crate 自身）は数えない
fn member_dependents(inverted: &str) -> BTreeSet<String> {
    inverted
        .lines()
        .filter_map(|l| {
            let rest = l.trim_start_matches(|c: char| c.is_ascii_digit());
            let depth: &str = &l[..l.len() - rest.len()];
            if depth == "0" {
                return None;
            }
            let (name, source) = rest.split_once(' ')?;
            // `(*)`（重複の印）と `(proc-macro)` はパス依存ではない
            let is_path = source
                .split_once('(')
                .is_some_and(|(_, tail)| !tail.starts_with('*') && !tail.starts_with("proc-macro"));
            is_path.then(|| name.to_owned())
        })
        .collect()
}

/// 1 つの対象の依存の木を調べて、問題の文を返す
fn check_target(root: &Path, target: &str) -> Result<Vec<String>, Failure> {
    let tree = |extra: &[&str]| {
        let mut cmd = cargo(root);
        cmd.args([
            "tree",
            "--locked",
            "--workspace",
            "-e",
            "normal,build",
            "--target",
            target,
        ]);
        cmd.args(extra);
        capture(&mut cmd)
            .ok_or_else(|| Failure::new(format!("cargo tree（{target}）に失敗しました")))
    };
    let all = tree(&["--prefix", "none", "--format", "{p}"])?;
    let present: Vec<&str> = NET_CRATES
        .iter()
        .copied()
        .filter(|n| tree_names(&all).contains(n))
        .collect();
    let mut problems = Vec::new();
    for name in &present {
        if !ONLY_UNDER_DATA.contains(name) {
            problems.push(format!("{target}: {name}（どこにも置けない）"));
        }
    }
    let allowed: Vec<&str> = present
        .iter()
        .copied()
        .filter(|n| ONLY_UNDER_DATA.contains(n))
        .collect();
    if !allowed.is_empty() {
        let mut args = vec!["--prefix", "depth"];
        for name in &allowed {
            args.push("-i");
            args.push(name);
        }
        let inverted = tree(&args)?;
        for member in member_dependents(&inverted) {
            if member != DATA_CRATE {
                problems.push(format!(
                    "{target}: {member} が通信の手段（{}）に依存している（{DATA_CRATE} の下にだけ置ける）",
                    allowed.join(", ")
                ));
            }
        }
    }
    Ok(problems)
}

/// 配る対象の依存に、通信の手段の crate が `ikari-data` の下以外に無いこと（ADR 0066 §6.2 の L1、ADR 0076 §9）
pub fn net_crates(root: &Path) -> Result<(), Failure> {
    log(
        "依存の通信の手段の crate が ikari-data の下だけにあること（cargo tree。ADR 0066 §6.2、0076 §9）",
    );
    let mut problems = Vec::new();
    for target in DIST_TARGETS {
        problems.extend(check_target(root, target)?);
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(Failure::new(format!(
        "通信の手段の crate が許した場所の外にあります:\n  {}",
        problems.join("\n  ")
    )))
}

/// workflow の 1 行が `uses:` なら、その参照（`owner/repo@ref`）を返す。コメントと `with:` の中は見ない。
fn uses_reference(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let body = trimmed.strip_prefix("- ").unwrap_or(trimmed);
    let value = body.strip_prefix("uses:")?.trim();
    // 行末のコメントを外す。引用符も外す
    let value = value.split(" #").next().unwrap_or(value).trim();
    Some(value.trim_matches(|c| c == '"' || c == '\''))
}

/// 参照が、コミットの SHA で固定されているか。リポジトリの中の Action（`./`）は対象外。
fn is_pinned(reference: &str) -> bool {
    if reference.starts_with("./") {
        return true;
    }
    match reference.split_once('@') {
        Some((_, rev)) => rev.len() == SHA_LEN && rev.bytes().all(|b| b.is_ascii_hexdigit()),
        None => false,
    }
}

fn unpinned_in(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(uses_reference)
        .filter(|r| !is_pinned(r))
        .map(str::to_string)
        .collect()
}

/// workflow の Action が、すべてコミットの SHA で固定されていること（ADR 0069 §4.1）
pub fn actions_pinned(root: &Path) -> Result<(), Failure> {
    log("workflow の Action が SHA で固定されていること（ADR 0069 §4.1）");
    let dir = root.join(WORKFLOW_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(());
    };
    let mut bad = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_yaml = path.extension().is_some_and(|e| e == "yml" || e == "yaml");
        if !is_yaml {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| Failure::new(format!("{} を読めません: {e}", path.display())))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        bad.extend(
            unpinned_in(&text)
                .into_iter()
                .map(|r| format!("{name}: {r}")),
        );
    }
    if bad.is_empty() {
        return Ok(());
    }
    bad.sort();
    Err(Failure::new(format!(
        "SHA で固定されていない Action があります（40 桁のコミットの SHA にする）:\n  {}",
        bad.join("\n  ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_names_take_the_first_word_of_each_line() {
        let out = "ikari-data v0.1.0 (/x/fetch)\nureq v3.5.2\nnative-tls v0.2.18\n";
        assert_eq!(
            tree_names(out).into_iter().collect::<Vec<_>>(),
            ["ikari-data", "native-tls", "ureq"]
        );
    }

    #[test]
    fn only_workspace_members_count_as_dependents() {
        let inverted = "0ureq v3.5.2\n1ikari-data v0.1.0 (/x/fetch)\n2other v0.1.0 (/x/o)\n1http v1.0.0\n1dup v1.0.0 (*)\n1macro v1.0.0 (proc-macro) (/x/m)\n";
        let members: Vec<String> = member_dependents(inverted).into_iter().collect();
        assert_eq!(members, ["ikari-data", "other"]);
    }

    #[test]
    fn the_starting_crate_itself_is_not_a_dependent() {
        // 起点がパス依存でも（深さ 0）、依存元には数えない
        assert!(member_dependents("0ikari-data v0.1.0 (/x/fetch)\n").is_empty());
    }

    #[test]
    fn allowed_set_is_inside_the_banned_list() {
        for name in ONLY_UNDER_DATA {
            assert!(NET_CRATES.contains(name), "{name}");
        }
        for name in [
            "ring",
            "webpki-roots",
            "webpki-root-certs",
            "rustls",
            "openssl",
            "openssl-sys",
        ] {
            assert!(
                NET_CRATES.contains(&name) && !ONLY_UNDER_DATA.contains(&name),
                "{name}"
            );
        }
    }

    /// 本物のワークスペースで通る（`cargo tree` が使える環境）
    #[test]
    fn the_real_workspace_passes() {
        let root = crate::data::run::repo_root();
        net_crates(&root).unwrap_or_else(|e| panic!("{}", e.message));
    }

    #[test]
    fn pinned_and_unpinned_actions() {
        let sha = "3d3c42e5aac5ba805825da76410c181273ba90b1";
        let text = format!(
            "steps:\n  - uses: actions/checkout@{sha}\n  # uses: a/b@v1\n  - uses: x/y@{sha} # v2\n  - uses: ./local\n  - uses: a/b@v4\n  - uses: c/d@main\n  - uses: e/f\n    with:\n      name: uses: not-an-action\n"
        );
        assert_eq!(unpinned_in(&text), ["a/b@v4", "c/d@main", "e/f"]);
    }

    #[test]
    fn short_sha_is_not_pinned() {
        assert!(!is_pinned("a/b@3d3c42e"));
        assert!(!is_pinned("a/b@zzzc42e5aac5ba805825da76410c181273ba90b1"));
    }
}
