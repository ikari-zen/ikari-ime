//! `macos.rs` のテスト。偽の束（Mach-O の先頭を持つ小さなファイル、署名の代わりの `CodeResources`）で、
//! 正しい束は通り、悪い束は理由の文つきで落ちることを確かめる

use super::*;

/// 64 ビットの Mach-O の先頭
const MACHO: &[u8] = &[0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0x00, 0x00, 0x01];
const MODEL: &[u8] = b"fake-model-payload";
const MODEL_FILE: &str = "model.gguf";
const SETTINGS_EXE: &str = "Contents/Helpers/IkariSettings.app/Contents/MacOS/IkariSettings";
const ENGINE_EXE: &str = "Contents/XPCServices/IkariEngine.xpc/Contents/MacOS/IkariEngine";
/// 偽の ikari-data の中身（取得の印の 3 つと、通常版の ikari-data にだけ許す OS の通信の部品）
const DATA_CHILD_BODY: &[u8] =
    b"ureq/2 HTTPS_PROXY https://github.com/x/y/releases/latest getaddrinfo SecTrust";

fn exe(body: &[u8]) -> Vec<u8> {
    [MACHO, body].concat()
}

fn write(root: &Path, relative: &str, bytes: &[u8]) {
    let path = at(root, relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// 偽の束（署名の代わりに、CodeResources にデータの鍵を書く）。通常版は取得の部品と、取得の印を持つ実行ファイルを置く
struct Fake {
    dir: tempfile::TempDir,
    sha: String,
}

impl Fake {
    fn new(edition: Edition) -> Fake {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("ikari.app");
        write(&app, "Contents/Info.plist", b"<plist/>");
        write(&app, "Contents/MacOS/ikari", &exe(b"shell"));
        write(&app, ENGINE_EXE, &exe(b"engine"));
        write(
            &app,
            "Contents/XPCServices/IkariLMCage.xpc/Contents/MacOS/ikari-lm-cage",
            &exe(b"cage"),
        );
        // LM の子は llama.cpp を持ってよい
        write(
            &app,
            "Contents/XPCServices/IkariLMCage.xpc/Contents/MacOS/ikari-lm-host",
            &exe(b"llama_model_load ggml_backend"),
        );
        write(&app, "Contents/Resources/keymap/ms-ime.tsv", b"a\tb\n");
        // 今の実物と同じく、取得の印は ikari-data にだけある（設定アプリは取得の核をリンクしていない）
        write(&app, SETTINGS_EXE, &exe(b"settings"));
        let mut app_seal = b"<key>files2</key>".to_vec();
        if edition == Edition::Normal {
            write(&app, DATA_HELPER, &exe(DATA_CHILD_BODY));
            app_seal.extend_from_slice(b"<key>Helpers/ikari-data</key>");
        }
        write(&app, APP_CODE_RESOURCES, &app_seal);
        let data = at(&app, DATA_REL);
        for name in DICT_REQUIRED.iter().chain(&["BUILD_INFO"]) {
            write(&data, &format!("dict/{name}"), b"dict");
        }
        write(&data, &format!("models/{MODEL_FILE}"), MODEL);
        write(&data, "NOTICE.txt", b"notice");
        write(&data, "licenses/Apache-2.0.txt", b"license");
        let sha = sha256_file(&at(&data, &format!("models/{MODEL_FILE}"))).unwrap();
        let fake = Fake { dir, sha };
        fake.reseal();
        fake
    }

    fn app(&self) -> PathBuf {
        self.dir.path().join("ikari.app")
    }

    fn data(&self) -> PathBuf {
        at(&self.app(), DATA_REL)
    }

    /// データの今のファイルを、すべてエンジンの署名に載せる（署名し直しに当たる）
    fn reseal(&self) {
        let mut seal = String::from("<plist><dict><key>files2</key><dict>");
        for entry in walk(&self.data())
            .unwrap()
            .iter()
            .filter(|e| e.kind == Kind::File)
        {
            seal.push_str(&format!("<key>{SEAL_KEY_PREFIX}{}</key><dict/>", entry.rel));
        }
        write(&self.app(), XPC_CODE_RESOURCES, seal.as_bytes());
    }

    fn check(&self, edition: Edition) -> Result<Vec<String>, String> {
        let pinned = Pinned {
            model_file: MODEL_FILE,
            model_sha256: &self.sha,
        };
        check_with(&self.app(), edition, &pinned)
    }

    fn problems(&self, edition: Edition) -> Vec<String> {
        self.check(edition).unwrap()
    }
}

fn assert_one(found: &[String], want: &str) {
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains(want), "'{want}' を含まない: {found:?}");
}

#[test]
fn good_bundles_pass() {
    for edition in [Edition::Normal, Edition::Offline] {
        let fake = Fake::new(edition);
        assert_eq!(fake.problems(edition), Vec::<String>::new(), "{edition:?}");
    }
}

#[test]
fn not_a_bundle_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(check_macos_app(&dir.path().join("none.app"), Edition::Normal).is_err());
    assert!(check_macos_app(dir.path(), Edition::Offline).is_err());
}

#[test]
fn a_normal_bundle_checked_as_offline_fails_for_the_data_child_and_every_mark() {
    let fake = Fake::new(Edition::Normal);
    let found = fake.problems(Edition::Offline);
    // ikari-data の実行ファイル、殻の署名、ikari-data の取得の印 3 つ、OS の通信の部品（getaddrinfo。オフライン版では許さない）
    assert_eq!(found.len(), 6, "{found:?}");
    assert!(
        found
            .iter()
            .any(|p| p.contains("オフライン版の束に取得の部品（ikari-data）があります"))
    );
    assert!(
        found
            .iter()
            .any(|p| p.contains("Helpers/ikari-data を指しています"))
    );
    for mark in FETCH_MARKS {
        assert!(
            found
                .iter()
                .any(|p| p.contains(&format!("取得の印 `{mark}`"))),
            "{mark}: {found:?}"
        );
    }
}

#[test]
fn an_offline_bundle_with_a_fetch_mark_in_any_executable_fails() {
    // 名前を変えて置いた取得の部品（ikari-data の名前の検査をすり抜ける）も、印で落ちる
    let fake = Fake::new(Edition::Offline);
    write(&fake.app(), "Contents/Helpers/fetcher", &exe(b"ureq/2.12"));
    assert_one(&fake.problems(Edition::Offline), "取得の印 `ureq`");

    // 設定アプリに取得の起こし方（子に渡すプロキシの環境の表）が入った
    let fake = Fake::new(Edition::Offline);
    write(
        &fake.app(),
        SETTINGS_EXE,
        &exe(b"settings HTTPS_PROXY https_proxy"),
    );
    assert_one(&fake.problems(Edition::Offline), "取得の印 `HTTPS_PROXY`");
}

#[test]
fn os_network_parts_outside_the_normal_data_child_fail() {
    // オフライン版の設定アプリが Swift で直接通信する（URLSession と、その framework の道）
    let fake = Fake::new(Edition::Offline);
    write(
        &fake.app(),
        SETTINGS_EXE,
        &exe(b"settings _OBJC_CLASS_$_NSURLSession /System/Library/Frameworks/CFNetwork.framework/CFNetwork"),
    );
    let found = fake.problems(Edition::Offline);
    // CFNetwork の framework の道は Network.framework にも当たる
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(
        found
            .iter()
            .any(|p| p.contains("OS の通信の部品（`URLSession`）")),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .any(|p| p.contains("OS の通信の部品（`CFNetwork`）")),
        "{found:?}"
    );

    // 通常版でも、殻やエンジンは通信しない（Network.framework の NWConnection）
    let fake = Fake::new(Edition::Normal);
    write(
        &fake.app(),
        "Contents/MacOS/ikari",
        &exe(b"shell 7Network12NWConnectionC"),
    );
    assert_one(
        &fake.problems(Edition::Normal),
        "OS の通信の部品（`NWConnection`）",
    );
    let fake = Fake::new(Edition::Normal);
    write(
        &fake.app(),
        ENGINE_EXE,
        &exe(b"engine /System/Library/Frameworks/Network.framework/Network"),
    );
    assert_one(
        &fake.problems(Edition::Normal),
        "OS の通信の部品（`Network.framework`）",
    );

    // 許すのは通常版の Contents/Helpers/ikari-data だけ。名前を変えて置いた物は落ちる
    let fake = Fake::new(Edition::Normal);
    write(
        &fake.app(),
        "Contents/Helpers/fetcher",
        &exe(b"getaddrinfo"),
    );
    assert_one(
        &fake.problems(Edition::Normal),
        "OS の通信の部品（`getaddrinfo`）",
    );
}

#[test]
fn marks_in_non_executables_are_not_counted() {
    // 文書（README の URL など）は実行ファイルではない
    let fake = Fake::new(Edition::Offline);
    write(
        &fake.app(),
        "Contents/Resources/README.md",
        b"https://github.com/x/y/releases/latest ureq",
    );
    assert_eq!(fake.problems(Edition::Offline), Vec::<String>::new());
}

#[test]
fn an_offline_bundle_with_a_renamed_or_unsealed_data_child_fails() {
    let fake = Fake::new(Edition::Offline);
    write(
        &fake.app(),
        "Contents/Resources/IKARI-DATA",
        b"not even mach-o",
    );
    assert_one(
        &fake.problems(Edition::Offline),
        "オフライン版の束に取得の部品",
    );

    let fake = Fake::new(Edition::Offline);
    write(
        &fake.app(),
        APP_CODE_RESOURCES,
        b"<key>Helpers/ikari-data</key>",
    );
    assert_one(
        &fake.problems(Edition::Offline),
        "Helpers/ikari-data を指しています",
    );

    // 殻の署名が無い束は、読めないので通さない
    let fake = Fake::new(Edition::Offline);
    std::fs::remove_file(at(&fake.app(), APP_CODE_RESOURCES)).unwrap();
    assert!(fake.check(Edition::Offline).is_err());
}

#[test]
fn a_normal_bundle_without_the_data_child_fails() {
    let fake = Fake::new(Edition::Normal);
    std::fs::remove_file(at(&fake.app(), DATA_HELPER)).unwrap();
    let found = fake.problems(Edition::Normal);
    // 部品が無い、ikari-data にしかない印の 3 つがどこにも無い
    assert_eq!(found.len(), 4, "{found:?}");
    assert!(
        found
            .iter()
            .any(|p| p.contains("通常版に取得の部品（ikari-data の実行ファイル）がありません")),
        "{found:?}"
    );

    // Mach-O でない物を置いても通らない
    let fake = Fake::new(Edition::Normal);
    write(
        &fake.app(),
        DATA_HELPER,
        b"#!/bin/sh\nureq HTTPS_PROXY /releases/latest\n",
    );
    let found = fake.problems(Edition::Normal);
    assert!(
        found.iter().any(|p| p.contains("通常版に取得の部品")),
        "{found:?}"
    );
}

#[test]
fn a_normal_bundle_missing_a_mark_reports_a_stale_mark_table() {
    // 取得の部品の中身が変わって、印の 1 つが残らなくなった
    let fake = Fake::new(Edition::Normal);
    write(
        &fake.app(),
        DATA_HELPER,
        &exe(b"ureq/2 https://github.com/x/y/releases/latest"),
    );
    assert_one(
        &fake.problems(Edition::Normal),
        "取得の印 `HTTPS_PROXY` がありません（印の表が古く",
    );
}

#[test]
fn smoke_anywhere_fails() {
    let fake = Fake::new(Edition::Normal);
    write(&fake.app(), "Contents/MacOS/ikari-smoke", &exe(b"smoke"));
    write(&fake.app(), "Contents/Resources/Ikari-Smoke.dSYM/x", b"x");
    let found = fake.problems(Edition::Normal);
    // 実行ファイル、フォルダ（.dSYM）の 2 つ
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found
            .iter()
            .all(|p| p.contains("開発用の確認の道具（ikari-smoke）")),
        "{found:?}"
    );
}

#[test]
fn dev_strings_probe_and_llama_outside_the_lm_host_fail() {
    let fake = Fake::new(Edition::Offline);
    write(
        &fake.app(),
        ENGINE_EXE,
        &exe(b"engine IKARI_USER_DIR llama_model_load ikari_npu_probe"),
    );
    let found = fake.problems(Edition::Offline);
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(
        found
            .iter()
            .any(|p| p.contains("開発用の文字列 `IKARI_USER_DIR`"))
    );
    assert!(
        found
            .iter()
            .any(|p| p.contains("llama.cpp の印 `llama_model_load`"))
    );
    assert!(
        found
            .iter()
            .any(|p| p.contains("測定ツールの印 `ikari_npu_probe`"))
    );
}

#[test]
fn required_executables_must_be_seen_as_macho() {
    let fake = Fake::new(Edition::Offline);
    write(&fake.app(), "Contents/MacOS/ikari", b"#!/bin/sh\n");
    assert_one(
        &fake.problems(Edition::Offline),
        "Mach-O）として見ていません",
    );
    std::fs::remove_file(at(&fake.app(), "Contents/MacOS/ikari")).unwrap();
    assert_one(
        &fake.problems(Edition::Offline),
        "Mach-O）として見ていません",
    );
}

#[test]
fn missing_or_wrong_bundled_data_fails() {
    let fake = Fake::new(Edition::Normal);
    std::fs::remove_file(at(&fake.data(), "dict/vectors.bin")).unwrap();
    fake.reseal();
    assert_one(&fake.problems(Edition::Normal), "辞書がありません");

    let fake = Fake::new(Edition::Normal);
    std::fs::remove_file(at(&fake.data(), &format!("models/{MODEL_FILE}"))).unwrap();
    fake.reseal();
    assert_one(&fake.problems(Edition::Normal), "モデルがありません");

    let fake = Fake::new(Edition::Normal);
    write(
        &fake.data(),
        &format!("models/{MODEL_FILE}"),
        b"other-model",
    );
    fake.reseal();
    assert_one(
        &fake.problems(Edition::Normal),
        "モデルの sha256 が表の値と違います",
    );

    let fake = Fake::new(Edition::Offline);
    std::fs::remove_dir_all(fake.data()).unwrap();
    assert_one(
        &fake.problems(Edition::Offline),
        "同梱のデータの場所がありません",
    );
}

#[test]
fn unexpected_data_entries_fail() {
    let fake = Fake::new(Edition::Normal);
    write(&fake.data(), "dict/extra.bin", b"x");
    write(&fake.data(), "models/other.gguf", b"x");
    write(&fake.data(), "run.sh", b"x");
    // フォルダ（cache/）と、その中のファイルの 2 つ
    write(&fake.data(), "cache/x", b"x");
    #[cfg(unix)]
    std::os::unix::fs::symlink("system.dict", at(&fake.data(), "dict/link")).unwrap();
    fake.reseal();
    let found = fake.problems(Edition::Normal);
    let not_allowed = found
        .iter()
        .filter(|p| p.contains("同梱のデータに置く物ではありません"))
        .count();
    assert_eq!(not_allowed, 5, "{found:?}");
    let links = found
        .iter()
        .filter(|p| p.contains("シンボリックリンクを置かない"))
        .count();
    assert_eq!(links, usize::from(cfg!(unix)), "{found:?}");
    assert_eq!(found.len(), not_allowed + links, "{found:?}");
}

#[test]
fn data_not_in_the_engine_seal_fails() {
    // 署名のあとに足した物（署名し直していない）
    let fake = Fake::new(Edition::Normal);
    write(&fake.data(), "dict/emoji.tsv", b"x");
    assert_one(&fake.problems(Edition::Normal), "エンジンの署名");

    // エンジンの署名が無い束は、読めないので通さない
    let fake = Fake::new(Edition::Offline);
    std::fs::remove_file(at(&fake.app(), XPC_CODE_RESOURCES)).unwrap();
    assert!(fake.check(Edition::Offline).is_err());
}

#[test]
fn the_real_table_is_the_resource_crate() {
    assert_eq!(PINNED.model_file, ZENZ_SMALL_FILE_NAME);
    assert_eq!(PINNED.model_sha256, ZENZ_SMALL_SHA256);
}

/// 名前の表は lib-data.sh と同じ（build.sh が置く物と、ここが許す物が食い違わない）
#[test]
fn data_names_match_lib_data_sh() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let text = std::fs::read_to_string(root.join("macos/scripts/lib-data.sh")).unwrap();
    let array = |name: &str| -> Vec<String> {
        let line = text
            .lines()
            .find(|l| l.starts_with(&format!("readonly {name}=(")))
            .unwrap_or_else(|| panic!("lib-data.sh に {name} がありません"));
        let inner = line.split_once('(').unwrap().1.trim_end_matches(')');
        inner.split_whitespace().map(str::to_owned).collect()
    };
    assert_eq!(array("BUNDLED_DICT_REQUIRED"), DICT_REQUIRED);
    assert_eq!(array("BUNDLED_DICT_OPTIONAL"), DICT_OPTIONAL);
    assert!(text.contains(&format!("readonly BUNDLED_DATA_REL=\"{DATA_REL}\"")));
}

#[test]
fn macho_magic_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let cases: [(&str, &[u8], bool); 4] = [
        ("thin", MACHO, true),
        ("fat", &[0xca, 0xfe, 0xba, 0xbe, 0, 0], true),
        ("script", b"#!/bin/sh\n", false),
        ("short", b"\xcf\xfa", false),
    ];
    for (name, bytes, want) in cases {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(is_macho(&path).unwrap(), want, "{name}");
    }
    assert!(!is_macho(&dir.path().join("none")).unwrap());
    assert!(!is_macho(dir.path()).unwrap());
}
