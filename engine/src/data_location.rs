//! 辞書とモデルを読む場所の候補（ADR 0080 §3。契約。レーン 0）。
//!
//! - 同梱の場所（読むだけ。アプリの中）と、利用者のデータの場所（今のまま）の 2 つを、上から順に並べる。
//!   部品ごとに、検査に通った最初の場所を使う（§3.2）。選び方と検査はレーン 1（`lib.rs`、`model_select.rs`、`stack`）。
//! - 同梱の場所は、エンジンが**自分の実行ファイルのパスから**決める（R2）。環境変数や設定では変えない（R5）。
//!   OS ごとの差は [`bundled_data_dir_for_exe`] 1 つだけに置く（§3.2）。
//!
//! 判断の置き場所: Rust（engine）。殻は場所を知らない。
//!
//! 今の中身はスタブ: 同梱の場所は無い（[`bundled_data_dir_for_exe`] は `None`）。候補は利用者のデータの場所だけになり、
//! 挙動は今と同じ。シグネチャは契約（レーン 1 は中身だけを埋める）。

use std::path::{Path, PathBuf};

/// 同梱の場所の中の、データの置き場所の名前（ADR 0080 §3.1。macOS は `…/IkariEngine.xpc/Contents/Resources/data/`、
/// Windows は `C:\Program Files\ikari-ime\data\`）
pub const BUNDLED_DATA_DIR_NAME: &str = "data";

/// 場所の種類（§3.2 の「どちらを使っているか」。状態として返すときにも使う）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocationKind {
    /// 利用者のデータの場所（macOS は `~/Library/Application Support/ikari-ime/`、Windows は `%LOCALAPPDATA%\ikari-ime\`）。
    /// 取り込みや開発者が置いた物。書く先はここだけ
    User,
    /// 同梱の場所（読むだけ。アプリの更新でだけ変わる）
    Bundled,
}

/// 場所の候補 1 つ。`dir` の下に `dict/` と `models/` がある形（ADR 0004 §2 の配置と同じ）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataLocation {
    pub kind: LocationKind,
    pub dir: PathBuf,
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
}

/// このエンジンの同梱の場所（実行ファイルのパスから決める。R2）。見つからなければ None
pub fn bundled_data_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    bundled_data_dir_for_exe(&exe)
}

/// 実行ファイルのパスから同梱の場所を決める（OS ごとの差はこの関数だけ。§3.2）。
///
/// レーン 1 で埋める形（ADR 0080 §3.1）: macOS は `<束>/Contents/MacOS/<exe>` から `<束>/Contents/Resources/data`、
/// Windows は `<exe のフォルダ>\data`。ディレクトリでなければ None。
///
/// スタブ: 常に None（同梱の場所は無い。今の挙動）。
pub fn bundled_data_dir_for_exe(_exe: &Path) -> Option<PathBuf> {
    None
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
    }

    #[test]
    fn without_bundled_only_the_user_location() {
        let locs = DataLocations::new(Path::new("/u"), None);
        assert_eq!(locs.iter().count(), 1);
        assert_eq!(locs.user().kind, LocationKind::User);
        assert!(locs.bundled().is_none());
    }

    /// スタブの挙動は今と同じ: 同梱の場所は無く、候補は利用者のデータの場所だけ
    #[test]
    fn stub_keeps_todays_behavior() {
        assert_eq!(
            bundled_data_dir_for_exe(Path::new("/x/IkariEngine.xpc/Contents/MacOS/IkariEngine")),
            None
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
}
