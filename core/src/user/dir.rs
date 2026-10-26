//! ユーザーデータの置き場所（ADR 0002 §14, §21.1-8）。

use std::path::{Path, PathBuf};

pub const USER_DIR_ENV: &str = "IKARI_USER_DIR";
pub const LEARNING_FILE_NAME: &str = "learning.tsv";
pub const PROFILE_FILE_NAME: &str = "profile.tsv";
pub const USER_DICT_FILE_NAME: &str = "user_dict.tsv";
pub const INGEST_SOURCES_FILE_NAME: &str = "ingest_sources.tsv";
pub const INGEST_CANDIDATES_FILE_NAME: &str = "ingest_candidates.tsv";
/// エンジン（`ikari-engine`）が保持するロックのファイル名。CLI がエンジンの稼働を調べるのにも使う。
const ENGINE_LOCK_FILE_NAME: &str = "engine.lock";
/// インストールされた辞書（`system.dict` など）のサブディレクトリ名
const DICT_DIR_NAME: &str = "dict";

const APP_DIR_NAME: &str = "ikari-ime";
const MACOS_SUPPORT_DIR: &str = "Library/Application Support";
const XDG_DATA_FALLBACK: &str = ".local/share";

/// ユーザーデータのディレクトリ。ファイルの場所をここで一元化する。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserDir {
    root: PathBuf,
}

/// 既定の置き場所を決めるための環境の値。環境変数の読み出しと分けて、純粋関数でテストできるようにする。
#[derive(Clone, Debug, Default)]
pub struct UserDirEnv {
    pub ikari_user_dir: Option<PathBuf>,
    pub xdg_data_home: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

impl UserDirEnv {
    pub fn from_process() -> Self {
        let get = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Self {
            ikari_user_dir: get(USER_DIR_ENV),
            xdg_data_home: get("XDG_DATA_HOME"),
            home: get("HOME"),
        }
    }
}

impl UserDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 優先順位: 明示の指定、`$IKARI_USER_DIR`、macOS は `~/Library/Application Support/ikari-ime`、
    /// その他は `$XDG_DATA_HOME/ikari-ime`（なければ `~/.local/share/ikari-ime`）。
    /// 決められなければ None（HOME もない環境）。
    pub fn resolve(explicit: Option<&Path>) -> Option<Self> {
        Self::resolve_with(
            explicit,
            &UserDirEnv::from_process(),
            cfg!(target_os = "macos"),
        )
    }

    pub fn resolve_with(explicit: Option<&Path>, env: &UserDirEnv, macos: bool) -> Option<Self> {
        if let Some(p) = explicit {
            return Some(Self::new(p));
        }
        if let Some(p) = &env.ikari_user_dir {
            return Some(Self::new(p));
        }
        if macos {
            let home = env.home.as_ref()?;
            return Some(Self::new(home.join(MACOS_SUPPORT_DIR).join(APP_DIR_NAME)));
        }
        if let Some(x) = &env.xdg_data_home {
            return Some(Self::new(x.join(APP_DIR_NAME)));
        }
        let home = env.home.as_ref()?;
        Some(Self::new(home.join(XDG_DATA_FALLBACK).join(APP_DIR_NAME)))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn learning_tsv(&self) -> PathBuf {
        self.root.join(LEARNING_FILE_NAME)
    }

    pub fn profile_tsv(&self) -> PathBuf {
        self.root.join(PROFILE_FILE_NAME)
    }

    pub fn user_dict_tsv(&self) -> PathBuf {
        self.root.join(USER_DICT_FILE_NAME)
    }

    pub fn ingest_sources_tsv(&self) -> PathBuf {
        self.root.join(INGEST_SOURCES_FILE_NAME)
    }

    pub fn ingest_candidates_tsv(&self) -> PathBuf {
        self.root.join(INGEST_CANDIDATES_FILE_NAME)
    }

    pub fn engine_lock(&self) -> PathBuf {
        self.root.join(ENGINE_LOCK_FILE_NAME)
    }

    pub fn dict_dir(&self) -> PathBuf {
        self.root.join(DICT_DIR_NAME)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(user: Option<&str>, xdg: Option<&str>, home: Option<&str>) -> UserDirEnv {
        UserDirEnv {
            ikari_user_dir: user.map(PathBuf::from),
            xdg_data_home: xdg.map(PathBuf::from),
            home: home.map(PathBuf::from),
        }
    }

    #[test]
    fn explicit_wins() {
        let e = env(Some("/env"), Some("/xdg"), Some("/home/u"));
        let d = UserDir::resolve_with(Some(Path::new("/explicit")), &e, true).unwrap();
        assert_eq!(d.root(), Path::new("/explicit"));
    }

    #[test]
    fn env_var_beats_platform_default() {
        let e = env(Some("/env"), Some("/xdg"), Some("/home/u"));
        assert_eq!(
            UserDir::resolve_with(None, &e, true).unwrap().root(),
            Path::new("/env")
        );
        assert_eq!(
            UserDir::resolve_with(None, &e, false).unwrap().root(),
            Path::new("/env")
        );
    }

    #[test]
    fn macos_default() {
        let e = env(None, Some("/xdg"), Some("/Users/u"));
        let d = UserDir::resolve_with(None, &e, true).unwrap();
        assert_eq!(
            d.root(),
            Path::new("/Users/u/Library/Application Support/ikari-ime")
        );
    }

    #[test]
    fn xdg_default_on_other_platforms() {
        let e = env(None, Some("/xdg"), Some("/home/u"));
        let d = UserDir::resolve_with(None, &e, false).unwrap();
        assert_eq!(d.root(), Path::new("/xdg/ikari-ime"));
        let e = env(None, None, Some("/home/u"));
        let d = UserDir::resolve_with(None, &e, false).unwrap();
        assert_eq!(d.root(), Path::new("/home/u/.local/share/ikari-ime"));
    }

    #[test]
    fn unresolvable_without_home() {
        assert!(UserDir::resolve_with(None, &env(None, None, None), true).is_none());
        assert!(UserDir::resolve_with(None, &env(None, None, None), false).is_none());
    }

    #[test]
    fn file_paths() {
        let d = UserDir::new("/u");
        assert_eq!(d.learning_tsv(), Path::new("/u/learning.tsv"));
        assert_eq!(d.profile_tsv(), Path::new("/u/profile.tsv"));
        assert_eq!(d.user_dict_tsv(), Path::new("/u/user_dict.tsv"));
        assert_eq!(d.ingest_sources_tsv(), Path::new("/u/ingest_sources.tsv"));
        assert_eq!(
            d.ingest_candidates_tsv(),
            Path::new("/u/ingest_candidates.tsv")
        );
        assert_eq!(d.engine_lock(), Path::new("/u/engine.lock"));
        assert_eq!(d.dict_dir(), Path::new("/u/dict"));
    }
}
