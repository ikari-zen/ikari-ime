//! LM のモデルの解決（ADR 0060 §7、ADR 0072 §4・§6・§12.8 の 1。D-4）。
//!
//! 起動のときに、どのモデルを、どの檻の段で読むかを決める。
//! - 設定 `lm.model` は ID だけ（プリセットの ID か `user-<12 桁>`。パスは受けない。条件 4）。無ければ既定のプリセット。
//! - プリセットは、今までどおり `models/` の表の名前のファイルを、アプリに埋めた表（名前、大きさ、sha256）と照合する。
//!   探す順は、利用者のデータの場所の `models/` → 同梱の場所の `models/`（ADR 0080 §3.2）。置かれていない、照合に
//!   通らない（理由はログに書く）ときは次の場所を見る。自由指定は利用者のデータの場所だけ（ADR 0060）。
//! - 自由指定は、`models/MANIFEST.json`（[`ModelList`]）の 1 件を、記録した大きさと sha256 で照合する（破損の検知。
//!   条件 7）。パスは ID から作る（一覧の文字列からは作らない。ADR 0072 §6）。一覧が壊れている・その ID が無いときは
//!   既定のプリセットに戻す（条件 3）。
//! - **檻の段は、一覧の記述から決めない。** 数えた sha256 がアプリに埋めた表にあればプリセット、無ければ自由指定
//!   （段 2 だけ）。条件 2。
//! - 自由指定は、段 2 のある構成でだけ読む（[`user_models_offered`]）。無ければ LM なし（既定に落とさない。条件 6）。
//! - 読み込みの印（`models/.loading`）が、選んだ自由指定のモデルの ID のまま残っていたら、既定のプリセットに戻す
//!   （そのモデルで読み込みが終わらなかった。条件 5）。印を読むのも書くのもエンジンだけ。
//!
//! 判断の置き場所: Rust（エンジン）。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use ikari_resource::{
    KNOWN_MODELS, ModelList, ModelPreset, ResourceError, UserModelId, preset_by_id,
    verify_open_expected,
};
use ikari_stack::{HostCage, LmModelHandle, PromptFormat};

use crate::data_location::{DataLocations, LocationKind};
use crate::model_check::{self, ModelReject};
use crate::settings::Settings;
use crate::{MODEL_FILE_NAME, MODELS_DIR_NAME};

/// プリセットを探す `models/` の候補（上から順。ADR 0080 §3.2）
type ModelDirs = Vec<(LocationKind, PathBuf)>;

/// 読み込みの印のファイルの名前（`models/` の下）
const LOADING_MARK_FILE_NAME: &str = ".loading";
/// 印を読む量の上限（バイト。中身は ID 1 つ）
const MAX_LOADING_MARK_BYTES: u64 = 64;

/// macOS の製品で、自由指定のモデルを出すか。ADR 0077 §6: 檻役の実装がマージされ、§8 の実機の確認が済むまでは
/// 出さない。**既定の判断はリードの確認待ち**（D-4 の PR の説明）
const MACOS_USER_MODELS_RELEASED: bool = false;

/// 設定 `lm.model` の値
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelChoice {
    /// アプリに埋めた表のプリセット
    Preset(&'static ModelPreset),
    /// 取り込んだ自由指定のモデル
    User(UserModelId),
}

impl ModelChoice {
    /// ID だけを受ける（プリセットの ID か `user-<12 桁>`）
    pub fn parse(value: &str) -> Option<ModelChoice> {
        preset_by_id(value)
            .map(ModelChoice::Preset)
            .or_else(|| UserModelId::parse(value).map(ModelChoice::User))
    }

    /// 設定に書く ID
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn id(&self) -> String {
        match self {
            ModelChoice::Preset(p) => p.id.to_owned(),
            ModelChoice::User(id) => id.to_string(),
        }
    }
}

/// 自由指定に受ける形式の語（`PromptFormat` の語。取り込みと一覧の読みに渡す）
pub(crate) fn format_words() -> Vec<&'static str> {
    PromptFormat::ALL.iter().map(|f| f.as_str()).collect()
}

/// この構成で、自由指定のモデルを出すか（段 2 があるか。ADR 0072 §4 の 2、ADR 0077 §6）。
/// `lm_host`: 別プロセスで動かすか（同じプロセスでは檻が無いので、自由指定は読まない）
pub(crate) fn user_models_offered(lm_host: bool) -> bool {
    lm_host && platform_offers_user_models()
}

/// Windows は段 2（AppContainer。張れなければ起動しない）。macOS の製品は檻役（[`MACOS_USER_MODELS_RELEASED`]）。
/// ほか（Linux、macOS の開発用のビルド）は段 2 が無いので出さない
fn platform_offers_user_models() -> bool {
    if cfg!(windows) {
        true
    } else if crate::lm_cage::product_uses_cage() {
        MACOS_USER_MODELS_RELEASED
    } else {
        false
    }
}

/// モデルの出どころ（檻の段の決め手）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelOrigin {
    /// アプリに埋めた表の sha256 で確かめたもの
    Preset,
    /// 利用者が指定したもの（ファイル、URL。ADR 0060 の段 4・5）
    User,
}

/// 解決したモデル
#[derive(Debug, Clone)]
pub(crate) struct ResolvedModel {
    /// 読むモデル（照合したハンドル。FC-3）。None なら LM なし
    pub(crate) model: Option<LmModelHandle>,
    /// プロンプトの形式（自由指定のときだけ。プリセットは profile の既定）
    pub(crate) format: Option<PromptFormat>,
    /// ログの `lm_reject` に書く、ファイルの名前（パスは書かない）と理由。通したとき、LM を使わないときは空
    pub(crate) rejects: Vec<(String, ModelReject)>,
    pub(crate) origin: ModelOrigin,
    /// 読み込みの印を書く自由指定のモデル（出どころが自由指定で、読むときだけ）
    pub(crate) loading: Option<UserModelId>,
    /// 読むモデルを置いていた場所（ADR 0080 §3.2 の状態）。LM なしなら None
    pub(crate) location: Option<LocationKind>,
}

/// 起動のときに読むモデルを決める。`user_models_allowed` は [`user_models_offered`] の値。
/// 読み込みの印と自由指定のモデルは、利用者のデータの場所（`locations.user()`）だけを見る
pub(crate) fn resolve_model(
    lm_requested: bool,
    locations: &DataLocations,
    settings: &Settings,
    user_models_allowed: bool,
) -> ResolvedModel {
    let models = locations.user().models_dir();
    let dirs: ModelDirs = locations
        .iter()
        .map(|loc| (loc.kind, loc.models_dir()))
        .collect();
    let mark = read_loading_mark(&models);
    let user = match settings.lm_model {
        Some(ModelChoice::User(id)) => Some(id),
        _ => None,
    };
    // 印は、今選んでいる自由指定のモデルのものだけを意味を持つ。ほかは消す（設定を変えたら、もう一度試せる）
    if mark.is_some() && mark != user.map(Some) {
        clear_loading_mark(&models);
    }
    let preset = match settings.lm_model {
        Some(ModelChoice::Preset(p)) => Some(p),
        _ => None,
    };
    let Some(id) = user else {
        return resolve_preset(lm_requested, &dirs, preset, Vec::new());
    };
    if !lm_requested {
        return resolve_preset(false, &dirs, None, Vec::new());
    }
    if mark == Some(Some(id)) {
        let rejects = vec![(id.file_name(), ModelReject::LoadingMark)];
        return resolve_preset(true, &dirs, None, rejects);
    }
    resolve_user(id, &models, &dirs, user_models_allowed)
}

/// プリセット（既定は `MODEL_FILE_NAME`）。今までと同じ照合を、`dirs` の上から順に、通った最初の場所で
fn resolve_preset(
    lm_requested: bool,
    dirs: &[(LocationKind, PathBuf)],
    preset: Option<&'static ModelPreset>,
    mut rejects: Vec<(String, ModelReject)>,
) -> ResolvedModel {
    let file_name = preset.map_or(MODEL_FILE_NAME, |p| p.file_name);
    let mut chosen = None;
    for (kind, models) in dirs {
        // 読む前に、既知のモデルの表と照合する。通らなければ次の場所、どこも通らなければ LM なしで起動する
        // （理由はログに書く。LM のないビルドでも照合して記録する。読むのは LM のあるビルドだけ）
        let (model, reject) = model_check::select_lm_model(
            lm_requested,
            &models.join(file_name),
            file_name,
            KNOWN_MODELS,
        );
        rejects.extend(reject.map(|r| (file_name.to_owned(), r)));
        if let Some(model) = model {
            chosen = Some((for_location(model, *kind), *kind));
            break;
        }
    }
    let (model, location) = chosen.unzip();
    ResolvedModel {
        model,
        format: None,
        rejects,
        origin: ModelOrigin::Preset,
        loading: None,
        location,
    }
}

/// 同梱の場所のモデルには、段 2 の子への許可を足さない印を付ける（Windows の `Program Files` は継承の読み取りで
/// 読め、許可を足す権限が無い。足そうとすると段 2 の起動が失敗する。#578 の C4）
fn for_location(model: LmModelHandle, kind: LocationKind) -> LmModelHandle {
    match kind {
        LocationKind::User => model,
        LocationKind::Bundled => model.readable_without_cage_grant(),
    }
}

/// 自由指定のモデル（`models` は利用者のデータの場所の `models/`）。一覧が壊れている・ID が無いときは既定のプリセット
/// （`dirs` の順）、照合に通らない・段 2 が無いときは LM なし
fn resolve_user(
    id: UserModelId,
    models: &Path,
    dirs: &[(LocationKind, PathBuf)],
    user_models_allowed: bool,
) -> ResolvedModel {
    let file_name = id.file_name();
    let fallback = |reason| resolve_preset(true, dirs, None, vec![(file_name.clone(), reason)]);
    let list = match ModelList::read(models, &format_words()) {
        Ok(list) => list,
        Err(_) => return fallback(ModelReject::Manifest),
    };
    let Some(entry) = list.get(id) else {
        return fallback(ModelReject::UnknownName);
    };
    let Some(format) = PromptFormat::parse(&entry.format) else {
        return fallback(ModelReject::Manifest);
    };
    // 段は、数えた sha256 が表にあるかで決める（一覧の記述は信用しない）。照合で一覧の値と同じだと確かめるので、
    // ここでは一覧の値で表を引き、表に無いものは段 2 が無ければ数える前に断る
    let in_table = KNOWN_MODELS
        .iter()
        .any(|k| k.sha256 == entry.sha256 && k.size == entry.size);
    let origin = if in_table {
        ModelOrigin::Preset
    } else {
        ModelOrigin::User
    };
    let no_lm = |reason| ResolvedModel {
        model: None,
        format: None,
        rejects: vec![(file_name.clone(), reason)],
        origin,
        loading: None,
        location: None,
    };
    if origin == ModelOrigin::User && !user_models_allowed {
        return no_lm(ModelReject::NoCage);
    }
    let path = models.join(&file_name);
    // 開く前に種類を見る（FIFO などを open して止まらない）
    match fs::metadata(&path) {
        Ok(meta) if meta.is_file() => {}
        _ => return no_lm(ModelReject::Unreadable),
    }
    let Ok(handle) = LmModelHandle::open(&path) else {
        return no_lm(ModelReject::Unreadable);
    };
    if let Err(e) =
        verify_open_expected(handle.file(), &path, &file_name, entry.size, &entry.sha256)
    {
        return no_lm(reject_of(&e));
    }
    ResolvedModel {
        model: Some(handle),
        format: Some(format),
        rejects: Vec::new(),
        origin,
        loading: (origin == ModelOrigin::User).then_some(id),
        location: Some(LocationKind::User),
    }
}

fn reject_of(e: &ResourceError) -> ModelReject {
    match e {
        ResourceError::Truncated { .. } | ResourceError::SizeMismatch { .. } => ModelReject::Size,
        ResourceError::Sha256Mismatch { .. } => ModelReject::Sha256,
        _ => ModelReject::Unreadable,
    }
}

/// 出どころから、求める檻の段を決める（ADR 0072 §12.8 の 1。プリセットは段 1（`PreferAppContainer` に替えるかは
/// #424 のあと）、自由指定は段 2 だけ）
pub(crate) fn cage_for(origin: ModelOrigin) -> HostCage {
    match origin {
        ModelOrigin::Preset => HostCage::Standard,
        ModelOrigin::User => HostCage::RequireAppContainer,
    }
}

fn loading_mark_path(models: &Path) -> PathBuf {
    models.join(LOADING_MARK_FILE_NAME)
}

/// 印を読む。無ければ None、読めない・形の合わない印は `Some(None)`
fn read_loading_mark(models: &Path) -> Option<Option<UserModelId>> {
    let path = loading_mark_path(models);
    match ikari_textfile::read_to_string_limited(&path, MAX_LOADING_MARK_BYTES) {
        Ok(text) => Some(UserModelId::parse(text.trim())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(None),
    }
}

fn clear_loading_mark(models: &Path) {
    let _ = fs::remove_file(loading_mark_path(models));
}

/// 読み込みの印（条件 5）。起動のときに書き（子を起こす前）、最初に読み込めたら消す
#[derive(Debug, Default)]
pub(crate) struct LoadingMark {
    path: OnceLock<PathBuf>,
    cleared: AtomicBool,
}

impl LoadingMark {
    /// 印を書いて、消す場所を覚える。書けなければ覚えない（印が無いのと同じ）
    pub(crate) fn arm(&self, data_dir: &Path, id: UserModelId) {
        let models = data_dir.join(MODELS_DIR_NAME);
        let path = loading_mark_path(&models);
        if fs::write(&path, id.to_string()).is_ok() {
            let _ = self.path.set(path);
        }
    }

    /// LM が読み込めた。印があれば 1 回だけ消す
    pub(crate) fn loaded(&self) {
        if let Some(path) = self.path.get()
            && !self.cleared.swap(true, Ordering::AcqRel)
        {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use ikari_resource::{
        Created, MODEL_LIST_FILE_NAME, Manifest, Origin, UserModelEntry, sha256_reader,
    };

    use super::*;

    const BODY: &[u8] = b"GGUF a user model";

    struct Fixture {
        dir: tempfile::TempDir,
        id: UserModelId,
    }

    impl Fixture {
        fn models(&self) -> PathBuf {
            self.dir.path().join(MODELS_DIR_NAME)
        }

        fn settings(&self) -> Settings {
            Settings {
                lm_model: Some(ModelChoice::User(self.id)),
                ..Settings::default()
            }
        }

        fn resolve(&self, allowed: bool) -> ResolvedModel {
            resolve_model(true, &user_only(self.dir.path()), &self.settings(), allowed)
        }
    }

    /// 自由指定のモデルを 1 つ入れた（取り込みの済んだ）データの場所
    fn installed(format: &str) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let models = dir.path().join(MODELS_DIR_NAME);
        fs::create_dir_all(&models).unwrap();
        let sha = sha256_reader(BODY).unwrap();
        let id = UserModelId::from_sha256(&sha).unwrap();
        fs::File::create(models.join(id.file_name()))
            .unwrap()
            .write_all(BODY)
            .unwrap();
        let mut list = ModelList::default();
        list.upsert(UserModelEntry {
            id,
            size: BODY.len() as u64,
            sha256: sha,
            format: format.into(),
            display_name: None,
        });
        list.write(&models, Created::parse("2026-11-18T00:00:00Z").unwrap())
            .unwrap();
        Fixture { dir, id }
    }

    /// 同梱の場所の無い候補の列（開発のビルドと同じ）
    fn user_only(dir: &Path) -> DataLocations {
        DataLocations::new(dir, None)
    }

    fn words(r: &ResolvedModel) -> Vec<ModelReject> {
        r.rejects.iter().map(|(_, w)| *w).collect()
    }

    #[test]
    fn user_models_require_level_2_and_presets_stay_at_level_1() {
        assert_eq!(cage_for(ModelOrigin::Preset), HostCage::Standard);
        assert_eq!(cage_for(ModelOrigin::User), HostCage::RequireAppContainer);
    }

    #[test]
    fn lm_model_setting_selects_a_user_model() {
        let f = installed("kanakanji-edge");
        let r = f.resolve(true);
        assert!(r.model.is_some());
        assert_eq!(r.origin, ModelOrigin::User);
        assert_eq!(cage_for(r.origin), HostCage::RequireAppContainer);
        assert_eq!(r.format, Some(PromptFormat::KanakanjiEdge));
        assert_eq!(r.loading, Some(f.id));
        assert!(r.rejects.is_empty());
    }

    #[test]
    fn a_leftover_loading_mark_falls_back_to_the_default() {
        let f = installed("zenz-v3");
        let mark = LoadingMark::default();
        mark.arm(f.dir.path(), f.id);
        // 読み込めないまま終わった → 次の起動は既定のプリセット（ここでは置いていないので LM なし）
        let r = f.resolve(true);
        assert_eq!(words(&r), [ModelReject::LoadingMark]);
        assert_eq!(r.origin, ModelOrigin::Preset);
        assert!(r.loading.is_none());
        // 印は残る（同じ設定のままでは、もう一度は試さない）
        assert!(loading_mark_path(&f.models()).exists());
        // 設定を替えると印は消え、戻せばもう一度試せる
        resolve_model(true, &user_only(f.dir.path()), &Settings::default(), true);
        assert!(!loading_mark_path(&f.models()).exists());
        assert!(f.resolve(true).model.is_some());
    }

    #[test]
    fn the_mark_is_removed_once_the_model_loads() {
        let f = installed("zenz-v3");
        let mark = LoadingMark::default();
        mark.arm(f.dir.path(), f.id);
        assert!(loading_mark_path(&f.models()).exists());
        mark.loaded();
        assert!(!loading_mark_path(&f.models()).exists());
        assert!(f.resolve(true).model.is_some());
    }

    #[test]
    fn user_models_are_not_used_without_level_2() {
        let f = installed("zenz-v3");
        let r = f.resolve(false);
        assert!(r.model.is_none());
        assert_eq!(words(&r), [ModelReject::NoCage]);
        // 既定のプリセットにも落とさない（LM なし）
        assert_eq!(r.origin, ModelOrigin::User);
        // 同じプロセスで動かす構成は、段 2 が無い
        assert!(!user_models_offered(false));
    }

    #[test]
    fn a_list_saying_preset_does_not_make_an_unknown_file_a_preset() {
        let f = installed("zenz-v3");
        let path = f.models().join(MODEL_LIST_FILE_NAME);
        let mut manifest = Manifest::read(&path).unwrap();
        manifest.origin = Origin::Preset;
        manifest.write(&path).unwrap();
        let r = f.resolve(true);
        assert_eq!(r.origin, ModelOrigin::User);
        assert_eq!(cage_for(r.origin), HostCage::RequireAppContainer);
    }

    #[test]
    fn a_changed_file_is_not_read() {
        let f = installed("zenz-v3");
        let mut other = BODY.to_vec();
        other[0] ^= 0xff;
        fs::write(f.models().join(f.id.file_name()), &other).unwrap();
        let r = f.resolve(true);
        assert!(r.model.is_none());
        assert_eq!(words(&r), [ModelReject::Sha256]);
        other.push(0);
        fs::write(f.models().join(f.id.file_name()), &other).unwrap();
        assert_eq!(words(&f.resolve(true)), [ModelReject::Size]);
    }

    #[test]
    fn a_broken_or_missing_list_falls_back_to_the_default() {
        let f = installed("zenz-v3");
        let path = f.models().join(MODEL_LIST_FILE_NAME);
        fs::write(&path, "{").unwrap();
        let r = f.resolve(true);
        assert_eq!(words(&r), [ModelReject::Manifest]);
        assert_eq!(r.origin, ModelOrigin::Preset);
        fs::remove_file(&path).unwrap();
        assert_eq!(words(&f.resolve(true)), [ModelReject::UnknownName]);
    }

    #[test]
    fn without_lm_nothing_is_read() {
        let f = installed("zenz-v3");
        let r = resolve_model(false, &user_only(f.dir.path()), &f.settings(), true);
        assert!(r.model.is_none());
        assert!(r.rejects.is_empty());
    }

    /// プリセットは、利用者の場所 → 同梱の場所の順に見る。通らなければ理由を残して次の場所（ADR 0080 §3.2）
    #[test]
    fn the_preset_is_looked_up_in_the_user_location_then_the_bundled_one() {
        let user = tempfile::tempdir().unwrap();
        let bundled = tempfile::tempdir().unwrap();
        let locs = DataLocations::new(user.path(), Some(bundled.path()));
        let put = |dir: &Path| {
            let models = dir.join(MODELS_DIR_NAME);
            fs::create_dir_all(&models).unwrap();
            // 表の sha256 に合わない（壊れた）モデル
            fs::write(models.join(MODEL_FILE_NAME), b"GGUF broken").unwrap();
        };
        // どこにも無い → 黙って LM なし（今と同じ）
        let r = resolve_model(true, &locs, &Settings::default(), true);
        assert!(r.model.is_none() && r.rejects.is_empty());
        assert_eq!(r.location, None);
        // 同梱の場所だけにある（壊れている）→ 同梱の場所まで見て、理由を 1 つ残す
        put(bundled.path());
        let r = resolve_model(true, &locs, &Settings::default(), true);
        assert_eq!(r.rejects.len(), 1);
        assert!(r.model.is_none());
        // 両方にある（両方壊れている）→ 利用者の場所で落ちて、同梱の場所も見る
        put(user.path());
        let r = resolve_model(true, &locs, &Settings::default(), true);
        assert_eq!(r.rejects.len(), 2);
        assert!(r.model.is_none() && r.location.is_none());
        // LM を使わない設定なら、どこも見ない
        let r = resolve_model(false, &locs, &Settings::default(), true);
        assert!(r.rejects.is_empty());
    }

    /// 自由指定のモデルは利用者の場所だけ。一覧が壊れていて既定のプリセットに戻すときは、同梱の場所も見る
    #[test]
    fn a_user_model_falls_back_to_the_preset_in_either_location() {
        let f = installed("zenz-v3");
        let bundled = tempfile::tempdir().unwrap();
        let models = bundled.path().join(MODELS_DIR_NAME);
        fs::create_dir_all(&models).unwrap();
        fs::write(models.join(MODEL_FILE_NAME), b"GGUF broken").unwrap();
        let locs = DataLocations::new(f.dir.path(), Some(bundled.path()));
        let r = resolve_model(true, &locs, &f.settings(), true);
        assert!(r.model.is_some());
        assert_eq!(r.location, Some(LocationKind::User));
        fs::write(f.models().join(MODEL_LIST_FILE_NAME), "{").unwrap();
        let r = resolve_model(true, &locs, &f.settings(), true);
        assert_eq!(words(&r)[0], ModelReject::Manifest);
        assert_eq!(r.rejects.len(), 2, "the bundled preset was checked too");
    }

    #[test]
    fn only_a_bundled_model_skips_the_cage_grant() {
        let f = installed("zenz-v3");
        let path = f.models().join(f.id.file_name());
        let open = || LmModelHandle::open(&path).unwrap();
        assert!(for_location(open(), LocationKind::User).needs_cage_grant());
        assert!(!for_location(open(), LocationKind::Bundled).needs_cage_grant());
        // 自由指定のモデル（利用者の場所）は、今までどおり許可を足す
        assert!(f.resolve(true).model.unwrap().needs_cage_grant());
    }

    #[test]
    fn the_default_preset_is_the_fixed_file() {
        let dir = tempfile::tempdir().unwrap();
        // ファイルが無ければ黙って LM なし（今までと同じ）
        let r = resolve_model(true, &user_only(dir.path()), &Settings::default(), true);
        assert!(r.model.is_none() && r.rejects.is_empty());
        assert_eq!(r.origin, ModelOrigin::Preset);
        let preset = ModelChoice::parse("zenz-small").unwrap();
        assert!(matches!(preset, ModelChoice::Preset(p) if p.file_name == MODEL_FILE_NAME));
    }
}
