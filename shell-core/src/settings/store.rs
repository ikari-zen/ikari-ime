//! `settings.tsv` の読み書き（今の `SettingsStore`。ADR 0005 §7.1、§7.2）。
//!
//! **ファイルを書くのは殻だけ**で、メニューの操作も設定アプリの `UpdateSettings` も、ここを通る。書き手を 1 つにすると、
//! 同時の書き込みの競合と、反映の通知の漏れがなくなる。手で編集してもよい（`reload_if_file_changed` が mtime と
//! 大きさで気づく）。スレッドの安全は呼び出す側の責任（`&mut self`。必要なら `Mutex` で包む）。
//! 現在の版（`version`）は設定が変わるたびに増える。キー割り当ての読み直しは呼び出す側が `SettingsChange` を見て行う。

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use super::{SettingsChanges, SettingsEntry, SettingsError, rewrite, validate};
use crate::types::{SettingsIssue, SettingsSnapshot};

/// ファイルの変わった目印（mtime と大きさ。stat 1 回で取れる）
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    modified: SystemTime,
    size: u64,
}

/// 設定が変わった（自分で書いたとき、手の編集に気づいたとき）。呼び出す側が、ロックの外で通知に使う
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsChange {
    pub old: SettingsSnapshot,
    pub new: SettingsSnapshot,
}

/// 書き込みの誤り。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// 書いてはいけない（未知のキー、不正な値、行を混ぜるおそれのある文字、警告が増える）。ファイルは変えていない
    Rejected {
        issues: Vec<SettingsIssue>,
    },
    WriteFailed,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected { issues } => write!(f, "settings rejected ({} issue(s))", issues.len()),
            Self::WriteFailed => f.write_str("settings write failed"),
        }
    }
}

impl std::error::Error for StoreError {}

/// 読み直しの結果
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reload {
    /// 設定が変わったときだけ
    pub change: Option<SettingsChange>,
    /// 読み直したときの警告（読み直さなかったときは空）。ログには件数だけを書く
    pub issues: Vec<SettingsIssue>,
}

/// 書き込みの結果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Updated {
    /// 書いたあとの今の設定
    pub snapshot: SettingsSnapshot,
    pub version: u64,
    /// 設定が変わったときだけ
    pub change: Option<SettingsChange>,
}

/// 殻の今の設定と、ファイルの置き場所。
#[derive(Debug)]
pub struct SettingsStore {
    path: PathBuf,
    /// 殻の各所へ複製せずに渡せるよう `Arc` で持つ
    current: Arc<SettingsSnapshot>,
    version: u64,
    stamp: Option<FileStamp>,
}

impl SettingsStore {
    /// ファイルを読んで始める。ファイルがない、読めないときは既定値。警告も一緒に返す（ログには件数だけを書く）
    pub fn open(path: impl Into<PathBuf>) -> (Self, Vec<SettingsIssue>) {
        let path = path.into();
        let parsed = parse_file(&path);
        let store = Self {
            current: Arc::new(parsed.snapshot),
            version: 0,
            stamp: stamp_of(&path),
            path,
        };
        (store, parsed.issues)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn snapshot(&self) -> &SettingsSnapshot {
        &self.current
    }

    /// 今の設定の共有。複製しない（`version` が変わるまで同じもの）
    pub fn shared(&self) -> Arc<SettingsSnapshot> {
        Arc::clone(&self.current)
    }

    /// `snapshot` が変わるたびに増える（コントローラが適用済みの版と比べる）
    pub fn version(&self) -> u64 {
        self.version
    }

    /// ファイルの mtime か大きさが変わっていれば読み直す（`activateServer` で呼ぶ。stat 1 回なので打鍵には影響しない）
    pub fn reload_if_file_changed(&mut self) -> Reload {
        let latest = stamp_of(&self.path);
        if latest == self.stamp {
            return Reload::default();
        }
        self.stamp = latest;
        let parsed = parse_file(&self.path);
        let change = self.adopt(parsed.snapshot);
        Reload {
            change,
            issues: parsed.issues,
        }
    }

    /// `set` の値を書き、`reset` のキーの行を消す（既定に戻す）。行を保ったまま値だけを差し替える。
    /// 書く前に、キーと値を検査し、結果の警告が増えていないことを確かめる（増えたら書かない。`rewrite`）。
    /// 一時ファイルに書いて rename する。権限は 0600（Unix）。
    pub fn update(
        &mut self,
        set: Vec<SettingsEntry>,
        reset: Vec<String>,
    ) -> Result<Updated, StoreError> {
        let text = read_text(&self.path).unwrap_or_default();
        let updated = match rewrite(&text, &SettingsChanges { set, reset }) {
            Ok(updated) => updated,
            Err(SettingsError::IssuesAdded { issues }) => {
                return Err(StoreError::Rejected { issues });
            }
            Err(SettingsError::Unimplemented) => return Err(StoreError::WriteFailed),
        };
        if updated != text {
            write_atomically(&self.path, &updated).map_err(|_| StoreError::WriteFailed)?;
            self.stamp = stamp_of(&self.path);
        }
        // `validate` は失敗しない。失敗したら既定値に戻さず、今の設定のままにする
        let snapshot = validate(&updated)
            .map(|result| result.snapshot)
            .unwrap_or_else(|_| (*self.current).clone());
        let change = self.adopt(snapshot);
        Ok(Updated {
            snapshot: (*self.current).clone(),
            version: self.version,
            change,
        })
    }

    /// 設定が違えば、版を進めて変化を返す
    fn adopt(&mut self, snapshot: SettingsSnapshot) -> Option<SettingsChange> {
        if snapshot == *self.current {
            return None;
        }
        let old = std::mem::replace(&mut self.current, Arc::new(snapshot));
        self.version += 1;
        Some(SettingsChange {
            old: Arc::unwrap_or_clone(old),
            new: (*self.current).clone(),
        })
    }
}

fn read_text(path: &Path) -> Option<String> {
    // BOM つき（Windows のメモ帳）でも、最初のキーを読めるようにする（ADR 0040 §2.9）
    ikari_textfile::read_to_string(path).ok()
}

fn parse_file(path: &Path) -> super::SettingsValidation {
    // `validate` は失敗しない。万一失敗しても既定値で続ける（入力を止めない）
    validate(&read_text(path).unwrap_or_default()).unwrap_or_else(|_| super::SettingsValidation {
        snapshot: super::default_snapshot(),
        issues: Vec::new(),
    })
}

fn stamp_of(path: &Path) -> Option<FileStamp> {
    let metadata = fs::metadata(path).ok()?;
    Some(FileStamp {
        modified: metadata.modified().ok()?,
        size: metadata.len(),
    })
}

/// 同じ場所の一時ファイルに書いて rename する（途中の状態を読ませない）
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let mut temp_name = path.file_name().unwrap_or_default().to_os_string();
    temp_name.push(format!(".tmp-{}", std::process::id()));
    let temp = path.with_file_name(temp_name);
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::default_snapshot;

    /// テストごとの一時ディレクトリ（終わったら消す）
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "ikari-settings-test-{}-{}-{name}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos())
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join("settings.tsv")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn entry(key: &str, value: &str) -> SettingsEntry {
        SettingsEntry {
            key: key.into(),
            value: value.into(),
        }
    }

    #[test]
    fn a_missing_file_gives_defaults() {
        let dir = TempDir::new("missing");
        let (store, issues) = SettingsStore::open(dir.file());
        assert!(issues.is_empty());
        assert_eq!(store.snapshot(), &default_snapshot());
        assert_eq!(store.version(), 0);
    }

    #[test]
    fn update_creates_the_file_keeps_lines_and_bumps_the_version() {
        let dir = TempDir::new("update");
        let nested = dir.0.join("sub").join("settings.tsv");
        let (mut store, _) = SettingsStore::open(&nested);
        let done = store
            .update(vec![entry("paging.size", "5")], vec![])
            .unwrap();
        assert_eq!(done.snapshot.page_size, 5);
        assert_eq!(done.version, 1);
        let change = done.change.unwrap();
        assert_eq!(change.old.page_size, 3);
        assert_eq!(change.new.page_size, 5);
        assert_eq!(fs::read_to_string(&nested).unwrap(), "paging.size\t5\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&nested).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        // 同じ値を書いても、版は進まず、変化も返さない
        let again = store
            .update(vec![entry("paging.size", "5")], vec![])
            .unwrap();
        assert!(again.change.is_none());
        assert_eq!(again.version, 1);
        // 自分で書いたものは「手の編集」として読み直さない
        assert_eq!(store.reload_if_file_changed(), Reload::default());
    }

    #[test]
    fn update_resets_to_the_default_by_removing_the_line() {
        let dir = TempDir::new("reset");
        fs::write(dir.file(), "# keep\npaging.size\t5\n").unwrap();
        let (mut store, issues) = SettingsStore::open(dir.file());
        assert!(issues.is_empty());
        assert_eq!(store.snapshot().page_size, 5);
        let done = store.update(vec![], vec!["paging.size".into()]).unwrap();
        assert_eq!(done.snapshot.page_size, 3);
        assert_eq!(fs::read_to_string(dir.file()).unwrap(), "# keep\n");
    }

    #[test]
    fn open_reads_lf_crlf_and_bom_files_and_update_keeps_the_newline_style() {
        use ikari_textfile::testing::{Variant, read_variants, variant};
        let original = "# notes\ninput.method\tkana\npaging.size\t5\n";
        for (v, text) in read_variants(original) {
            let dir = TempDir::new("variants");
            fs::write(dir.file(), &text).unwrap();
            let (mut store, issues) = SettingsStore::open(dir.file());
            assert!(issues.is_empty(), "{v:?}: {issues:?}");
            assert_eq!(store.snapshot().page_size, 5, "{v:?}");
            assert_eq!(
                store.snapshot().input_method,
                crate::types::InputMethodKind::Kana,
                "{v:?}"
            );
            store
                .update(vec![entry("paging.size", "7")], vec![])
                .unwrap();
            let written = fs::read_to_string(dir.file()).unwrap();
            let expected = match v {
                Variant::Crlf => variant(&original.replace('5', "7"), Variant::Crlf),
                _ => original.replace('5', "7"),
            };
            assert_eq!(written, expected, "{v:?}");
        }
    }

    #[test]
    fn update_rejects_bad_input_without_touching_the_file() {
        let dir = TempDir::new("reject");
        fs::write(dir.file(), "paging.size\t5\n").unwrap();
        let (mut store, _) = SettingsStore::open(dir.file());
        for (set, reset) in [
            (vec![entry("paging.size", "10")], vec![]),
            (vec![entry("no.such", "1")], vec![]),
            (vec![entry("paging.wrap", "true\nengine.lm\toff")], vec![]),
            (vec![], vec!["no.such".to_string()]),
        ] {
            assert!(matches!(
                store.update(set, reset),
                Err(StoreError::Rejected { .. })
            ));
        }
        assert_eq!(fs::read_to_string(dir.file()).unwrap(), "paging.size\t5\n");
        assert_eq!(store.version(), 0);
    }

    #[test]
    fn reload_notices_a_hand_edit_and_reports_issues() {
        let dir = TempDir::new("reload");
        fs::write(dir.file(), "paging.size\t5\n").unwrap();
        let (mut store, _) = SettingsStore::open(dir.file());
        assert_eq!(store.reload_if_file_changed(), Reload::default());

        // 大きさが変われば mtime が同じでも気づく
        fs::write(dir.file(), "paging.size\t7\nbogus\n").unwrap();
        let reloaded = store.reload_if_file_changed();
        let change = reloaded.change.unwrap();
        assert_eq!(change.old.page_size, 5);
        assert_eq!(change.new.page_size, 7);
        assert_eq!(reloaded.issues.len(), 1);
        assert_eq!(store.version(), 1);
        // もう一度呼んでも変わらない
        assert_eq!(store.reload_if_file_changed(), Reload::default());
    }

    #[test]
    fn deleting_the_file_goes_back_to_defaults() {
        let dir = TempDir::new("delete");
        fs::write(dir.file(), "paging.size\t5\n").unwrap();
        let (mut store, _) = SettingsStore::open(dir.file());
        fs::remove_file(dir.file()).unwrap();
        let reloaded = store.reload_if_file_changed();
        assert_eq!(reloaded.change.unwrap().new, default_snapshot());
    }

    #[test]
    fn a_write_failure_is_reported_and_keeps_the_state() {
        let dir = TempDir::new("failure");
        // 親がファイルなので、ディレクトリを作れない
        fs::write(dir.0.join("blocker"), "x").unwrap();
        let (mut store, _) = SettingsStore::open(dir.0.join("blocker").join("settings.tsv"));
        assert_eq!(
            store.update(vec![entry("paging.size", "5")], vec![]),
            Err(StoreError::WriteFailed)
        );
        assert_eq!(store.snapshot(), &default_snapshot());
        assert_eq!(store.version(), 0);
    }
}
