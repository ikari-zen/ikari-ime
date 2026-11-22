//! 辞書とモデルを読む場所の候補（ADR 0080 §3。契約。レーン 0）。
//!
//! - 同梱の場所（読むだけ。アプリの中）と、利用者のデータの場所（今のまま）の 2 つを、上から順に並べる。
//!   部品ごとに、検査に通った最初の場所を使う（§3.2）。選び方と検査はレーン 1（`lib.rs`、`model_select.rs`、`stack`）。
//! - 同梱の場所は、エンジンが**自分の実行ファイルのパスから**決める（R2）。環境変数や設定では変えない（R5）。
//!   OS ごとの差は [`bundled_data_dir_for_exe`] 1 つだけに置く（§3.2）。
//!
//! 判断の置き場所: Rust（engine）。殻は場所を知らない。
//!
//! 同梱の場所が無い配置（開発のビルド、`cargo test`、CLI、評価）では、候補は利用者のデータの場所だけになり、
//! 挙動は今と同じ。シグネチャは契約（レーン 0）。

use std::path::{Path, PathBuf};

use ikari_core::user::UserDir;

use crate::MODELS_DIR_NAME;

/// 同梱の場所の中の、データの置き場所の名前（ADR 0080 §3.1。macOS は `…/IkariEngine.xpc/Contents/Resources/data/`、
/// Windows は `C:\Program Files\ikari-ime\data\`）
pub const BUNDLED_DATA_DIR_NAME: &str = "data";

/// macOS の束の中の、実行ファイルのフォルダの名前（`<束>/Contents/MacOS/<exe>`）
const MACOS_BUNDLE_EXE_DIR: &str = "MacOS";
/// macOS の束の `Contents`
const MACOS_BUNDLE_CONTENTS_DIR: &str = "Contents";
/// macOS の束の `Contents/Resources`
const MACOS_BUNDLE_RESOURCES_DIR: &str = "Resources";

/// 場所の種類（§3.2 の「どちらを使っているか」。状態として返すときにも使う）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocationKind {
    /// 利用者のデータの場所（macOS は `~/Library/Application Support/ikari-ime/`、Windows は `%LOCALAPPDATA%\ikari-ime\`）。
    /// 取り込みや開発者が置いた物。書く先はここだけ
    User,
    /// 同梱の場所（読むだけ。アプリの更新でだけ変わる）
    Bundled,
}

impl LocationKind {
    /// ログに書く固定の語
    pub fn as_word(self) -> &'static str {
        match self {
            LocationKind::User => "user",
            LocationKind::Bundled => "bundled",
        }
    }
}

/// 場所の候補 1 つ。`dir` の下に `dict/` と `models/` がある形（ADR 0004 §2 の配置と同じ）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataLocation {
    pub kind: LocationKind,
    pub dir: PathBuf,
}

impl DataLocation {
    /// 辞書の置き場所（`<dir>/dict`）
    pub fn dict_dir(&self) -> PathBuf {
        UserDir::new(&self.dir).dict_dir()
    }

    /// モデルの置き場所（`<dir>/models`）
    pub fn models_dir(&self) -> PathBuf {
        self.dir.join(MODELS_DIR_NAME)
    }
}

/// 場所の候補の列（上から順。§3.2）。利用者のデータの場所が先、同梱の場所が後
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataLocations {
    user: DataLocation,
    bundled: Option<DataLocation>,
}

impl DataLocations {
    /// 利用者のデータの場所と、同梱の場所（あれば）から、候補の列を作る（§3.2 の順）
    pub fn new(user_data_dir: &Path, bundled_data_dir: Option<&Path>) -> DataLocations {
        DataLocations {
            user: DataLocation {
                kind: LocationKind::User,
                dir: user_data_dir.to_path_buf(),
            },
            bundled: bundled_data_dir.map(|dir| DataLocation {
                kind: LocationKind::Bundled,
                dir: dir.to_path_buf(),
            }),
        }
    }

    /// 上から順の候補
    pub fn iter(&self) -> impl Iterator<Item = &DataLocation> {
        std::iter::once(&self.user).chain(self.bundled.as_ref())
    }

    /// 利用者のデータの場所（必ずある。書く先）
    pub fn user(&self) -> &DataLocation {
        &self.user
    }

    /// 同梱の場所（無いビルド・無い配置では None）
    pub fn bundled(&self) -> Option<&DataLocation> {
        self.bundled.as_ref()
    }

    /// 郵便番号や絵文字の表（`dict/` の下のファイル 1 つ）を読む場所（§3.2 の `postal.tsv`、`emoji.tsv` の行）。
    /// 上から順に、普通のファイルがある最初の場所。どこにも無ければ利用者のデータの場所（読めば空の表。今と同じ）。
    /// 表は読むときに行を検査し、壊れた表は空の表になる（今と同じ）ので、ここでは置かれているかだけを見る
    pub fn table_path(&self, file_name: &str) -> PathBuf {
        self.iter()
            .map(|loc| loc.dict_dir().join(file_name))
            .find(|path| path.is_file())
            .unwrap_or_else(|| self.user.dict_dir().join(file_name))
    }
}

/// どちらの場所の物を使っているか（§3.2 の状態。部品ごと）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataStatus {
    /// 辞書の組（`grammar.bin`、`system.dict`、`vectors.bin`）
    pub dict: LocationKind,
    /// 読むモデル。None は LM を読まない（使わない設定、置かれていない、照合に通らない）。
    /// 自由指定のモデルは利用者のデータの場所だけ（ADR 0060）なので、いつも `User`
    pub model: Option<LocationKind>,
}

/// このエンジンの同梱の場所（実行ファイルのパスから決める。R2）。見つからなければ None
pub fn bundled_data_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    bundled_data_dir_for_exe(&exe)
}

/// 実行ファイルのパスから同梱の場所を決める（OS ごとの差はこの関数だけ。§3.2）。
///
/// ADR 0080 §3.1: macOS は `<束>/Contents/MacOS/<exe>` から `<束>/Contents/Resources/data`（エンジンは
/// `IkariEngine.xpc`）、Windows は `<exe のフォルダ>\data`（`C:\Program Files\ikari-ime\data`）。
/// ディレクトリでなければ None（開発のビルドの `target/…` には無い）。ほかの OS は None
pub fn bundled_data_dir_for_exe(exe: &Path) -> Option<PathBuf> {
    let dir = if cfg!(target_os = "macos") {
        macos_bundle_data_dir(exe)
    } else if cfg!(windows) {
        beside_exe_data_dir(exe)
    } else {
        None
    }?;
    dir.is_dir().then_some(dir)
}

/// `<束>/Contents/MacOS/<exe>` → `<束>/Contents/Resources/data`。束の形でなければ None
fn macos_bundle_data_dir(exe: &Path) -> Option<PathBuf> {
    let exe_dir = exe.parent()?;
    let contents = exe_dir.parent()?;
    let in_bundle = exe_dir.file_name()? == MACOS_BUNDLE_EXE_DIR
        && contents.file_name()? == MACOS_BUNDLE_CONTENTS_DIR;
    in_bundle.then(|| {
        contents
            .join(MACOS_BUNDLE_RESOURCES_DIR)
            .join(BUNDLED_DATA_DIR_NAME)
    })
}

/// `<exe のフォルダ>/data`
fn beside_exe_data_dir(exe: &Path) -> Option<PathBuf> {
    Some(exe.parent()?.join(BUNDLED_DATA_DIR_NAME))
}

/// エンジンが使う候補の列（利用者のデータの場所 → このエンジンの同梱の場所）
pub fn data_locations(user_data_dir: &Path) -> DataLocations {
    DataLocations::new(user_data_dir, bundled_data_dir().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_location_comes_first_then_bundled() {
        let locs = DataLocations::new(Path::new("/u"), Some(Path::new("/b")));
        let kinds: Vec<_> = locs.iter().map(|l| l.kind).collect();
        assert_eq!(kinds, vec![LocationKind::User, LocationKind::Bundled]);
        assert_eq!(locs.user().dir, PathBuf::from("/u"));
        assert_eq!(locs.bundled().map(|l| l.dir.clone()), Some("/b".into()));
        assert_eq!(locs.user().dict_dir(), PathBuf::from("/u/dict"));
        assert_eq!(locs.user().models_dir(), PathBuf::from("/u/models"));
    }

    #[test]
    fn without_bundled_only_the_user_location() {
        let locs = DataLocations::new(Path::new("/u"), None);
        assert_eq!(locs.iter().count(), 1);
        assert_eq!(locs.user().kind, LocationKind::User);
        assert!(locs.bundled().is_none());
    }

    /// 開発のビルド（`target/…` のテスト）には同梱の場所が無く、候補は利用者のデータの場所だけ（今と同じ）
    #[test]
    fn a_development_build_keeps_todays_behavior() {
        assert_eq!(
            bundled_data_dir_for_exe(Path::new("/x/IkariEngine.xpc/Contents/MacOS/IkariEngine")),
            None,
            "the directory does not exist"
        );
        assert_eq!(bundled_data_dir(), None);
        let dir = Path::new("/home/someone/ikari-ime");
        let locs = data_locations(dir);
        assert_eq!(
            locs.iter().cloned().collect::<Vec<_>>(),
            vec![DataLocation {
                kind: LocationKind::User,
                dir: dir.to_path_buf(),
            }]
        );
    }

    #[test]
    fn the_macos_bundle_form_points_into_resources() {
        assert_eq!(
            macos_bundle_data_dir(Path::new(
                "/a/ikari.app/Contents/XPCServices/IkariEngine.xpc/Contents/MacOS/IkariEngine"
            )),
            Some(PathBuf::from(
                "/a/ikari.app/Contents/XPCServices/IkariEngine.xpc/Contents/Resources/data"
            ))
        );
        // 束の形でない（開発のビルド）
        assert_eq!(
            macos_bundle_data_dir(Path::new("/r/target/release/ikari-engine")),
            None
        );
        assert_eq!(macos_bundle_data_dir(Path::new("/MacOS/x")), None);
        assert_eq!(macos_bundle_data_dir(Path::new("x")), None);
    }

    #[test]
    fn the_windows_form_is_beside_the_exe() {
        assert_eq!(
            beside_exe_data_dir(Path::new("/pf/ikari-ime/ikari-engine-host.exe")),
            Some(PathBuf::from("/pf/ikari-ime/data"))
        );
    }

    /// 置かれたディレクトリがあれば、それを同梱の場所にする（この OS の形で）
    #[test]
    fn an_existing_directory_becomes_the_bundled_location() {
        let root = tempfile::tempdir().unwrap();
        let (exe, data) = if cfg!(target_os = "macos") {
            (
                root.path().join("E.xpc/Contents/MacOS/E"),
                root.path().join("E.xpc/Contents/Resources/data"),
            )
        } else {
            (
                root.path().join("ikari-engine-host.exe"),
                root.path().join("data"),
            )
        };
        assert_eq!(bundled_data_dir_for_exe(&exe), None);
        std::fs::create_dir_all(&data).unwrap();
        let expected = (cfg!(target_os = "macos") || cfg!(windows)).then(|| data.clone());
        assert_eq!(bundled_data_dir_for_exe(&exe), expected);
        // ディレクトリでなければ使わない
        std::fs::remove_dir(&data).unwrap();
        std::fs::write(&data, b"").unwrap();
        assert_eq!(bundled_data_dir_for_exe(&exe), None);
    }

    #[test]
    fn tables_come_from_the_first_location_that_has_them() {
        let user = tempfile::tempdir().unwrap();
        let bundled = tempfile::tempdir().unwrap();
        let locs = DataLocations::new(user.path(), Some(bundled.path()));
        let name = "postal.tsv";
        let user_path = user.path().join("dict").join(name);
        let bundled_path = bundled.path().join("dict").join(name);
        // どこにも無い → 利用者の場所（空の表。今と同じ）
        assert_eq!(locs.table_path(name), user_path);
        std::fs::create_dir_all(bundled_path.parent().unwrap()).unwrap();
        std::fs::write(&bundled_path, b"").unwrap();
        assert_eq!(locs.table_path(name), bundled_path);
        std::fs::create_dir_all(user_path.parent().unwrap()).unwrap();
        std::fs::write(&user_path, b"").unwrap();
        assert_eq!(locs.table_path(name), user_path);
        assert_eq!(
            DataLocations::new(user.path(), None).table_path("emoji.tsv"),
            user.path().join("dict").join("emoji.tsv")
        );
    }
}
