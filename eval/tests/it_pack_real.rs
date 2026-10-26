//! IT の語彙集（`core/assets/packs/it.tsv`）を実辞書の Full 構成で確かめる（ADR 0012 §9-3、`#[ignore]`）。
//!
//! ```text
//! IKARI_DICT_DIR=data/build/dict cargo test --release -p ikari-eval --test it_pack_real -- --ignored --nocapture
//! ```
//!
//! `IKARI_DICT_DIR` がなければ失敗する（黙って成功させない）。LM は使わない。
//! - 全行: カタカナ（読みのとおり）が1位で英字が2位（形 `latin` も。ADR 0020 §12、B32）
//! - 1回英字を選べば、次から英字が1位（形 `kana` は差 `PACK_LATIN_OFFSET` が学習の加点より小さいこと。ADR 0012 §10-8。
//!   形 `latin` は ADR 0020 の E4）
//! - 語彙集の読みを含まない入力の1位は、語彙集の有無で変わらない（評価セットの全ケース）

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use ikari_core::user::{PackForm, builtin_it_pack, today_unix_day};
use ikari_core::{Context, open_dir};
use ikari_eval::setup::{Setup, SetupOptions, build, default_vectors_path};
use ikari_eval::{EvalCase, load_cases};
use ikari_stack::RankerSet;

const DICT_DIR_ENV: &str = "IKARI_DICT_DIR";
const SET_FILES: [&str; 8] = [
    "homophones.tsv",
    "homophones-ext.tsv",
    "hiragana.tsv",
    "katakana-suffix.tsv",
    "context-compound.tsv",
    "adverbs.tsv",
    "verbs.tsv",
    "compound-affix.tsv",
];
const SHOWN_FAILURES: usize = 30;

fn dict_dir() -> PathBuf {
    std::env::var_os(DICT_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{DICT_DIR_ENV} is required (a real dictionary)"))
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("eval/ has a parent")
        .to_path_buf()
}

fn setup(packs: bool) -> Setup {
    let dir = dict_dir();
    let loaded = open_dir(&dir).expect("open the dictionary");
    let opts = SetupOptions {
        ranker: Some(RankerSet::Full),
        default_vectors: Some(default_vectors_path(&dir)),
        packs,
        ..SetupOptions::default()
    };
    build(loaded.dict, loaded.grammar, &opts).expect("build the stack")
}

/// 1ページ目の表記
fn first_page(setup: &Setup, context: &str, reading: &str) -> Vec<String> {
    let ctx = Context::new().with_left_text(context);
    let mut conversion = setup.converter.convert_until(reading, &ctx, None);
    conversion
        .page_until(0, None)
        .map(|p| p.candidates.iter().map(|c| c.surface.clone()).collect())
        .unwrap_or_default()
}

fn katakana(reading: &str) -> String {
    reading
        .chars()
        .map(|c| match c {
            '\u{3041}'..='\u{3096}' => char::from_u32(c as u32 + 0x60).unwrap_or(c),
            other => other,
        })
        .collect()
}

#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary)"]
fn every_row_puts_katakana_first_and_latin_second() {
    let setup = setup(true);
    let pack = builtin_it_pack().unwrap();
    let mut failures = Vec::new();
    for e in pack.entries() {
        let page = first_page(&setup, "", &e.reading);
        // 形 `latin` も、読みのカタカナが1位で英字が2位（ADR 0020 §12。B32）
        let ok = page.first().map(String::as_str) == Some(katakana(&e.reading).as_str())
            && page.get(1).map(String::as_str) == Some(e.latin.as_str());
        if !ok {
            failures.push(format!("{} ({:?}) -> {:?}", e.reading, e.form, page));
        }
    }
    for f in failures.iter().take(SHOWN_FAILURES) {
        eprintln!("FAIL {f}");
    }
    assert!(
        failures.is_empty(),
        "{} of {} rows",
        failures.len(),
        pack.entries().len()
    );
}

#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary)"]
fn choosing_latin_once_makes_it_first() {
    let setup = setup(true);
    let store = setup.store.clone().expect("full has a learning store");
    let today = today_unix_day();
    let pack = builtin_it_pack().unwrap();
    let rows: Vec<_> = pack.entries().iter().collect();
    let mut failures = Vec::new();
    for e in &rows {
        let ctx = Context::new();
        let mut conversion = setup.converter.convert_until(&e.reading, &ctx, None);
        let index = conversion
            .page_until(0, None)
            .and_then(|p| p.candidates.iter().position(|c| c.surface == e.latin));
        let Some(index) = index else {
            failures.push(format!(
                "{}: {} is not on the first page",
                e.reading, e.latin
            ));
            continue;
        };
        if let Some(ev) = conversion.commit(index) {
            store.record(&ev, setup.vectors.as_deref(), today);
        }
    }
    for e in &rows {
        let page = first_page(&setup, "", &e.reading);
        if page.first().map(String::as_str) != Some(e.latin.as_str()) {
            failures.push(format!("{} -> {:?}", e.reading, page));
        }
    }
    for f in failures.iter().take(SHOWN_FAILURES) {
        eprintln!("FAIL {f}");
    }
    assert!(
        failures.is_empty(),
        "{} of {} rows",
        failures.len(),
        rows.len()
    );
}

/// 評価セットの全ケースで、語彙集の読みを含まない入力の1位が変わらない。
/// 語彙集の読みを含む入力（らすと、ぎっとはぶ…）は変わってよいので、変わった数を出す。
#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary)"]
fn everyday_inputs_keep_their_first_candidate() {
    let with = setup(true);
    let without = setup(false);
    let pack = builtin_it_pack().unwrap();
    let readings: BTreeSet<&str> = pack.entries().iter().map(|e| e.reading.as_str()).collect();
    let mut cases: Vec<EvalCase> = Vec::new();
    for name in SET_FILES {
        let path = workspace_root().join("eval/sets").join(name);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        cases.extend(load_cases(&text).unwrap_or_else(|e| panic!("{path:?}: {e}")));
    }
    let (mut unrelated, mut related, mut changed_related) = (0, 0, 0);
    let mut broken = Vec::new();
    for case in &cases {
        let a = first_page(&with, &case.context, &case.reading);
        let b = first_page(&without, &case.context, &case.reading);
        let touches_pack = readings.iter().any(|r| case.reading.contains(r));
        if touches_pack {
            related += 1;
            if a.first() != b.first() {
                changed_related += 1;
                eprintln!(
                    "changed (contains a pack reading): {} {:?} -> {:?}",
                    case.reading,
                    b.first(),
                    a.first()
                );
            }
        } else {
            unrelated += 1;
            if a.first() != b.first() {
                broken.push(format!(
                    "{} {:?} -> {:?}",
                    case.reading,
                    b.first(),
                    a.first()
                ));
            }
        }
    }
    // 語彙集の読みそのもの: 語彙集がなくても1位がカタカナの語は、1位が変わらない。変わる語は一覧に出す（情報）
    let mut changed_rows = Vec::new();
    for e in pack.entries().iter().filter(|e| e.form == PackForm::Kana) {
        let a = first_page(&with, "", &e.reading);
        let b = first_page(&without, "", &e.reading);
        if a.first() != b.first() {
            changed_rows.push(format!("{} {:?} -> {:?}", e.reading, b.first(), a.first()));
        }
    }
    eprintln!(
        "pack readings whose first candidate changes ({}): {}",
        changed_rows.len(),
        changed_rows.join(" / ")
    );
    eprintln!(
        "cases: {} unrelated to the pack, {} contain a pack reading ({} changed)",
        unrelated, related, changed_related
    );
    for f in broken.iter().take(SHOWN_FAILURES) {
        eprintln!("FAIL {f}");
    }
    assert!(broken.is_empty(), "{} cases changed", broken.len());
}
