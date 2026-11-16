//! 配る物の供給の検査（ADR 0069 §4.1、ADR 0066 §6.2 の層 1）。どちらもファイルを読むだけで、道具もネットワークも要らない。
//!   - `Cargo.lock` に、通信の手段（HTTP・TLS・ソケットの crate）が無いこと
//!   - `.github/workflows` の Action が、すべてコミットの SHA で固定されていること

use std::path::Path;

use crate::data::run::{Failure, log};

const LOCK_FILE: &str = "Cargo.lock";
const WORKFLOW_DIR: &str = ".github/workflows";
const SHA_LEN: usize = 40;

/// 製品に入れない crate（HTTP・TLS・ソケット・非同期の実行環境）。`deny.toml` の `bans.deny` と同じ一覧にする。
/// 取得する部品（`ikari-data`。ADR 0071）を足すときは、別の実行ファイルにして、`NET_CRATES_ALLOWED` に名前で足す。
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
/// 上の一覧のうち、例外として許す crate の名前。今は無い（取得する部品は未実装）。
const NET_CRATES_ALLOWED: &[&str] = &[];

/// `Cargo.lock` の `name = "…"`（`[[package]]` の名前）を並べる。
fn lock_package_names(lock: &str) -> Vec<&str> {
    lock.lines()
        .filter_map(|l| l.strip_prefix("name = \""))
        .filter_map(|l| l.strip_suffix('"'))
        .collect()
}

fn forbidden_in_lock(lock: &str) -> Vec<&str> {
    let mut found: Vec<&str> = lock_package_names(lock)
        .into_iter()
        .filter(|n| NET_CRATES.contains(n) && !NET_CRATES_ALLOWED.contains(n))
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

/// 依存に、通信の手段の crate が無いこと（ADR 0066 §6.2 の L1）
pub fn net_crates(root: &Path) -> Result<(), Failure> {
    log("依存に通信の手段の crate が無いこと（Cargo.lock。ADR 0066 §6.2）");
    let lock = std::fs::read_to_string(root.join(LOCK_FILE))
        .map_err(|e| Failure::new(format!("{LOCK_FILE} を読めません: {e}")))?;
    let found = forbidden_in_lock(&lock);
    if found.is_empty() {
        return Ok(());
    }
    Err(Failure::new(format!(
        "{LOCK_FILE} に通信の手段の crate があります: {}（取得する部品だけに許す。ADR 0066 §6.2）",
        found.join(", ")
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
    fn lock_with_clean_deps_passes() {
        let lock = "[[package]]\nname = \"serde\"\n\n[[package]]\nname = \"windows-sys\"\n";
        assert!(forbidden_in_lock(lock).is_empty());
    }

    #[test]
    fn lock_with_net_crate_is_found_once() {
        let lock = "[[package]]\nname = \"tokio\"\n[[package]]\nname = \"tokio\"\n[[package]]\nname = \"ureq\"\n";
        assert_eq!(forbidden_in_lock(lock), ["tokio", "ureq"]);
    }

    #[test]
    fn similar_names_are_not_flagged() {
        // 部分一致で誤検出しない（名前が完全に一致したものだけ）
        let lock = "[[package]]\nname = \"tokio-like\"\n[[package]]\nname = \"ikari-socket2\"\n";
        assert!(forbidden_in_lock(lock).is_empty());
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
