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
//!   `Cargo.toml` だけは、staticlib の時刻ではなく「`build.sh` が最後に XCFramework を作り終えた時刻」（`macos/Frameworks/.built`）と
//!   比べる（#296）。dev-dependencies だけを変えると cargo は staticlib を作り直さない（cargo は manifest の変更を時刻ではなく
//!   ハッシュで見る）ので、staticlib の時刻と比べると、build.sh を流しても古いままになる。build.sh を流したあとなら、
//!   cargo が必要な作り直しを済ませているので、Cargo.toml はそれより古ければよい。
//!   さらに、ブランチの切り替えなどで更新時刻だけが新しく中身が同じとき（`Cargo.toml` は staticlib に影響する非 dev 部分）に
//!   誤検知しないよう、前回この検査に通ったときのソースの SHA-256 ダイジェストを `.built` に書いておき、内容が同じなら
//!   古くないと判定する（#296）。ダイジェストを使うのは、`.built` と staticlib の更新時刻が書いたときのままのときだけ
//!   （`build.sh` が作り直したら捨てる）。ダイジェストは「更新時刻で新しいと分かったソース」の中身だけを書く
//!   （中身を読んでから時刻を見る。読んだあとで書き換えられたら時刻で古いと分かる）。git の履歴からは補わない
//!   （未コミットの編集から作ったあと元に戻すと、コミットと一致して古いのに通る道があるため）。
//!
//! XCFramework がなければ、swift test の前に作り方を案内して失敗する（#165。スタブのまま通すと、スタブの設定の検証が
//! Rust の本物とずれて別のテストが落ち、原因が分かりにくい）。Swift を触らない作業は `--docs-only` か `--no-swift`。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::data::run::{Failure, capture, cargo, log};
use crate::data::sha256::{sha256_file, sha256_hex};

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
/// `build.sh` が XCFramework を作り終えたときに触る印（`macos/Frameworks/` の下。gitignore）
const BUILT_MARK: &str = ".built";
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
    let stamp = BuiltStamp::load(root);
    let mut verified = Verified::default();
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
        if let Some(reason) = stale_reason(
            &framework,
            &root.join(spec.header),
            &sources,
            &stamp,
            &mut verified,
        )? {
            stale.push(format!("{}.{XCFRAMEWORK_EXT}: {reason}", spec.name));
        }
    }
    if stale.is_empty() {
        stamp.save(root, &verified);
        return Ok(());
    }
    Err(Failure::new(format!(
        "macos/Frameworks の XCFramework が無い・古いので swift test を止めます（そのまま通すと検査が環境によって変わる。#148、#165）\n  {}\n作ってから、もう一度流してください: {BUILD_SCRIPT}",
        stale.join("\n  ")
    )))
}

/// `.built` の 1 行目。`.built` の更新時刻（ミリ秒）。`build.sh` の `touch` で変われば、書いたダイジェストは使わない
const BUILT_AT_PREFIX: &str = "# built_at=";
/// `.built` の staticlib の行。`<更新時刻のミリ秒>\t<ルートからのパス>`。staticlib が作り直されていたら、ダイジェストは使わない
const LIB_PREFIX: &str = "# lib=";

/// `macos/Frameworks/.built` の情報（`build.sh` が最後に作り終えた時刻と、前回の検査に通ったときの内容ダイジェスト）。
#[derive(Clone, Debug, Default)]
struct BuiltStamp {
    checked_at: Option<SystemTime>,
    /// ダイジェストを書いたときの staticlib の更新時刻（ミリ秒）
    libs: HashMap<PathBuf, u128>,
    hashes: HashMap<PathBuf, String>,
}

/// この回の検査で新しいと確かめた staticlib とソースのダイジェスト（検査がすべて通ったときだけ `.built` に書く）
#[derive(Debug, Default)]
struct Verified {
    libs: HashMap<PathBuf, u128>,
    hashes: HashMap<PathBuf, String>,
    /// 同じソースを 2 回読んで中身が違った（検査の途中で書き換えられた）。書かない
    conflicted: bool,
}

impl Verified {
    fn add_hash(&mut self, path: &Path, digest: String) {
        match self.hashes.get(path) {
            Some(existing) if *existing != digest => self.conflicted = true,
            Some(_) => {}
            None => {
                self.hashes.insert(path.to_path_buf(), digest);
            }
        }
    }
}

impl BuiltStamp {
    fn from_checked_at(checked_at: Option<SystemTime>) -> Self {
        Self {
            checked_at,
            ..Self::default()
        }
    }

    /// テスト用: `sources` の今の中身を、`lib` に対して記録したものとして作る
    #[cfg(test)]
    fn record(
        lib: &Path,
        sources: &[PathBuf],
        checked_at: Option<SystemTime>,
    ) -> Result<Self, Failure> {
        let mut hashes = HashMap::new();
        for path in sources {
            hashes.insert(path.clone(), source_digest(path)?);
        }
        let mut libs = HashMap::new();
        if let Some(ms) = modified(lib).ok().and_then(unix_millis) {
            libs.insert(lib.to_path_buf(), ms);
        }
        Ok(Self {
            checked_at,
            libs,
            hashes,
        })
    }

    /// `lib` に対して、記録したダイジェストを使ってよいか（staticlib が書いたときのまま）
    fn trusts(&self, lib: &Path, lib_time: SystemTime) -> bool {
        unix_millis(lib_time).is_some_and(|ms| self.libs.get(lib) == Some(&ms))
    }

    fn load(root: &Path) -> Self {
        let path = root.join(FRAMEWORKS_DIR).join(BUILT_MARK);
        let checked_at = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let Some(at) = checked_at else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::from_checked_at(checked_at);
        };
        let mut lines = text.lines();
        // 1 行目が今の `.built` の時刻と一致しなければ（`touch` の直後、手で書いたもの、形式が違う）、ダイジェストは使わない
        let recorded_at = lines
            .next()
            .and_then(|first| first.strip_prefix(BUILT_AT_PREFIX))
            .and_then(|ms| ms.trim().parse::<u128>().ok());
        if recorded_at.is_none() || recorded_at != unix_millis(at) {
            return Self::from_checked_at(checked_at);
        }
        let mut libs = HashMap::new();
        let mut hashes = HashMap::new();
        for line in lines {
            if let Some(rest) = line.strip_prefix(LIB_PREFIX) {
                if let Some((ms, rel)) = rest.split_once('\t')
                    && let Ok(ms) = ms.parse::<u128>()
                    && !rel.is_empty()
                {
                    libs.insert(root.join(rel), ms);
                }
            } else if let Some((hash, rel)) = line.split_once('\t')
                && !hash.is_empty()
                && !rel.is_empty()
            {
                hashes.insert(root.join(rel), hash.to_owned());
            }
        }
        Self {
            checked_at,
            libs,
            hashes,
        }
    }

    /// 検査に通ったときの staticlib の時刻とソースのダイジェストを `.built` に書く。`.built` の更新時刻（ビルド完了時刻）は保つ。
    /// 書けなければ何もしない（次の検査は更新時刻だけで判定する。古い側に倒れるだけ）。
    fn save(&self, root: &Path, verified: &Verified) {
        let path = root.join(FRAMEWORKS_DIR).join(BUILT_MARK);
        if verified.conflicted || !path.is_file() {
            return;
        }
        let Some(at) = self.checked_at else {
            return;
        };
        let Some(ms) = unix_millis(at) else {
            return;
        };
        let rel_of = |p: &Path| {
            p.strip_prefix(root)
                .ok()
                .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        };
        let mut libs: Vec<(String, u128)> = verified
            .libs
            .iter()
            .filter_map(|(p, t)| Some((rel_of(p)?, *t)))
            .collect();
        libs.sort();
        let mut entries: Vec<(String, &str)> = verified
            .hashes
            .iter()
            .filter_map(|(p, d)| Some((rel_of(p)?, d.as_str())))
            .collect();
        entries.sort();
        let mut content = format!("{BUILT_AT_PREFIX}{ms}\n");
        for (rel, t) in libs {
            content.push_str(&format!("{LIB_PREFIX}{t}\t{rel}\n"));
        }
        for (rel, digest) in entries {
            content.push_str(&format!("{digest}\t{rel}\n"));
        }
        // 書いてから時刻を戻す間に失敗すると、`.built` の時刻が進んで Cargo.toml の基準が新しくなる。
        // 別のファイルに書いて時刻を合わせてから置き換え、途中で失敗したら元の `.built` に触らない
        let tmp = path.with_extension("tmp");
        let written = std::fs::write(&tmp, &content).is_ok()
            && std::fs::File::options()
                .write(true)
                .open(&tmp)
                .and_then(|file| file.set_modified(at))
                .is_ok()
            && std::fs::rename(&tmp, &path).is_ok();
        if !written {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

fn unix_millis(time: SystemTime) -> Option<u128> {
    time.duration_since(UNIX_EPOCH).ok().map(|d| d.as_millis())
}

/// ソースファイルの SHA-256 ダイジェスト。`Cargo.toml` は staticlib に影響しない `[dev-dependencies]` 等を除いて計算する。
fn source_digest(path: &Path) -> Result<String, Failure> {
    if path.file_name().is_some_and(|n| n == MANIFEST) {
        let bytes = std::fs::read(path)
            .map_err(|e| Failure::new(format!("{} を読めません: {e}", path.display())))?;
        return Ok(source_digest_bytes(path, &bytes));
    }
    sha256_file(path).map_err(|e| Failure::new(format!("{} を読めません: {e}", path.display())))
}

fn source_digest_bytes(path: &Path, bytes: &[u8]) -> String {
    if path.file_name().is_some_and(|n| n == MANIFEST)
        && let Ok(text) = std::str::from_utf8(bytes)
    {
        return sha256_hex(manifest_lib_content(text).as_bytes());
    }
    sha256_hex(bytes)
}

/// `Cargo.toml` のうち、staticlib（`--lib`）のビルドに影響する部分だけを正規化して取り出す（#296）。
/// `[dev-dependencies]`（`[target.*.dev-dependencies]` を含む）、`[[test]]`、`[[bench]]`、`[[example]]`、
/// コメント、空行は staticlib に入らないので除く。
fn manifest_lib_content(text: &str) -> String {
    // 複数行の文字列は行ごとの読み取りで見分けられない。そのまま全体を比べる（少しでも変われば古い側に倒す）
    if text.contains("\"\"\"") || text.contains("'''") {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut skip_section = false;
    let mut bracket_depth: usize = 0;
    for raw_line in text.lines() {
        let line = strip_toml_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if bracket_depth == 0
            && let Some(header) = parse_section_header(line)
        {
            skip_section = is_non_lib_section(header);
        }
        bracket_depth = update_bracket_depth(bracket_depth, line);
        if !skip_section {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// 文字列リテラルの外の `#` 以降（TOML のコメント）を除く。
fn strip_toml_comment(line: &str) -> &str {
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for (idx, c) in line.char_indices() {
        if in_double {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_double = false;
            }
        } else if in_single {
            if c == '\'' {
                in_single = false;
            }
        } else {
            match c {
                '"' => in_double = true,
                '\'' => in_single = true,
                '#' => return &line[..idx],
                _ => {}
            }
        }
    }
    line
}

fn update_bracket_depth(mut depth: usize, line: &str) -> usize {
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for c in line.chars() {
        if in_double {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_double = false;
            }
        } else if in_single {
            if c == '\'' {
                in_single = false;
            }
        } else {
            match c {
                '"' => in_double = true,
                '\'' => in_single = true,
                '[' => depth = depth.saturating_add(1),
                ']' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    depth
}

fn parse_section_header(line: &str) -> Option<&str> {
    if line.starts_with("[[") && line.ends_with("]]") && line.len() >= 4 {
        return Some(line);
    }
    if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
        return Some(&line[1..line.len() - 1]);
    }
    None
}

fn is_non_lib_section(header: &str) -> bool {
    if let Some(array_table) = header
        .strip_prefix("[[")
        .and_then(|h| h.strip_suffix("]]"))
        .map(str::trim)
    {
        return matches!(array_table, "test" | "bench" | "example")
            || array_table.starts_with("test.")
            || array_table.starts_with("bench.")
            || array_table.starts_with("example.");
    }
    let table = header.trim();
    table == "dev-dependencies"
        || table == "dev_dependencies"
        || table.ends_with(".dev-dependencies")
        || table.ends_with(".dev_dependencies")
        || table.starts_with("dev-dependencies.")
        || table.starts_with("dev_dependencies.")
}

/// 古い理由（新しければ None）。`header` は Rust 側のヘッダ、`sources` は staticlib の元になるソースのファイル。
/// `stamp` は `build.sh` が XCFramework を作り終えた時刻と、前回の検査に通ったときの内容ダイジェスト（印が無ければ空）。
/// ダイジェストが記録されていて staticlib がそのときのままなら、更新時刻が新しくても内容（`Cargo.toml` は非 dev 部分）が
/// 同じソースは古くない。新しいと確かめたものは `verified` に足す。
fn stale_reason(
    framework: &Path,
    header: &Path,
    sources: &[PathBuf],
    stamp: &BuiltStamp,
    verified: &mut Verified,
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
    let manifest_floor = stamp
        .checked_at
        .map_or(lib_time, |checked| checked.max(lib_time));
    let trusted = stamp.trusts(&lib, lib_time);
    let mut newest: Option<(&PathBuf, SystemTime)> = None;
    let mut fresh = Vec::new();
    for path in sources {
        let recorded = stamp.hashes.get(path).filter(|_| trusted);
        // 中身を先に読み、時刻はそのあとで見る（読んだあとで書き換えられたら、時刻で古いと分かる）
        let digest = match source_digest(path) {
            Ok(digest) => digest,
            // 消えたソース。前回は在ったなら staticlib はそれを含んでいる（古い）。記録が無ければ、これまでどおり見ない
            Err(_) if !path.exists() => {
                if recorded.is_some() {
                    return Ok(Some(format!("ソースが消えた（{}）", path.display())));
                }
                continue;
            }
            Err(e) => return Err(e),
        };
        if recorded == Some(&digest) {
            fresh.push((path, digest));
            continue;
        }
        let floor = if path.file_name().is_some_and(|n| n == MANIFEST) {
            manifest_floor
        } else {
            lib_time
        };
        let time = modified(path)?;
        if recorded.is_some() || time > floor {
            if newest.is_none_or(|(_, best)| time >= best) {
                newest = Some((path, time));
            }
        } else {
            fresh.push((path, digest));
        }
    }
    if let Some((path, _)) = newest {
        return Ok(Some(format!(
            "staticlib より新しいソースがある（{}）",
            path.display()
        )));
    }
    if let Some(ms) = unix_millis(lib_time) {
        verified.libs.insert(lib, ms);
        for (path, digest) in fresh {
            verified.add_hash(path, digest);
        }
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
        fn lib(&self) -> PathBuf {
            self.framework().join("macos-arm64/libx.a")
        }
        fn check(&self, sources: &[PathBuf], stamp: &BuiltStamp) -> Option<String> {
            self.check_into(sources, stamp, &mut Verified::default())
        }
        fn check_into(
            &self,
            sources: &[PathBuf],
            stamp: &BuiltStamp,
            verified: &mut Verified,
        ) -> Option<String> {
            stale_reason(&self.framework(), &self.header(), sources, stamp, verified).unwrap()
        }
        fn reason(&self) -> Option<String> {
            std::fs::create_dir_all(self.header().parent().unwrap()).unwrap();
            self.check(&[self.source()], &BuiltStamp::default())
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

    /// Cargo.toml だけが staticlib より新しくても、build.sh がそのあとに流れていれば古くない（#296）。
    /// build.sh の前に変えた Cargo.toml は、これまでどおり古い。ソースは、build.sh の時刻に関係なく staticlib と比べる。
    #[test]
    fn manifest_is_judged_by_the_last_build() {
        let fx = Fixture::new();
        let manifest = fx.dir.path().join("Cargo.toml");
        std::fs::write(&manifest, "").unwrap();
        fx.set_time(&manifest, 10);
        let at = |secs: u64| SystemTime::now() + Duration::from_secs(secs);
        let run =
            |sources: &[PathBuf], checked| fx.check(sources, &BuiltStamp::from_checked_at(checked));
        let sources = [manifest.clone()];
        assert!(run(&sources, None).unwrap().contains("Cargo.toml"));
        assert!(run(&sources, Some(at(5))).unwrap().contains("Cargo.toml"));
        assert_eq!(run(&sources, Some(at(20))), None);
        fx.set_time(&fx.source(), 30);
        let both = [manifest, fx.source()];
        assert!(run(&both, Some(at(40))).unwrap().contains("lib.rs"));
    }

    /// ブランチの切り替えなどでソースの更新時刻だけが新しくなっても、記録されたダイジェストと内容が同じなら古くない（#296）。
    /// 中身が変わっていれば、これまでどおり古いと判定する。
    #[test]
    fn identical_source_with_newer_mtime_passes_when_digest_is_recorded() {
        let fx = Fixture::new();
        let sources = [fx.source()];
        let stamp = BuiltStamp::record(&fx.lib(), &sources, Some(SystemTime::now())).unwrap();
        // 更新時刻だけを未来にする（中身は同じ）
        fx.set_time(&fx.source(), 100);
        assert_eq!(fx.check(&sources, &stamp), None);
        // 中身を変えると古い
        std::fs::write(fx.source(), "fn main() { let _ = 1; }").unwrap();
        fx.set_time(&fx.source(), 100);
        let reason = fx.check(&sources, &stamp).unwrap();
        assert!(reason.contains("lib.rs"), "{reason}");
        // 記録と違う中身は、更新時刻が staticlib より古くても古い（記録があれば中身が優先）
        fx.set_time(&fx.source(), -100);
        let reason = fx.check(&sources, &stamp).unwrap();
        assert!(reason.contains("lib.rs"), "{reason}");
    }

    /// staticlib が記録のあとで作り直されていたら（`.built` を触らずに `build.sh` が途中で止まったなど）、
    /// 記録したダイジェストは使わず更新時刻で判定する（#296。偽陰性を作らない）
    #[test]
    fn recorded_digests_are_ignored_when_the_staticlib_changed() {
        let fx = Fixture::new();
        let sources = [fx.source()];
        let stamp = BuiltStamp::record(&fx.lib(), &sources, Some(SystemTime::now())).unwrap();
        fx.set_time(&fx.source(), 100);
        assert_eq!(fx.check(&sources, &stamp), None);
        // staticlib の時刻が変わった（中身は記録と同じでも、記録は別のビルドのもの）
        fx.set_time(&fx.lib(), 50);
        let reason = fx.check(&sources, &stamp).unwrap();
        assert!(reason.contains("lib.rs"), "{reason}");
    }

    /// 記録にあったソースが消えたら古い。記録がなければ、これまでどおり見ない（dep-info に残った消えたファイル）
    #[test]
    fn a_recorded_source_that_disappeared_is_stale() {
        let fx = Fixture::new();
        let gone = fx.dir.path().join("src/gone.rs");
        std::fs::write(&gone, "fn gone() {}").unwrap();
        fx.set_time(&gone, -100);
        let sources = [fx.source(), gone.clone()];
        let stamp = BuiltStamp::record(&fx.lib(), &sources, Some(SystemTime::now())).unwrap();
        std::fs::remove_file(&gone).unwrap();
        let reason = fx.check(&sources, &stamp).unwrap();
        assert!(reason.contains("gone.rs"), "{reason}");
        assert_eq!(fx.check(&sources, &BuiltStamp::default()), None);
    }

    /// 未コミットの編集 X から作り、元の Y に戻した: Y の更新時刻は staticlib より新しいので、記録が無ければ古い
    /// （git の履歴で補わない。補うと Y がコミットと一致して通ってしまう。リードのレビューの指摘 1）
    #[test]
    fn reverting_an_uncommitted_edit_after_the_build_is_stale() {
        let fx = Fixture::new();
        // Y（元の中身）は Fixture のソース。X に書き換えてから作った
        std::fs::write(fx.source(), "fn main() { /* X */ }").unwrap();
        fx.set_time(&fx.source(), -10);
        let sources = [fx.source()];
        let mut verified = Verified::default();
        assert_eq!(
            fx.check_into(&sources, &BuiltStamp::default(), &mut verified),
            None
        );
        // Y に戻す（時刻は今より新しい）
        std::fs::write(fx.source(), "fn main() {}").unwrap();
        fx.set_time(&fx.source(), 10);
        // 記録が無い
        assert!(fx.check(&sources, &BuiltStamp::default()).is_some());
        // 検査に通ったときの記録（X）があっても、Y とは違うので古い
        let stamp = BuiltStamp {
            checked_at: Some(SystemTime::now()),
            libs: verified.libs,
            hashes: verified.hashes,
        };
        assert!(fx.check(&sources, &stamp).is_some());
    }

    /// `Cargo.toml` の `dev-dependencies` やテスト・ベンチ・コメントだけが変わって更新時刻が新しくなっても、
    /// staticlib に影響する部分が変わっていなければ古くない（#296）。通常の `dependencies` や `features` の変更は検知する。
    #[test]
    fn dev_dependencies_only_change_in_manifest_does_not_trigger_stale() {
        let fx = Fixture::new();
        let manifest = fx.dir.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            r#"[package]
name = "demo"
version = "0.1.0"

[dependencies]
ikari-core = { path = "../core" }

[dev-dependencies]
ikari-testtime = { path = "../testtime" }
"#,
        )
        .unwrap();
        fx.set_time(&manifest, -50);
        let sources = [manifest.clone(), fx.source()];
        let stamp = BuiltStamp::record(&fx.lib(), &sources, Some(SystemTime::now())).unwrap();

        // dev-dependencies、target.*.dev-dependencies、[[test]]、[[bench]]、コメントだけを追加して更新時刻を未来にする
        std::fs::write(
            &manifest,
            r#"# comment added
[package]
name = "demo"
version = "0.1.0"

[dependencies]
ikari-core = { path = "../core" } # inline comment

[dev-dependencies]
ikari-testtime = { path = "../testtime" }
tempfile = "3"

[target.'cfg(windows)'.dev-dependencies]
windows-sys = "0.59"

[[test]]
name = "integration"

[[bench]]
name = "perf"
"#,
        )
        .unwrap();
        fx.set_time(&manifest, 100);
        assert_eq!(fx.check(&sources, &stamp), None);

        // 本番の dependencies が変われば古いと検知する
        std::fs::write(
            &manifest,
            r#"[package]
name = "demo"
version = "0.1.0"

[dependencies]
ikari-core = { path = "../core" }
serde = "1"

[dev-dependencies]
tempfile = "3"
"#,
        )
        .unwrap();
        fx.set_time(&manifest, 100);
        let reason = fx.check(&sources, &stamp).unwrap();
        assert!(reason.contains("Cargo.toml"), "{reason}");
    }

    /// 複数行の文字列がある `Cargo.toml` は、行ごとに読み分けず、全体を比べる（古い側に倒す）
    #[test]
    fn manifest_with_multiline_strings_is_compared_as_a_whole() {
        let text = "[package]\ndescription = \"\"\"\n[dev-dependencies]\n\"\"\"\nname = \"a\"\n";
        assert_eq!(manifest_lib_content(text), text);
    }

    /// 検査に通ったら、staticlib の時刻とダイジェストを `.built` に書き、`.built` の更新時刻は保つ。
    /// `build.sh` が `touch .built` で時刻を進めたら、書いたダイジェストは捨てる。1 行目の無い `.built` も使わない。
    #[test]
    fn built_stamp_save_preserves_mtime_and_touch_invalidates_old_hashes() {
        let fx = Fixture::new();
        let root = fx.dir.path();
        let frameworks = root.join(FRAMEWORKS_DIR);
        std::fs::create_dir_all(&frameworks).unwrap();
        let built = frameworks.join(BUILT_MARK);
        std::fs::write(&built, "").unwrap();
        let build_time = SystemTime::now() - Duration::from_secs(60);
        let set_built_time = |time: SystemTime| {
            File::options()
                .write(true)
                .open(&built)
                .unwrap()
                .set_modified(time)
                .unwrap();
        };
        set_built_time(build_time);

        let sources = [fx.source()];
        let initial = BuiltStamp::load(root);
        assert!(initial.hashes.is_empty());
        let mut verified = Verified::default();
        assert_eq!(fx.check_into(&sources, &initial, &mut verified), None);
        initial.save(root, &verified);
        assert!(!built.with_extension("tmp").exists());

        let loaded = BuiltStamp::load(root);
        assert_eq!(
            unix_millis(loaded.checked_at.unwrap()),
            unix_millis(build_time)
        );
        assert_eq!(
            loaded.hashes.get(&fx.source()).map(String::as_str),
            Some(source_digest(&fx.source()).unwrap().as_str())
        );
        assert!(loaded.trusts(&fx.lib(), modified(&fx.lib()).unwrap()));
        // 記録のおかげで、更新時刻だけが進んだソースは古くない
        fx.set_time(&fx.source(), 100);
        assert_eq!(fx.check(&sources, &loaded), None);

        // build.sh が `touch .built` で時刻を更新したら、古いダイジェストは使わない
        set_built_time(SystemTime::now());
        let after_touch = BuiltStamp::load(root);
        assert!(after_touch.hashes.is_empty());
        assert!(fx.check(&sources, &after_touch).is_some());

        // 1 行目（時刻）の無い `.built` のダイジェストは使わない
        let digest = source_digest(&fx.source()).unwrap();
        std::fs::write(&built, format!("{digest}\tsrc/lib.rs\n")).unwrap();
        assert!(BuiltStamp::load(root).hashes.is_empty());
    }

    /// 同じソースを 2 回読んで中身が違ったら（検査の途中で書き換えられた）、`.built` に書かない
    #[test]
    fn conflicting_digests_are_not_saved() {
        let fx = Fixture::new();
        let root = fx.dir.path();
        let frameworks = root.join(FRAMEWORKS_DIR);
        std::fs::create_dir_all(&frameworks).unwrap();
        let built = frameworks.join(BUILT_MARK);
        std::fs::write(&built, "").unwrap();
        let mut verified = Verified::default();
        verified.add_hash(&fx.source(), "a".to_owned());
        verified.add_hash(&fx.source(), "b".to_owned());
        assert!(verified.conflicted);
        BuiltStamp::load(root).save(root, &verified);
        assert_eq!(std::fs::read_to_string(&built).unwrap(), "");
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
