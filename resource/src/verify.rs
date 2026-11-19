//! 束の検証（ADR 0058 §6.1、§6.3）。入れ替えの前に、すべて通ったものだけを受ける。
//!
//! 順番は、安い検査を先に、sha256（束の全バイトを読む）を最後にする。
//! 試しの組み立て・表の全行の検査・ディスクの余り（§6.1 の 4 の一部と 5）は、エンジン側（#246-2）で行う。
//! 形式の版は `SupportedFormats` で渡す（この crate は辞書の中身を読まない）。

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use ikari_dict_format::{
    FORMAT_VERSION, GRAMMAR_FILE_NAME, SYSTEM_DICT_FILE_NAME, VECTORS_FILE_NAME,
};

use crate::error::ResourceError;
use crate::manifest::{
    Component, ComponentKind, Freshness, MANIFEST_FILE_NAME, Manifest, check_file_name,
};
use crate::model_list::{UserModelId, profile_format};
use crate::models::{KNOWN_MODELS, KnownModel};
use crate::sha256::{sha256_file, sha256_reader};

/// 表（postal、emoji）とモデルの形式の版
pub const TABLE_FORMAT_VERSION: u32 = 1;
const MODEL_FORMAT_VERSION: u32 = 1;

/// 辞書の部品に要る 3 ファイル
const DICT_FILE_NAMES: [&str; 3] = [GRAMMAR_FILE_NAME, SYSTEM_DICT_FILE_NAME, VECTORS_FILE_NAME];

/// このアプリが読める形式の版
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupportedFormats {
    pub dict: u32,
    pub table: u32,
    pub model: u32,
}

impl SupportedFormats {
    /// このビルドの `dict-format` の定数と、表・モデルの版
    pub const fn current() -> Self {
        Self {
            dict: FORMAT_VERSION,
            table: TABLE_FORMAT_VERSION,
            model: MODEL_FORMAT_VERSION,
        }
    }

    fn for_kind(&self, kind: ComponentKind) -> u32 {
        match kind {
            ComponentKind::Dict => self.dict,
            ComponentKind::Table => self.table,
            ComponentKind::Model => self.model,
        }
    }
}

pub struct VerifyOptions<'a> {
    pub formats: SupportedFormats,
    /// 受けるモデル。普段は `KNOWN_MODELS`
    pub known_models: &'a [KnownModel],
    /// 表に無いモデル（自由指定）を受ける形式の語（`PromptFormat` の語）。空なら受けない（既定）。
    /// 受けるのは、manifest の `profile.format` がこの語のどれかで、ファイル名が sha256 から作った
    /// `user-<12 桁>.gguf` のものだけ。manifest の `origin` は見ない（信用しない。D-4 の条件 2）
    pub user_model_formats: &'a [&'a str],
    /// 今入っている版（`dict/MANIFEST.json` などを束ねたもの）。なければ（`install-data.sh` で入れた、初めて）比べない
    pub installed: Option<&'a Manifest>,
}

impl Default for VerifyOptions<'_> {
    fn default() -> Self {
        Self {
            formats: SupportedFormats::current(),
            known_models: KNOWN_MODELS,
            user_model_formats: &[],
            installed: None,
        }
    }
}

/// 検証を通った束
#[derive(Debug)]
pub struct VerifiedBundle {
    pub dir: PathBuf,
    pub manifest: Manifest,
    /// 部品ごとの、今の版との関係。`Same` の部品は入れ替えない
    pub freshness: BTreeMap<String, Freshness>,
}

/// 束のディレクトリを検証する。最初に見つけた問題の理由で拒む。
pub fn verify_bundle(
    dir: &Path,
    opts: &VerifyOptions<'_>,
) -> Result<VerifiedBundle, ResourceError> {
    let manifest_path = dir.join(MANIFEST_FILE_NAME);
    reject_if_symlink(&manifest_path, MANIFEST_FILE_NAME)?;
    let manifest = Manifest::read(&manifest_path)?;

    for (name, component) in &manifest.components {
        check_component(name, component, opts)?;
    }
    let freshness = check_freshness(&manifest, opts.installed)?;

    check_directory_entries(dir, &manifest)?;
    check_sizes(dir, &manifest)?;
    check_digests(dir, &manifest)?;

    Ok(VerifiedBundle {
        dir: dir.to_owned(),
        manifest,
        freshness,
    })
}

/// 形式の版、辞書の 3 ファイル、モデルの既知の表（中身を読まない検査）
fn check_component(
    name: &str,
    component: &Component,
    opts: &VerifyOptions<'_>,
) -> Result<(), ResourceError> {
    let supported = opts.formats.for_kind(component.kind);
    if component.format != supported {
        return Err(ResourceError::FormatMismatch {
            component: name.to_owned(),
            found: component.format,
            supported,
        });
    }
    match component.kind {
        ComponentKind::Dict => {
            let present: HashSet<&str> = component.files.iter().map(|f| f.name.as_str()).collect();
            if let Some(missing) = DICT_FILE_NAMES.iter().find(|n| !present.contains(**n)) {
                return Err(ResourceError::IncompleteDict {
                    component: name.to_owned(),
                    missing: (*missing).to_owned(),
                });
            }
        }
        ComponentKind::Model => {
            for f in &component.files {
                let known = opts
                    .known_models
                    .iter()
                    .any(|m| m.file_name == f.name && m.sha256 == f.sha256);
                let user = profile_format(component)
                    .is_some_and(|w| opts.user_model_formats.contains(&w))
                    && UserModelId::from_sha256(&f.sha256)
                        .is_some_and(|id| id.file_name() == f.name);
                if !known && !user {
                    return Err(ResourceError::UnknownModel {
                        name: f.name.clone(),
                        sha256: f.sha256.clone(),
                    });
                }
            }
        }
        ComponentKind::Table => {}
    }
    Ok(())
}

/// 部品ごとに今の版と比べる。古い部品が 1 つでもあれば拒む（束は丸ごと入れるか、入れないか）
fn check_freshness(
    manifest: &Manifest,
    installed: Option<&Manifest>,
) -> Result<BTreeMap<String, Freshness>, ResourceError> {
    let mut out = BTreeMap::new();
    for name in manifest.components.keys() {
        let f = match installed {
            Some(inst) => manifest.freshness_of(name, inst),
            None => Freshness::Newer,
        };
        if f == Freshness::Older
            && let Some(inst) = installed
        {
            return Err(ResourceError::Stale {
                component: name.clone(),
                bundle_created: manifest.created.as_str().to_owned(),
                installed_created: inst.created.as_str().to_owned(),
            });
        }
        out.insert(name.clone(), f);
    }
    Ok(out)
}

/// 束の直下にあるものを lstat で見る。manifest にないもの、シンボリックリンク、普通のファイルでないものを拒む
fn check_directory_entries(dir: &Path, manifest: &Manifest) -> Result<(), ResourceError> {
    let listed: HashSet<&str> = manifest
        .components
        .values()
        .flat_map(|c| c.files.iter().map(|f| f.name.as_str()))
        .collect();
    let entries = fs::read_dir(dir).map_err(|e| ResourceError::io(dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| ResourceError::io(dir, e))?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            return Err(ResourceError::UnsafePath {
                name: file_name.to_string_lossy().into_owned(),
            });
        };
        let meta =
            fs::symlink_metadata(entry.path()).map_err(|e| ResourceError::io(entry.path(), e))?;
        if meta.file_type().is_symlink() {
            return Err(ResourceError::Symlink {
                name: name.to_owned(),
            });
        }
        if name == MANIFEST_FILE_NAME {
            continue;
        }
        if !listed.contains(name) {
            return Err(ResourceError::UnlistedFile {
                name: name.to_owned(),
            });
        }
        if !meta.is_file() {
            return Err(ResourceError::NotRegularFile {
                name: name.to_owned(),
            });
        }
    }
    Ok(())
}

/// ある・大きさが合う。足りなければ「途中で切れた」、多ければ「大きさが違う」
fn check_sizes(dir: &Path, manifest: &Manifest) -> Result<(), ResourceError> {
    for f in manifest.components.values().flat_map(|c| &c.files) {
        check_file_name(&f.name)?;
        let path = dir.join(&f.name);
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(ResourceError::MissingFile {
                    name: f.name.clone(),
                });
            }
            Err(e) => return Err(ResourceError::io(&path, e)),
        };
        let actual = meta.len();
        if actual < f.size {
            return Err(ResourceError::Truncated {
                name: f.name.clone(),
                expected: f.size,
                actual,
            });
        }
        if actual > f.size {
            return Err(ResourceError::SizeMismatch {
                name: f.name.clone(),
                expected: f.size,
                actual,
            });
        }
    }
    Ok(())
}

fn check_digests(dir: &Path, manifest: &Manifest) -> Result<(), ResourceError> {
    for f in manifest.components.values().flat_map(|c| &c.files) {
        let path = dir.join(&f.name);
        let actual = sha256_file(&path).map_err(|e| ResourceError::io(&path, e))?;
        if actual != f.sha256 {
            return Err(ResourceError::Sha256Mismatch {
                name: f.name.clone(),
                expected: f.sha256.clone(),
                actual,
            });
        }
    }
    Ok(())
}

/// 置いてあるモデル 1 件（`models/` の GGUF）を、読む前に照合する。
///
/// 表（`known`）にある名前で、大きさ、sha256 の順に合うものだけを通す（大きさで先に弾く）。
/// 開いたハンドルで大きさも sha256 も見る。
pub fn verify_model_file(
    path: &Path,
    file_name: &str,
    known: &[KnownModel],
) -> Result<(), ResourceError> {
    known_model(file_name, known)?;
    let file = File::open(path).map_err(|e| ResourceError::io(path, e))?;
    verify_model_open(&file, path, file_name, known)
}

/// [`verify_model_file`] と同じ照合を、開いたハンドルで行う（照合した物と、あとで子に渡す物を同じにする。C-03、FC-3）。
/// 読むのは位置を動かさない読み方（unix の `read_at`）で、ハンドルの読み書きの位置に依らない。`path` は誤りの文にだけ使う
pub fn verify_model_open(
    file: &File,
    path: &Path,
    file_name: &str,
    known: &[KnownModel],
) -> Result<(), ResourceError> {
    let model = known_model(file_name, known)?;
    verify_open_expected(file, path, file_name, model.size, model.sha256)
}

/// 開いたハンドルを、期待する大きさと sha256 で照合する（自由指定のモデルは、取り込みのときに数えて一覧に書いた値。
/// D-4 の条件 7）。読み方は [`verify_model_open`] と同じ（位置を動かさない）
pub fn verify_open_expected(
    file: &File,
    path: &Path,
    file_name: &str,
    size: u64,
    sha256: &str,
) -> Result<(), ResourceError> {
    let meta = file.metadata().map_err(|e| ResourceError::io(path, e))?;
    if !meta.is_file() {
        return Err(ResourceError::NotRegularFile {
            name: file_name.to_owned(),
        });
    }
    let actual = meta.len();
    if actual < size {
        return Err(ResourceError::Truncated {
            name: file_name.to_owned(),
            expected: size,
            actual,
        });
    }
    if actual > size {
        return Err(ResourceError::SizeMismatch {
            name: file_name.to_owned(),
            expected: size,
            actual,
        });
    }
    let digest = sha256_reader(PositionalReader { file, offset: 0 })
        .map_err(|e| ResourceError::io(path, e))?;
    if digest != sha256 {
        return Err(ResourceError::Sha256Mismatch {
            name: file_name.to_owned(),
            expected: sha256.to_owned(),
            actual: digest,
        });
    }
    Ok(())
}

fn known_model<'a>(
    file_name: &str,
    known: &'a [KnownModel],
) -> Result<&'a KnownModel, ResourceError> {
    known
        .iter()
        .find(|m| m.file_name == file_name)
        .ok_or_else(|| ResourceError::UnknownModel {
            name: file_name.to_owned(),
            sha256: String::new(),
        })
}

/// 先頭から順に、位置を指定して読む（ハンドルの位置を共有する写しがあっても、その位置に依らない）。
/// Windows の `seek_read` はハンドルの位置も動かすが、Windows の子はモデルをパスで開くので関わらない
struct PositionalReader<'a> {
    file: &'a File,
    offset: u64,
}

impl std::io::Read for PositionalReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        #[cfg(unix)]
        let n = std::os::unix::fs::FileExt::read_at(self.file, buf, self.offset)?;
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_read(self.file, buf, self.offset)?;
        self.offset += n as u64;
        Ok(n)
    }
}

fn reject_if_symlink(path: &Path, name: &str) -> Result<(), ResourceError> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Err(ResourceError::Symlink {
            name: name.to_owned(),
        }),
        _ => Ok(()),
    }
}
