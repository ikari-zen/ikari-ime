//! 書き出しと取り込みを、ファイルのパスで行う（ADR 0068 §4。security-design.md §8.3 の B-2）。
//!
//! Windows の設定の口は学習の全文を運ばない（`settings_pipe` が `ExportData` と `ImportData` の中継を断る）。設定画面は
//! 利用者が選んだファイルのパスだけを送り、engine-host がそのファイルを読み書きする。engine-host は利用者の権限で
//! 動くので、利用者が書ける場所にしか書けない（権限は広がらない）。
//!
//! - パスの形は `shell-core` の `bridge::check_transfer_path`（判断）で確かめてから開く。
//! - 取り込み: ふつうのファイルだけ。大きさは開いたファイルで見て、`settings::import_size` の上限の中だけ読む
//!   （読む間に大きくなっても、上限の 1 バイト先までしか読まない）。
//! - 書き出し: `replace` でなければ、新しく作るときだけ書く（既にあれば `Exists`）。`replace` は、画面で上書きの確認を
//!   済ませたときだけ立てる。そのときは、同じフォルダの一時のファイルに書いてから置き換える（途中で失敗しても、
//!   元のファイルを壊さない）。置き換える先が、ふつうのファイルでなければ断る。
//!
//! TODO(W4-s3 第 2 段): この関数を呼ぶ枠（パスを運ぶ `bridge.proto` の要求）は、まだ無い（契約なのでリードが決める）。
//!
//! 判断の置き場所: Rust。パスの規則は shell-core、ここはファイルを開いて読み書きするだけ。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use ikari_proto::SCHEMA_VERSION;
use ikari_proto::v1::{Envelope, ExportData, ImportData, envelope};
use ikari_shell_core::bridge::{TransferPathError, check_transfer_path};
use ikari_shell_core::settings::{ImportSize, MAX_IMPORT_BYTES, import_size};
use prost::Message;

use crate::slot::EngineSlot;

/// 置き換えの一時のファイルの名前の後ろに付けるもの（`<名前>.ikari-partial`）
const PARTIAL_SUFFIX: &str = ".ikari-partial";

/// 書き出しと取り込みの失敗（ログと返事には固定の語だけを出す。パスや中身は出さない）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferError {
    /// パスの形が使えない
    Path(TransferPathError),
    /// 取り込み: ファイルが無い
    NotFound,
    /// ふつうのファイルでない（フォルダ、デバイスなど）
    NotAFile,
    /// 取り込み: 上限を超える
    TooLarge,
    /// 取り込み: 空
    Empty,
    /// 書き出し: 既にある（上書きの確認がまだ）
    Exists,
    /// エンジンが開けていない、返事が読めない
    Engine,
    /// 読み書きの失敗
    Io,
}

impl TransferError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Path(e) => e.as_str(),
            Self::NotFound => "not-found",
            Self::NotAFile => "not-a-file",
            Self::TooLarge => "too-large",
            Self::Empty => "empty",
            Self::Exists => "exists",
            Self::Engine => "engine",
            Self::Io => "io",
        }
    }
}

fn io_error(e: &io::Error) -> TransferError {
    match e.kind() {
        io::ErrorKind::NotFound => TransferError::NotFound,
        io::ErrorKind::AlreadyExists => TransferError::Exists,
        _ => TransferError::Io,
    }
}

/// エンジンに書き出しを頼み、その中身を `path` に書く。返すのは、中身を空にした返事（件数と版だけ。全文は口に流さない）。
/// エンジンが誤りを返したら、その返事をそのまま返す（ファイルは作らない）。
pub fn export_to_path(
    engine: &EngineSlot,
    path: &str,
    replace: bool,
) -> Result<Envelope, TransferError> {
    let path = check_transfer_path(path).map_err(TransferError::Path)?;
    export_to_file(engine, Path::new(path.as_str()), replace)
}

/// `path` を読み、エンジンに取り込みを頼む（`dry_run` は検証して件数を返すだけ）。返すのはエンジンの返事。
pub fn import_from_path(
    engine: &EngineSlot,
    path: &str,
    dry_run: bool,
) -> Result<Envelope, TransferError> {
    let path = check_transfer_path(path).map_err(TransferError::Path)?;
    import_from_file(engine, Path::new(path.as_str()), dry_run)
}

/// `export_to_path` の、パスの形を確かめたあと（テストは一時のフォルダで呼ぶ）。
fn export_to_file(
    engine: &EngineSlot,
    path: &Path,
    replace: bool,
) -> Result<Envelope, TransferError> {
    // 書けない場所なら、エンジンに頼む前に分かるものは先に断る
    if !replace && fs::symlink_metadata(path).is_ok() {
        return Err(TransferError::Exists);
    }
    let mut reply = ask(engine, envelope::Body::ExportData(ExportData {}))?;
    let Some(envelope::Body::ExportReply(export)) = reply.body.as_mut() else {
        return Ok(reply);
    };
    let content = std::mem::take(&mut export.content);
    write_export(path, &content, replace)?;
    Ok(reply)
}

/// `import_from_path` の、パスの形を確かめたあと。
fn import_from_file(
    engine: &EngineSlot,
    path: &Path,
    dry_run: bool,
) -> Result<Envelope, TransferError> {
    let content = read_import(path)?;
    ask(
        engine,
        envelope::Body::ImportData(ImportData { content, dry_run }),
    )
}

/// エンジンに 1 つ頼み、返事を読む
fn ask(engine: &EngineSlot, body: envelope::Body) -> Result<Envelope, TransferError> {
    let request = Envelope {
        schema_version: SCHEMA_VERSION,
        request_id: 0,
        body: Some(body),
    }
    .encode_to_vec();
    let reply = engine.handle(&request);
    Envelope::decode(reply.as_slice()).map_err(|_| TransferError::Engine)
}

/// 取り込むファイルを読む（ふつうのファイルだけ。上限の中だけ）
pub(crate) fn read_import(path: &Path) -> Result<Vec<u8>, TransferError> {
    let file = File::open(path).map_err(|e| io_error(&e))?;
    let meta = file.metadata().map_err(|e| io_error(&e))?;
    if !meta.is_file() {
        return Err(TransferError::NotAFile);
    }
    check_size(meta.len())?;
    let mut content = Vec::new();
    file.take(MAX_IMPORT_BYTES + 1)
        .read_to_end(&mut content)
        .map_err(|e| io_error(&e))?;
    check_size(content.len() as u64)?;
    Ok(content)
}

fn check_size(bytes: u64) -> Result<(), TransferError> {
    match import_size(bytes) {
        ImportSize::Ok => Ok(()),
        ImportSize::TooLarge => Err(TransferError::TooLarge),
        ImportSize::Empty => Err(TransferError::Empty),
    }
}

/// 書き出しの中身を `path` に書く
pub(crate) fn write_export(
    path: &Path,
    content: &[u8],
    replace: bool,
) -> Result<(), TransferError> {
    if !replace {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| io_error(&e))?;
        return write_all_and_sync(&mut file, content).inspect_err(|_| {
            let _ = fs::remove_file(path);
        });
    }
    // 置き換える先は、ふつうのファイルだけ（リンクやフォルダは断る）
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_file() => return Err(TransferError::NotAFile),
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_error(&e)),
    }
    let partial = partial_path(path).ok_or(TransferError::Io)?;
    let written = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|e| io_error(&e))
        .and_then(|mut file| write_all_and_sync(&mut file, content))
        .and_then(|()| fs::rename(&partial, path).map_err(|e| io_error(&e)));
    if written.is_err() {
        let _ = fs::remove_file(&partial);
    }
    written
}

fn write_all_and_sync(file: &mut File, content: &[u8]) -> Result<(), TransferError> {
    file.write_all(content).map_err(|e| io_error(&e))?;
    file.sync_all().map_err(|e| io_error(&e))
}

/// 同じフォルダの一時のファイル（`<名前>.ikari-partial`）
fn partial_path(path: &Path) -> Option<PathBuf> {
    let mut name = path.file_name()?.to_os_string();
    name.push(PARTIAL_SUFFIX);
    Some(path.with_file_name(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn a_new_export_is_written_and_an_existing_file_is_kept_without_replace() {
        let dir = dir();
        let path = dir.path().join("export.tsv");
        assert_eq!(write_export(&path, b"one", false), Ok(()));
        assert_eq!(fs::read(&path).unwrap(), b"one");
        assert_eq!(
            write_export(&path, b"two", false),
            Err(TransferError::Exists)
        );
        assert_eq!(fs::read(&path).unwrap(), b"one");
    }

    #[test]
    fn replace_swaps_the_file_and_leaves_no_partial() {
        let dir = dir();
        let path = dir.path().join("export.tsv");
        fs::write(&path, b"old").unwrap();
        assert_eq!(write_export(&path, b"new", true), Ok(()));
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert!(!partial_path(&path).unwrap().exists());
        // 無いファイルへの replace は、ふつうに作る
        let fresh = dir.path().join("fresh.tsv");
        assert_eq!(write_export(&fresh, b"x", true), Ok(()));
        assert_eq!(fs::read(&fresh).unwrap(), b"x");
    }

    #[test]
    fn replace_refuses_a_directory() {
        let dir = dir();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        assert_eq!(write_export(&sub, b"x", true), Err(TransferError::NotAFile));
        assert!(sub.is_dir());
    }

    #[test]
    fn import_reads_a_regular_file_within_the_limit() {
        let dir = dir();
        let path = dir.path().join("import.tsv");
        fs::write(&path, b"data").unwrap();
        assert_eq!(read_import(&path), Ok(b"data".to_vec()));
    }

    #[test]
    fn import_refuses_missing_empty_and_non_files() {
        let dir = dir();
        assert_eq!(
            read_import(&dir.path().join("missing.tsv")),
            Err(TransferError::NotFound)
        );
        let empty = dir.path().join("empty.tsv");
        fs::write(&empty, b"").unwrap();
        assert_eq!(read_import(&empty), Err(TransferError::Empty));
        // フォルダは開けても（OS による）、ふつうのファイルでない
        assert!(matches!(
            read_import(dir.path()),
            Err(TransferError::NotAFile | TransferError::Io | TransferError::NotFound)
        ));
    }

    #[test]
    fn import_refuses_a_file_over_the_limit_without_reading_it() {
        let dir = dir();
        let path = dir.path().join("big.tsv");
        let file = File::create(&path).unwrap();
        // 中身を書かずに大きさだけ上限の 1 バイト先にする（疎なファイル）
        file.set_len(MAX_IMPORT_BYTES + 1).unwrap();
        assert_eq!(read_import(&path), Err(TransferError::TooLarge));
    }

    /// fixture の辞書でエンジンを開いた枠（`tests/common` と同じ作り。単体のテストからは共通部品を使えない）
    fn opened_engine() -> (EngineSlot, tempfile::TempDir) {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/sudachi-mini");
        let data = dir();
        let dict = ikari_core::user::UserDir::new(data.path()).dict_dir();
        fs::create_dir_all(&dict).unwrap();
        ikari_dict_builder::build(&ikari_dict_builder::BuildConfig {
            lex: vec![src.join("lex.csv")],
            matrix: src.join("matrix.def"),
            out_dir: dict,
            unk_pos: "名詞,普通名詞,一般,*,*,*".to_owned(),
            unk_cost: 10_000,
            label: "engine-host-transfer-test".to_owned(),
        })
        .unwrap();
        let engine = ikari_engine::Engine::open(&ikari_engine::EngineConfig {
            data_dir: data.path().to_string_lossy().into_owned(),
            enable_lm: false,
            idle_unload_secs: 300,
        })
        .unwrap();
        (EngineSlot::opened(std::sync::Arc::new(engine)), data)
    }

    #[test]
    fn export_writes_the_file_and_returns_the_reply_without_the_content_then_import_reads_it() {
        let (engine, _data) = opened_engine();
        let out = dir();
        let path = out.path().join("ikari-export.tsv");
        let reply = export_to_file(&engine, &path, false).unwrap();
        let Some(envelope::Body::ExportReply(export)) = reply.body else {
            panic!("expected an export reply: {reply:?}");
        };
        assert!(
            export.content.is_empty(),
            "the whole text stays off the pipe"
        );
        let written = fs::read(&path).unwrap();
        assert!(!written.is_empty());
        // 2 回目は上書きの確認がないので書かない
        assert_eq!(
            export_to_file(&engine, &path, false),
            Err(TransferError::Exists)
        );
        let reply = import_from_file(&engine, &path, true).unwrap();
        let Some(envelope::Body::ImportReply(import)) = reply.body else {
            panic!("expected an import reply: {reply:?}");
        };
        assert!(!import.applied, "dry run");
        assert!(
            import.issue.as_ref().is_none_or(|i| i.kind == 0),
            "{:?}",
            import.issue
        );
    }

    #[test]
    fn bad_paths_are_refused_before_touching_the_engine() {
        let engine = EngineSlot::new(
            Box::new(|| panic!("the engine must not be opened")),
            std::time::Duration::from_secs(3600),
        );
        assert_eq!(
            export_to_path(&engine, r"\\server\share\x.tsv", false),
            Err(TransferError::Path(TransferPathError::NetworkOrDevice))
        );
        assert_eq!(
            import_from_path(&engine, "relative.tsv", true),
            Err(TransferError::Path(TransferPathError::NotAbsolute))
        );
    }

    #[test]
    fn error_texts_are_distinct() {
        let all = [
            TransferError::Path(TransferPathError::BadName),
            TransferError::NotFound,
            TransferError::NotAFile,
            TransferError::TooLarge,
            TransferError::Empty,
            TransferError::Exists,
            TransferError::Engine,
            TransferError::Io,
        ];
        let texts: std::collections::HashSet<_> = all.iter().map(|e| e.as_str()).collect();
        assert_eq!(texts.len(), all.len());
    }
}
