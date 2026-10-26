//! エンジンとの排他（ADR 0004 §7.4）。
//!
//! 常駐エンジンは `<UserDir>/engine.lock` を flock で持つ。学習ファイル（learning.tsv）を書き換える
//! コマンドは、ロックが取られていればエラーにする（エンジンのメモリ上の学習で上書きされるため）。
//! `--force` で強制できる。ユーザー辞書とプロファイルの編集は対象外（エンジンは読むだけ）。

use std::error::Error;
use std::fs::{File, TryLockError};

use ikari_core::user::UserDir;

fn held_message(dir: &UserDir) -> String {
    format!(
        "エンジンが動いています（{}）。学習ファイルを書き換えるので、エンジンを止めてから実行してください \
         （`killall IkariEngine`。強制するなら --force。エンジンのメモリ上の学習で上書きされることがあります）",
        dir.engine_lock().display()
    )
}

/// 学習ファイルを書き換える前に呼ぶ。エンジンが動いていて、`force` でなければエラー。
/// ロックのファイルがなければエンジンは動いていない。
pub fn ensure_engine_stopped(dir: &UserDir, force: bool) -> Result<(), Box<dyn Error>> {
    if force {
        return Ok(());
    }
    let Ok(file) = File::open(dir.engine_lock()) else {
        return Ok(());
    };
    match file.try_lock() {
        // 取れた = 誰も持っていない。すぐ放す（File を落とすと解放される）
        Ok(()) => Ok(()),
        Err(TryLockError::WouldBlock) => Err(held_message(dir).into()),
        // 調べられないときは止めない（ロックのためにコマンドを使えなくしない）
        Err(TryLockError::Error(_)) => Ok(()),
    }
}

/// `UserDir` が決まらないとき（HOME もない環境）は学習ファイルも書けないので、確認しない。
pub fn ensure_engine_stopped_at(
    user_dir: Option<&std::path::Path>,
    force: bool,
) -> Result<(), Box<dyn Error>> {
    match UserDir::resolve(user_dir) {
        Some(dir) => ensure_engine_stopped(&dir, force),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ikari-cli-lock-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn no_lock_file_means_no_engine() {
        let root = temp_dir("none");
        assert!(ensure_engine_stopped(&UserDir::new(&root), false).is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn held_lock_blocks_unless_forced() {
        let root = temp_dir("held");
        let engine = File::create(UserDir::new(&root).engine_lock()).unwrap();
        engine.try_lock().unwrap();
        let dir = UserDir::new(&root);
        let message = ensure_engine_stopped(&dir, false).unwrap_err().to_string();
        assert!(message.contains("エンジンが動いています"), "{message}");
        assert!(message.contains("--force"));
        assert!(ensure_engine_stopped(&dir, true).is_ok());
        drop(engine);
        assert!(ensure_engine_stopped(&dir, false).is_ok());
        let _ = std::fs::remove_dir_all(root);
    }
}
