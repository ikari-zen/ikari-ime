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
/// Windows でユーザーデータの親になる環境変数（`%LOCALAPPDATA%`。Local。辞書が大きく、ローミングの同期に向かない。2026-11-12 ユーザー決定）
const LOCAL_APPDATA_ENV: &str = "LOCALAPPDATA";

/// 既定の置き場所の決め方を選ぶ OS。Mac 上で他の OS の分岐をテストできるよう、引数で渡す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostOs {
    Macos,
    Windows,
    /// macOS と Windows 以外（XDG に従う）
    Other,
}

impl HostOs {
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Other
        }
    }
}

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
    /// Windows の `%LOCALAPPDATA%`
    pub local_appdata: Option<PathBuf>,
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
            local_appdata: get(LOCAL_APPDATA_ENV),
        }
    }
}

impl UserDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 優先順位: 明示の指定、`$IKARI_USER_DIR`、macOS は `~/Library/Application Support/ikari-ime`、
    /// Windows は `%LOCALAPPDATA%\ikari-ime`（`%LOCALAPPDATA%` がなければ決めない）、
    /// その他は `$XDG_DATA_HOME/ikari-ime`（なければ `~/.local/share/ikari-ime`）。
    /// 決められなければ None（HOME もない環境）。
    pub fn resolve(explicit: Option<&Path>) -> Option<Self> {
        Self::resolve_with(explicit, &UserDirEnv::from_process(), HostOs::current())
    }

    pub fn resolve_with(explicit: Option<&Path>, env: &UserDirEnv, os: HostOs) -> Option<Self> {
        if let Some(p) = explicit {
            return Some(Self::new(p));
        }
        if let Some(p) = &env.ikari_user_dir {
            return Some(Self::new(p));
        }
        match os {
            HostOs::Macos => {
                let home = env.home.as_ref()?;
                return Some(Self::new(home.join(MACOS_SUPPORT_DIR).join(APP_DIR_NAME)));
            }
            HostOs::Windows => {
                let local_appdata = env.local_appdata.as_ref()?;
                return Some(Self::new(local_appdata.join(APP_DIR_NAME)));
            }
            HostOs::Other => {}
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
            local_appdata: None,
        }
    }

    #[test]
    fn explicit_wins() {
        let e = env(Some("/env"), Some("/xdg"), Some("/home/u"));
        let d = UserDir::resolve_with(Some(Path::new("/explicit")), &e, HostOs::Macos).unwrap();
        assert_eq!(d.root(), Path::new("/explicit"));
    }

    #[test]
    fn env_var_beats_platform_default() {
        let e = env(Some("/env"), Some("/xdg"), Some("/home/u"));
        assert_eq!(
            UserDir::resolve_with(None, &e, HostOs::Macos)
                .unwrap()
                .root(),
            Path::new("/env")
        );
        assert_eq!(
            UserDir::resolve_with(None, &e, HostOs::Other)
                .unwrap()
                .root(),
            Path::new("/env")
        );
    }

    #[test]
    fn macos_default() {
        let e = env(None, Some("/xdg"), Some("/Users/u"));
        let d = UserDir::resolve_with(None, &e, HostOs::Macos).unwrap();
        assert_eq!(
            d.root(),
            Path::new("/Users/u/Library/Application Support/ikari-ime")
        );
    }

    #[test]
    fn xdg_default_on_other_platforms() {
        let e = env(None, Some("/xdg"), Some("/home/u"));
        let d = UserDir::resolve_with(None, &e, HostOs::Other).unwrap();
        assert_eq!(d.root(), Path::new("/xdg/ikari-ime"));
        let e = env(None, None, Some("/home/u"));
        let d = UserDir::resolve_with(None, &e, HostOs::Other).unwrap();
        assert_eq!(d.root(), Path::new("/home/u/.local/share/ikari-ime"));
    }

    #[test]
    fn windows_default_is_local_appdata() {
        let e = UserDirEnv {
            local_appdata: Some(PathBuf::from(r"C:\Users\u\AppData\Local")),
            home: Some(PathBuf::from("/ignored")),
            xdg_data_home: Some(PathBuf::from("/ignored")),
            ..UserDirEnv::default()
        };
        let d = UserDir::resolve_with(None, &e, HostOs::Windows).unwrap();
        assert_eq!(
            d.root(),
            Path::new(r"C:\Users\u\AppData\Local").join("ikari-ime")
        );
        // IKARI_USER_DIR は Windows でも優先される
        let e = UserDirEnv {
            ikari_user_dir: Some(PathBuf::from("/env")),
            ..e
        };
        assert_eq!(
            UserDir::resolve_with(None, &e, HostOs::Windows)
                .unwrap()
                .root(),
            Path::new("/env")
        );
        // %LOCALAPPDATA% がなければ決められない（%USERPROFILE% などからは推測しない）
        assert!(UserDir::resolve_with(None, &UserDirEnv::default(), HostOs::Windows).is_none());
    }

    #[test]
    fn unresolvable_without_home() {
        assert!(UserDir::resolve_with(None, &env(None, None, None), HostOs::Macos).is_none());
        assert!(UserDir::resolve_with(None, &env(None, None, None), HostOs::Other).is_none());
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
