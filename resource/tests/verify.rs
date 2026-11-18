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
        profile: None,
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
        size: MODEL_BYTES.len() as u64,
    });
}

fn verify(dir: &Path, installed: Option<&Manifest>) -> Result<(), ResourceError> {
    let mut models = Vec::new();
    known(&mut models);
    let opts = VerifyOptions {
        formats: SupportedFormats::current(),
        known_models: &models,
        user_model_formats: &[],
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
        user_model_formats: &[],
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
        user_model_formats: &[],
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

// ---- verify_model_file（models/ の単体の照合） ----

fn model_file(content: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(MODEL_NAME);
    fs::write(&path, content).unwrap();
    (dir, path)
}

fn known_table() -> Vec<KnownModel> {
    let mut models = Vec::new();
    known(&mut models);
    models
}

#[test]
fn model_file_with_known_name_size_and_sha256_passes() {
    let (_d, path) = model_file(MODEL_BYTES);
    assert!(ikari_resource::verify_model_file(&path, MODEL_NAME, &known_table()).is_ok());
}

#[test]
fn model_file_with_other_content_of_same_size_is_a_sha256_mismatch() {
    let mut bytes = MODEL_BYTES.to_vec();
    bytes[0] ^= 0xff;
    let (_d, path) = model_file(&bytes);
    assert!(matches!(
        ikari_resource::verify_model_file(&path, MODEL_NAME, &known_table()),
        Err(ResourceError::Sha256Mismatch { .. })
    ));
}

#[test]
fn model_file_of_unexpected_size_is_rejected_by_size() {
    let (_d, path) = model_file(&MODEL_BYTES[1..]);
    assert!(matches!(
        ikari_resource::verify_model_file(&path, MODEL_NAME, &known_table()),
        Err(ResourceError::Truncated { .. })
    ));
    let mut longer = MODEL_BYTES.to_vec();
    longer.push(0);
    let (_d2, path2) = model_file(&longer);
    assert!(matches!(
        ikari_resource::verify_model_file(&path2, MODEL_NAME, &known_table()),
        Err(ResourceError::SizeMismatch { .. })
    ));
}

#[test]
fn model_file_with_unknown_name_or_missing_is_rejected() {
    let (_d, path) = model_file(MODEL_BYTES);
    assert!(matches!(
        ikari_resource::verify_model_file(&path, "other.gguf", &known_table()),
        Err(ResourceError::UnknownModel { .. })
    ));
    let gone = path.with_file_name("nothing");
    assert!(matches!(
        ikari_resource::verify_model_file(&gone, MODEL_NAME, &known_table()),
        Err(ResourceError::Io { .. })
    ));
}

// ---- 自由指定のモデルの束（D-4） ----

fn user_bundle(inbox: &Path, file_name: Option<&str>, origin: Origin) -> std::path::PathBuf {
    let sha = sha256_reader(MODEL_BYTES).unwrap();
    let id = ikari_resource::UserModelId::from_sha256(&sha).unwrap();
    let plan = ikari_resource::StagePlan {
        name: id.to_string(),
        file_name: file_name.map_or_else(|| id.file_name(), str::to_owned),
        kind: ComponentKind::Model,
        origin,
        created: Created::parse(CREATED).unwrap(),
        expected: None,
        profile_format: Some("zenz-v3".into()),
        display_name: None,
        source: None,
        max_size: 1024,
    };
    ikari_resource::stage_model_bundle(&mut &MODEL_BYTES[..], inbox, &plan)
        .unwrap()
        .path
}

#[test]
fn user_models_pass_only_with_a_known_format_and_a_name_from_their_sha256() {
    let inbox = tempfile::tempdir().unwrap();
    let dir = user_bundle(inbox.path(), None, Origin::User);
    // 既定（形式を渡さない）では受けない
    assert!(matches!(
        verify_bundle(&dir, &VerifyOptions::default()),
        Err(ResourceError::UnknownModel { .. })
    ));
    let opts = VerifyOptions {
        user_model_formats: &["zenz-v3"],
        ..VerifyOptions::default()
    };
    verify_bundle(&dir, &opts).unwrap();
    // 知らない形式は受けない
    let other = VerifyOptions {
        user_model_formats: &["kanakanji-edge"],
        ..VerifyOptions::default()
    };
    assert!(verify_bundle(&dir, &other).is_err());
    // `origin` の語は見ない（プリセットと書いても、表に無ければ自由指定として形式の検査を受ける）
    let inbox2 = tempfile::tempdir().unwrap();
    let preset_labelled = user_bundle(inbox2.path(), None, Origin::Preset);
    verify_bundle(&preset_labelled, &opts).unwrap();
    // ファイル名が sha256 から作ったものでなければ受けない
    let inbox3 = tempfile::tempdir().unwrap();
    let renamed = user_bundle(inbox3.path(), Some("user-000000000000.gguf"), Origin::User);
    assert!(matches!(
        verify_bundle(&renamed, &opts),
        Err(ResourceError::UnknownModel { .. })
    ));
}
