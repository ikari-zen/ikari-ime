//! inbox のモデルの束を `models/` に取り込む（D-4 の条件 1。ADR 0060 §6、§7.2）。
//!
//! - 取り込むのはエンジンだけで、起動のとき（照合したハンドルを開く前）に 1 か所で行う。ほかの部品（`ikari-data`、
//!   設定の画面）は束を inbox に置くまでで、`models/` を直に書かない。
//! - 1 つの束の順: 束を検証する（数え直す）→ 新しいファイルへ**写しながら数え**、検証した値と照らす → 置き換える →
//!   一覧（`models/MANIFEST.json`）を一時ファイルから置き換える → inbox の束を消す。一覧を最後に書くので、途中で
//!   落ちても、束が残っていれば次の起動でやり直せる。
//! - 扱うのはモデルだけの束（部品が `model` 1 つ）。辞書や表の束には触れない（#246）。
//! - 入れる名前は sha256 から決める（プリセットは表の名前、自由指定は `user-<12 桁>.gguf`）。束の名前や manifest の
//!   文字列からパスを作らない。
//! - 檻の段（プリセットか）はここでは決めない（`model_select` が sha256 の表で決める。D-4 の条件 2）。
//!
//! 判断の置き場所: Rust（エンジン）。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use ikari_resource::stage::MODEL_COMPONENT;
use ikari_resource::{
    ComponentKind, KNOWN_MODELS, MANIFEST_FILE_NAME, MODEL_MAX_SIZE, Manifest, ModelList,
    PARTIAL_SUFFIX, ResourceError, UserModelEntry, UserModelId, VerifyOptions, sha256_reader,
    verify_bundle,
};

use crate::MODELS_DIR_NAME;

/// 束を置く場所（データの場所の下。`ikari-data` の `inbox` と同じ）
pub(crate) const INBOX_DIR_NAME: &str = "inbox";
/// 写している途中のファイルの末尾（`models/<名前>.new`）
const NEW_SUFFIX: &str = ".new";

/// 束 1 つの結果（ログの語）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportResult {
    /// 入れた
    Installed,
    /// manifest が読めない（束は残す。モデルの束かが分からないため）
    Manifest,
    /// 検証に通らない（束は消す）
    Rejected,
    /// 写す・置き換える・一覧を書くのに失敗した（束は残し、次の起動でやり直す）
    Io,
}

impl ImportResult {
    pub(crate) fn as_word(self) -> &'static str {
        match self {
            ImportResult::Installed => "installed",
            ImportResult::Manifest => "manifest",
            ImportResult::Rejected => "rejected",
            ImportResult::Io => "io",
        }
    }
}

/// 束 1 つの取り込みの記録
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportOutcome {
    /// ログに書く名前: 自由指定の ID かプリセットの ID。それ以外の名前は書かない（`other`）
    pub(crate) name: String,
    pub(crate) result: ImportResult,
}

/// ログに書いてよい名前（ID の形のものだけ。束の名前は利用者が付けられる）
fn log_name(bundle: &str) -> String {
    if UserModelId::parse(bundle).is_some() || ikari_resource::preset_by_id(bundle).is_some() {
        bundle.to_owned()
    } else {
        "other".to_owned()
    }
}

/// inbox のモデルの束を、すべて取り込む。`formats` は自由指定に受ける形式の語。inbox が無ければ何もしない
pub(crate) fn import_models(data_dir: &Path, formats: &[&str]) -> Vec<ImportOutcome> {
    let inbox = data_dir.join(INBOX_DIR_NAME);
    let Ok(entries) = fs::read_dir(&inbox) else {
        return Vec::new();
    };
    let mut bundles: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| is_bundle_name(name))
        .collect();
    bundles.sort();
    let models = data_dir.join(MODELS_DIR_NAME);
    bundles
        .into_iter()
        .filter_map(|name| {
            let dir = inbox.join(&name);
            // リンクとフォルダでないものは見ない
            let is_dir = fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_dir());
            if !is_dir {
                return None;
            }
            let result = import_bundle(&dir, &models, formats)?;
            Some(ImportOutcome {
                name: log_name(&name),
                result,
            })
        })
        .collect()
}

/// 束のフォルダの名前として扱うか（隠しファイル、作りかけ、パスに使えない名前は見ない）
fn is_bundle_name(name: &str) -> bool {
    !name.starts_with('.')
        && !name.ends_with(PARTIAL_SUFFIX)
        && ikari_resource::manifest::check_file_name(name).is_ok()
}

/// 束 1 つ。モデルだけの束でなければ None（触れない）
fn import_bundle(dir: &Path, models: &Path, formats: &[&str]) -> Option<ImportResult> {
    let manifest = match Manifest::read(&dir.join(MANIFEST_FILE_NAME)) {
        Ok(m) => m,
        Err(ResourceError::ManifestMissing) => return None,
        Err(_) => return Some(ImportResult::Manifest),
    };
    if !is_model_only(&manifest) {
        return None;
    }
    let opts = VerifyOptions {
        user_model_formats: formats,
        ..VerifyOptions::default()
    };
    let verified = match verify_bundle(dir, &opts) {
        Ok(v) => v,
        // 読めなかっただけなら残してやり直す。中身が合わないものは消す（起動のたびに数え直さない）
        Err(ResourceError::Io { .. }) => return Some(ImportResult::Io),
        Err(_) => {
            let _ = fs::remove_dir_all(dir);
            return Some(ImportResult::Rejected);
        }
    };
    let component = &verified.manifest.components[MODEL_COMPONENT];
    let file = &component.files[0];
    // 表にあるもの（プリセット）は表の名前で、ほかは一覧の 1 件として入れる
    let known = KNOWN_MODELS
        .iter()
        .find(|k| k.sha256 == file.sha256 && k.size == file.size);
    let user = match known {
        Some(_) => None,
        None => match UserModelEntry::from_component(component, formats) {
            Ok(entry) => Some(entry),
            Err(_) => {
                let _ = fs::remove_dir_all(dir);
                return Some(ImportResult::Rejected);
            }
        },
    };
    let dest_name = match (known, &user) {
        (Some(k), _) => k.file_name.to_owned(),
        (None, Some(entry)) => entry.file_name(),
        (None, None) => return Some(ImportResult::Rejected),
    };
    if fs::create_dir_all(models).is_err() {
        return Some(ImportResult::Io);
    }
    match install_file(
        &dir.join(&file.name),
        &models.join(&dest_name),
        &file.sha256,
        file.size,
    ) {
        Ok(()) => {}
        Err(InstallError::Mismatch) => {
            let _ = fs::remove_dir_all(dir);
            return Some(ImportResult::Rejected);
        }
        Err(InstallError::Io) => return Some(ImportResult::Io),
    }
    if let Some(entry) = user {
        // 一覧が壊れていれば、作り直す（壊れた一覧は、読む側でも使わない）
        let mut list = ModelList::read(models, formats).unwrap_or_default();
        list.upsert(entry);
        if list
            .write(models, verified.manifest.created.clone())
            .is_err()
        {
            return Some(ImportResult::Io);
        }
    }
    // 消せなくても入れ替えは済んでいる（次の起動で、同じものをもう一度入れる）
    let _ = fs::remove_dir_all(dir);
    Some(ImportResult::Installed)
}

/// 部品が `model` 1 つで、ファイルが 1 つの束か
fn is_model_only(manifest: &Manifest) -> bool {
    manifest.components.len() == 1
        && manifest
            .components
            .get(MODEL_COMPONENT)
            .is_some_and(|c| c.kind == ComponentKind::Model && c.files.len() == 1)
}

enum InstallError {
    /// 写したものが、検証した値と合わない（検証のあとに書き換えられた）
    Mismatch,
    Io,
}

/// `src` を `dest.new` に写しながら数え、`sha256` と `size` に合えば `dest` に置き換える（合わなければ消す）
fn install_file(src: &Path, dest: &Path, sha256: &str, size: u64) -> Result<(), InstallError> {
    let tmp = with_suffix(dest, NEW_SUFFIX);
    let result = copy_counting(src, &tmp).map_err(|_| InstallError::Io);
    match result {
        Ok((digest, copied)) if digest == sha256 && copied == size => fs::rename(&tmp, dest)
            .map_err(|_| {
                let _ = fs::remove_file(&tmp);
                InstallError::Io
            }),
        Ok(_) => {
            let _ = fs::remove_file(&tmp);
            Err(InstallError::Mismatch)
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// リンクを辿らずに開き、`dest` に写しながら sha256 を数える。上限を超えたら誤り
fn copy_counting(src: &Path, dest: &Path) -> io::Result<(String, u64)> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let input = options.open(src)?;
    if !input.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut out = File::create(dest)?;
    let mut tee = Tee {
        input: input.take(MODEL_MAX_SIZE.saturating_add(1)),
        out: &mut out,
        copied: 0,
        error: None,
    };
    let digest = sha256_reader(&mut tee)?;
    if let Some(e) = tee.error.take() {
        return Err(e);
    }
    let copied = tee.copied;
    if copied > MODEL_MAX_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            "model too large",
        ));
    }
    out.sync_all()?;
    Ok((digest, copied))
}

/// 読んだものを、そのまま `out` にも書く
struct Tee<'a, R> {
    input: R,
    out: &'a mut File,
    copied: u64,
    error: Option<io::Error>,
}

impl<R: Read> Read for Tee<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.input.read(buf)?;
        if let Err(e) = self.out.write_all(&buf[..n]) {
            self.error = Some(e);
            return Err(io::Error::other("write failed"));
        }
        self.copied += n as u64;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use ikari_resource::{
        ComponentKind, Created, MODEL_LIST_FILE_NAME, Origin, StagePlan, sha256_reader,
        stage_model_bundle,
    };

    use super::*;

    const BODY: &[u8] = b"GGUF user model";
    const FORMATS: &[&str] = &["zenz-v3", "kanakanji-edge"];

    fn stage(data_dir: &Path, body: &[u8], format: &str, origin: Origin) -> (UserModelId, PathBuf) {
        let sha = sha256_reader(body).unwrap();
        let id = UserModelId::from_sha256(&sha).unwrap();
        let plan = StagePlan {
            name: id.to_string(),
            file_name: id.file_name(),
            kind: ComponentKind::Model,
            origin,
            created: Created::parse("2026-11-18T00:00:00Z").unwrap(),
            expected: None,
            profile_format: Some(format.into()),
            display_name: Some("my\u{202E}model".into()),
            source: None,
            max_size: 1024,
        };
        let inbox = data_dir.join(INBOX_DIR_NAME);
        fs::create_dir_all(&inbox).unwrap();
        let staged = stage_model_bundle(&mut &body[..], &inbox, &plan).unwrap();
        (id, staged.path)
    }

    #[test]
    fn a_user_bundle_is_copied_listed_and_removed_from_the_inbox() {
        let dir = tempfile::tempdir().unwrap();
        let (id, bundle) = stage(dir.path(), BODY, "kanakanji-edge", Origin::User);
        let out = import_models(dir.path(), FORMATS);
        assert_eq!(
            out,
            [ImportOutcome {
                name: id.to_string(),
                result: ImportResult::Installed
            }]
        );
        let models = dir.path().join(MODELS_DIR_NAME);
        assert_eq!(fs::read(models.join(id.file_name())).unwrap(), BODY);
        assert!(!bundle.exists());
        assert!(
            !models
                .join(format!("{}{NEW_SUFFIX}", id.file_name()))
                .exists()
        );
        let list = ModelList::read(&models, FORMATS).unwrap();
        let entry = list.get(id).unwrap();
        assert_eq!(entry.format, "kanakanji-edge");
        assert_eq!(entry.size, BODY.len() as u64);
        assert_eq!(entry.display_name.as_deref(), Some("mymodel"));
        // 2 回目は何もしない（束が無い）
        assert!(import_models(dir.path(), FORMATS).is_empty());
    }

    #[test]
    fn unknown_formats_are_rejected_and_the_bundle_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let (_, bundle) = stage(dir.path(), BODY, "gpt", Origin::User);
        let out = import_models(dir.path(), FORMATS);
        assert_eq!(out[0].result, ImportResult::Rejected);
        assert!(!bundle.exists());
        assert!(
            !dir.path()
                .join(MODELS_DIR_NAME)
                .join(MODEL_LIST_FILE_NAME)
                .exists()
        );
    }

    #[test]
    fn a_bundle_changed_after_staging_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (id, bundle) = stage(dir.path(), BODY, "zenz-v3", Origin::User);
        let mut other = BODY.to_vec();
        other[0] ^= 0xff;
        fs::write(bundle.join(id.file_name()), other).unwrap();
        let out = import_models(dir.path(), FORMATS);
        assert_eq!(out[0].result, ImportResult::Rejected);
        assert!(
            !dir.path()
                .join(MODELS_DIR_NAME)
                .join(id.file_name())
                .exists()
        );
    }

    #[test]
    fn a_crash_before_the_list_is_written_is_redone_on_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let (id, _bundle) = stage(dir.path(), BODY, "zenz-v3", Origin::User);
        // 写して置き換えたところで落ちた（一覧も束もそのまま）
        let models = dir.path().join(MODELS_DIR_NAME);
        fs::create_dir_all(&models).unwrap();
        fs::write(models.join(id.file_name()), BODY).unwrap();
        fs::write(
            models.join(format!("{}{NEW_SUFFIX}", id.file_name())),
            b"half",
        )
        .unwrap();
        let out = import_models(dir.path(), FORMATS);
        assert_eq!(out[0].result, ImportResult::Installed);
        assert!(ModelList::read(&models, FORMATS).unwrap().get(id).is_some());
        assert!(
            !models
                .join(format!("{}{NEW_SUFFIX}", id.file_name()))
                .exists()
        );
    }

    #[test]
    fn other_bundles_partials_and_broken_manifests_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = dir.path().join(INBOX_DIR_NAME);
        // 作りかけ
        fs::create_dir_all(inbox.join(format!("user-000000000000{PARTIAL_SUFFIX}"))).unwrap();
        // manifest の無いフォルダ
        fs::create_dir_all(inbox.join("notes")).unwrap();
        // 壊れた manifest（モデルの束か分からないので残す）
        let broken = inbox.join("broken");
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join(MANIFEST_FILE_NAME), "{").unwrap();
        let out = import_models(dir.path(), FORMATS);
        assert_eq!(
            out,
            [ImportOutcome {
                name: "other".into(),
                result: ImportResult::Manifest
            }]
        );
        assert!(broken.exists());
        assert!(inbox.join("notes").exists());
    }

    #[test]
    fn a_broken_list_is_rebuilt() {
        let dir = tempfile::tempdir().unwrap();
        let models = dir.path().join(MODELS_DIR_NAME);
        fs::create_dir_all(&models).unwrap();
        fs::write(models.join(MODEL_LIST_FILE_NAME), "{").unwrap();
        let (id, _) = stage(dir.path(), BODY, "zenz-v3", Origin::User);
        assert_eq!(
            import_models(dir.path(), FORMATS)[0].result,
            ImportResult::Installed
        );
        assert!(ModelList::read(&models, FORMATS).unwrap().get(id).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_the_bundle_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (id, bundle) = stage(dir.path(), BODY, "zenz-v3", Origin::User);
        let elsewhere = dir.path().join("elsewhere");
        fs::write(&elsewhere, BODY).unwrap();
        fs::remove_file(bundle.join(id.file_name())).unwrap();
        std::os::unix::fs::symlink(&elsewhere, bundle.join(id.file_name())).unwrap();
        assert_eq!(
            import_models(dir.path(), FORMATS)[0].result,
            ImportResult::Rejected
        );
        assert!(
            !dir.path()
                .join(MODELS_DIR_NAME)
                .join(id.file_name())
                .exists()
        );
    }
}
