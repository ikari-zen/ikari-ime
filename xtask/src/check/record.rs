//! check の結果の記録（`scripts/check-record.sh` の `check_record_write` と同じ置き場所・書式・挙動。issue #107）。
//! 記録はコミット SHA ごとに 1 ファイル。置き場所は git の共有ディレクトリ（リポジトリの外。worktree 間で共有され、コミットされない）。
//! 環境変数 `IKARI_CHECK_RECORD_DIR` で置き場所を変えられる（テスト用）。読む側は `.claude/hooks/guard-pr-merge.sh`。

use std::path::{Path, PathBuf};

use crate::data::run::{capture, git, is_jj_workspace, jj};

const RECORD_SUBDIR: &str = "ikari-check";
const RECORD_DIR_ENV: &str = "IKARI_CHECK_RECORD_DIR";
/// jj の `@` が空か、と親のコミット ID（git の SHA）を `<true|false> <SHA,SHA…>` で出すテンプレート。
const JJ_INFO_TEMPLATE: &str =
    r#"empty ++ " " ++ parents.map(|p| p.commit_id()).join(",") ++ "\n""#;
/// jj のルートコミット（親にはできない）。
const JJ_ROOT_COMMIT: &str = "0000000000000000000000000000000000000000";

fn record_dir(repo: &Path) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(RECORD_DIR_ENV).filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    if let Some(common) =
        capture(git(repo).args(["rev-parse", "--path-format=absolute", "--git-common-dir"]))
    {
        return Some(PathBuf::from(common.trim_end_matches(['\n', '\r'])).join(RECORD_SUBDIR));
    }
    // git が使えない jj の workspace（共存の形でない）: jj の裏にある git のディレクトリ
    if is_jj_workspace(repo) {
        let root = capture(jj(repo).args(["git", "root"]))?;
        return Some(PathBuf::from(root.trim_end_matches(['\n', '\r'])).join(RECORD_SUBDIR));
    }
    None
}

/// 記録の対象にするコミットの SHA と、作業コピーに未コミットの変更があるか。
/// git は `HEAD` と `git status`。jj は `@` の親（PR に載るのは `@-`）と、`@` が空でないこと。
fn target_commit(repo: &Path) -> Option<(String, bool)> {
    if is_jj_workspace(repo) {
        let info =
            capture(jj(repo).args(["log", "--no-graph", "-r", "@", "-T", JJ_INFO_TEMPLATE]))?;
        let (empty, parents) = info.trim().split_once(' ')?;
        if parents.is_empty() || parents.contains(',') || parents == JJ_ROOT_COMMIT {
            return None;
        }
        return Some((parents.to_owned(), empty != "true"));
    }
    let sha = capture(git(repo).args(["rev-parse", "HEAD"]))?;
    let dirty = capture(git(repo).args(["status", "--porcelain"])).is_none_or(|s| !s.is_empty());
    Some((sha.trim().to_owned(), dirty))
}

/// 記録を書く。未コミットの変更があるときは記録しない（結果が SHA の内容と一致しないため）。jj の作業コピーでは `@` の親の SHA に記録し、`@` に変更があれば記録しない。同じ SHA の古い記録は消す。
/// 書けなくても check の結果は変えない（黙って戻る）。
pub fn write(repo: &Path, code: i32, scope: &str) {
    let Some(dir) = record_dir(repo) else { return };
    let Some((sha, dirty)) = target_commit(repo) else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let file = dir.join(&sha);
    let _ = std::fs::remove_file(&file);
    if dirty {
        if is_jj_workspace(repo) {
            eprintln!(
                "check の記録: jj の @ に変更があるので記録しません。jj commit などで @ を空にして（PR に載るのは @-）、もう一度 check を流してください"
            );
        }
        return;
    }
    let _ = std::fs::write(&file, format!("CHECK_EXIT={code}\nSCOPE={scope}\n"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::run::succeeds_quietly;
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

    /// jj の共存の形では、`@` の親の SHA に記録し、`@` に変更があれば記録しない。jj がない環境では飛ばす。
    #[test]
    fn jj_colocated_records_parent_of_working_copy_only_when_empty() {
        if !succeeds_quietly(Command::new("jj").arg("--version")) {
            eprintln!("jj がないので飛ばします");
            return;
        }
        let dir = std::env::temp_dir().join(format!("xtask-record-jj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(dir.clone());
        let jj_in = |args: &[&str]| {
            let status = Command::new("jj")
                .current_dir(&dir)
                .args([
                    "--config",
                    "user.name=t",
                    "--config",
                    "user.email=t@example.com",
                ])
                .args(args)
                .status()
                .unwrap();
            assert!(status.success());
        };
        git_in(&dir, &["init", "-q"]);
        git_in(&dir, &["commit", "-q", "--allow-empty", "-m", "init"]);
        jj_in(&["git", "init", "--colocate"]);
        let records = dir.join(".git").join(RECORD_SUBDIR);
        let parent = |rev: &str| {
            let out = capture(jj(&dir).args(["log", "--no-graph", "-r", rev, "-T", "commit_id"]));
            out.unwrap().trim().to_owned()
        };
        // @ が空なら、@ の親（git の HEAD と同じ）に記録する
        let first = parent("@-");
        write(&dir, 0, "full");
        assert_eq!(
            std::fs::read_to_string(records.join(&first)).unwrap(),
            "CHECK_EXIT=0\nSCOPE=full\n"
        );
        // @ に変更があれば、古い記録も消えて書かれない
        std::fs::write(dir.join("x"), "x").unwrap();
        write(&dir, 0, "full");
        assert!(!records.join(&first).exists());
        // jj commit で確定すると、新しい @- に記録される
        jj_in(&["commit", "-m", "x"]);
        let second = parent("@-");
        assert_ne!(first, second);
        write(&dir, 0, "full");
        assert!(records.join(&second).exists());
    }
}
