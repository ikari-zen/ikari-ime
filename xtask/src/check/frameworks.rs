//! swift test の前に、`macos/Frameworks` の XCFramework が古くないことを確かめる（issue #148）。
//!
//! XCFramework（Rust の staticlib とヘッダ。gitignore）が残っていると、`swift test` はそれを使う。Rust 側のヘッダや実装を変えても
//! XCFramework が古いままだと、検査が環境によって通ったり落ちたりする（#146: 古い XCFramework のとき
//! `cannot find type 'IkariShellHost'` で失敗。`build.sh` で作り直すと通った）。ここで古さをはっきり失敗にする。
//! 毎回は作り直さない（数分かかる）。作り直しは `macos/scripts/build.sh` で、失敗のメッセージにその手順を書く。
//!
//! 古さの判定（どちらかに当たれば古い）:
//! - XCFramework の中のヘッダが、Rust 側のヘッダ（`shell-ffi/include/*.h` など）と内容が違う
//! - XCFramework の中の staticlib（`.a`）より、その staticlib のビルドに入ったソースのほうが新しい。
//!   「入ったソース」は、macOS 向けのビルドが残す dep-info（`target/**/<triple>/release/lib<crate>.d`。rustc が実際に読んだ
//!   ファイルの一覧。cfg で外れた `windows.rs` などは載らない。#476）と、依存する crate の `Cargo.toml`。dep-info が
//!   無いときは、crate のディレクトリを丸ごと見る（広めに「古い」と言う側に倒す）。
//!
//! XCFramework がなければ、swift test の前に作り方を案内して失敗する（#165。スタブのまま通すと、スタブの設定の検証が
//! Rust の本物とずれて別のテストが落ち、原因が分かりにくい）。Swift を触らない作業は `--docs-only` か `--no-swift`。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::data::run::{Failure, capture, cargo, log};

const FRAMEWORKS_DIR: &str = "macos/Frameworks";
const BUILD_SCRIPT: &str = "macos/scripts/build.sh --signing team --release";
const XCFRAMEWORK_EXT: &str = "xcframework";
const STATICLIB_EXT: &str = "a";
/// macOS 向けのビルドの成果物のディレクトリを見分ける印。`macos/scripts/build.sh` は `--target <triple>` 付きで作るので、
/// `target/<triple>/release/` に dep-info が残る。ほかの OS 向けのビルドの dep-info は読まない。
const MAC_TARGET_MARK: &str = "apple-darwin";
/// `build.sh` が engine-ffi を作る別の target（feature を混ぜないため。`CARGO_TARGET_DIR`）
const LM_TARGET_DIR: &str = "lm";
const DEP_INFO_EXT: &str = "d";
const MANIFEST: &str = "Cargo.toml";
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

/// どの XCFramework も有って新しければ Ok。無い・古いものがあれば、作り直しの手順つきで失敗する。
pub fn verify(root: &Path) -> Result<(), Failure> {
    log("XCFramework の鮮度（macos/Frameworks）");
    let mut metadata: Option<Vec<Package>> = None;
    let mut stale = Vec::new();
    for spec in &SPECS {
        let framework = root
            .join(FRAMEWORKS_DIR)
            .join(format!("{}.{XCFRAMEWORK_EXT}", spec.name));
        if !framework.is_dir() {
            stale.push(format!("{}.{XCFRAMEWORK_EXT}: ありません", spec.name));
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
        let sources = source_files(root, packages, spec.package);
        if let Some(reason) = stale_reason(&framework, &root.join(spec.header), &sources)? {
            stale.push(format!("{}.{XCFRAMEWORK_EXT}: {reason}", spec.name));
        }
    }
    if stale.is_empty() {
        return Ok(());
    }
    Err(Failure::new(format!(
        "macos/Frameworks の XCFramework が無い・古いので swift test を止めます（そのまま通すと検査が環境によって変わる。#148、#165）\n  {}\n作ってから、もう一度流してください: {BUILD_SCRIPT}",
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

/// staticlib の元になるファイル。dep-info があれば、実際に読まれたファイル（と依存する crate の `Cargo.toml`）だけ。
/// 無ければ、`package` と依存する workspace の crate のディレクトリの全ファイル。
fn source_files(root: &Path, packages: &[Package], package: &str) -> Vec<PathBuf> {
    let closure = closure(packages, package);
    if let Some(mut files) = dep_info_path(root, package)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| parse_dep_info(&text, root))
        .filter(|files| !files.is_empty())
    {
        files.extend(closure.iter().map(|found| found.dir.join(MANIFEST)));
        return files;
    }
    let mut files = Vec::new();
    for found in closure {
        walk(&found.dir, &mut files);
    }
    files
}

/// macOS 向けの release ビルドが残した `lib<crate>.d` のうち、いちばん新しいもの。
fn dep_info_path(root: &Path, package: &str) -> Option<PathBuf> {
    let file_name = format!("lib{}.{DEP_INFO_EXT}", package.replace('-', "_"));
    let target = root.join("target");
    let mut candidates = Vec::new();
    for base in [target.clone(), target.join(LM_TARGET_DIR)] {
        let Ok(entries) = std::fs::read_dir(&base) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let is_mac = entry
                .file_name()
                .to_string_lossy()
                .contains(MAC_TARGET_MARK);
            let path = entry.path().join("release").join(&file_name);
            if is_mac && path.is_file() {
                candidates.push(path);
            }
        }
    }
    candidates
        .into_iter()
        .filter_map(|path| Some((modified(&path).ok()?, path)))
        .max()
        .map(|(_, path)| path)
}

/// dep-info（Makefile 形式。`<成果物>: <依存>...`。空白は `\ ` で逃がす）の、最初の規則の依存を取り出す。
/// 相対パスは `root` からとみなす。
fn parse_dep_info(text: &str, root: &Path) -> Vec<PathBuf> {
    let Some(rule) = text.lines().find(|line| line.contains(": ")) else {
        return Vec::new();
    };
    let deps = rule.split_once(": ").map_or("", |(_, deps)| deps);
    let mut paths = Vec::new();
    let mut current = String::new();
    let mut chars = deps.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&' ') => {
                current.push(' ');
                chars.next();
            }
            ' ' => {
                if !current.is_empty() {
                    paths.push(root.join(std::mem::take(&mut current)));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        paths.push(root.join(current));
    }
    paths
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
    fn missing_xcframework_fails_with_build_instruction() {
        let root = tempfile::tempdir().unwrap();
        let err = verify(root.path()).unwrap_err().to_string();
        assert!(err.contains("ありません"), "{err}");
        assert!(err.contains(BUILD_SCRIPT), "{err}");
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
    fn dep_info_lists_only_files_the_build_read() {
        let root = Path::new("/w");
        let text = "/w/target/x/libx.rlib: /w/a/src/lib.rs /w/a/src/with\\ space.rs b/src/rel.rs\n/w/a/src/lib.rs:\n";
        let files = parse_dep_info(text, root);
        assert_eq!(
            files,
            [
                PathBuf::from("/w/a/src/lib.rs"),
                PathBuf::from("/w/a/src/with space.rs"),
                PathBuf::from("/w/b/src/rel.rs"),
            ]
        );
        assert!(parse_dep_info("", root).is_empty());
    }

    /// dep-info から外れた（Windows 専用の）ファイルは対象に入らず、載っているファイルは入る（#476）。
    #[test]
    fn windows_only_file_is_excluded_but_built_file_is_not() {
        let root = tempfile::tempdir().unwrap();
        let pkg = "ikari-demo";
        let lib = root.path().join("demo/src/lib.rs");
        let windows = root.path().join("demo/src/os/windows.rs");
        let manifest = root.path().join("demo/Cargo.toml");
        for path in [&lib, &windows, &manifest] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        let dep_dir = root.path().join("target/aarch64-apple-darwin/release");
        std::fs::create_dir_all(&dep_dir).unwrap();
        std::fs::write(
            dep_dir.join("libikari_demo.d"),
            format!(
                "{}: {}\n",
                dep_dir.join("libikari_demo.rlib").display(),
                lib.display()
            ),
        )
        .unwrap();
        let packages = vec![Package {
            name: pkg.to_owned(),
            dir: root.path().join("demo"),
            deps: Vec::new(),
        }];
        let files = source_files(root.path(), &packages, pkg);
        assert!(files.contains(&lib), "{files:?}");
        assert!(files.contains(&manifest), "{files:?}");
        assert!(!files.contains(&windows), "{files:?}");

        // dep-info が無ければ、従来どおり crate の全ファイル（Windows 専用も含む）
        std::fs::remove_dir_all(root.path().join("target")).unwrap();
        let files = source_files(root.path(), &packages, pkg);
        assert!(files.contains(&windows), "{files:?}");
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
                    // 区切りは OS による（Windows は \）
                    .replace('\\', "/")
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

    /// 実物の dep-info: Mac の staticlib に入るファイルは載り、Windows 専用のファイルは載らない（#476）。
    #[test]
    #[ignore = "macos/Frameworks を作った target が要る"]
    fn real_dep_info_excludes_windows_only_files() {
        let root = crate::data::run::repo_root();
        let packages = workspace_packages(&root).unwrap();
        let check = |spec: &Spec, rel: &str, expected: bool| {
            let files = source_files(&root, &packages, spec.package);
            assert_eq!(files.contains(&root.join(rel)), expected, "{rel}");
        };
        check(&SPECS[0], "shell-core/src/machine/mod.rs", true);
        check(&SPECS[0], "shell-ffi/src/lib.rs", true);
        check(&SPECS[0], "runmark/src/signal_win.rs", false);
        check(&SPECS[1], "engine-ffi/src/lib.rs", true);
        check(&SPECS[1], "lm/src/host/os/windows.rs", false);
    }
}
