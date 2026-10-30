//! `engine.lock`: エンジンが動いている間、flock で持つ（ADR 0004 §7.4）。
//!
//! CLI の学習ファイルを書き換えるコマンドは、同じファイルのロックを試して、取られていればエラーにする。

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use ikari_core::user::UserDir;

#[derive(Debug)]
pub enum LockError {
    /// 別のエンジンが持っている
    Held,
    Io(String),
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockError::Held => write!(f, "another engine holds the data directory lock"),
            LockError::Io(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for LockError {}

/// 持っている間だけロックが有効。落とすと（プロセスが死んでも）解放される。
#[derive(Debug)]
pub struct EngineLock {
    _file: File,
}

impl EngineLock {
    pub fn acquire(data_dir: &Path) -> Result<EngineLock, LockError> {
        let path = UserDir::new(data_dir).engine_lock();
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .mode(ikari_core::user::PRIVATE_FILE_MODE)
            .write(true)
            .open(&path)
            .map_err(|e| LockError::Io(format!("{}: {e}", path.display())))?;
        match file.try_lock() {
            Ok(()) => Ok(EngineLock { _file: file }),
            Err(TryLockError::WouldBlock) => Err(LockError::Held),
            Err(TryLockError::Error(e)) => Err(LockError::Io(format!("{}: {e}", path.display()))),
        }
    }
}

/// テスト用: 解放したはずのロックが、一瞬だけ「使用中」に見えることがあるので、少しだけ待ち直す。
///
/// 同じプロセスの別のスレッドが外部コマンド（`ps` など）を起動すると、子プロセスが exec するまでの
/// 間、親の開いているファイル（他のテストの `engine.lock` を含む）を引き継ぎ、flock の解放が遅れる
/// （#77）。実機の排他の仕様は変えず、テストの側で、解放を期待する箇所だけ待ち直す。
/// 本当に持たれているときは、`LOCK_RETRY_LIMIT` が過ぎたら最後の結果を返す。
#[cfg(test)]
pub(crate) fn retry_while_held<T, E: fmt::Display>(
    mut f: impl FnMut() -> Result<T, E>,
) -> Result<T, E> {
    const LOCK_RETRY_LIMIT: std::time::Duration = std::time::Duration::from_secs(5);
    const LOCK_RETRY_STEP: std::time::Duration = std::time::Duration::from_millis(10);
    let started = std::time::Instant::now();
    loop {
        match f() {
            Err(e)
                if e.to_string().contains(&LockError::Held.to_string())
                    && started.elapsed() < LOCK_RETRY_LIMIT =>
            {
                std::thread::sleep(LOCK_RETRY_STEP);
            }
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ikari-engine-lock-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lock_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("mode");
        let _lock = EngineLock::acquire(&dir).unwrap();
        let path = UserDir::new(&dir).engine_lock();
        let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn second_lock_fails_until_the_first_is_dropped() {
        let dir = temp_dir("exclusive");
        let first = EngineLock::acquire(&dir).unwrap();
        assert!(matches!(EngineLock::acquire(&dir), Err(LockError::Held)));
        drop(first);
        assert!(retry_while_held(|| EngineLock::acquire(&dir)).is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_directory_is_an_io_error() {
        // 普通のファイルの下は、root でも作れない（/nonexistent は root だと作れてしまう）
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(matches!(
            EngineLock::acquire(&file.path().join("ikari-dir")),
            Err(LockError::Io(_))
        ));
    }
}
