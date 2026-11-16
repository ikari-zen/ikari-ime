//! 取得したモデルの束を inbox に作る（ADR 0076 §5.1 の 2〜4、§7）。
//!
//! `ikari-data`（URL から取得したもの）と、ファイルからの取り込み（Windows の設定の口、macOS の `shell-ffi`）が
//! 同じこの関数を使う。流れ: `inbox/<名前>.partial/` に**写しながら** sha256 と大きさを数える → 期待と照らす →
//! `manifest.json` を書く → `<名前>/` に rename する。途中の失敗では `.partial` を消す。
//! 入力のバイト列は信じない（低い権限のプロセスが書けた場所のものかもしれない）。この関数は中身を解釈しない。

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::error::ResourceError;
use crate::manifest::{
    Component, ComponentKind, Created, FileEntry, MANIFEST_FILE_NAME, Manifest, Origin,
    check_file_name,
};
use crate::sha256::{SHA256_HEX_LEN, is_sha256_hex};

/// 作りかけの束のディレクトリの名前の末尾（エンジンは見ない。ADR 0058 §4.3）
pub const PARTIAL_SUFFIX: &str = ".partial";
/// 部品の名前（モデルの束は 1 部品）
pub const MODEL_COMPONENT: &str = "model";
/// モデルの形式の版（manifest の `format`。ADR 0058 §5.1）
const MODEL_FORMAT_VERSION: u32 = 1;
/// 写すときの読み取りバッファ
const COPY_BUFFER_BYTES: usize = 64 * 1024;

/// 期待する sha256 と大きさ（プリセットは表の値。自由指定は利用者が sha256 を指定したときだけ）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected {
    /// 小文字 16 進 64 文字
    pub sha256: String,
    /// 大きさが分かっているときだけ
    pub size: Option<u64>,
}

/// 取得元の記録（manifest の `sources`。`sha256_given` が偽なら「sha256 なしで取得した」印になる）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub url: String,
    pub sha256_given: bool,
}

/// 束を作る計画。名前とファイル名は呼び出し側（Rust）が作る（URL や応答から作らない。ADR 0076 §5.1）
#[derive(Debug, Clone)]
pub struct StagePlan {
    /// 束のディレクトリの名前（`inbox/<name>/`）
    pub name: String,
    /// 束の中のファイルの名前
    pub file_name: String,
    pub kind: ComponentKind,
    pub origin: Origin,
    /// manifest の作成日時（呼び出し側が時計から作る。この crate は時計を持たない）
    pub created: Created,
    pub expected: Option<Expected>,
    /// 利用者が指定したモデルの形式（`PromptFormat` の語）。manifest の `profile.format`
    pub profile_format: Option<String>,
    pub source: Option<Source>,
    /// 写してよい大きさの上限（バイト）。超えたら `TooLarge`
    pub max_size: u64,
}

/// 作った束
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedBundle {
    pub name: String,
    pub sha256: String,
    pub size: u64,
    /// `inbox/<name>/`
    pub path: PathBuf,
}

#[derive(Debug)]
pub enum StageError {
    /// 名前、期待の sha256 の形が不正（何も作っていない）
    BadPlan(String),
    /// 同じ名前の束がすでにある（何も壊していない）
    Exists(PathBuf),
    /// 上限を超えた
    TooLarge {
        max: u64,
    },
    /// sha256 か大きさが期待と違う
    Checksum,
    /// ディスクが足りない
    DiskFull,
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Resource(ResourceError),
}

impl fmt::Display for StageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadPlan(why) => write!(f, "束の計画が不正です: {why}"),
            Self::Exists(p) => write!(f, "同じ名前の束がすでにあります: {}", p.display()),
            Self::TooLarge { max } => write!(f, "大きさの上限（{max} バイト）を超えました"),
            Self::Checksum => write!(f, "sha256 か大きさが期待と違います"),
            Self::DiskFull => write!(f, "ディスクの空きが足りません"),
            Self::Io { path, source } => {
                write!(f, "{} を読み書きできません: {source}", path.display())
            }
            Self::Resource(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Resource(e) => Some(e),
            _ => None,
        }
    }
}

/// `src` を `inbox/<name>.partial/<file_name>` に写しながら sha256 と大きさを数え、照らし、manifest を書き、
/// `inbox/<name>/` に rename する。失敗したら `.partial` を消す。
pub fn stage_model_bundle(
    src: &mut impl Read,
    inbox: &Path,
    plan: &StagePlan,
) -> Result<StagedBundle, StageError> {
    check_plan(plan)?;
    let final_dir = inbox.join(&plan.name);
    let partial_dir = inbox.join(format!("{}{PARTIAL_SUFFIX}", plan.name));
    if final_dir.exists() {
        return Err(StageError::Exists(final_dir));
    }
    // 前の落ちた残りを消してから始める
    remove_dir_if_exists(&partial_dir)?;
    fs::create_dir_all(&partial_dir).map_err(|e| io_error(&partial_dir, e))?;

    let staged = copy_and_publish(src, plan, &partial_dir, &final_dir);
    if staged.is_err() {
        // 消せなかったときも、元の失敗を返す
        let _ = fs::remove_dir_all(&partial_dir);
    }
    staged
}

fn copy_and_publish(
    src: &mut impl Read,
    plan: &StagePlan,
    partial_dir: &Path,
    final_dir: &Path,
) -> Result<StagedBundle, StageError> {
    let file_path = partial_dir.join(&plan.file_name);
    let (sha256, size) = copy_counting(src, &file_path, plan.max_size)?;
    if let Some(expected) = &plan.expected
        && (expected.sha256 != sha256 || expected.size.is_some_and(|s| s != size))
    {
        return Err(StageError::Checksum);
    }

    let component = Component {
        kind: plan.kind,
        format: MODEL_FORMAT_VERSION,
        files: vec![FileEntry {
            name: plan.file_name.clone(),
            size,
            sha256: sha256.clone(),
        }],
        builder: None,
        sources: plan
            .source
            .as_ref()
            .map(|s| json!({ "url": s.url, "sha256_given": s.sha256_given })),
        profile: plan
            .profile_format
            .as_ref()
            .map(|format| json!({ "format": format })),
    };
    let manifest = Manifest::new(
        plan.created.clone(),
        plan.origin,
        [(MODEL_COMPONENT.to_owned(), component)].into(),
    );
    manifest
        .write(&partial_dir.join(MANIFEST_FILE_NAME))
        .map_err(StageError::Resource)?;
    fs::rename(partial_dir, final_dir).map_err(|e| io_error(final_dir, e))?;
    Ok(StagedBundle {
        name: plan.name.clone(),
        sha256,
        size,
        path: final_dir.to_path_buf(),
    })
}

/// `dest` に写しながら数える。上限を 1 バイトでも超えたら `TooLarge`
fn copy_counting(
    src: &mut impl Read,
    dest: &Path,
    max_size: u64,
) -> Result<(String, u64), StageError> {
    let mut out = File::create(dest).map_err(|e| io_error(dest, e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; COPY_BUFFER_BYTES];
    let mut size = 0u64;
    loop {
        let n = match src.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io_error(dest, e)),
        };
        size += n as u64;
        if size > max_size {
            return Err(StageError::TooLarge { max: max_size });
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| io_error(dest, e))?;
    }
    out.sync_all().map_err(|e| io_error(dest, e))?;
    Ok((hex(&hasher.finalize()), size))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(SHA256_HEX_LEN);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn check_plan(plan: &StagePlan) -> Result<(), StageError> {
    for name in [&plan.name, &plan.file_name] {
        check_file_name(name).map_err(|e| StageError::BadPlan(e.to_string()))?;
    }
    if plan.file_name == MANIFEST_FILE_NAME || plan.name.ends_with(PARTIAL_SUFFIX) {
        return Err(StageError::BadPlan(format!(
            "予約された名前です: {} / {}",
            plan.name, plan.file_name
        )));
    }
    if let Some(expected) = &plan.expected
        && !is_sha256_hex(&expected.sha256)
    {
        return Err(StageError::BadPlan(
            "期待する sha256 は小文字 16 進 64 文字です".into(),
        ));
    }
    Ok(())
}

fn remove_dir_if_exists(dir: &Path) -> Result<(), StageError> {
    match fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_error(dir, e)),
    }
}

fn io_error(path: &Path, source: io::Error) -> StageError {
    if source.kind() == io::ErrorKind::StorageFull {
        StageError::DiskFull
    } else {
        StageError::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABC_SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn plan(expected: Option<Expected>) -> StagePlan {
        StagePlan {
            name: "user-ba7816bf8f01".into(),
            file_name: "user-ba7816bf8f01.gguf".into(),
            kind: ComponentKind::Model,
            origin: Origin::User,
            created: Created::parse("2026-11-17T00:00:00Z").unwrap(),
            expected,
            profile_format: Some("zenz".into()),
            source: Some(Source {
                url: "https://example.com/m.gguf".into(),
                sha256_given: false,
            }),
            max_size: 1024,
        }
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn stages_counts_sha_writes_manifest_and_renames() {
        let inbox = tempfile::tempdir().unwrap();
        let p = plan(Some(Expected {
            sha256: ABC_SHA.into(),
            size: Some(3),
        }));
        let staged = stage_model_bundle(&mut &b"abc"[..], inbox.path(), &p).unwrap();
        assert_eq!(staged.sha256, ABC_SHA);
        assert_eq!(staged.size, 3);
        // .partial は残らず、束だけがある
        assert_eq!(entries(inbox.path()), ["user-ba7816bf8f01"]);
        assert_eq!(
            entries(&staged.path),
            ["manifest.json", "user-ba7816bf8f01.gguf"]
        );
        assert_eq!(fs::read(staged.path.join(&p.file_name)).unwrap(), b"abc");
        let m = Manifest::read(&staged.path.join(MANIFEST_FILE_NAME)).unwrap();
        assert_eq!(m.origin, Origin::User);
        let c = &m.components[MODEL_COMPONENT];
        assert_eq!(c.kind, ComponentKind::Model);
        assert_eq!(c.files[0].sha256, ABC_SHA);
        assert_eq!(c.profile.as_ref().unwrap()["format"], "zenz");
        assert_eq!(
            c.sources.as_ref().unwrap()["url"],
            "https://example.com/m.gguf"
        );
    }

    #[test]
    fn no_expectation_is_allowed_and_recorded() {
        let inbox = tempfile::tempdir().unwrap();
        let staged = stage_model_bundle(&mut &b"abc"[..], inbox.path(), &plan(None)).unwrap();
        assert_eq!(staged.sha256, ABC_SHA);
        let m = Manifest::read(&staged.path.join(MANIFEST_FILE_NAME)).unwrap();
        assert_eq!(
            m.components[MODEL_COMPONENT].sources.as_ref().unwrap()["sha256_given"],
            false
        );
    }

    #[test]
    fn sha_mismatch_removes_partial_and_creates_nothing() {
        let inbox = tempfile::tempdir().unwrap();
        let p = plan(Some(Expected {
            sha256: "0".repeat(64),
            size: None,
        }));
        let err = stage_model_bundle(&mut &b"abc"[..], inbox.path(), &p).unwrap_err();
        assert!(matches!(err, StageError::Checksum), "{err}");
        assert!(entries(inbox.path()).is_empty());
    }

    #[test]
    fn size_mismatch_is_a_checksum_error() {
        let inbox = tempfile::tempdir().unwrap();
        let p = plan(Some(Expected {
            sha256: ABC_SHA.into(),
            size: Some(4),
        }));
        let err = stage_model_bundle(&mut &b"abc"[..], inbox.path(), &p).unwrap_err();
        assert!(matches!(err, StageError::Checksum), "{err}");
        assert!(entries(inbox.path()).is_empty());
    }

    #[test]
    fn over_the_limit_is_too_large_and_cleans_up() {
        let inbox = tempfile::tempdir().unwrap();
        let mut p = plan(None);
        p.max_size = 2;
        let err = stage_model_bundle(&mut &b"abc"[..], inbox.path(), &p).unwrap_err();
        assert!(matches!(err, StageError::TooLarge { max: 2 }), "{err}");
        assert!(entries(inbox.path()).is_empty());
        // ちょうど上限は通る
        p.max_size = 3;
        stage_model_bundle(&mut &b"abc"[..], inbox.path(), &p).unwrap();
    }

    #[test]
    fn stale_partial_from_a_crash_is_replaced() {
        let inbox = tempfile::tempdir().unwrap();
        let stale = inbox.path().join("user-ba7816bf8f01.partial");
        fs::create_dir_all(&stale).unwrap();
        fs::write(stale.join("leftover"), b"x").unwrap();
        let staged = stage_model_bundle(&mut &b"abc"[..], inbox.path(), &plan(None)).unwrap();
        assert_eq!(entries(inbox.path()), ["user-ba7816bf8f01"]);
        assert!(!staged.path.join("leftover").exists());
    }

    #[test]
    fn existing_bundle_is_not_overwritten() {
        let inbox = tempfile::tempdir().unwrap();
        let p = plan(None);
        stage_model_bundle(&mut &b"abc"[..], inbox.path(), &p).unwrap();
        let err = stage_model_bundle(&mut &b"abc"[..], inbox.path(), &p).unwrap_err();
        assert!(matches!(err, StageError::Exists(_)), "{err}");
        assert_eq!(entries(inbox.path()), ["user-ba7816bf8f01"]);
    }

    #[test]
    fn read_error_cleans_up() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("boom"))
            }
        }
        let inbox = tempfile::tempdir().unwrap();
        let err = stage_model_bundle(&mut Broken, inbox.path(), &plan(None)).unwrap_err();
        assert!(matches!(err, StageError::Io { .. }), "{err}");
        assert!(entries(inbox.path()).is_empty());
    }

    #[test]
    fn unsafe_or_reserved_names_are_refused_before_touching_disk() {
        let inbox = tempfile::tempdir().unwrap();
        for (name, file) in [
            ("../x", "a.gguf"),
            ("ok", "../a.gguf"),
            ("ok", "manifest.json"),
            ("x.partial", "a.gguf"),
            ("", "a.gguf"),
        ] {
            let mut p = plan(None);
            p.name = name.into();
            p.file_name = file.into();
            let err = stage_model_bundle(&mut &b"abc"[..], inbox.path(), &p).unwrap_err();
            assert!(
                matches!(err, StageError::BadPlan(_)),
                "{name} {file}: {err}"
            );
        }
        let mut p = plan(Some(Expected {
            sha256: "ABC".into(),
            size: None,
        }));
        p.name = "ok".into();
        assert!(matches!(
            stage_model_bundle(&mut &b"abc"[..], inbox.path(), &p),
            Err(StageError::BadPlan(_))
        ));
        assert!(entries(inbox.path()).is_empty());
    }
}
