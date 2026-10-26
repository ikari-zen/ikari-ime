//! 管理: 学習

use super::*;

#[test]
fn learning_status_counts_each_layer() {
    let t = Real::new();
    let empty = learning_status(t.e());
    assert_eq!((empty.usage_rows, empty.initial_rows), (0, 0));
    assert_eq!(empty.max_rows as usize, ikari_core::user::MAX_LEARNING_ROWS);
    assert_eq!(empty.backup_file, "");

    begin(t.e(), 1, false);
    commit_first(t.e(), 1, "いかり");
    add_initial_row(t.e(), 3);
    let (usage, initial) = t
        .e()
        .inner
        .stack()
        .learning
        .as_ref()
        .unwrap()
        .layer_counts();
    assert!(usage > 0 && initial > 0);
    let status = learning_status(t.e());
    assert_eq!(
        (status.usage_rows as usize, status.initial_rows as usize),
        (usage, initial)
    );
}

#[test]
fn reset_removes_only_the_chosen_layer_and_saves_at_once() {
    use pb::LearningLayer::{All, Initial, Usage};
    for (layer, keeps_usage, keeps_initial) in [
        (Usage, false, true),
        (Initial, true, false),
        (All, false, false),
    ] {
        let t = engine_with_both_layers();
        let before = learning_status(t.e());
        assert!(before.usage_rows > 0 && before.initial_rows > 0);

        let after = reset_ok(t.e(), layer);
        assert_eq!(after.usage_rows > 0, keeps_usage, "{layer:?}");
        assert_eq!(after.initial_rows > 0, keeps_initial, "{layer:?}");
        if keeps_usage {
            assert_eq!(after.usage_rows, before.usage_rows);
        }
        if keeps_initial {
            assert_eq!(after.initial_rows, before.initial_rows);
        }
        // すぐファイルに書かれている（5 秒のタイマーを待たない）
        let (usage, initial) = rows_on_disk(&t);
        assert_eq!(
            (usage as u32, initial as u32),
            (after.usage_rows, after.initial_rows),
            "{layer:?}"
        );
        // バックアップができていて、返事がその場所を指す
        let files = backup_files(&t);
        assert_eq!(files.len(), 1, "{files:?}");
        assert!(files[0].starts_with("before-reset-") && files[0].ends_with(".tsv"));
        assert!(
            after.backup_file.ends_with(&files[0]),
            "{}",
            after.backup_file
        );
        assert_eq!(learning_status(t.e()).backup_file, after.backup_file);
    }
}

#[test]
fn the_reset_backup_holds_the_state_before_the_reset_including_unsaved_memory() {
    let t = Real::new();
    begin(t.e(), 1, false);
    // 確定してすぐ（5 秒のタイマーの保存より前）にリセットする
    commit_first(t.e(), 1, "いかり");
    let rows = learning_status(t.e()).usage_rows;
    let after = reset_ok(t.e(), pb::LearningLayer::All);
    assert_eq!(after.usage_rows, 0);
    let backup = std::fs::read_to_string(&after.backup_file).unwrap();
    let counts = ikari_core::user::validate_export(&backup).unwrap();
    assert_eq!(counts.learning_rows as u32, rows);
}

#[test]
fn reset_needs_a_layer_and_a_working_backup() {
    let t = engine_with_both_layers();
    let before = learning_status(t.e());
    let r = learning_reset(t.e(), pb::LearningLayer::Unspecified);
    assert_eq!(error_code(&r), ErrorCode::BadRequest);
    let r = send(
        t.e(),
        envelope::Body::ResetLearning(pb::LearningReset { layer: 99 }),
    );
    assert_eq!(error_code(&r), ErrorCode::BadRequest);
    assert!(backup_files(&t).is_empty());

    // バックアップを書けないときは、何も消さない
    std::fs::write(t.dir.join("backups"), "not a directory").unwrap();
    let r = learning_reset(t.e(), pb::LearningLayer::All);
    assert_eq!(error_code(&r), ErrorCode::Internal);
    let after = learning_status(t.e());
    assert_eq!(
        (after.usage_rows, after.initial_rows),
        (before.usage_rows, before.initial_rows)
    );
}

#[test]
fn only_the_newest_three_backups_are_kept() {
    let t = engine_with_both_layers();
    // 同じ秒に重なっても上書きしない（連番が付く）
    let mut written = Vec::new();
    for i in 0..5 {
        // 直前と同じ内容のバックアップは取らないので、毎回ユーザー辞書を変えておく
        dict_add(t.e(), "いかり", &format!("ikari{i}"), "名詞");
        written.push(reset_ok(t.e(), pb::LearningLayer::Usage).backup_file);
    }
    let unique: std::collections::HashSet<&String> = written.iter().collect();
    assert_eq!(unique.len(), 5, "{written:?}");
    let files = backup_files(&t);
    assert_eq!(files.len(), 3, "{files:?}");
    for path in &written[2..] {
        let name = Path::new(path).file_name().unwrap().to_string_lossy();
        assert!(
            files.iter().any(|f| *f == name),
            "{name} missing: {files:?}"
        );
    }
    // 一時ファイルや無関係のファイルを数えず、消さない
    std::fs::write(t.dir.join("backups").join("memo.txt"), "x").unwrap();
    reset_ok(t.e(), pb::LearningLayer::Usage);
    assert!(t.dir.join("backups").join("memo.txt").exists());
    assert_eq!(
        backup_files(&t)
            .iter()
            .filter(|f| f.starts_with("before-"))
            .count(),
        3
    );
}

#[test]
fn backup_names_sort_by_time_across_reasons() {
    assert_eq!(
        backup_key("before-import-20261012T153045Z.tsv"),
        Some(("20261012T153045Z".to_owned(), 1))
    );
    assert_eq!(
        backup_key("before-reset-20261012T153045Z-12.tsv"),
        Some(("20261012T153045Z".to_owned(), 12))
    );
    for other in [
        "before-reset-20261012T153045Z.tsv.tmp",
        "before-other-20261012T153045Z.tsv",
        "memo.txt",
        "before-reset-.tsv",
    ] {
        assert_eq!(backup_key(other), None, "{other}");
    }
    let stamp = utc_stamp();
    assert_eq!(stamp.len(), 16, "{stamp}");
    assert!(stamp.ends_with('Z') && stamp.contains('T'));
}

#[test]
fn consecutive_resets_keep_the_filled_backup_and_skip_empty_ones() {
    use pb::LearningLayer::All;
    let t = engine_with_both_layers();
    let first = reset_ok(t.e(), All);
    assert!(first.backup_file.contains("before-reset-"));
    let filled = std::fs::read(&first.backup_file).unwrap();
    assert!(
        !ikari_core::user::validate_export(std::str::from_utf8(&filled).unwrap())
            .map(|s| s.learning_rows == 0)
            .unwrap()
    );
    // 空になったあとの消去は、バックアップを取らない（返事にも保存先を出さない）
    for _ in 0..5 {
        let again = reset_ok(t.e(), All);
        assert_eq!(again.backup_file, "");
    }
    assert_eq!(backup_files(&t).len(), 1);
    assert_eq!(std::fs::read(&first.backup_file).unwrap(), filled);
}

#[test]
fn an_unchanged_state_is_not_backed_up_twice() {
    use pb::LearningLayer::Usage;
    let t = engine_with_both_layers();
    reset_ok(t.e(), Usage);
    // 初期の学習が残っているので空ではない。2 回目は 1 回目の消去後と同じ内容
    let second = reset_ok(t.e(), Usage);
    assert!(second.backup_file.contains("before-reset-"));
    let third = reset_ok(t.e(), Usage);
    assert_eq!(third.backup_file, "");
    assert_eq!(backup_files(&t).len(), 2);
}

#[test]
fn the_newest_filled_backup_survives_pruning_by_count() {
    let t = engine_with_both_layers();
    let first = reset_ok(t.e(), pb::LearningLayer::All).backup_file;
    // 空のバックアップが古い版の名前で 3 つ以上たまっていても（旧版の動き）、中身のあるものは消えない
    let dir = t.dir.join("backups");
    let empty_text = export(Real::new().e()).content;
    for n in 0..4 {
        std::fs::write(
            dir.join(format!("before-reset-30000101T00000{n}Z.tsv")),
            &empty_text,
        )
        .unwrap();
    }
    dict_add(t.e(), "いかり", "ikari-ime", "名詞");
    reset_ok(t.e(), pb::LearningLayer::All);
    assert!(Path::new(&first).exists(), "{:?}", backup_files(&t));
}
