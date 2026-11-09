//! 束の検証の単体テスト（一時ディレクトリ。ADR 0058 §12.1）。壊れ方ごとに、区別できるエラーで拒む。

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use ikari_resource::{
    Component, ComponentKind, Created, FileEntry, Freshness, KnownModel, MANIFEST_FILE_NAME,
    Manifest, Origin, ResourceError, SupportedFormats, VerifyOptions, sha256_reader, verify_bundle,
};

const DICT_FORMAT: u32 = ikari_dict_format::FORMAT_VERSION;
const CREATED: &str = "2026-11-10T03:00:00Z";
const MODEL_NAME: &str = "test-model.gguf";
const MODEL_BYTES: &[u8] = b"pretend-gguf";

fn entry(dir: &Path, name: &str, bytes: &[u8]) -> FileEntry {
    fs::write(dir.join(name), bytes).unwrap();
    FileEntry {
        name: name.into(),
        size: bytes.len() as u64,
        sha256: sha256_reader(bytes).unwrap(),
    }
}

fn component(kind: ComponentKind, format: u32, files: Vec<FileEntry>) -> Component {
    Component {
        kind,
        format,
        files,
        builder: None,
        sources: None,
    }
}

/// 辞書 + 郵便番号の表 + モデルの正常な束を作る
fn good_bundle(dir: &Path) -> Manifest {
    let mut c = BTreeMap::new();
    c.insert(
        "dict".to_owned(),
        component(
            ComponentKind::Dict,
            DICT_FORMAT,
            vec![
                entry(dir, "grammar.bin", b"grammar-bytes"),
                entry(dir, "system.dict", &vec![7u8; 20_000]), // 8KB のバッファをまたぐ
                entry(dir, "vectors.bin", b"vectors-bytes"),
            ],
        ),
    );
    c.insert(
        "postal".to_owned(),
        component(
            ComponentKind::Table,
            1,
            vec![entry(dir, "postal.tsv", b"1000001\tTokyo\n")],
        ),
    );
    c.insert(
        "model".to_owned(),
        component(
            ComponentKind::Model,
            1,
            vec![entry(dir, MODEL_NAME, MODEL_BYTES)],
        ),
    );
    let m = Manifest::new(Created::parse(CREATED).unwrap(), Origin::Export, c);
    write_manifest(dir, &m);
    m
}

fn write_manifest(dir: &Path, m: &Manifest) {
    m.write(&dir.join(MANIFEST_FILE_NAME)).unwrap();
}

fn known(models: &mut Vec<KnownModel>) {
    models.push(KnownModel {
        file_name: MODEL_NAME,
        sha256: Box::leak(sha256_reader(MODEL_BYTES).unwrap().into_boxed_str()),
    });
}

fn verify(dir: &Path, installed: Option<&Manifest>) -> Result<(), ResourceError> {
    let mut models = Vec::new();
    known(&mut models);
    let opts = VerifyOptions {
        formats: SupportedFormats::current(),
        known_models: &models,
        installed,
    };
    verify_bundle(dir, &opts).map(|_| ())
}

#[test]
fn good_bundle_passes() {
    let dir = tempfile::tempdir().unwrap();
    good_bundle(dir.path());
    let mut models = Vec::new();
    known(&mut models);
    let opts = VerifyOptions {
        formats: SupportedFormats::current(),
        known_models: &models,
        installed: None,
    };
    let v = verify_bundle(dir.path(), &opts).unwrap();
    assert_eq!(v.freshness.len(), 3);
    assert!(v.freshness.values().all(|f| *f == Freshness::Newer));
}

#[test]
fn partial_bundle_with_only_a_table_passes() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = good_bundle(dir.path());
    m.components.retain(|k, _| k == "postal");
    for f in ["grammar.bin", "system.dict", "vectors.bin", MODEL_NAME] {
        fs::remove_file(dir.path().join(f)).unwrap();
    }
    write_manifest(dir.path(), &m);
    verify(dir.path(), None).unwrap();
}

#[test]
fn missing_manifest() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        verify(dir.path(), None),
        Err(ResourceError::ManifestMissing)
    ));
}

#[test]
fn corrupt_manifest() {
    let dir = tempfile::tempdir().unwrap();
    good_bundle(dir.path());
    fs::write(dir.path().join(MANIFEST_FILE_NAME), "{ broken").unwrap();
    assert!(matches!(
        verify(dir.path(), None),
        Err(ResourceError::ManifestMalformed(_))
    ));
}

#[test]
fn unknown_schema() {
    let dir = tempfile::tempdir().unwrap();
    good_bundle(dir.path());
    let path = dir.path().join(MANIFEST_FILE_NAME);
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("\"schema\": 1", "\"schema\": 99");
    fs::write(&path, text).unwrap();
    assert!(matches!(
        verify(dir.path(), None),
        Err(ResourceError::UnknownSchema { found: 99, .. })
    ));
}

#[test]
fn truncated_file() {
    let dir = tempfile::tempdir().unwrap();
    good_bundle(dir.path());
    let p = dir.path().join("system.dict");
    let f = fs::OpenOptions::new().write(true).open(&p).unwrap();
    f.set_len(100).unwrap();
    assert!(matches!(
        verify(dir.path(), None),
        Err(ResourceError::Truncated { name, expected: 20_000, actual: 100 }) if name == "system.dict"
    ));
}

#[test]
fn oversized_file() {
    let dir = tempfile::tempdir().unwrap();
    good_bundle(dir.path());
    fs::write(dir.path().join("grammar.bin"), b"grammar-bytes-and-more").unwrap();
    assert!(matches!(
        verify(dir.path(), None),
        Err(ResourceError::SizeMismatch { .. })
    ));
}

#[test]
fn sha256_mismatch_with_same_size() {
    let dir = tempfile::tempdir().unwrap();
    good_bundle(dir.path());
    fs::write(dir.path().join("grammar.bin"), b"GRAMMAR-BYTES").unwrap(); // 同じ長さ
    assert!(matches!(
        verify(dir.path(), None),
        Err(ResourceError::Sha256Mismatch { name, .. }) if name == "grammar.bin"
    ));
}

#[test]
fn file_in_manifest_but_missing() {
    let dir = tempfile::tempdir().unwrap();
    good_bundle(dir.path());
    fs::remove_file(dir.path().join("postal.tsv")).unwrap();
    assert!(matches!(
        verify(dir.path(), None),
        Err(ResourceError::MissingFile { name }) if name == "postal.tsv"
    ));
}

#[test]
fn file_in_bundle_but_not_in_manifest() {
    let dir = tempfile::tempdir().unwrap();
    good_bundle(dir.path());
    fs::write(dir.path().join("extra.bin"), b"x").unwrap();
    assert!(matches!(
        verify(dir.path(), None),
        Err(ResourceError::UnlistedFile { name }) if name == "extra.bin"
    ));
}

#[test]
fn subdirectory_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    good_bundle(dir.path());
    fs::create_dir(dir.path().join("sub")).unwrap();
    assert!(matches!(
        verify(dir.path(), None),
        Err(ResourceError::UnlistedFile { .. })
    ));
}

#[test]
fn format_version_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = good_bundle(dir.path());
    m.components.get_mut("dict").unwrap().format = DICT_FORMAT + 1;
    write_manifest(dir.path(), &m);
    assert!(matches!(
        verify(dir.path(), None),
        Err(ResourceError::FormatMismatch { component, found, supported })
            if component == "dict" && found == DICT_FORMAT + 1 && supported == DICT_FORMAT
    ));
}

#[test]
fn partial_dict_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = good_bundle(dir.path());
    m.components
        .get_mut("dict")
        .unwrap()
        .files
        .retain(|f| f.name != "vectors.bin");
    fs::remove_file(dir.path().join("vectors.bin")).unwrap();
    write_manifest(dir.path(), &m);
    assert!(matches!(
        verify(dir.path(), None),
        Err(ResourceError::IncompleteDict { missing, .. }) if missing == "vectors.bin"
    ));
}

#[test]
fn unknown_model_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    good_bundle(dir.path());
    // 既知の表（本物の KNOWN_MODELS）にはテスト用のモデルはない
    let opts = VerifyOptions {
        installed: None,
        ..VerifyOptions::default()
    };
    assert!(matches!(
        verify_bundle(dir.path(), &opts),
        Err(ResourceError::UnknownModel { name, .. }) if name == MODEL_NAME
    ));
}

#[test]
fn known_name_with_different_content_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = good_bundle(dir.path());
    let f = &mut m.components.get_mut("model").unwrap().files[0];
    fs::write(dir.path().join(MODEL_NAME), b"other-gguf!!").unwrap(); // 同じ長さ
    f.sha256 = sha256_reader(&b"other-gguf!!"[..]).unwrap(); // manifest を合わせても、表にない
    write_manifest(dir.path(), &m);
    assert!(matches!(
        verify(dir.path(), None),
        Err(ResourceError::UnknownModel { .. })
    ));
}

#[test]
fn path_escape_in_manifest() {
    for bad in ["../evil.bin", "/etc/passwd", "sub/x.bin", "..", "a\\b"] {
        let dir = tempfile::tempdir().unwrap();
        let mut m = good_bundle(dir.path());
        m.components.get_mut("postal").unwrap().files[0].name = bad.into();
        // to_json は検査しないので、そのまま書いて読む側で拒まれることを確かめる
        fs::write(dir.path().join(MANIFEST_FILE_NAME), m.to_json().unwrap()).unwrap();
        assert!(
            matches!(
                verify(dir.path(), None),
                Err(ResourceError::UnsafePath { .. })
            ),
            "{bad}"
        );
    }
}

#[cfg(unix)]
#[test]
fn symlink_is_rejected() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    good_bundle(dir.path());
    // 載っているファイルを、同じ中身の外のファイルへのリンクに替える
    fs::write(outside.path().join("postal.tsv"), b"1000001\tTokyo\n").unwrap();
    fs::remove_file(dir.path().join("postal.tsv")).unwrap();
    symlink(
        outside.path().join("postal.tsv"),
        dir.path().join("postal.tsv"),
    )
    .unwrap();
    assert!(matches!(
        verify(dir.path(), None),
        Err(ResourceError::Symlink { name }) if name == "postal.tsv"
    ));
}

#[cfg(unix)]
#[test]
fn symlinked_manifest_is_rejected() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    good_bundle(dir.path());
    let real = outside.path().join("m.json");
    fs::rename(dir.path().join(MANIFEST_FILE_NAME), &real).unwrap();
    symlink(&real, dir.path().join(MANIFEST_FILE_NAME)).unwrap();
    assert!(matches!(
        verify(dir.path(), None),
        Err(ResourceError::Symlink { .. })
    ));
}

#[test]
fn older_bundle_is_rejected_and_same_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = good_bundle(dir.path());

    // 今の版が、束より新しい（sha256 は違う）
    let mut installed = bundle.clone();
    installed.created = Created::parse("2026-12-01T00:00:00Z").unwrap();
    installed.components.get_mut("postal").unwrap().files[0].sha256 = "b".repeat(64);
    assert!(matches!(
        verify(dir.path(), Some(&installed)),
        Err(ResourceError::Stale { component, .. }) if component == "postal"
    ));

    // 中身が同じなら、日付が新しい今の版でも「同じ版」（拒まない）
    let mut same = bundle.clone();
    same.created = Created::parse("2026-12-01T00:00:00Z").unwrap();
    let mut models = Vec::new();
    known(&mut models);
    let opts = VerifyOptions {
        formats: SupportedFormats::current(),
        known_models: &models,
        installed: Some(&same),
    };
    let v = verify_bundle(dir.path(), &opts).unwrap();
    assert!(v.freshness.values().all(|f| *f == Freshness::Same));

    // 今の版が古ければ入れる
    let mut old = bundle.clone();
    old.created = Created::parse("2026-01-01T00:00:00Z").unwrap();
    old.components.get_mut("postal").unwrap().files[0].sha256 = "b".repeat(64);
    let opts = VerifyOptions {
        installed: Some(&old),
        ..opts
    };
    let v = verify_bundle(dir.path(), &opts).unwrap();
    assert_eq!(v.freshness["postal"], Freshness::Newer);
}
