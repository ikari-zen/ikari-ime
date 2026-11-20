//! macOS の `ikari-data`（App Sandbox の中。ADR 0079）だけの部品。
//!
//! - データの場所は `HOME` から決めない（§3.4。Sandbox は `HOME` をコンテナに差し替える）。`getpwuid_r` の `pw_dir` から決める
//! - `fetch/` と `inbox/` は自分で作る。根（データの場所そのもの）は作らず、あってリンクでないことだけ確かめる（§3.5）
//! - 親が終わったら取得を止めて終わる（§3.6。`lm/src/host/serve.rs` と同じ `getppid()` の変化）
//!
//! 判断の置き場所: すべて Rust。純粋な決め方は引数で受け、Sandbox の外でもテストする。

use std::ffi::{CStr, OsStr};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ikari_core::user::{HostOs, UserDir, UserDirEnv};

use crate::nofollow;

/// 親が生きているかを見る間隔。親が終わってから止まるまでの遅れの上限
pub const PARENT_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// 親が終わって止めるときの終了コード（`error` の行は出さない。読む側は既に居ない）
pub const ORPHAN_EXIT_CODE: i32 = 1;
/// launchd（孤児を引き取る親）
const INIT_PID: libc::pid_t = 1;
/// `getpwuid_r` の最初のバッファ（足りなければ倍にして読み直す）
const PASSWD_BUF_INITIAL: usize = 1024;
/// バッファの上限（超えたら諦める）
const PASSWD_BUF_MAX: usize = 1 << 20;

/// 本当のホーム（`getpwuid_r(getuid())` の `pw_dir`）。引けなければ `None`
pub fn passwd_home() -> Option<PathBuf> {
    let mut size = PASSWD_BUF_INITIAL;
    while size <= PASSWD_BUF_MAX {
        let mut buf = vec![0u8; size];
        // SAFETY: すべて 0 の値は `passwd` として正しい
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: `pwd` と `buf`（長さは `size`）は呼び出しの間生きていて書ける。`result` も書ける
        let rc = unsafe {
            libc::getpwuid_r(
                libc::getuid(),
                &mut pwd,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut result,
            )
        };
        if rc == libc::ERANGE {
            size *= 2;
            continue;
        }
        if rc != 0 || result.is_null() || pwd.pw_dir.is_null() {
            return None;
        }
        // SAFETY: `pw_dir` は成功した呼び出しが `buf` の中に作った、NUL で終わる文字列。`buf` はここでまだ生きている
        let dir = unsafe { CStr::from_ptr(pwd.pw_dir) };
        let bytes = dir.to_bytes();
        if bytes.is_empty() {
            return None;
        }
        return Some(PathBuf::from(OsStr::from_bytes(bytes)));
    }
    None
}

/// データの場所の決め方（純粋）。`env.home` を `passwd_home` で差し替えて、`core` の規則（`dev-knobs` の上書きを含む）に渡す。
/// `passwd_home` が無ければ決めない（`HOME` にはフォールバックしない。コンテナを指すため）
pub fn resolve_data_root(mut env: UserDirEnv, passwd_home: Option<PathBuf>) -> Option<UserDir> {
    env.home = Some(passwd_home?);
    UserDir::resolve_with(None, &env, HostOs::Macos)
}

/// 製品の場所
pub fn resolve_user_dir() -> Option<UserDir> {
    resolve_data_root(UserDirEnv::from_process(), passwd_home())
}

/// `root/name` を 1 段だけ作り（あれば何もしない）、リンクでないディレクトリであることを確かめる。
/// `root` 自体は作らない。`root` が無い・リンク・ディレクトリでない・`lstat` できないときは失敗（呼ぶ側が `internal` に写す。
/// 根が無いときの `mkdir` の EPERM も、そのまま別の種類にせず失敗として返す）
pub fn ensure_child_dir(root: &Path, name: &str) -> io::Result<()> {
    // 根は確かめるだけ（Sandbox の中から通るのは lstat だけ。ADR 0079 §8 の 2）
    nofollow::hold_dir(root)?;
    let child = root.join(name);
    match std::fs::create_dir(&child) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    nofollow::hold_dir(&child)?;
    Ok(())
}

/// 親が終わったか（純粋）。始めから launchd の下なら、起動の直後に親が終わっていた
pub fn is_orphaned(first_parent: libc::pid_t, current_parent: libc::pid_t) -> bool {
    first_parent == INIT_PID || current_parent != first_parent
}

fn parent_id() -> libc::pid_t {
    // SAFETY: 引数なしの呼び出し
    unsafe { libc::getppid() }
}

/// 親の終わりの見張りを始める。親が終わったら、取得を止めてプロセスを終わらせる（途中のファイルは次の
/// `cleanup` が片づける。錠は OS が放す）。`main` の最初に 1 回呼ぶ
pub fn watch_parent() {
    let first = parent_id();
    if is_orphaned(first, first) {
        std::process::exit(ORPHAN_EXIT_CODE);
    }
    let spawned = std::thread::Builder::new()
        .name("parent-watch".into())
        .spawn(move || {
            loop {
                std::thread::sleep(PARENT_POLL_INTERVAL);
                if is_orphaned(first, parent_id()) {
                    std::process::exit(ORPHAN_EXIT_CODE);
                }
            }
        });
    // 見張りを置けなければ、親なしで走り続けないよう終わる
    if spawned.is_err() {
        std::process::exit(ORPHAN_EXIT_CODE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn env_with_home(home: &str) -> UserDirEnv {
        UserDirEnv {
            home: Some(PathBuf::from(home)),
            ..UserDirEnv::default()
        }
    }

    #[test]
    fn data_root_comes_from_passwd_not_home() {
        let root = resolve_data_root(
            env_with_home("/container/Data"),
            Some(PathBuf::from("/real-home")),
        )
        .unwrap();
        assert_eq!(
            root.root(),
            Path::new("/real-home/Library/Application Support/ikari-ime")
        );
    }

    #[test]
    fn data_root_is_unknown_without_passwd_even_if_home_is_set() {
        assert!(resolve_data_root(env_with_home("/container/Data"), None).is_none());
    }

    #[test]
    fn dev_override_is_kept() {
        let env = UserDirEnv {
            ikari_user_dir: Some(PathBuf::from("/tmp/dev")),
            ..env_with_home("/container/Data")
        };
        let root = resolve_data_root(env, Some(PathBuf::from("/real-home"))).unwrap();
        assert_eq!(root.root(), Path::new("/tmp/dev"));
    }

    #[test]
    fn passwd_home_is_an_absolute_path() {
        let home = passwd_home().expect("passwd entry");
        assert!(home.is_absolute());
    }

    #[test]
    fn ensure_child_dir_makes_one_level_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        ensure_child_dir(tmp.path(), "inbox").unwrap();
        ensure_child_dir(tmp.path(), "inbox").unwrap();
        assert!(tmp.path().join("inbox").is_dir());
    }

    #[test]
    fn ensure_child_dir_does_not_create_the_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ikari-ime");
        assert!(ensure_child_dir(&root, "inbox").is_err());
        assert!(!root.exists());
    }

    #[test]
    fn ensure_child_dir_rejects_links_and_plain_files() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        fs::create_dir(&real).unwrap();
        // 根がリンク
        let linked_root = tmp.path().join("root-link");
        std::os::unix::fs::symlink(&real, &linked_root).unwrap();
        assert!(ensure_child_dir(&linked_root, "inbox").is_err());
        // 子がリンク
        let root = tmp.path().join("root");
        fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink(&real, root.join("inbox")).unwrap();
        assert!(ensure_child_dir(&root, "inbox").is_err());
        // 子がファイル
        fs::write(root.join("fetch"), b"x").unwrap();
        assert!(ensure_child_dir(&root, "fetch").is_err());
        // 根がファイル
        let file_root = tmp.path().join("file");
        fs::write(&file_root, b"x").unwrap();
        assert!(ensure_child_dir(&file_root, "inbox").is_err());
    }

    #[test]
    fn orphan_detection() {
        assert!(!is_orphaned(4242, 4242));
        assert!(is_orphaned(4242, INIT_PID));
        assert!(is_orphaned(4242, 777));
        // 始めから launchd の下
        assert!(is_orphaned(INIT_PID, INIT_PID));
    }
}
