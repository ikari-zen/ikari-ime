//! 住所 ↔ 郵便番号（ADR 0032 §3）: 表の読み込み、設定、辞書と一緒の読み直し

use super::*;

const POSTAL_SAMPLE: &str = include_str!("../../../dict-format/tests/fixtures/postal.sample.tsv");
const ADDRESS: &str = "東京都千代田区千代田";
const CHAIN_READING: &str = "とうきょうとちよだくちよだ";

fn force_check(e: &Engine) {
    let watch = e.inner.watch.as_ref().unwrap();
    lock_mutex(watch).last_check = Instant::now() - DATA_CHECK_INTERVAL * 2;
}

fn candidates(e: &Engine, reading: &str) -> Vec<String> {
    begin(e, 1, false);
    surfaces(&convert(e, 1, reading, 0))
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn write_table(t: &Real, text: &str) {
    std::fs::write(crate::dispatch::postal_tsv_path_in(&t.dir), text).unwrap();
}

#[test]
fn no_table_gives_no_postal_candidates_and_no_error() {
    let t = Real::new();
    assert!(!candidates(t.e(), "1000001").contains(&ADDRESS.to_owned()));
    assert!(!candidates(t.e(), CHAIN_READING).contains(&"1000001".to_owned()));
}

#[test]
fn table_is_used_at_open_in_both_directions() {
    let mut t = Real::new();
    write_table(&t, POSTAL_SAMPLE);
    t.engine = None;
    t.open();
    let by_code = candidates(t.e(), "100-0001");
    assert!(by_code.contains(&ADDRESS.to_owned()), "{by_code:?}");
    let by_address = candidates(t.e(), CHAIN_READING);
    let plain = by_address.iter().position(|s| s == "1000001").unwrap();
    assert_eq!(by_address[plain + 1], "100-0001", "{by_address:?}");
    assert!(plain <= 2, "{by_address:?}");
}

#[test]
fn postal_japanese_off_in_settings_gives_nothing() {
    let mut t = Real::new();
    write_table(&t, POSTAL_SAMPLE);
    std::fs::write(t.dir.join(SETTINGS_FILE_NAME), "postal.japanese\toff\n").unwrap();
    t.engine = None;
    t.open();
    assert!(!candidates(t.e(), "1000001").contains(&ADDRESS.to_owned()));
    assert!(!candidates(t.e(), CHAIN_READING).contains(&"1000001".to_owned()));
}

#[test]
fn table_is_reloaded_with_the_dictionary_when_the_file_changes() {
    let t = Real::new();
    assert!(!candidates(t.e(), "1000001").contains(&ADDRESS.to_owned()));
    write_table(&t, POSTAL_SAMPLE);
    force_check(t.e());
    t.e().inner.refresh_data();
    assert!(candidates(t.e(), "1000001").contains(&ADDRESS.to_owned()));
    assert_eq!(count_lines(&t.log(), " reload "), 1, "{}", t.log());

    // 壊れた表は何も出さずに続ける（変換は動く）
    write_table(&t, "あ\tア\t123\tf\n");
    force_check(t.e());
    t.e().inner.refresh_data();
    assert!(!candidates(t.e(), "1000001").contains(&ADDRESS.to_owned()));
}
