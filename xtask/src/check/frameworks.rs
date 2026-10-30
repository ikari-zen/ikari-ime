//! swift test の前に、`macos/Frameworks` の XCFramework が古くないことを確かめる（issue #148）。
//!
//! XCFramework（Rust の staticlib とヘッダ。gitignore）が残っていると、`swift test` はそれを使う。Rust 側のヘッダや実装を変えても
//! XCFramework が古いままだと、検査が環境によって通ったり落ちたりする（#146: 古い XCFramework のとき
//! `cannot find type 'IkariShellHost'` で失敗。`build.sh` で作り直すと通った）。ここで古さをはっきり失敗にする。
//! 毎回は作り直さない（数分かかる）。作り直しは `macos/scripts/build.sh` で、失敗のメッセージにその手順を書く。
//!
//! 古さの判定（どちらかに当たれば古い）:
//! - XCFramework の中のヘッダが、Rust 側のヘッダ（`shell-ffi/include/*.h` など）と内容が違う
//! - XCFramework の中の staticlib（`.a`）より、その crate と依存する crate（workspace の中のもの）のソースのほうが新しい
//!
//! XCFramework がなければ検査しない（`Package.swift` が FFI なしのスタブの形にして swift test を通す）。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::data::run::{Failure, capture, cargo, log, warn};

const FRAMEWORKS_DIR: &str = "macos/Frameworks";
const BUILD_SCRIPT: &str = "macos/scripts/build.sh --signing team --release";
const XCFRAMEWORK_EXT: &str = "xcframework";
const STATICLIB_EXT: &str = "a";
/// ソースの走査で入らないディレクトリ（成果物と、staticlib に入らないテスト類）
const SKIPPED_DIRS: [&str; 5] = ["target", "tests", "benches", "examples", ".git"];

/// 検査する XCFramework。名前は `macos/scripts/build.sh` の `make_xcframework` と同じ。
struct Spec {
    /// XCFramework の名前（`<名前>.xcframework`）
    name: &'static str,
    /// Rust 側のヘッダ（リポジトリのルートから）
    header: &'static str,
    /// staticlib を作る crate（workspace のパッケージ名）
    package: &'static str,
}

const SPECS: [Spec; 2] = [
    Spec {
        name: "IkariShellFFI",
        header: "shell-ffi/include/ikari_shell.h",
        package: "ikari-shell-ffi",
    },
    Spec {
        name: "IkariEngineFFI",
        header: "engine-ffi/include/ikari_engine.h",
        package: "ikari-engine-ffi",
    },
];

/// どの XCFramework も新しい（またはない）なら Ok。古いものがあれば、作り直しの手順つきで失敗する。
pub fn verify(root: &Path) -> Result<(), Failure> {
    log("XCFramework の鮮度（macos/Frameworks）");
    let mut metadata: Option<Vec<Package>> = None;
    let mut stale = Vec::new();
    for spec in &SPECS {
        let framework = root
            .join(FRAMEWORKS_DIR)
            .join(format!("{}.{XCFRAMEWORK_EXT}", spec.name));
        if !framework.is_dir() {
            warn(&format!(
                "{} がありません。FFI なしのスタブで swift test します（FFI まで確かめるなら {BUILD_SCRIPT}）",
                framework.display()
            ));
            continue;
        }
        if metadata.is_none() {
            metadata = workspace_packages(root);
        }
        let Some(packages) = &metadata else {
            return Err(Failure::new(
                "cargo metadata を読めないので、XCFramework の鮮度を確かめられません",
            ));
        };
        let sources = source_files(packages, spec.package);
        if let Some(reason) = stale_reason(&framework, &root.join(spec.header), &sources)? {
            stale.push(format!("{}.{XCFRAMEWORK_EXT}: {reason}", spec.name));
        }
    }
    if stale.is_empty() {
        return Ok(());
    }
    Err(Failure::new(format!(
        "macos/Frameworks の XCFramework が古いので swift test を止めます（古いまま通ると検査が環境によって変わる。#148）\n  {}\n作り直してから、もう一度流してください: {BUILD_SCRIPT}",
        stale.join("\n  ")
    )))
}

/// 古い理由（新しければ None）。`header` は Rust 側のヘッダ、`sources` は staticlib の元になるソースのファイル。
fn stale_reason(
    framework: &Path,
    header: &Path,
    sources: &[PathBuf],
) -> Result<Option<String>, Failure> {
    let header_name = header.file_name().unwrap_or_default();
    let packaged_header = find_file(framework, &|path| path.file_name() == Some(header_name));
    let Some(packaged_header) = packaged_header else {
        return Ok(Some(format!(
            "中に {} がありません",
            header_name.to_string_lossy()
        )));
    };
    let current = std::fs::read(header)
        .map_err(|e| Failure::new(format!("{} を読めません: {e}", header.display())))?;
    if std::fs::read(&packaged_header).ok().as_deref() != Some(current.as_slice()) {
        return Ok(Some(format!(
            "ヘッダ {} が {} と違う",
            packaged_header
                .strip_prefix(framework)
                .unwrap_or(&packaged_header)
                .display(),
            header.display()
        )));
    }
    let lib = find_file(framework, &|path| {
        path.extension().is_some_and(|ext| ext == STATICLIB_EXT)
    });
    let Some(lib) = lib else {
        return Ok(Some("中に staticlib（.a）がありません".to_owned()));
    };
    let lib_time = modified(&lib)?;
    let newest = sources
        .iter()
        .filter_map(|path| Some((path, modified(path).ok()?)))
        .max_by_key(|(_, time)| *time);
    if let Some((path, time)) = newest
        && time > lib_time
    {
        return Ok(Some(format!(
            "staticlib より新しいソースがある（{}）",
            path.display()
        )));
    }
    Ok(None)
}

fn modified(path: &Path) -> Result<SystemTime, Failure> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map_err(|e| Failure::new(format!("{} の更新時刻を読めません: {e}", path.display())))
}

/// ディレクトリの下から、条件に合う最初のファイル（名前順）を探す。
fn find_file(dir: &Path, matches: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if let Some(found) = find_file(&path, matches) {
                return Some(found);
            }
        } else if matches(&path) {
            return Some(path);
        }
    }
    None
}

/// workspace のパッケージ: 名前、crate のディレクトリ、依存する workspace のパッケージ名（dev は除く）。
struct Package {
    name: String,
    dir: PathBuf,
    deps: Vec<String>,
}

fn workspace_packages(root: &Path) -> Option<Vec<Package>> {
    let json = capture(cargo(root).args(["metadata", "--format-version", "1", "--no-deps"]))?;
    parse_metadata(&json)
}

fn parse_metadata(json: &str) -> Option<Vec<Package>> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let packages = value.get("packages")?.as_array()?;
    packages
        .iter()
        .map(|package| {
            let name = package.get("name")?.as_str()?.to_owned();
            let manifest = PathBuf::from(package.get("manifest_path")?.as_str()?);
            let deps = package
                .get("dependencies")?
                .as_array()?
                .iter()
                // path を持つものが workspace の中の依存。dev-dependencies は staticlib に入らない
                .filter(|dep| {
                    dep.get("path").is_some()
                        && dep.get("kind").is_none_or(|k| k.is_null() || k == "build")
                })
                .filter_map(|dep| Some(dep.get("name")?.as_str()?.to_owned()))
                .collect();
            Some(Package {
                name,
                dir: manifest.parent()?.to_path_buf(),
                deps,
            })
        })
        .collect()
}

/// `package` と、それが（推移的に）依存する workspace のパッケージ。
fn closure<'a>(packages: &'a [Package], package: &str) -> Vec<&'a Package> {
    let mut found: Vec<&Package> = Vec::new();
    let mut todo = vec![package];
    while let Some(name) = todo.pop() {
        if found.iter().any(|p| p.name == name) {
            continue;
        }
        if let Some(package) = packages.iter().find(|p| p.name == name) {
            found.push(package);
            todo.extend(package.deps.iter().map(String::as_str));
        }
    }
    found
}

/// `package` と、それが依存する workspace の crate のソースのファイル。
fn source_files(packages: &[Package], package: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for found in closure(packages, package) {
        walk(&found.dir, &mut files);
    }
    files
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            let skipped = path
                .file_name()
                .is_some_and(|n| SKIPPED_DIRS.iter().any(|s| n == *s));
            if !skipped {
                walk(&path, files);
            }
        } else {
            files.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::time::Duration;

    use super::*;

    /// 偽の XCFramework とソースを作る。返す値: (ルート, framework, header, source)。
    struct Fixture {
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let fx = Self { dir };
            std::fs::create_dir_all(fx.framework().join("macos-arm64/Headers/Mod")).unwrap();
            std::fs::create_dir_all(fx.header().parent().unwrap()).unwrap();
            std::fs::write(fx.header(), "v1").unwrap();
            std::fs::write(
                fx.framework().join("macos-arm64/Headers/Mod/ikari_x.h"),
                "v1",
            )
            .unwrap();
            std::fs::write(fx.framework().join("macos-arm64/libx.a"), "lib").unwrap();
            std::fs::create_dir_all(fx.source().parent().unwrap()).unwrap();
            std::fs::write(fx.source(), "fn main() {}").unwrap();
            // ソース、ヘッダは過去、staticlib は今
            fx.set_time(&fx.source(), -100);
            fx.set_time(&fx.header(), -100);
            fx.set_time(&fx.framework().join("macos-arm64/libx.a"), 0);
            fx
        }
        fn framework(&self) -> PathBuf {
            self.dir.path().join("X.xcframework")
        }
        fn header(&self) -> PathBuf {
            self.dir.path().join("include/ikari_x.h")
        }
        fn source(&self) -> PathBuf {
            self.dir.path().join("src/lib.rs")
        }
        fn set_time(&self, path: &Path, offset_secs: i64) {
            let base = SystemTime::now();
            let time = if offset_secs < 0 {
                base - Duration::from_secs(offset_secs.unsigned_abs())
            } else {
                base + Duration::from_secs(offset_secs.unsigned_abs())
            };
            File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(time)
                .unwrap();
        }
        fn reason(&self) -> Option<String> {
            std::fs::create_dir_all(self.header().parent().unwrap()).unwrap();
            stale_reason(&self.framework(), &self.header(), &[self.source()]).unwrap()
        }
    }

    #[test]
    fn fresh_framework_passes() {
        assert_eq!(Fixture::new().reason(), None);
    }

    #[test]
    fn changed_header_is_stale() {
        let fx = Fixture::new();
        std::fs::write(fx.header(), "v2").unwrap();
        let reason = fx.reason().unwrap();
        assert!(reason.contains("ヘッダ"), "{reason}");
    }

    #[test]
    fn newer_source_is_stale() {
        let fx = Fixture::new();
        fx.set_time(&fx.source(), 100);
        let reason = fx.reason().unwrap();
        assert!(reason.contains("新しいソース"), "{reason}");
    }

    #[test]
    fn missing_pieces_are_stale() {
        let fx = Fixture::new();
        std::fs::remove_file(fx.framework().join("macos-arm64/libx.a")).unwrap();
        assert!(fx.reason().unwrap().contains(".a"));
        std::fs::remove_dir_all(fx.framework().join("macos-arm64/Headers")).unwrap();
        assert!(fx.reason().unwrap().contains("ikari_x.h"));
    }

    #[test]
    fn metadata_closure_follows_workspace_deps_and_skips_dev() {
        let json = r#"{"packages":[
          {"name":"a","manifest_path":"/w/a/Cargo.toml","dependencies":[
            {"name":"b","path":"/w/b","kind":null},
            {"name":"d","path":"/w/d","kind":"dev"},
            {"name":"serde","kind":null}]},
          {"name":"b","manifest_path":"/w/b/Cargo.toml","dependencies":[]},
          {"name":"d","manifest_path":"/w/d/Cargo.toml","dependencies":[]}]}"#;
        let packages = parse_metadata(json).unwrap();
        let names: Vec<&str> = closure(&packages, "a")
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, ["a", "b"]);
    }

    #[test]
    fn walk_skips_target_and_tests() {
        let dir = tempfile::tempdir().unwrap();
        for path in ["src/lib.rs", "target/x.rs", "tests/t.rs", "Cargo.toml"] {
            let full = dir.path().join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, "").unwrap();
        }
        let mut files = Vec::new();
        walk(dir.path(), &mut files);
        files.sort();
        let names: Vec<_> = files
            .iter()
            .map(|p| {
                p.strip_prefix(dir.path())
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, ["Cargo.toml", "src/lib.rs"]);
    }

    /// 実物の確認（macos/scripts/build.sh のあとで、`cargo test -p xtask -- --ignored real_frameworks`）。
    #[test]
    #[ignore = "macos/Frameworks が要る"]
    fn real_frameworks_are_fresh_after_build() {
        verify(&crate::data::run::repo_root()).unwrap();
    }
}
