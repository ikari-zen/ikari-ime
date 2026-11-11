//! 読むファイルの「LF」「CRLF」「BOM つき」の 3 通りのテスト（ADR 0040 §2.9、W0-d）。
//!
//! 各ファイルを API で作って保存し（LF）、3 通りの形に書き直して読み、同じ内容が読めることを確かめる。
//! 学習・ユーザー辞書・プロファイル・取り込み元・取り込み候補のファイルを対象にする。

use std::fs;
use std::path::Path;

use ikari_textfile::testing::read_variants;

use super::{IngestCandidates, ProfileWeight};
use super::{
    IngestSource, IngestSources, LearningStore, Profile, UserDictEntry, UserDictionary, UserDir,
};

/// LF のファイルを読み、3 通りに書き直しては `check` に渡す。
fn for_each_variant(path: &Path, mut check: impl FnMut(&str)) {
    let lf = fs::read_to_string(path).unwrap();
    for (variant, text) in read_variants(&lf) {
        fs::write(path, text).unwrap();
        check(&format!("{variant:?}"));
    }
}

#[test]
fn learning_file_reads_in_all_variants() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("learning.tsv");
    let store = LearningStore::open(&path).unwrap();
    store.save().unwrap();
    let salt = store.salt();
    for_each_variant(&path, |variant| {
        let again = LearningStore::open(&path).unwrap_or_else(|e| panic!("{variant}: {e}"));
        assert_eq!(again.salt(), salt, "{variant}");
    });
}

#[test]
fn user_dictionary_reads_in_all_variants() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("user_dict.tsv");
    let mut dict = UserDictionary::default();
    dict.add(UserDictEntry {
        reading: "いかり".into(),
        surface: "碇".into(),
        pos: "名詞".into(),
        comment: "メモ".into(),
    })
    .unwrap();
    dict.save(&path).unwrap();
    for_each_variant(&path, |variant| {
        let again = UserDictionary::open(&path).unwrap_or_else(|e| panic!("{variant}: {e}"));
        assert_eq!(again.entries(), dict.entries(), "{variant}");
        // コメントの末尾に CR が残らない
        assert_eq!(again.entries()[0].comment, "メモ", "{variant}");
    });
}

#[test]
fn profile_reads_in_all_variants() {
    let dir = tempfile::tempdir().unwrap();
    let user = UserDir::new(dir.path());
    let profile = Profile::from_settings(&[("it".to_string(), ProfileWeight::OFTEN)]).unwrap();
    profile.save(&user).unwrap();
    for_each_variant(&user.profile_tsv(), |variant| {
        let again = Profile::load(&user)
            .unwrap_or_else(|e| panic!("{variant}: {e}"))
            .unwrap_or_else(|| panic!("{variant}: empty"));
        assert_eq!(again.settings(), profile.settings(), "{variant}");
    });
}

#[test]
fn ingest_sources_read_in_all_variants() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingest_sources.tsv");
    let mut sources = IngestSources::default();
    sources
        .add(IngestSource {
            id: 1,
            path: "a.jsonl".into(),
            imported_at: 1_700_000_000,
            fnv64: 0x1234,
            prompt_version: "v1".into(),
            sentences_ok: 3,
            sentences_dropped: 1,
        })
        .unwrap();
    sources.save(&path).unwrap();
    for_each_variant(&path, |variant| {
        let again = IngestSources::open(&path).unwrap_or_else(|e| panic!("{variant}: {e}"));
        assert_eq!(again.to_text(), sources.to_text(), "{variant}");
    });
}

#[test]
fn ingest_candidates_read_in_all_variants() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingest_candidates.tsv");
    let mut candidates = IngestCandidates::default();
    candidates.add(1, "いかり", "碇", 2);
    candidates.save(&path).unwrap();
    for_each_variant(&path, |variant| {
        let again = IngestCandidates::open(&path).unwrap_or_else(|e| panic!("{variant}: {e}"));
        assert_eq!(again.list(), candidates.list(), "{variant}");
    });
}
