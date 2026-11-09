//! 入れ替えの手順の記録（ADR 0058 §7.2、§7.4。`inbox/.journal.json`）。
//!
//! ここは型と保存・読み出しだけ。段階を進める・起動時に直すのはエンジン（#246-2）。
//! 辞書の手順 4（`dict` → `dict.prev`、`dict.new` → `dict`）の途中で落ちると `dict/` がない状態になるので、
//! 手順 3 の前に journal を書き、段階ごとに更新する。次の起動で、段階から「先に進める」か「戻す」を決める。

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::ResourceError;

pub const JOURNAL_FILE_NAME: &str = ".journal.json";
const JOURNAL_SCHEMA: u32 = 1;

/// 何の入れ替えか
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalOp {
    /// inbox の束を入れる
    Install,
    /// 前の版に戻す（`current` と `prev` の入れ替え）
    Rollback,
}

/// 「どこまで済んだか」。済んだ段階を書く（書いたあとにその次の操作をする）。
/// 辞書の手順（§7.2 の 3〜5）に対応する。表とモデルは 1 回の rename なので `Swapping` と `ManifestWritten` だけを使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalStage {
    /// 手順 3: 入れ替えを始める（試しの組み立ては済み。まだ `current` に触れていない）
    Swapping,
    /// 手順 4: 前の `.prev` を消した
    PrevRemoved,
    /// 手順 4: `current` を `.prev` に移した（`current` がない状態。ここで落ちると `.new` を `current` にして進める）
    CurrentMovedToPrev,
    /// 手順 4: `.new` を `current` にした
    NewInPlace,
    /// 手順 5: `MANIFEST.json` を書いた（残りは journal を消すだけ）
    ManifestWritten,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    pub schema: u32,
    /// 部品の名前（`dict`、`postal`、`emoji`、`model`）
    pub component: String,
    pub op: JournalOp,
    pub stage: JournalStage,
    /// 入れる束の名前（`inbox/` の下。`Rollback` では空）
    #[serde(default)]
    pub bundle: Option<String>,
}

impl Journal {
    pub fn new(component: &str, op: JournalOp, bundle: Option<&str>) -> Self {
        Self {
            schema: JOURNAL_SCHEMA,
            component: component.to_owned(),
            op,
            stage: JournalStage::Swapping,
            bundle: bundle.map(str::to_owned),
        }
    }

    /// `inbox` の中の journal のパス
    pub fn path_in(inbox: &Path) -> PathBuf {
        inbox.join(JOURNAL_FILE_NAME)
    }

    /// 保存する（一時ファイルに書いて sync してから rename。書きかけが残らない）
    pub fn save(&self, inbox: &Path) -> Result<(), ResourceError> {
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| ResourceError::ManifestMalformed(e.to_string()))?;
        write_atomic(&Self::path_in(inbox), &json)
    }

    /// 読む。なければ `None`（入れ替え中ではない）。読めない・知らない版はエラー
    /// （エンジンは、壊れた journal を勝手に捨てず、状態に出す）。
    pub fn load(inbox: &Path) -> Result<Option<Self>, ResourceError> {
        let path = Self::path_in(inbox);
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(ResourceError::io(&path, e)),
        };
        let journal: Self = serde_json::from_str(&text)
            .map_err(|e| ResourceError::ManifestMalformed(format!("journal: {e}")))?;
        if journal.schema != JOURNAL_SCHEMA {
            return Err(ResourceError::UnknownSchema {
                found: u64::from(journal.schema),
                supported: JOURNAL_SCHEMA,
            });
        }
        Ok(Some(journal))
    }

    /// 入れ替えが済んだら消す（なくても成功）
    pub fn clear(inbox: &Path) -> Result<(), ResourceError> {
        let path = Self::path_in(inbox);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(ResourceError::io(&path, e)),
        }
    }
}

/// `<path>.tmp` に書いて sync し、rename する
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ResourceError> {
    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);
    let write = || -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    };
    write().map_err(|e| {
        let _ = fs::remove_file(&tmp);
        ResourceError::io(path, e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_every_stage() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Journal::load(dir.path()).unwrap(), None);
        for stage in [
            JournalStage::Swapping,
            JournalStage::PrevRemoved,
            JournalStage::CurrentMovedToPrev,
            JournalStage::NewInPlace,
            JournalStage::ManifestWritten,
        ] {
            let mut j = Journal::new("dict", JournalOp::Install, Some("ikari-resources-20261110"));
            j.stage = stage;
            j.save(dir.path()).unwrap();
            assert_eq!(Journal::load(dir.path()).unwrap(), Some(j));
        }
        Journal::clear(dir.path()).unwrap();
        assert_eq!(Journal::load(dir.path()).unwrap(), None);
        Journal::clear(dir.path()).unwrap(); // なくても成功
    }

    #[test]
    fn stages_are_ordered() {
        assert!(JournalStage::Swapping < JournalStage::CurrentMovedToPrev);
        assert!(JournalStage::NewInPlace < JournalStage::ManifestWritten);
    }

    #[test]
    fn rollback_has_no_bundle_and_save_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        Journal::new("model", JournalOp::Rollback, None)
            .save(dir.path())
            .unwrap();
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [JOURNAL_FILE_NAME]);
        assert_eq!(Journal::load(dir.path()).unwrap().unwrap().bundle, None);
    }

    #[test]
    fn corrupt_or_unknown_schema_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(Journal::path_in(dir.path()), "{not json").unwrap();
        assert!(matches!(
            Journal::load(dir.path()),
            Err(ResourceError::ManifestMalformed(_))
        ));
        fs::write(
            Journal::path_in(dir.path()),
            r#"{"schema":9,"component":"dict","op":"install","stage":"swapping"}"#,
        )
        .unwrap();
        assert!(matches!(
            Journal::load(dir.path()),
            Err(ResourceError::UnknownSchema { found: 9, .. })
        ));
    }
}
