//! `cargo xtask scan-product --macos-app <ikari.app> --edition normal|offline`: 配る macOS の束（dmg に入れる `ikari.app`）の検査
//! （ADR 0080 §7.4 の「配る物の中身」と「実行ファイルの文字列」、ADR 0069 §4.1・§6）。ファイルのバイト列を読むだけで、道具は要らない。
//!   - 束のどこにも `ikari-smoke`（開発用の確認の道具。ADR 0069 §6）が無いこと
//!   - 通常版は取得の部品（`Contents/Helpers/ikari-data`）があり、オフライン版は束のどこにも無く、殻の署名も指していないこと
//!   - 取得の印（`FETCH_MARKS`）: オフライン版は、束の中のどの実行ファイル（Mach-O）にも無いこと（通信の部品が入っていないことの直接の検査）。
//!     通常版は、印のそれぞれが、どれかの実行ファイルにあること（印の表が古くなって、検査が空振りしていないことの確かめ）
//!   - 実行ファイルに、開発用の文字列と測定ツールの印が無く、LM の子のほかに llama.cpp の印が無いこと（`scan-product` と同じ語）
//!   - 同梱のデータ（ADR 0080 §3.1）: 決まった名前のファイルだけがあり、モデルが表の sha256（`ikari_resource::ZENZ_SMALL_SHA256`）と合い、
//!     データの全ファイルがエンジンの署名（`CodeResources`）に載っていること
//!
//! 封印の検査の分け方: 署名そのものの検証（署名のあとに中身が変わっていないか）は `codesign --verify --deep --strict` が持つ
//! （`build.sh` と `make-dmg.sh` が流す。ここでは道具を使わない）。ビルドのときの一覧との照合は `macos/scripts/lib-data.sh` の
//! `bundled_data_seal_error`（`build.sh` の中だけで使える。一覧は束の外に置く）。ここは、配る物（dmg の中の束）だけを見て、
//! 「決まった物が、決まった名前で、署名に載っている」ことを独立に確かめる。名前の表は `lib-data.sh` と同じ物を持つ（テストで照らす）。

use std::path::{Path, PathBuf};

use ikari_resource::{ZENZ_SMALL_FILE_NAME, ZENZ_SMALL_SHA256};

use super::{find_dev_strings, find_llama, find_probe};
use crate::data::sha256::sha256_file;

/// 版（ADR 0080 §7.3。`build.sh --edition` と同じ名前）
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Edition {
    Normal,
    Offline,
}

/// 開発用の確認の道具の名前（ファイルやフォルダの名前の一部。大文字小文字を区別しない）
const SMOKE_NAME_PART: &str = "ikari-smoke";
/// 取得の部品の名前と、束の中の置き場所（ADR 0079）
const DATA_NAME: &str = "ikari-data";
const DATA_HELPER: &str = "Contents/Helpers/ikari-data";
/// 殻の署名（取得の部品はこれが封印する）
const APP_CODE_RESOURCES: &str = "Contents/_CodeSignature/CodeResources";
/// LM の子（llama.cpp が入っていて正しい。ADR 0072 §5）
const LM_HOST_NAME: &str = "ikari-lm-host";
/// Mach-O として見たことを確かめる実行ファイル（殻、エンジン、設定アプリ）。見ていなければ、検査が空振りしている
const REQUIRED_EXECUTABLES: &[&str] = &[
    "Contents/MacOS/ikari",
    "Contents/XPCServices/IkariEngine.xpc/Contents/MacOS/IkariEngine",
    "Contents/Helpers/IkariSettings.app/Contents/MacOS/IkariSettings",
];

/// 取得の印。取得のコード（`ikari-data` と、設定アプリがそれを起こす部分）にしか無い文字列（ASCII）。
///   - `/releases/latest`: 版の確認の固定の URL（`fetch/src/version.rs`。`ikari-data` だけ）
///   - `ureq`: 通信の crate の名前（`ikari-data` だけが依る。`xtask/src/check/supply.rs` の `net_crates`）
///   - `HTTPS_PROXY`: プロキシの環境変数（`ikari-data` の通信の crate と、設定アプリが子に渡す環境の表
///     `shell-ffi/src/fetch_launcher.rs` の `PROXY_VARS`。どちらも feature `online` か `ikari-data` だけ）
///
/// 2026-11-23 の実物の束（main 9a38399 + この変更）で測った: 通常版では 3 つとも `ikari-data` にだけあり、
/// オフライン版のどの実行ファイルにも無い。今の macOS の設定アプリ（Swift）は取得の FFI を呼ばないので、
/// 取得の核（`shell-core` の `settings::fetch`、`shell-ffi` の `fetch_launcher`）はリンクで落ち、通常版の設定アプリにも入っていない。
/// 短い語（`version-check` など）は、命令の即値に埋められて文字列として残らないことがある（実物で見つからなかった）ので、印にしない
pub const FETCH_MARKS: &[&str] = &["/releases/latest", "ureq", "HTTPS_PROXY"];

/// 同梱のデータの置き場所（`macos/scripts/lib-data.sh` の `BUNDLED_DATA_REL`、`engine/src/data_location.rs`）
const DATA_REL: &str = "Contents/XPCServices/IkariEngine.xpc/Contents/Resources/data";
/// エンジンの束の署名（同梱のデータはこれが封印する。ADR 0080 §3.4）
const XPC_CODE_RESOURCES: &str =
    "Contents/XPCServices/IkariEngine.xpc/Contents/_CodeSignature/CodeResources";
/// `CodeResources` の中の、データのファイルの鍵の前置き（エンジンの束の `Contents/` からの相対）
const SEAL_KEY_PREFIX: &str = "Resources/data/";
/// 辞書（`lib-data.sh` の `BUNDLED_DICT_REQUIRED` と `BUNDLED_DICT_OPTIONAL`）
pub const DICT_REQUIRED: &[&str] = &["grammar.bin", "system.dict", "vectors.bin"];
pub const DICT_OPTIONAL: &[&str] = &["emoji.tsv", "postal.tsv", "BUILD_INFO"];
const DICT_DIR: &str = "dict";
const MODELS_DIR: &str = "models";
/// データの場所の直下に置いてよいもの（`NOTICE.txt` と `licenses/` は `docs/dist-notices/` にあれば）
const DATA_TOP_DIRS: &[&str] = &[DICT_DIR, MODELS_DIR, "licenses"];
const DATA_TOP_FILES: &[&str] = &["NOTICE.txt"];

/// Mach-O の先頭の 4 バイト（64 ビット、32 ビット、fat、fat 64）
const MACHO_MAGICS: &[[u8; 4]] = &[
    [0xcf, 0xfa, 0xed, 0xfe],
    [0xce, 0xfa, 0xed, 0xfe],
    [0xca, 0xfe, 0xba, 0xbe],
    [0xca, 0xfe, 0xba, 0xbf],
];

/// プリセットのモデルの名前と sha256（表の値。テストでは差し替える）
struct Pinned<'a> {
    model_file: &'a str,
    model_sha256: &'a str,
}

const PINNED: Pinned<'static> = Pinned {
    model_file: ZENZ_SMALL_FILE_NAME,
    model_sha256: ZENZ_SMALL_SHA256,
};

/// 束の問題の文を返す（空なら合格）。束として読めなければ Err
pub fn check_macos_app(app: &Path, edition: Edition) -> Result<Vec<String>, String> {
    check_with(app, edition, &PINNED)
}

fn check_with(app: &Path, edition: Edition, pinned: &Pinned) -> Result<Vec<String>, String> {
    if !app.join("Contents").join("Info.plist").is_file() {
        return Err(format!(
            "{} は束ではありません（Contents/Info.plist がない）",
            app.display()
        ));
    }
    let entries = walk(app)?;
    let mut problems = Vec::new();
    problems.extend(smoke_problems(&entries));
    problems.extend(data_helper_problems(app, edition, &entries)?);
    problems.extend(executable_problems(app, edition, &entries)?);
    problems.extend(bundled_data_problems(app, pinned)?);
    Ok(problems)
}

/// 束の中の 1 項目（束からの相対パス。区切りは `/`）
struct Entry {
    rel: String,
    path: PathBuf,
    kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Dir,
    Symlink,
}

impl Entry {
    fn name(&self) -> &str {
        self.rel.rsplit('/').next().unwrap_or(&self.rel)
    }
}

/// 束の下の全項目（シンボリックリンクはたどらない）。読めないところがあれば Err（通さない）
fn walk(root: &Path) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = pending.pop() {
        let read =
            std::fs::read_dir(&dir).map_err(|e| format!("{} を読めません: {e}", dir.display()))?;
        for entry in read {
            let entry = entry.map_err(|e| format!("{} を読めません: {e}", dir.display()))?;
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path)
                .map_err(|e| format!("{} を読めません: {e}", path.display()))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let kind = if meta.file_type().is_symlink() {
                Kind::Symlink
            } else if meta.is_dir() {
                pending.push((path.clone(), rel.clone()));
                Kind::Dir
            } else {
                Kind::File
            };
            entries.push(Entry { rel, path, kind });
        }
    }
    entries.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(entries)
}

/// 束のどこにも Smoke が無い
fn smoke_problems(entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .filter(|e| e.name().to_ascii_lowercase().contains(SMOKE_NAME_PART))
        .map(|e| {
            format!(
                "{}: 配る束に開発用の確認の道具（{SMOKE_NAME_PART}）があります（--distribution でビルドする。ADR 0069 §6）",
                e.path.display()
            )
        })
        .collect()
}

/// 通常版は取得の部品がある。オフライン版は、束のどこにも無く、殻の署名も指していない
fn data_helper_problems(
    app: &Path,
    edition: Edition,
    entries: &[Entry],
) -> Result<Vec<String>, String> {
    let mut problems = Vec::new();
    match edition {
        Edition::Normal => {
            let helper = at(app, DATA_HELPER);
            if !is_macho(&helper)? {
                problems.push(format!(
                    "{}: 通常版に取得の部品（{DATA_NAME} の実行ファイル）がありません（ADR 0080 §7.3）",
                    helper.display()
                ));
            }
        }
        Edition::Offline => {
            for entry in entries
                .iter()
                .filter(|e| e.name().eq_ignore_ascii_case(DATA_NAME))
            {
                problems.push(format!(
                    "{}: オフライン版の束に取得の部品（{DATA_NAME}）があります（ADR 0080 §7.3）",
                    entry.path.display()
                ));
            }
            let resources = at(app, APP_CODE_RESOURCES);
            let text = std::fs::read(&resources).map_err(|e| {
                format!(
                    "{} を読めません（署名していない束）: {e}",
                    resources.display()
                )
            })?;
            if contains(&text, format!("Helpers/{DATA_NAME}").as_bytes()) {
                problems.push(format!(
                    "{}: オフライン版の殻の署名が Helpers/{DATA_NAME} を指しています",
                    resources.display()
                ));
            }
        }
    }
    Ok(problems)
}

/// 束の中の実行ファイル（Mach-O）の文字列の検査
fn executable_problems(
    app: &Path,
    edition: Edition,
    entries: &[Entry],
) -> Result<Vec<String>, String> {
    let mut problems = Vec::new();
    let mut seen_marks: Vec<&str> = Vec::new();
    let mut seen_files: Vec<&str> = Vec::new();
    for entry in entries.iter().filter(|e| e.kind == Kind::File) {
        if !is_macho(&entry.path)? {
            continue;
        }
        seen_files.push(&entry.rel);
        let bytes = std::fs::read(&entry.path)
            .map_err(|e| format!("{} を読めません: {e}", entry.path.display()))?;
        let file = entry.path.display();
        for word in find_dev_strings(&bytes) {
            problems.push(format!(
                "{file}: 開発用の文字列 `{word}` が入っています（feature `dev-knobs` の外に出ています）"
            ));
        }
        for word in find_probe(&bytes) {
            problems.push(format!(
                "{file}: 測定ツールの印 `{word}` が入っています（tools/npu-probe は製品に入れない。ADR 0075 §4）"
            ));
        }
        if entry.name() != LM_HOST_NAME {
            for word in find_llama(&bytes) {
                problems.push(format!(
                    "{file}: llama.cpp の印 `{word}` が入っています（LM は別プロセスの {LM_HOST_NAME} だけが持つ。ADR 0072 §5）"
                ));
            }
        }
        for mark in find_fetch_marks(&bytes) {
            if edition == Edition::Offline {
                problems.push(format!(
                    "{file}: オフライン版の実行ファイルに取得の印 `{mark}` が入っています（通信の部品が入っている。ADR 0080 §7.4）"
                ));
            }
            if !seen_marks.contains(&mark) {
                seen_marks.push(mark);
            }
        }
    }
    for required in REQUIRED_EXECUTABLES {
        if !seen_files.contains(required) {
            problems.push(format!(
                "{}: 実行ファイル（Mach-O）として見ていません（無いか、Mach-O でない。検査が空振りしている）",
                at(app, required).display()
            ));
        }
    }
    if edition == Edition::Normal {
        for mark in FETCH_MARKS.iter().filter(|m| !seen_marks.contains(m)) {
            problems.push(format!(
                "{}: 通常版のどの実行ファイルにも取得の印 `{mark}` がありません（印の表が古く、オフライン版の検査が空振りしうる。xtask/src/scan/macos.rs の FETCH_MARKS を直す）",
                app.display()
            ));
        }
    }
    Ok(problems)
}

/// 取得の印のうち、`bytes` に入っているもの
pub fn find_fetch_marks(bytes: &[u8]) -> Vec<&'static str> {
    FETCH_MARKS
        .iter()
        .copied()
        .filter(|mark| contains(bytes, mark.as_bytes()))
        .collect()
}

/// 同梱のデータ（ADR 0080 §3.1、§3.4）
fn bundled_data_problems(app: &Path, pinned: &Pinned) -> Result<Vec<String>, String> {
    let data = at(app, DATA_REL);
    if !data.is_dir() {
        return Ok(vec![format!(
            "{}: 同梱のデータの場所がありません（配る物は辞書とモデルを同梱する。ADR 0080 §0）",
            data.display()
        )]);
    }
    let mut problems = Vec::new();
    let entries = walk(&data)?;
    for entry in &entries {
        if let Some(problem) = data_entry_problem(entry, pinned) {
            problems.push(format!("{}: {problem}", entry.path.display()));
        }
    }
    for name in DICT_REQUIRED {
        let rel = format!("{DICT_DIR}/{name}");
        if !entries.iter().any(|e| e.rel == rel && e.kind == Kind::File) {
            problems.push(format!("{}: 辞書がありません", at(&data, &rel).display()));
        }
    }
    let model = at(&data, &format!("{MODELS_DIR}/{}", pinned.model_file));
    match model.is_file() {
        false => problems.push(format!("{}: モデルがありません", model.display())),
        true => {
            let got = sha256_file(&model)
                .map_err(|e| format!("{} を読めません: {e}", model.display()))?;
            if got != pinned.model_sha256 {
                problems.push(format!(
                    "{}: モデルの sha256 が表の値と違います（{got}。表は {}。resource/src/models.rs）",
                    model.display(),
                    pinned.model_sha256
                ));
            }
        }
    }
    // データの全ファイルが、エンジンの束の署名に載っている（封印から漏れていない）
    let resources = at(app, XPC_CODE_RESOURCES);
    let seal = std::fs::read(&resources).map_err(|e| {
        format!(
            "{} を読めません（署名していない束）: {e}",
            resources.display()
        )
    })?;
    for entry in entries.iter().filter(|e| e.kind == Kind::File) {
        let key = format!("<key>{SEAL_KEY_PREFIX}{}</key>", entry.rel);
        if !contains(&seal, key.as_bytes()) {
            problems.push(format!(
                "{}: エンジンの署名（{XPC_CODE_RESOURCES}）に載っていません（封印から漏れている）",
                entry.path.display()
            ));
        }
    }
    Ok(problems)
}

/// データの場所の 1 項目が、置いてよい物か（よければ None）
fn data_entry_problem(entry: &Entry, pinned: &Pinned) -> Option<String> {
    if entry.kind == Kind::Symlink {
        return Some("同梱のデータにシンボリックリンクを置かない".to_owned());
    }
    let parts: Vec<&str> = entry.rel.split('/').collect();
    let allowed = match (parts.as_slice(), entry.kind) {
        ([dir], Kind::Dir) => DATA_TOP_DIRS.contains(dir),
        ([file], Kind::File) => DATA_TOP_FILES.contains(file),
        ([DICT_DIR, file], Kind::File) => {
            DICT_REQUIRED.contains(file) || DICT_OPTIONAL.contains(file)
        }
        ([MODELS_DIR, file], Kind::File) => *file == pinned.model_file,
        (["licenses", ..], _) => true,
        _ => false,
    };
    (!allowed).then(|| {
        format!(
            "同梱のデータに置く物ではありません（{DICT_DIR}/ の {}、{MODELS_DIR}/{}、{}、licenses/ だけ）",
            DICT_REQUIRED.iter().chain(DICT_OPTIONAL).copied().collect::<Vec<_>>().join("・"),
            pinned.model_file,
            DATA_TOP_FILES.join("・")
        )
    })
}

/// Mach-O か（先頭の 4 バイト）。ファイルでなければ false。読めなければ Err（通さない）
fn is_macho(path: &Path) -> Result<bool, String> {
    use std::io::Read;
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("{} を読めません: {e}", path.display())),
    };
    if !meta.is_file() {
        return Ok(false);
    }
    let mut head = [0u8; 4];
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("{} を読めません: {e}", path.display()))?;
    match file.read_exact(&mut head) {
        Ok(()) => Ok(MACHO_MAGICS.contains(&head)),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(format!("{} を読めません: {e}", path.display())),
    }
}

/// 束の中の相対パス（区切りは `/`）を、OS のパスにする
fn at(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    super::contains(haystack, needle)
}

#[cfg(test)]
mod tests;
