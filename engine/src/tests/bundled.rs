//! 同梱の場所（ADR 0080 §3.2。#580）。利用者のデータの場所の辞書が無い・壊れている・形式が合わないとき、
//! 同梱の場所の辞書で起動する（C8 の単体テスト）。同梱の場所は `Engine::open_in` に渡す（製品は実行ファイルから決める）

use super::*;

use crate::data_location::{DataStatus, LocationKind};

/// fixture の辞書を `<root>/dict/` に写す
fn put_fixture_dict(root: &Path) {
    let dict = UserDir::new(root).dict_dir();
    std::fs::create_dir_all(&dict).unwrap();
    for entry in std::fs::read_dir(fixture_dict_dir()).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), dict.join(entry.file_name())).unwrap();
    }
}

/// 開けない辞書（magic の合わない 2 ファイル）を `<root>/dict/` に置く
fn put_broken_dict(root: &Path) {
    let dict = UserDir::new(root).dict_dir();
    std::fs::create_dir_all(&dict).unwrap();
    std::fs::write(dict.join("grammar.bin"), b"not a grammar").unwrap();
    std::fs::write(dict.join("system.dict"), b"not a dictionary").unwrap();
}

/// 利用者のデータの場所と同梱の場所の組。開いたエンジンとログを見る
struct Layout {
    user: PathBuf,
    bundled: PathBuf,
}

impl Layout {
    fn new() -> Layout {
        Layout {
            user: temp_dir("bundled-user"),
            bundled: temp_dir("bundled-app"),
        }
    }

    fn open(&self) -> Result<Engine, EngineError> {
        crate::lock::retry_while_held(|| {
            Engine::open_in(
                &EngineConfig {
                    data_dir: self.user.to_string_lossy().into_owned(),
                    enable_lm: false,
                    idle_unload_secs: 300,
                },
                &LmHostChoice::InProcess,
                Some(&self.bundled),
            )
        })
    }

    fn log(&self, e: &Engine) -> String {
        e.inner.log.flush_for_test();
        std::fs::read_to_string(self.user.join("logs").join("engine.log")).unwrap_or_default()
    }
}

impl Drop for Layout {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.user);
        let _ = std::fs::remove_dir_all(&self.bundled);
    }
}

fn dict_of(e: &Engine) -> LocationKind {
    e.data_status().expect("opened with open_in").dict
}

/// 変換できる（fixture の語が出る）
fn converts(e: &Engine) -> bool {
    begin(e, 1, false);
    let r = convert(e, 1, "この", 1);
    !r.candidates.is_empty()
}

#[test]
fn the_user_dictionary_comes_first() {
    let l = Layout::new();
    put_fixture_dict(&l.user);
    put_fixture_dict(&l.bundled);
    let e = l.open().expect("open");
    assert_eq!(
        e.data_status(),
        Some(DataStatus {
            dict: LocationKind::User,
            model: None,
        })
    );
    let log = l.log(&e);
    assert!(log.contains("err=- dict=user model=none"), "{log}");
    assert!(!log.contains("dict_reject"), "{log}");
}

#[test]
fn without_a_user_dictionary_the_bundled_one_is_used_quietly() {
    let l = Layout::new();
    put_fixture_dict(&l.bundled);
    let e = l.open().expect("open");
    assert_eq!(dict_of(&e), LocationKind::Bundled);
    assert!(converts(&e));
    let log = l.log(&e);
    assert!(log.contains("err=- dict=bundled"), "{log}");
    // 置かれていないのは落とした理由ではない
    assert!(!log.contains("dict_reject"), "{log}");
}

/// 壊れた辞書・形式の古い辞書で、変換が止まらない（今の「起動に失敗する」を改めた。§3.2）
#[test]
fn a_broken_user_dictionary_falls_back_to_the_bundled_one() {
    let l = Layout::new();
    put_broken_dict(&l.user);
    put_fixture_dict(&l.bundled);
    let e = l.open().expect("open");
    assert_eq!(dict_of(&e), LocationKind::Bundled);
    assert!(converts(&e));
    let log = l.log(&e);
    assert!(log.contains("err=- location=user reason=invalid"), "{log}");
    // パスは書かない
    assert!(!log.contains(&*l.user.to_string_lossy()), "{log}");
    // 利用者の物は自動では消さない・動かさない
    assert!(
        UserDir::new(&l.user)
            .dict_dir()
            .join("grammar.bin")
            .exists()
    );
}

/// 3 ファイルは組で選ぶ: 利用者の場所の vectors.bin が合わなければ、組ごと同梱の場所にする（混ぜない）
#[test]
fn a_mismatched_vectors_file_drops_the_whole_user_set() {
    let l = Layout::new();
    put_fixture_dict(&l.user);
    std::fs::write(
        UserDir::new(&l.user)
            .dict_dir()
            .join(ikari_core::VECTORS_FILE_NAME),
        b"not vectors",
    )
    .unwrap();
    put_fixture_dict(&l.bundled);
    let e = l.open().expect("open");
    assert_eq!(dict_of(&e), LocationKind::Bundled);
    let log = l.log(&e);
    assert!(log.contains("err=- location=user reason=vectors"), "{log}");
}

/// どちらも落ちるときは、今までどおり起動に失敗する（ロックは放す）
#[test]
fn when_both_fail_the_engine_does_not_open() {
    let l = Layout::new();
    put_broken_dict(&l.user);
    put_broken_dict(&l.bundled);
    assert!(matches!(l.open(), Err(EngineError::Open(_))));
    let empty = Layout::new();
    assert!(matches!(empty.open(), Err(EngineError::Open(_))));
    assert!(crate::lock::retry_while_held(|| EngineLock::acquire(&l.user)).is_ok());
}

/// 絵文字と郵便番号の表も、利用者の場所に無ければ同梱の場所の物を使う
#[test]
fn tables_fall_back_to_the_bundled_location() {
    let l = Layout::new();
    put_fixture_dict(&l.user);
    put_fixture_dict(&l.bundled);
    let bundled_dict = UserDir::new(&l.bundled).dict_dir();
    std::fs::write(
        bundled_dict.join(ikari_core::EMOJI_TSV_FILE_NAME),
        "s\teyes\t👀\n",
    )
    .unwrap();
    let e = l.open().expect("open");
    let emoji = match send(&e, envelope::Body::EmojiTable(pb::EmojiTableRequest {})) {
        envelope::Body::EmojiTableReply(r) => r,
        other => panic!("unexpected reply: {other:?}"),
    };
    assert_eq!(emoji.entries.len(), 1);
    assert_eq!(emoji.entries[0].name, "eyes");
    // 利用者が置いた表が先
    let user_emoji = crate::dispatch::emoji_tsv_path_in(&l.user);
    std::fs::write(&user_emoji, "s\t+1\t👍\ns\tok\t👌\n").unwrap();
    assert_eq!(
        e.inner.emoji_table_path(&user_emoji),
        user_emoji,
        "the user's table wins once it is there"
    );
    std::fs::remove_file(&user_emoji).unwrap();
    assert_eq!(
        e.inner.emoji_table_path(&user_emoji),
        bundled_dict.join(ikari_core::EMOJI_TSV_FILE_NAME)
    );
}

/// 利用者の表が壊れていれば同梱の表を使い、落としたことを固定の語でログに書く（パスは書かない）
#[test]
fn a_broken_user_table_falls_back_to_the_bundled_one() {
    let l = Layout::new();
    put_fixture_dict(&l.user);
    put_fixture_dict(&l.bundled);
    let bundled_dict = UserDir::new(&l.bundled).dict_dir();
    std::fs::write(
        bundled_dict.join(ikari_core::EMOJI_TSV_FILE_NAME),
        "s\teyes\t👀\n",
    )
    .unwrap();
    let user_emoji = crate::dispatch::emoji_tsv_path_in(&l.user);
    std::fs::write(&user_emoji, "this is not a table\n").unwrap();
    let e = l.open().expect("open");
    assert_eq!(
        e.inner.emoji_table_path(&user_emoji),
        bundled_dict.join(ikari_core::EMOJI_TSV_FILE_NAME)
    );
    let log = l.log(&e);
    assert!(
        log.contains("table_reject") && log.contains("table=emoji.tsv location=user"),
        "{log}"
    );
    assert!(!log.contains(l.user.to_string_lossy().as_ref()), "{log}");
}

/// 同梱の場所を渡さない（開発のビルド）と、今と同じ（利用者の場所の辞書だけ。無ければ起動しない）
#[test]
fn without_a_bundled_location_nothing_changes() {
    let dir = temp_dir("bundled-none");
    let cfg = EngineConfig {
        data_dir: dir.to_string_lossy().into_owned(),
        enable_lm: false,
        idle_unload_secs: 300,
    };
    assert!(matches!(
        Engine::open_in(&cfg, &LmHostChoice::InProcess, None),
        Err(EngineError::Open(_))
    ));
    put_fixture_dict(&dir);
    let e = crate::lock::retry_while_held(|| Engine::open_in(&cfg, &LmHostChoice::InProcess, None))
        .unwrap();
    assert_eq!(dict_of(&e), LocationKind::User);
    drop(e);
    let _ = std::fs::remove_dir_all(dir);
}

/// `Hello` の辞書の出どころと日付（設定画面の「同梱の辞書（日付）」。ADR 0080 §3.2）
fn hello_dict(e: &Engine) -> (i32, String) {
    match send(e, envelope::Body::Hello(pb::Hello::default())) {
        envelope::Body::HelloReply(h) => (h.dict_source, h.dict_date),
        other => panic!("{other:?}"),
    }
}

fn put_build_info(root: &Path, text: &str) {
    std::fs::write(UserDir::new(root).dict_dir().join("BUILD_INFO"), text).unwrap();
}

#[test]
fn hello_reports_the_bundled_dictionary_and_its_date() {
    let l = Layout::new();
    put_fixture_dict(&l.bundled);
    put_build_info(&l.bundled, "format=1\ndate=2026-11-22\n");
    let e = l.open().expect("open");
    assert_eq!(
        hello_dict(&e),
        (pb::DictSource::Bundled as i32, "2026-11-22".to_owned())
    );
}

#[test]
fn hello_reports_the_user_dictionary_with_its_own_date() {
    let l = Layout::new();
    put_fixture_dict(&l.user);
    put_build_info(&l.user, "format=1\ndate=2026-12-01\n");
    put_fixture_dict(&l.bundled);
    put_build_info(&l.bundled, "format=1\ndate=2026-11-22\n");
    let e = l.open().expect("open");
    assert_eq!(
        hello_dict(&e),
        (pb::DictSource::User as i32, "2026-12-01".to_owned())
    );
}

/// BUILD_INFO が無い、または日付の行が無いときは日付が空（画面に出さない）
#[test]
fn hello_leaves_the_date_empty_when_unknown() {
    let l = Layout::new();
    put_fixture_dict(&l.bundled);
    let e = l.open().expect("open");
    assert_eq!(
        hello_dict(&e),
        (pb::DictSource::Bundled as i32, String::new())
    );
    drop(e);
    put_build_info(&l.bundled, "format=1\n");
    let e = l.open().expect("open");
    assert_eq!(
        hello_dict(&e),
        (pb::DictSource::Bundled as i32, String::new())
    );
}
