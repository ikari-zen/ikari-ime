//! 管理: エクスポートとインポート

use super::*;

#[test]
fn export_then_import_restores_everything() {
    use ikari_core::user::ProfileWeight;
    let t = Real::new();
    begin(t.e(), 1, false);
    dict_add(t.e(), "いかり", "ikari-ime", "名詞");
    set_profile_file(&t, &[("it", ProfileWeight::OFTEN)]);
    commit_first(t.e(), 1, "いかり");
    add_initial_row(t.e(), 3);

    let exported = export(t.e());
    assert_eq!(exported.schema, ikari_core::user::EXPORT_SCHEMA);
    let counts = exported.counts.unwrap();
    assert_eq!((counts.profile_domains, counts.user_dict_entries), (1, 1));
    let status = learning_status(t.e());
    assert_eq!(
        counts.learning_rows,
        status.usage_rows + status.initial_rows
    );
    assert!(exported.content.starts_with(b"#ikari-export\tschema=1\n"));
    let files_before = user_files(&t);

    // いろいろ変える
    dict_add(t.e(), "う", "宇", "名詞");
    dict_remove(t.e(), "いかり", "ikari-ime");
    set_profile_file(&t, &[("novel", ProfileWeight::SOMETIMES)]);
    reset_ok(t.e(), pb::LearningLayer::All);
    assert_ne!(user_files(&t), files_before);

    let applied = import(t.e(), &exported.content, false);
    assert!(applied.applied, "{applied:?}");
    assert!(applied.issue.is_none());
    assert_eq!(applied.counts, exported.counts);

    // ファイルも、メモリ（件数、変換）も元に戻る（なかったファイルは、空のデータのファイルになる）
    assert_eq!(export(t.e()).content, exported.content);
    assert_eq!(user_files(&t)[..3], files_before[..3]);
    let restored = learning_status(t.e());
    assert_eq!(
        (restored.usage_rows, restored.initial_rows),
        (status.usage_rows, status.initial_rows)
    );
    let listed = dict_list(t.e());
    assert_eq!(listed.entries.len(), 1);
    assert_eq!(listed.entries[0].surface, "ikari-ime");
    assert!(all_surfaces(t.e(), 1, "いかり").contains(&"ikari-ime".to_owned()));
    assert!(!all_surfaces(t.e(), 1, "う").contains(&"宇".to_owned()));
    // 書き出し直すと、同じバイト列になる
    assert_eq!(export(t.e()).content, exported.content);
}

#[test]
fn a_commit_after_an_import_is_saved_on_top_of_the_imported_learning() {
    // 取り込み先: 保存前の使用の学習がメモリにある
    let a = Real::new();
    begin(a.e(), 1, false);
    commit_first(a.e(), 1, "いかり");
    assert!(learning_status(a.e()).usage_rows > 0);
    // 取り込む側: 初期学習だけ
    let b = Real::new();
    add_initial_row(b.e(), 9);
    save_learning(b.e());
    let content = export(b.e()).content;

    let applied = import(a.e(), &content, false);
    assert!(applied.applied, "{applied:?}");
    let status = learning_status(a.e());
    assert_eq!((status.usage_rows, status.initial_rows), (0, 1));
    // 古いメモリは捨てられている。次の確定と保存は、取り込んだ学習の上に載る
    commit_first(a.e(), 1, "ひんし");
    send(
        a.e(),
        envelope::Body::EndSession(pb::EndSession { session_id: 1 }),
    );
    let records = LearningStore::open(&a.user().learning_tsv())
        .unwrap()
        .records();
    assert!(
        records
            .iter()
            .any(|r| r.source == Some(9) && r.reading == "いかり"),
        "the imported row is gone: {records:?}"
    );
    assert!(records.iter().any(|r| r.reading == "ひんし"));
    assert!(
        !records
            .iter()
            .any(|r| r.reading == "いかり" && r.source.is_none()),
        "the old in-memory row came back: {records:?}"
    );
}

#[test]
fn a_conversion_started_before_an_import_cannot_be_committed() {
    let a = Real::new();
    begin(a.e(), 1, false);
    let before = convert(a.e(), 1, "いかり", 0);
    let b = Real::new();
    let content = export(b.e()).content;
    assert!(import(a.e(), &content, false).applied);
    let r = send(
        a.e(),
        envelope::Body::Commit(pb::Commit {
            conversion_id: before.conversion_id,
            candidate_index: 0,
        }),
    );
    assert_eq!(error_code(&r), ErrorCode::UnknownConversion);
    assert_eq!(learning_status(a.e()).usage_rows, 0);
    // 入力は止まらない: 新しい変換はできる
    commit_first(a.e(), 1, "いかり");
    assert!(learning_status(a.e()).usage_rows > 0);
}

#[test]
fn dry_run_validates_and_counts_without_writing() {
    let t = engine_with_both_layers();
    dict_add(t.e(), "いかり", "ikari-ime", "名詞");
    let exported = export(t.e());
    reset_ok(t.e(), pb::LearningLayer::Usage);
    let files = user_files(&t);
    let backups = backup_files(&t);

    let reply = import(t.e(), &exported.content, true);
    assert!(!reply.applied);
    assert!(reply.issue.is_none());
    assert_eq!(reply.counts, exported.counts);
    assert_eq!(reply.backup_file, "");
    assert_eq!(user_files(&t), files);
    assert_eq!(backup_files(&t), backups);
    assert_eq!(learning_status(t.e()).usage_rows, 0);
}

#[test]
fn a_bad_import_file_changes_nothing_and_says_why() {
    use pb::DataIssueKind::{InvalidEntry, NotIkariExport, Parse, SchemaUnsupported};
    let t = engine_with_both_layers();
    dict_add(t.e(), "いかり", "ikari-ime", "名詞");
    let good = String::from_utf8(export(t.e()).content).unwrap();
    let files = user_files(&t);
    let status = learning_status(t.e());
    let backups = backup_files(&t);

    let without_first_line = good.split_once('\n').unwrap().1.to_owned();
    let cases: Vec<(&str, Vec<u8>, pb::DataIssueKind)> = vec![
        ("empty", Vec::new(), NotIkariExport),
        ("junk", b"hello\n".to_vec(), NotIkariExport),
        ("no header", without_first_line.into_bytes(), NotIkariExport),
        (
            "schema 2",
            good.replacen("schema=1", "schema=2", 1).into_bytes(),
            SchemaUnsupported,
        ),
        (
            "section schema",
            good.replacen(
                "#ikari-user-dict\tschema=1",
                "#ikari-user-dict\tschema=3",
                1,
            )
            .into_bytes(),
            SchemaUnsupported,
        ),
        (
            "unknown pos",
            good.replace("ikari-ime\t名詞", "ikari-ime\t謎の品詞")
                .into_bytes(),
            Parse,
        ),
        (
            // 書式は正しいが、この辞書の文法では展開できない活用型
            "verb type missing in the grammar",
            good.replace(
                "いかり\tikari-ime\t名詞",
                "いかりする\tikari-imeする\t動詞:サ行変格",
            )
            .into_bytes(),
            InvalidEntry,
        ),
        (
            "missing section",
            good.split("[ingest_sources]")
                .next()
                .unwrap()
                .as_bytes()
                .to_vec(),
            Parse,
        ),
        ("not utf-8", vec![0xff, 0xfe, 0x00, 0x80], Parse),
    ];
    for (name, content, kind) in cases {
        for dry_run in [true, false] {
            let reply = import(t.e(), &content, dry_run);
            assert!(!reply.applied, "{name}");
            assert_eq!(issue_kind_of(&reply.issue), kind, "{name}: {reply:?}");
            assert!(reply.counts.is_none(), "{name}");
            assert_eq!(user_files(&t), files, "{name} changed the files");
            assert_eq!(backup_files(&t), backups, "{name} made a backup");
            let now = learning_status(t.e());
            assert_eq!(
                (now.usage_rows, now.initial_rows),
                (status.usage_rows, status.initial_rows),
                "{name}"
            );
        }
    }
    // 理由の細部
    let reply = import(
        t.e(),
        good.replacen("schema=1", "schema=2", 1).as_bytes(),
        true,
    );
    let issue = reply.issue.unwrap();
    assert_eq!((issue.found_schema, issue.supported_schema), (2, 1));
    let reply = import(
        t.e(),
        good.replace("ikari-ime\t名詞", "ikari-ime\t謎の品詞")
            .as_bytes(),
        true,
    );
    let issue = reply.issue.unwrap();
    assert_eq!((issue.section.as_str(), issue.line), ("user_dict", 2));
    assert!(!issue.detail.is_empty());
    // 辞書は使えたまま
    assert!(all_surfaces(t.e(), 1, "いかり").contains(&"ikari-ime".to_owned()));
}

#[test]
fn an_import_backs_up_first_and_the_backup_can_be_imported_back() {
    let t = engine_with_both_layers();
    dict_add(t.e(), "いかり", "ikari-ime", "名詞");
    let original = export(t.e());
    let files_before = user_files(&t);

    // 別のデータを取り込む（上書き）
    let other = Real::new();
    dict_add(other.e(), "う", "宇", "名詞");
    let other_export = export(other.e());
    let applied = import(t.e(), &other_export.content, false);
    assert!(applied.applied, "{applied:?}");
    assert_ne!(user_files(&t), files_before);
    assert_eq!(dict_list(t.e()).entries[0].surface, "宇");

    let files = backup_files(&t);
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].starts_with("before-import-"));
    assert!(applied.backup_file.ends_with(&files[0]));
    // バックアップは取り込み前のエクスポートと同じ形式・同じ内容
    let backup = std::fs::read(&applied.backup_file).unwrap();
    assert_eq!(backup, original.content);

    // バックアップを取り込むと元に戻る
    assert!(import(t.e(), &backup, false).applied);
    assert_eq!(export(t.e()).content, original.content);
    assert_eq!(user_files(&t)[1..3], files_before[1..3]);
    assert!(all_surfaces(t.e(), 1, "いかり").contains(&"ikari-ime".to_owned()));
}

#[test]
fn imports_above_the_normal_request_limit_are_accepted() {
    // 256 KiB を超える辞書（読みを変えた語を並べる）
    let t = Real::new();
    let mut dict = UserDictionary::default();
    let kana: Vec<char> = "あいうえおかきくけこさしすせそたちつてとなにぬねの"
        .chars()
        .collect();
    for a in &kana {
        for b in &kana {
            dict.add(ikari_core::user::UserDictEntry {
                reading: format!("{a}{b}"),
                surface: format!("語{a}{b}"),
                pos: "名詞".into(),
                comment: "x".repeat(500),
            })
            .unwrap();
        }
    }
    dict.save(&t.user().user_dict_tsv()).unwrap();
    let exported = export(t.e());
    assert!(
        exported.content.len() > MAX_REQUEST_BYTES,
        "{}",
        exported.content.len()
    );
    assert!(exported.content.len() < MAX_ADMIN_REQUEST_BYTES);

    let other = Real::new();
    let request = Envelope {
        schema_version: SCHEMA_VERSION,
        request_id: 1,
        body: Some(envelope::Body::ImportData(pb::ImportData {
            content: exported.content.clone(),
            dry_run: false,
        })),
    }
    .encode_to_vec();
    assert!(request.len() > MAX_REQUEST_BYTES);
    let reply = Envelope::decode(other.e().handle(&request).as_slice()).unwrap();
    match reply.body {
        Some(envelope::Body::ImportReply(r)) => assert!(r.applied, "{r:?}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(dict_list(other.e()).entries.len(), kana.len() * kana.len());
}

/// 学習の行数が上限いっぱいでも、エクスポートは 64 MiB に収まり、インポートで往復できる
#[test]
fn an_export_with_the_maximum_learning_rows_fits_the_limit() {
    use ikari_core::user::{ContextFeatures, Feature, FeatureKind, MAX_LEARNING_ROWS, SegmentKey};
    let t = Real::new();
    let store = t.e().inner.stack().learning.clone().unwrap();
    let feats = ContextFeatures {
        features: vec![Feature {
            kind: FeatureKind::Unconditional,
            hash: 0,
            weight: 0.2,
        }],
    };
    for n in 0..MAX_LEARNING_ROWS {
        let reading = kana_reading(n);
        let key = SegmentKey {
            surface: format!("表記{reading}"),
            reading: reading.clone(),
            pos_name: "名詞,普通名詞,一般,*,*,*".into(),
        };
        store.add_initial(1, &reading, &[key], &feats, 1);
    }
    assert_eq!(store.len(), MAX_LEARNING_ROWS);
    store.save().unwrap();

    let exported = export(t.e());
    assert_eq!(
        exported.counts.unwrap().learning_rows as usize,
        MAX_LEARNING_ROWS
    );
    assert!(
        exported.content.len() < MAX_ADMIN_REQUEST_BYTES,
        "{} bytes",
        exported.content.len()
    );
    // 往復
    let other = Real::new();
    let applied = import(other.e(), &exported.content, false);
    assert!(applied.applied, "{applied:?}");
    assert_eq!(
        learning_status(other.e()).initial_rows as usize,
        MAX_LEARNING_ROWS
    );
}

#[test]
fn an_export_over_the_limit_is_refused() {
    let t = engine_with_both_layers();
    let dir = t.user();
    assert!(t.e().inner.export_data(&dir, usize::MAX).is_ok());
    let size = export(t.e()).content.len();
    assert!(t.e().inner.export_data(&dir, size).is_ok());
    let too_small = t.e().inner.export_data(&dir, size - 1);
    assert!(matches!(too_small, Err(f) if f.message == "export too large"));
}

#[test]
fn admin_logs_carry_kinds_only_never_words() {
    let t = Real::new();
    begin(t.e(), 1, false);
    let secret_reading = "ひみつのよみ";
    let secret_surface = "秘密の表記";
    dict_add(t.e(), secret_reading, secret_surface, "名詞");
    dict_list(t.e());
    commit_first(t.e(), 1, secret_reading);
    reset_ok(t.e(), pb::LearningLayer::Usage);
    let exported = export(t.e());
    let bad = import(t.e(), b"#ikari-export\tschema=7\nsecret\n", false);
    assert_eq!(
        issue_kind_of(&bad.issue),
        pb::DataIssueKind::SchemaUnsupported
    );
    assert!(import(t.e(), &exported.content, false).applied);
    dict_remove(t.e(), secret_reading, secret_surface);
    dict_remove(t.e(), secret_reading, secret_surface);

    let log = t.log();
    for needle in [secret_reading, secret_surface, "secret", "表記"] {
        assert!(
            !log.contains(needle),
            "{needle} leaked into the log:\n{log}"
        );
    }
    for kind in [
        " add_user_dict ",
        " list_user_dict ",
        " reset_learning ",
        " export_data ",
        " import_data ",
        " remove_user_dict ",
    ] {
        assert!(log.contains(kind), "{kind} missing:\n{log}");
    }
    // 失敗は DataIssueKind の名前だけ
    assert!(log.contains("err=SCHEMA_UNSUPPORTED"), "{log}");
    assert!(log.contains("err=NOT_FOUND"), "{log}");
}

#[test]
fn broken_admin_requests_never_panic() {
    let t = engine_with_both_layers();
    let good_export = export(t.e()).content;
    let requests = [
        envelope::Body::ListUserDict(pb::UserDictList {}),
        envelope::Body::AddUserDict(pb::UserDictAdd {
            entry: Some(pb::UserDictEntry {
                reading: "あ".into(),
                surface: "亜".into(),
                pos: "名詞".into(),
                comment: "c".into(),
            }),
        }),
        envelope::Body::RemoveUserDict(pb::UserDictRemove {
            reading: "あ".into(),
            surface: "亜".into(),
        }),
        envelope::Body::LearningStatus(pb::LearningStatus {}),
        envelope::Body::ResetLearning(pb::LearningReset {
            layer: pb::LearningLayer::Usage as i32,
        }),
        envelope::Body::ExportData(pb::ExportData {}),
        envelope::Body::ImportData(pb::ImportData {
            content: good_export[..good_export.len().min(300)].to_vec(),
            dry_run: true,
        }),
        envelope::Body::GetProfile(pb::ProfileGet {}),
        envelope::Body::SetProfile(pb::ProfileSet {
            domains: vec![pb::ProfileDomainState {
                id: "it".into(),
                name: String::new(),
                weight: pb::ProfileWeightKind::ProfileWeightOften as i32,
            }],
        }),
    ];
    for body in requests {
        let valid = Envelope {
            schema_version: SCHEMA_VERSION,
            request_id: 9,
            body: Some(body),
        }
        .encode_to_vec();
        let mut inputs: Vec<Vec<u8>> = Vec::new();
        for cut in 0..valid.len() {
            inputs.push(valid[..cut].to_vec());
        }
        for i in 0..valid.len() {
            for flip in [0xffu8, 0x01, 0x80] {
                let mut v = valid.clone();
                v[i] ^= flip;
                inputs.push(v);
            }
        }
        for input in inputs {
            let reply = t.e().handle(&input);
            Envelope::decode(reply.as_slice()).expect("reply must decode");
        }
    }
    // 壊れた要求のあとも普通に動く
    assert!(!convert(t.e(), 1, "いかり", 0).candidates.is_empty());
}

#[test]
fn profile_lists_every_preset_and_applies_changes_at_once() {
    use pb::ProfileWeightKind::ProfileWeightSometimes as Sometimes;
    use pb::ProfileWeightKind::{ProfileWeightOff as Off, ProfileWeightOften as Often};
    let t = Real::new();
    let initial = profile_get(t.e());
    let ids: Vec<&str> = initial.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, ["it", "infra", "novel", "business"]);
    assert!(
        initial
            .iter()
            .all(|d| d.weight == Off as i32 && !d.name.is_empty())
    );
    assert!(t.e().inner.stack().profile.is_none());

    let reply = profile_set(
        t.e(),
        vec![
            profile_state("novel", Sometimes),
            profile_state("it", Often),
            profile_state("business", Off),
        ],
    );
    let envelope::Body::ProfileReply(reply) = reply else {
        panic!("{reply:?}");
    };
    assert_eq!(
        weights(&reply.domains),
        [
            ("it", Often as i32),
            ("infra", Off as i32),
            ("novel", Sometimes as i32),
            ("business", Off as i32)
        ]
    );
    assert_eq!(weights(&profile_get(t.e())), weights(&reply.domains));
    // OFF は書かない
    let file = std::fs::read_to_string(t.user().profile_tsv()).unwrap();
    assert_eq!(
        file,
        "#ikari-profile\tschema=1\nnovel\tsometimes\nit\toften\n"
    );
    // 組み立て直し済み（1 秒待たない）で、もう一度は組み立て直さない
    let profile = t.e().inner.stack().profile.clone().unwrap();
    assert_eq!(profile.domains.len(), 2);
    std::thread::sleep(DATA_CHECK_INTERVAL + Duration::from_millis(200));
    begin(t.e(), 1, false);
    convert(t.e(), 1, "いかり", 0);
    assert_eq!(count_lines(&t.log(), " reload "), 1, "{}", t.log());

    // 全部 OFF にすると、プロファイルなしに戻る
    profile_set(t.e(), vec![]);
    assert!(t.e().inner.stack().profile.is_none());
    assert!(profile_get(t.e()).iter().all(|d| d.weight == Off as i32));
}

#[test]
fn bad_profile_requests_are_refused_without_writing() {
    use pb::ProfileWeightKind::{ProfileWeightOften as Often, ProfileWeightUnspecified as Unset};
    let t = Real::new();
    profile_set(t.e(), vec![profile_state("it", Often)]);
    let path = t.user().profile_tsv();
    let before = std::fs::read_to_string(&path).unwrap();
    for domains in [
        vec![profile_state("nonexistent", Often)],
        vec![profile_state("it", Unset)],
        vec![profile_state("it", Often), profile_state("it", Often)],
        vec![pb::ProfileDomainState {
            id: "it".into(),
            name: String::new(),
            weight: 99,
        }],
    ] {
        let r = profile_set(t.e(), domains.clone());
        assert_eq!(error_code(&r), ErrorCode::BadRequest, "{domains:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }
}

#[test]
fn a_broken_profile_file_is_reported_and_replaced_by_a_set() {
    use pb::ProfileWeightKind::ProfileWeightOften as Often;
    let t = Real::new();
    std::fs::write(t.user().profile_tsv(), "garbage\n").unwrap();
    let r = send(t.e(), envelope::Body::GetProfile(pb::ProfileGet {}));
    assert_eq!(error_code(&r), ErrorCode::Internal);
    let r = profile_set(t.e(), vec![profile_state("infra", Often)]);
    assert!(matches!(r, envelope::Body::ProfileReply(_)), "{r:?}");
    assert_eq!(weights(&profile_get(t.e()))[1], ("infra", Often as i32));
}
