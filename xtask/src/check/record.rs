//! check の結果の記録（`scripts/check-record.sh` の `check_record_write` と同じ置き場所・書式・挙動。issue #107）。
//! 記録はコミット SHA ごとに 1 ファイル。置き場所は git の共有ディレクトリ（リポジトリの外。worktree 間で共有され、コミットされない）。
//! 環境変数 `IKARI_CHECK_RECORD_DIR` で置き場所を変えられる（テスト用）。読む側は `.claude/hooks/guard-pr-merge.sh`。

use std::path::{Path, PathBuf};

use crate::data::run::{capture, git};

const RECORD_SUBDIR: &str = "ikari-check";
const RECORD_DIR_ENV: &str = "IKARI_CHECK_RECORD_DIR";

fn record_dir(repo: &Path) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(RECORD_DIR_ENV).filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let common =
        capture(git(repo).args(["rev-parse", "--path-format=absolute", "--git-common-dir"]))?;
    Some(PathBuf::from(common.trim_end_matches(['\n', '\r'])).join(RECORD_SUBDIR))
}

/// 記録を書く。未コミットの変更があるときは記録しない（結果が SHA の内容と一致しないため）。同じ SHA の古い記録は消す。
/// 書けなくても check の結果は変えない（黙って戻る）。
pub fn write(repo: &Path, code: i32, scope: &str) {
    let Some(dir) = record_dir(repo) else { return };
    let Some(sha) = capture(git(repo).args(["rev-parse", "HEAD"])) else {
        return;
    };
    let sha = sha.trim();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let file = dir.join(sha);
    let _ = std::fs::remove_file(&file);
    let dirty = capture(git(repo).args(["status", "--porcelain"])).is_none_or(|s| !s.is_empty());
    if dirty {
        return;
    }
    let _ = std::fs::write(&file, format!("CHECK_EXIT={code}\nSCOPE={scope}\n"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    struct Repo(PathBuf);

    fn git_in(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn writes_two_lines_only_for_clean_tree() {
        let dir = std::env::temp_dir().join(format!("xtask-record-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(dir.clone());
        let repo = Repo(dir);
        git_in(repo.0.as_path(), &["init", "-q"]);
        git_in(
            repo.0.as_path(),
            &["commit", "-q", "--allow-empty", "-m", "init"],
        );
        let sha = capture(git(repo.0.as_path()).args(["rev-parse", "HEAD"])).unwrap();
        let sha = sha.trim();
        let records = repo.0.as_path().join(".git").join(RECORD_SUBDIR);
        // 環境変数を使わず、既定の置き場所（git の共有ディレクトリ）を確かめる
        write(repo.0.as_path(), 3, "no-swift");
        let text = std::fs::read_to_string(records.join(sha)).unwrap();
        assert_eq!(text, "CHECK_EXIT=3\nSCOPE=no-swift\n");
        // 未コミットの変更があれば、古い記録も消えて書かれない
        std::fs::write(repo.0.as_path().join("x"), "x").unwrap();
        write(repo.0.as_path(), 0, "full");
        assert!(!records.join(sha).exists());
    }
}
