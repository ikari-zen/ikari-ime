//! 実辞書のテスト（ADR 0001 §10.3）。`#[ignore]` なので `--ignored` で明示したときだけ動き、
//! `IKARI_DICT_DIR` がなければ失敗する（黙って成功しない）。
//!
//! `IKARI_DICT_DIR=data/build/dict cargo test -p ikari-core --test dict_real -- --ignored --nocapture`

use std::path::PathBuf;
use std::time::Instant;

use ikari_core::{Dictionary, Grammar, WordId, open_dir};
use ikari_dict_format::reading::normalize_key;

const DICT_DIR_ENV: &str = "IKARI_DICT_DIR";

fn dict_dir() -> PathBuf {
    std::env::var_os(DICT_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            panic!("{DICT_DIR_ENV} is required (a real dictionary built by ikari-dict-builder)")
        })
}

#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary built by ikari-dict-builder)"]
fn real_dictionary_has_hinshi_and_hinshi_candidates() {
    let dir = dict_dir();
    let started = Instant::now();
    let loaded = open_dir(&dir).unwrap();
    eprintln!("open_dir: {:?}", started.elapsed());
    eprintln!("meta:\n{}", loaded.dict.meta());

    // 品詞 / 瀕死 が引け、品詞名も引ける
    let mut hits = Vec::new();
    loaded
        .dict
        .lookup_exact(&normalize_key("ヒンシ"), &mut |e| {
            hits.push((
                e.surface.to_string(),
                e.cost,
                loaded.grammar.pos_name(e.pos_id).map(String::from),
            ));
        });
    eprintln!("ひんし: {hits:?}");
    let noun = Some("名詞,普通名詞,一般,*,*,*".to_string());
    assert!(
        hits.iter()
            .any(|h| h.0 == "品詞" && h.1 == 7392 && h.2 == noun),
        "{hits:?}"
    );
    assert!(
        hits.iter()
            .any(|h| h.0 == "瀕死" && h.1 == 7403 && h.2 == noun),
        "{hits:?}"
    );
    assert!(hits.windows(2).all(|w| w[0].1 <= w[1].1));

    // 共通接頭辞検索
    let started = Instant::now();
    let mut count = 0;
    loaded
        .dict
        .common_prefix_search("ひんしのこうせいをかいとうする", &mut |_, _| count += 1);
    eprintln!(
        "common_prefix_search: {count} hits in {:?}",
        started.elapsed()
    );
    assert!(count > 0);

    // 文法
    assert_eq!(loaded.dict.grammar_id(), loaded.grammar.grammar_id());
    let unk = loaded.grammar.unknown();
    assert_eq!(
        loaded.grammar.pos_name(unk.pos_id),
        Some("名詞,普通名詞,一般,*,*,*")
    );
    assert_eq!(loaded.grammar.bos_eos_id(), 0);
    assert!(loaded.grammar.connection_cost(unk.right_id, unk.left_id) < i16::MAX);
}

/// 常駐メモリ（KiB）。`ps` が使えなければ 0
fn resident_kib() -> u64 {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// 逆引き表（ADR 0006 §9.3、S0-6）: 作成時間と増えたメモリを出し、抜き取りで不変条件を確かめる
#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary built by ikari-dict-builder)"]
fn real_dictionary_reading_index_inverts_lookup_exact() {
    const SAMPLE_STEP: usize = 7;
    // 目安は release で約 0.1 秒、64 MiB 未満（S0-6）。debug は最適化がなく 10 倍ほど遅いので閾値だけ緩める
    const BUILD_LIMIT_RELEASE_SECS: f64 = 1.0;
    const BUILD_LIMIT_DEBUG_SECS: f64 = 10.0;
    const GROWN_LIMIT_MIB: f64 = 64.0;
    let build_limit = if cfg!(debug_assertions) {
        BUILD_LIMIT_DEBUG_SECS
    } else {
        BUILD_LIMIT_RELEASE_SECS
    };
    let loaded = open_dir(&dict_dir()).unwrap();
    let count = loaded.dict.entry_count();
    let before = resident_kib();
    let started = Instant::now();
    loaded.dict.warm_reading_index();
    let built = started.elapsed();
    let grown_mib = resident_kib().saturating_sub(before) as f64 / 1024.0;
    eprintln!("reading index: {count} entries, built in {built:?}, rss +{grown_mib:.1} MiB");
    assert!(
        built.as_secs_f64() < build_limit,
        "S0-6: must build in under {build_limit} s (built in {built:?})"
    );
    if !cfg!(debug_assertions) {
        assert!(
            grown_mib < GROWN_LIMIT_MIB,
            "S0-6: the index must stay under {GROWN_LIMIT_MIB} MiB (grew {grown_mib:.1})"
        );
    }

    let mut checked = 0;
    for index in (0..count).step_by(SAMPLE_STEP) {
        let id = WordId {
            dict: 0,
            index: index as u32,
        };
        let reading = loaded.dict.reading(id).expect("every entry has a reading");
        let mut found = false;
        loaded
            .dict
            .lookup_exact(&reading, &mut |e| found |= e.id == id);
        assert!(found, "entry {index} not found by reading {reading:?}");
        checked += 1;
    }
    let started = Instant::now();
    let mut sink = 0usize;
    for index in 0..100_000usize {
        let id = WordId {
            dict: 0,
            index: ((index * 15) % count) as u32,
        };
        sink += loaded.dict.reading(id).map_or(0, |r| r.len());
    }
    eprintln!(
        "checked {checked} entries; reading(): {:?}/call (sink {sink})",
        started.elapsed() / 100_000
    );
}
