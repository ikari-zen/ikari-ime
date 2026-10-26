//! 絵文字のショートコードの表（ADR 0027 §9.2、§9.3）

use super::*;

fn emoji_reply(e: &Engine) -> pb::EmojiTableReply {
    match send(e, envelope::Body::EmojiTable(pb::EmojiTableRequest {})) {
        envelope::Body::EmojiTableReply(r) => r,
        other => panic!("unexpected reply: {other:?}"),
    }
}

fn force_check(e: &Engine) {
    let watch = e.inner.watch.as_ref().unwrap();
    lock_mutex(watch).last_check = Instant::now() - DATA_CHECK_INTERVAL * 2;
}

fn names(r: &pb::EmojiTableReply) -> Vec<&str> {
    r.entries.iter().map(|x| x.name.as_str()).collect()
}

#[test]
fn no_data_dir_or_no_file_gives_an_empty_table() {
    let (e, _) = engine(0);
    assert_eq!(emoji_reply(&e), pb::EmojiTableReply::default());

    let t = Real::new();
    assert_eq!(emoji_reply(t.e()), pb::EmojiTableReply::default());
}

#[test]
fn the_table_is_read_at_open_in_name_order() {
    let mut t = Real::new();
    std::fs::write(
        crate::dispatch::emoji_tsv_path_in(&t.dir),
        "# h\ns\teyes\t👀\ns\t+1\t👍\n",
    )
    .unwrap();
    t.engine = None;
    t.open();
    let r = emoji_reply(t.e());
    assert_eq!(names(&r), ["+1", "eyes"]);
    assert_eq!(r.entries[1].emoji, "👀");
    assert!(!r.data_label.is_empty());
}

#[test]
fn the_table_is_reloaded_by_the_watch_and_the_label_changes() {
    let t = Real::new();
    let path = crate::dispatch::emoji_tsv_path_in(&t.dir);
    assert!(emoji_reply(t.e()).entries.is_empty());

    std::fs::write(&path, "s\teyes\t👀\n").unwrap();
    // 1 秒の確認の前は変わらない
    t.e().inner.refresh_data();
    assert!(emoji_reply(t.e()).entries.is_empty());
    force_check(t.e());
    t.e().inner.refresh_data();
    let first = emoji_reply(t.e());
    assert_eq!(names(&first), ["eyes"]);

    std::fs::write(&path, "s\teyes\t👀\ns\tcat\t🐱\n").unwrap();
    force_check(t.e());
    t.e().inner.refresh_data();
    let second = emoji_reply(t.e());
    assert_eq!(names(&second), ["cat", "eyes"]);
    assert_ne!(first.data_label, second.data_label);
    // 辞書は組み立て直さない
    assert_eq!(count_lines(&t.log(), " reload "), 0, "{}", t.log());

    // 消えたら空に戻る
    std::fs::remove_file(&path).unwrap();
    force_check(t.e());
    t.e().inner.refresh_data();
    assert_eq!(emoji_reply(t.e()), pb::EmojiTableReply::default());
}

#[test]
fn a_broken_file_gives_an_empty_table() {
    let t = Real::new();
    let path = crate::dispatch::emoji_tsv_path_in(&t.dir);
    std::fs::write(&path, "s\teyes\t👀\n").unwrap();
    force_check(t.e());
    t.e().inner.refresh_data();
    assert_eq!(emoji_reply(t.e()).entries.len(), 1);

    std::fs::write(&path, "s\teyes\ns\tcat\t🐱\n").unwrap();
    force_check(t.e());
    t.e().inner.refresh_data();
    assert_eq!(emoji_reply(t.e()), pb::EmojiTableReply::default());
    // 変換は動き続ける
    begin(t.e(), 1, false);
}
