//! 形式 v2 のビルド（ポリシー、活用規則、vectors.bin）の結合テスト。fixture を使う。

use std::fs;
use std::path::{Path, PathBuf};

use ikari_dict_builder::{
    BuildConfig, BuildError, BuildOptions, ChiveVocab, Policy, VectorsConfig, build, build_vectors,
    build_with, verify, verify_vectors,
};
use ikari_dict_format::{
    GRAMMAR_FILE_NAME, SYSTEM_DICT_FILE_NAME, VECTORS_FILE_NAME, index_get, parse_dict_layout,
    parse_grammar_layout, parse_vectors_layout, read_conj_rules, read_pos_record, read_raw_entry,
    unpack_index_value,
};

const NOUN: &str = "名詞,普通名詞,一般,*,*,*";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../testdata/sudachi-mini")
        .join(name)
}

fn chive_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../testdata/chive-mini/vectors.txt")
}

fn config(out: &Path) -> BuildConfig {
    BuildConfig {
        lex: vec![fixture("lex.csv")],
        matrix: fixture("matrix.def"),
        out_dir: out.to_path_buf(),
        unk_pos: NOUN.to_string(),
        unk_cost: 10000,
        label: "sudachi-mini".to_string(),
    }
}

/// 読みと表記が一致するエントリの ime_penalty。
fn penalties(dir: &Path, reading: &str, surface: &str) -> Vec<i16> {
    let dict = fs::read(dir.join(SYSTEM_DICT_FILE_NAME)).unwrap();
    let l = parse_dict_layout(&dict).unwrap();
    let Some(v) = index_get(&dict[l.index.clone()], reading).unwrap() else {
        return Vec::new();
    };
    let (first, count) = unpack_index_value(v);
    (first..first + count)
        .filter_map(|i| {
            let e = read_raw_entry(&dict[l.entries.clone()], i)?;
            let s = ikari_dict_format::read_surface(&dict[l.strings.clone()], &e)?;
            (s == surface).then_some(e.ime_penalty)
        })
        .collect()
}

#[test]
fn report_counts_policy_hits_and_conj_rules() {
    let dir = tempfile::tempdir().unwrap();
    let report = build(&config(dir.path())).unwrap();
    let hits = report.policy.hits_by_predicate();
    assert!(hits["ascii_lower"] >= 2, "{hits:?}"); // relax, hennkann
    assert!(hits["numeric_surface"] >= 1, "{hits:?}");
    assert!(hits["katakana_variant"] >= 1, "{hits:?}");
    assert!(
        hits["proper_noun"] >= 1 && hits["person_name"] >= 1,
        "{hits:?}"
    );
    assert_eq!(hits["lemma_rank"], 0);
    // ポリシーは除外せず、後ろに下げるだけ
    assert_eq!(report.policy.rows, report.entries);
    assert!(report.conj_rules >= 4, "{report}");
    assert!(report.lemma_count > 0 && report.lemma_count < report.entries);
    let text = report.to_string();
    assert!(text.contains("policy hit (ascii_lower)"), "{text}");
    assert!(text.contains("conj rules:"), "{text}");
}

#[test]
fn builds_are_deterministic_and_verify() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    build(&config(a.path())).unwrap();
    build(&config(b.path())).unwrap();
    for name in [GRAMMAR_FILE_NAME, SYSTEM_DICT_FILE_NAME] {
        assert_eq!(
            fs::read(a.path().join(name)).unwrap(),
            fs::read(b.path().join(name)).unwrap(),
            "{name}"
        );
    }
    let report = verify(a.path()).unwrap();
    assert!(report.surfaces > 0 && report.lemmas > 0);
}

#[test]
fn meta_records_the_policy_hash() {
    let dir = tempfile::tempdir().unwrap();
    let report = build(&config(dir.path())).unwrap();
    let dict = fs::read(dir.path().join(SYSTEM_DICT_FILE_NAME)).unwrap();
    let l = parse_dict_layout(&dict).unwrap();
    let meta = std::str::from_utf8(&dict[l.meta.clone()]).unwrap();
    assert!(
        meta.contains(&format!("policy={:016x}\n", report.policy_fingerprint)),
        "{meta}"
    );
    assert!(meta.contains("conj_rules="), "{meta}");
    assert!(!meta.contains("chive_vocab="), "{meta}");
}

#[test]
fn custom_policy_changes_penalties_but_not_costs() {
    let default_dir = tempfile::tempdir().unwrap();
    build(&config(default_dir.path())).unwrap();
    assert_eq!(penalties(default_dir.path(), "こう", "コウ"), [5000]);

    let custom_dir = tempfile::tempdir().unwrap();
    let opts = BuildOptions {
        policy: Policy::parse("katakana_variant\t-500\nascii_lower\t20000\n").unwrap(),
        ..BuildOptions::default()
    };
    let report = build_with(&config(custom_dir.path()), &opts).unwrap();
    assert_eq!(penalties(custom_dir.path(), "こう", "コウ"), [-500]);
    // 書かれていない述語は何もしない
    assert_eq!(penalties(custom_dir.path(), "よう", "4"), [0]);
    assert_eq!(
        penalties(custom_dir.path(), "へんかん", "hennkann"),
        [20000]
    );
    assert_ne!(
        report.policy_fingerprint,
        build(&config(default_dir.path()))
            .unwrap()
            .policy_fingerprint
    );
}

#[test]
fn lemma_rank_needs_a_vocabulary() {
    let dir = tempfile::tempdir().unwrap();
    let policy = Policy::parse("lemma_rank\t1000\n").unwrap();
    let opts = BuildOptions {
        policy: policy.clone(),
        ..BuildOptions::default()
    };
    let err = build_with(&config(dir.path()), &opts).unwrap_err();
    assert!(matches!(err, BuildError::Config(_)), "{err}");

    let vocab = ChiveVocab::from_keys(["構成".to_string()]);
    let opts = BuildOptions {
        policy,
        chive_vocab: Some(vocab),
        chive_vocab_label: Some("chive-test".to_string()),
        ..BuildOptions::default()
    };
    build_with(&config(dir.path()), &opts).unwrap();
    let dict = fs::read(dir.path().join(SYSTEM_DICT_FILE_NAME)).unwrap();
    let l = parse_dict_layout(&dict).unwrap();
    let meta = std::str::from_utf8(&dict[l.meta.clone()]).unwrap();
    assert!(meta.contains("chive_vocab=chive-test:1\n"), "{meta}");
}

#[test]
fn pos_records_carry_user_cost_and_rep_share() {
    let dir = tempfile::tempdir().unwrap();
    build(&config(dir.path())).unwrap();
    let bytes = fs::read(dir.path().join(GRAMMAR_FILE_NAME)).unwrap();
    let g = parse_grammar_layout(&bytes).unwrap();
    let section = &bytes[g.pos.clone()];
    let mut noun = None;
    for id in 0..g.pos_count {
        let r = read_pos_record(section, g.pos_count, id as u16).unwrap();
        assert!(r.rep_share_permille <= 1000);
        if r.name == NOUN {
            noun = Some(r);
        }
    }
    let noun = noun.unwrap();
    assert!(noun.rep_share_permille > 0);
    // 10パーセンタイルは、その品詞のコストの下位に当たる
    assert!(noun.user_cost < 6000, "{}", noun.user_cost);
}

#[test]
fn conj_rules_are_derived_from_fixture_families() {
    let dir = tempfile::tempdir().unwrap();
    build(&config(dir.path())).unwrap();
    let bytes = fs::read(dir.path().join(GRAMMAR_FILE_NAME)).unwrap();
    let g = parse_grammar_layout(&bytes).unwrap();
    let pos_section = &bytes[g.pos.clone()];
    let pos_id = |name: &str| {
        (0..g.pos_count)
            .find(|&i| {
                read_pos_record(pos_section, g.pos_count, i as u16)
                    .unwrap()
                    .name
                    == name
            })
            .map(|i| i as u16)
            .unwrap_or_else(|| panic!("no pos {name}"))
    };
    let rules = read_conj_rules(&bytes[g.conj.clone()], g.pos_count).unwrap();
    let find = |base: &str, target: &str| {
        rules
            .iter()
            .find(|r| r.base_pos == pos_id(base) && r.target_pos == pos_id(target))
    };
    let r = find(
        "動詞,一般,*,*,下一段-マ行,終止形-一般",
        "動詞,一般,*,*,下一段-マ行,連用形-一般",
    )
    .unwrap();
    assert_eq!((r.strip_chars, r.add_surface, r.add_reading), (1, "", ""));
    let r = find(
        "動詞,一般,*,*,五段-ラ行,終止形-一般",
        "動詞,一般,*,*,五段-ラ行,連用形-促音便",
    )
    .unwrap();
    assert_eq!(
        (r.strip_chars, r.add_surface, r.add_reading),
        (1, "っ", "っ")
    );
    // 活用型が1語しかない（支持が足りない）組は規則にならない
    assert!(
        find(
            "動詞,非自立可能,*,*,サ行変格,終止形-一般",
            "動詞,非自立可能,*,*,サ行変格,連用形-一般"
        )
        .is_none()
    );
}

#[test]
fn vectors_report_and_files() {
    let dir = tempfile::tempdir().unwrap();
    build(&config(dir.path())).unwrap();
    let out = dir.path().join(VECTORS_FILE_NAME);
    let mut cfg = VectorsConfig::new(chive_fixture(), dir.path().to_path_buf(), out.clone());
    cfg.clusters = 5;
    let report = build_vectors(&cfg).unwrap();
    assert_eq!(report.chive_rows, 20);
    assert_eq!(report.dims, 8);
    // 辞書の lemma と完全一致した語だけが行になる（存在しない語1,2 は入らない）
    assert_eq!(report.matched, 18, "{report}");
    assert_eq!(report.clusters, 5);
    assert_eq!(report.cluster_sizes.iter().sum::<usize>(), 18);
    assert!(report.chive_match_percent() > 80.0);
    assert!(report.lemma_coverage_percent() > 0.0);

    let bytes = fs::read(&out).unwrap();
    let l = parse_vectors_layout(&bytes).unwrap();
    assert_eq!((l.dims, l.row_count, l.cluster_count), (8, 18, 5));
    let verified = verify_vectors(dir.path()).unwrap();
    assert_eq!(verified.rows, 18);
    assert_eq!(verified.lemmas_with_vector, 18);

    // 再実行しても同じ（k-means の種は固定）
    let first = bytes;
    build_vectors(&cfg).unwrap();
    assert_eq!(first, fs::read(&out).unwrap());

    // 辞書を別の内容で作り直すと、古い vectors.bin は検証で弾かれる
    let mut other = config(dir.path());
    other.lex = vec![fixture("lex.csv")];
    let lex_text = fs::read_to_string(fixture("lex.csv")).unwrap();
    let trimmed = dir.path().join("trimmed.csv");
    fs::write(
        &trimmed,
        lex_text.lines().take(120).collect::<Vec<_>>().join("\n") + "\n",
    )
    .unwrap();
    other.lex = vec![trimmed];
    build(&other).unwrap();
    assert!(verify_vectors(dir.path()).is_err());
}

#[test]
fn vectors_errors_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    build(&config(dir.path())).unwrap();
    // chiVe のファイルがない
    let cfg = VectorsConfig::new(
        dir.path().join("none.txt"),
        dir.path().to_path_buf(),
        dir.path().join(VECTORS_FILE_NAME),
    );
    assert!(build_vectors(&cfg).is_err());
    // 辞書がない
    let empty = tempfile::tempdir().unwrap();
    let cfg = VectorsConfig::new(
        chive_fixture(),
        empty.path().to_path_buf(),
        empty.path().join(VECTORS_FILE_NAME),
    );
    assert!(build_vectors(&cfg).is_err());
}

/// 19 列の Sudachi 形式の行（辞書形の ID は行番号）
fn verb_line(
    surface: &str,
    cost: i32,
    form: &str,
    reading: &str,
    lemma: &str,
    dict_form: usize,
) -> String {
    format!(
        "{surface},1,1,{cost},{surface},動詞,一般,*,*,下一段-マ行,{form},{reading},{lemma},{dict_form},A,*,*,*,*"
    )
}

#[test]
fn kun_table_moves_only_the_penalty_in_a_built_dictionary() {
    let dir = tempfile::tempdir().unwrap();
    let lex = dir.path().join("lex.csv");
    let end = "終止形-一般";
    let lines = [
        verb_line("勧める", 6000, end, "ススメル", "勧める", 0),
        verb_line("薦める", 6100, end, "ススメル", "勧める", 1),
        verb_line("奨める", 6200, end, "ススメル", "勧める", 2),
        verb_line("すすめる", 7000, end, "ススメル", "勧める", 3),
        verb_line("進める", 9000, end, "ススメル", "進める", 4),
        verb_line("進めて", 9500, "連用形-促音便", "ススメテ", "進める", 4),
        // 取る: 既定の 取る が 捕る より 1734 高い。摂る は表外
        verb_line("取る", 7000, end, "トル", "取る", 6),
        verb_line("捕る", 5266, end, "トル", "取る", 7),
        verb_line("摂る", 7876, end, "トル", "取る", 8),
        "名詞,1,1,5000,名詞,名詞,普通名詞,一般,*,*,*,メイシ,名詞,*,A,*,*,*,*".to_string(),
    ];
    // 最後の行は未知語の品詞（NOUN）を作るための行
    fs::write(&lex, lines.join("\n") + "\n").unwrap();
    let matrix = dir.path().join("matrix.def");
    fs::write(&matrix, "2 2\n0 0 0\n0 1 0\n1 0 0\n1 1 0\n").unwrap();
    let out = dir.path().join("out");
    let cfg = BuildConfig {
        lex: vec![lex],
        matrix,
        out_dir: out.clone(),
        unk_pos: NOUN.to_string(),
        unk_cost: 10000,
        label: "kun".to_string(),
    };
    let report = build(&cfg).unwrap();
    // 勧める系: 既定が薦める以下の安さなので差は変えない。奨める（表外）は +3000。かなの表記は対象外
    assert_eq!(penalties(&out, "すすめる", "勧める"), [0]);
    assert_eq!(penalties(&out, "すすめる", "薦める"), [0]);
    assert_eq!(penalties(&out, "すすめる", "奨める"), [3000]);
    assert_eq!(penalties(&out, "すすめる", "すすめる"), [0]);
    // 読みの組の既定: 進める は -500（読みが すすめ で始まる行）
    assert_eq!(penalties(&out, "すすめる", "進める"), [-500]);
    assert_eq!(penalties(&out, "すすめて", "進めて"), [-500]);
    // 取る: 捕る(5266) より 50 安くなるように -1784、さらに読みの組の既定 -500
    assert_eq!(penalties(&out, "とる", "取る"), [-1784 - 500]);
    assert_eq!(penalties(&out, "とる", "捕る"), [0]);
    assert_eq!(penalties(&out, "とる", "摂る"), [3000]);
    let hits = report.policy.hits_by_predicate();
    assert_eq!(
        (
            hits["kun_outside"],
            hits["kun_lemma_default"],
            hits["kun_reading_default"]
        ),
        (2, 1, 3),
        "{hits:?}"
    );
    // どの表記も辞書に残る（除外しない）
    assert_eq!(report.entries, lines.len());
}
