//! 管理: ユーザー辞書

use super::*;

#[test]
fn a_word_added_to_the_user_dictionary_shows_up_in_the_very_next_convert() {
    let t = Real::new();
    begin(t.e(), 1, false);
    assert!(!all_surfaces(t.e(), 1, "いかり").contains(&"ikari-ime".to_owned()));

    let added = dict_add(t.e(), "いかり", "ikari-ime", "名詞");
    assert_eq!(
        issue_kind_of(&added.issue),
        pb::DataIssueKind::DataIssueNone
    );
    assert_eq!(added.entries.len(), 1);
    assert_eq!(added.entries[0].surface, "ikari-ime");
    // 1 秒の確認を待たずに出る
    assert!(all_surfaces(t.e(), 1, "いかり").contains(&"ikari-ime".to_owned()));

    // 自分が書いたファイルで、もう一度組み立て直さない（確認の間隔を過ぎても）
    std::thread::sleep(DATA_CHECK_INTERVAL + Duration::from_millis(200));
    convert(t.e(), 1, "いかり", 1);
    assert_eq!(count_lines(&t.log(), " reload "), 1, "{}", t.log());

    let removed = dict_remove(t.e(), "いかり", "ikari-ime");
    assert_eq!(
        issue_kind_of(&removed.issue),
        pb::DataIssueKind::DataIssueNone
    );
    assert!(removed.entries.is_empty());
    assert!(!all_surfaces(t.e(), 1, "いかり").contains(&"ikari-ime".to_owned()));
}

#[test]
fn user_dict_add_normalizes_the_reading_and_lists_the_choices() {
    let t = Real::new();
    // カタカナの読みはひらがなにそろえる
    let added = dict_add(t.e(), "イカリ", "ikari-ime", "名詞");
    assert_eq!(added.entries[0].reading, "いかり");
    assert!(
        added
            .pos_options
            .iter()
            .any(|o| o.name == "名詞" && !o.needs_conj_type)
    );
    assert!(
        added
            .pos_options
            .iter()
            .any(|o| o.name == "動詞" && o.needs_conj_type)
    );
    assert!(
        added.verb_conj_types.contains(&"五段-ラ行".to_owned()),
        "{:?}",
        added.verb_conj_types
    );
    // 一覧も同じ
    let listed = dict_list(t.e());
    assert_eq!(listed.entries, added.entries);
    assert_eq!(listed.pos_options, added.pos_options);
}

#[test]
fn rejected_entries_are_not_written() {
    let t = Real::new();
    dict_add(t.e(), "あ", "亜", "名詞");
    let path = t.user().user_dict_tsv();
    let before = std::fs::read_to_string(&path).unwrap();
    let cases = [
        ("abc", "x", "名詞"),               // 読みがひらがなでない
        ("あ", "亜", "謎の品詞"),           // 品詞がない
        ("かく", "書く", "動詞"),           // 活用型がない
        ("かく", "書く", "動詞:五段-ラ行"), // 語尾が活用型と合わない
        ("あい", "", "名詞"),               // 表記が空
        ("あい", "a\tb", "名詞"),           // タブ
    ];
    for (reading, surface, pos) in cases {
        let reply = dict_add(t.e(), reading, surface, pos);
        assert_eq!(
            issue_kind_of(&reply.issue),
            pb::DataIssueKind::InvalidEntry,
            "{reading} {surface} {pos}"
        );
        assert_eq!(reply.issue.as_ref().unwrap().section, "user_dict");
        assert!(!reply.issue.as_ref().unwrap().detail.is_empty());
        // 失敗しても、今の一覧を返す
        assert_eq!(reply.entries.len(), 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }
    // entry がない要求は形式の誤り
    let r = send(
        t.e(),
        envelope::Body::AddUserDict(pb::UserDictAdd::default()),
    );
    assert_eq!(error_code(&r), ErrorCode::BadRequest);
    // 削除できない語
    let reply = dict_remove(t.e(), "ない", "無い");
    assert_eq!(issue_kind_of(&reply.issue), pb::DataIssueKind::NotFound);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn verbs_are_conjugated_when_added() {
    let t = Real::new();
    begin(t.e(), 1, false);
    let r = dict_add(t.e(), "ひねくる", "捻くる", "動詞:五段-ラ行");
    assert_eq!(
        issue_kind_of(&r.issue),
        pb::DataIssueKind::DataIssueNone,
        "{r:?}"
    );
    let r = dict_add(t.e(), "かきすすめる", "書き進める", "動詞:下一段-マ行");
    assert_eq!(
        issue_kind_of(&r.issue),
        pb::DataIssueKind::DataIssueNone,
        "{r:?}"
    );
    assert_eq!(r.entries.len(), 2);
    assert!(
        all_surfaces(t.e(), 1, "ひねくった")
            .iter()
            .any(|s| s.starts_with("捻くっ"))
    );
    // 終止形がすぐ変換できる。活用形も出る
    assert!(all_surfaces(t.e(), 1, "かきすすめる").contains(&"書き進める".to_owned()));
    assert!(
        all_surfaces(t.e(), 1, "かきすすめた")
            .iter()
            .any(|s| s.starts_with("書き進め"))
    );
}

#[test]
fn the_cli_and_the_engine_do_not_overwrite_each_other() {
    let t = Real::new();
    dict_add(t.e(), "あ", "亜", "名詞");
    // CLI が（エンジンの外で）同じファイルに書いた
    let path = t.user().user_dict_tsv();
    let mut dict = UserDictionary::open(&path).unwrap();
    dict.add(ikari_core::user::UserDictEntry {
        reading: "い".into(),
        surface: "伊".into(),
        pos: "名詞".into(),
        comment: "cli".into(),
    })
    .unwrap();
    dict.save(&path).unwrap();
    // エンジンの次の追加は、ファイルを読み直してから書くので CLI の行を消さない
    let r = dict_add(t.e(), "う", "宇", "名詞");
    let surfaces: Vec<&str> = r.entries.iter().map(|e| e.surface.as_str()).collect();
    assert_eq!(surfaces, ["亜", "伊", "宇"]);
    let r = dict_remove(t.e(), "あ", "亜");
    let surfaces: Vec<&str> = r.entries.iter().map(|e| e.surface.as_str()).collect();
    assert_eq!(surfaces, ["伊", "宇"]);
}

#[test]
fn a_broken_user_dict_file_is_reported_and_left_alone() {
    let t = Real::new();
    let path = t.user().user_dict_tsv();
    std::fs::write(&path, "garbage\n").unwrap();
    for reply in [
        dict_list(t.e()),
        dict_add(t.e(), "あ", "亜", "名詞"),
        dict_remove(t.e(), "あ", "亜"),
    ] {
        assert_eq!(
            issue_kind_of(&reply.issue),
            pb::DataIssueKind::Parse,
            "{reply:?}"
        );
        assert!(reply.entries.is_empty());
        // 画面に出す選択肢は、壊れていても返す
        assert!(!reply.pos_options.is_empty());
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "garbage\n");
}

/// 上限は 2 段。64 MiB を超えるものは全部断り、256 KiB を超えるものは ImportData 以外を断る
#[test]
fn request_limits_have_two_tiers() {
    let (e, _mock) = engine(0);
    let message_of = |reply: &[u8]| match Envelope::decode(reply).unwrap().body.unwrap() {
        envelope::Body::Error(r) => (ErrorCode::try_from(r.code).unwrap(), r.message),
        other => panic!("expected an error, got {other:?}"),
    };
    let too_large = (ErrorCode::BadRequest, "request too large".to_owned());

    let big_add = Envelope {
        schema_version: SCHEMA_VERSION,
        request_id: 7,
        body: Some(envelope::Body::AddUserDict(pb::UserDictAdd {
            entry: Some(pb::UserDictEntry {
                surface: "a".repeat(MAX_REQUEST_BYTES),
                ..pb::UserDictEntry::default()
            }),
        })),
    }
    .encode_to_vec();
    assert!(big_add.len() > MAX_REQUEST_BYTES);
    assert_eq!(message_of(&e.handle(&big_add)), too_large);
    // Convert なども同じ（読みの上限より前に、要求の大きさで断る）
    let big_convert = Envelope {
        schema_version: SCHEMA_VERSION,
        request_id: 8,
        body: Some(convert_req(1, &"あ".repeat(MAX_REQUEST_BYTES), 0, 0)),
    }
    .encode_to_vec();
    assert_eq!(message_of(&e.handle(&big_convert)), too_large);

    // 64 MiB を超えるものは、decode する前に断る
    let huge = vec![0u8; MAX_ADMIN_REQUEST_BYTES + 1];
    assert_eq!(message_of(&e.handle(&huge)), too_large);
}
