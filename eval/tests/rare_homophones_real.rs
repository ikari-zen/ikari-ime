//! まれな同音語の規則 S・R を実辞書（Full）で確かめる（ADR 0016、`#[ignore]`）。
//!
//! ```text
//! IKARI_DICT_DIR=data/build/dict cargo test --release -p ikari-eval --test rare_homophones_real -- --ignored --nocapture
//! ```
//!
//! `IKARI_DICT_DIR` がなければ失敗する（黙って成功させない）。LM は使わない。辞書は規則 R（`cheap_rare_homophone`）を
//! 入れて作り直したものが要る。
//! - せいこうする: 1位は 成功する。製鋼する は規則 R（コストを 6518 に上げる）で上位 3 件に入らない
//! - じょうきょうする: 1位は 上京する（規則 S。サ変でない名詞 状況 + する に +3000）
//! - げーむする: 1位は ゲームする（ほかに候補がないので、規則 S でも消えない）
//! - うちゅう: 1位は 宇宙（規則 R。右中 のコストが 6518 まで上がる）

use std::path::PathBuf;

use ikari_core::{Context, open_dir};
use ikari_eval::setup::{Setup, SetupOptions, build, default_vectors_path};
use ikari_stack::RankerSet;

const DICT_DIR_ENV: &str = "IKARI_DICT_DIR";
/// 「上位に入らない」を確かめる件数
const TOP_N: usize = 3;

fn dict_dir() -> PathBuf {
    std::env::var_os(DICT_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{DICT_DIR_ENV} is required (a real dictionary)"))
}

fn setup() -> Setup {
    let dir = dict_dir();
    let loaded = open_dir(&dir).expect("open the dictionary");
    let opts = SetupOptions {
        ranker: Some(RankerSet::Full),
        default_vectors: Some(default_vectors_path(&dir)),
        ..SetupOptions::default()
    };
    build(loaded.dict, loaded.grammar, &opts).expect("build the stack")
}

/// 文脈なしの1ページ目の表記
fn first_page(setup: &Setup, reading: &str) -> Vec<String> {
    let mut conversion = setup
        .converter
        .convert_until(reading, &Context::new(), None);
    conversion
        .page_until(0, None)
        .map(|p| p.candidates.iter().map(|c| c.surface.clone()).collect())
        .unwrap_or_default()
}

fn assert_top(setup: &Setup, reading: &str, expected: &str) -> Vec<String> {
    let page = first_page(setup, reading);
    assert_eq!(
        page.first().map(String::as_str),
        Some(expected),
        "{reading}: {page:?}"
    );
    page
}

#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary)"]
fn seikou_suru_puts_seikou_first_and_seikou_steel_out_of_the_top() {
    let setup = setup();
    let page = assert_top(&setup, "せいこうする", "成功する");
    assert!(
        !page.iter().take(TOP_N).any(|s| s == "製鋼する"),
        "製鋼する is in the top {TOP_N}: {page:?}"
    );
}

#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary)"]
fn jokyo_suru_prefers_the_sahen_noun() {
    let setup = setup();
    assert_top(&setup, "じょうきょうする", "上京する");
}

#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary)"]
fn game_suru_keeps_the_only_candidate() {
    let setup = setup();
    assert_top(&setup, "げーむする", "ゲームする");
}

#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary)"]
fn cheap_rare_words_do_not_beat_the_frequent_homophone() {
    let setup = setup();
    assert_top(&setup, "うちゅう", "宇宙");
}
