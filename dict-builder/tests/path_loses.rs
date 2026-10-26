//! 経路が負ける型のコスト補正（ADR 0035）の結合テスト。
//!
//! 守ること: 規則 F（chiVe にない熟語に、部品がよく使う語なら `frequent_compound`）は、表記と読みを辞書の 2 語に切れるときだけ当たる。
//! 規則 H（`default_cost_homophone`）は、同じ読みの頻度がいちばん高い語の総コストを、よりまれな同音語の総コスト − 余白 まで下げる。

use std::fs;
use std::path::{Path, PathBuf};

use ikari_dict_builder::{BuildConfig, BuildOptions, ChiveVocab, Policy, build_with};
use ikari_dict_format::{
    SYSTEM_DICT_FILE_NAME, index_get, parse_dict_layout, read_raw_entry, read_surface,
    unpack_index_value,
};

const NOUN: &str = "名詞,普通名詞,一般,*,*,*";
const SAHEN: &str = "名詞,普通名詞,サ変可能,*,*,*";
const MATRIX: &str = "2 2\n0 0 0\n0 1 0\n1 0 0\n1 1 0\n";
const POLICY: &str = "lemma_rank\t2000\nfrequent_compound\t-8000\ndefault_cost_homophone\t600\n";
/// 語彙の大きさ。語彙にない語の `lemma_rank` は log10(20001) − 3 ≈ 1.3 桁で約 +2600。語彙の 1000 位までの語は 0
const VOCAB_SIZE: usize = 20_000;

fn row(surface: &str, cost: i32, pos: &str, reading: &str) -> String {
    format!("{surface},1,1,{cost},{surface},{pos},{reading},{surface},*,A,*,*,*,*")
}

fn write(dir: &Path, name: &str, lines: &[String]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    path
}

fn config(dir: &Path, lex: Vec<PathBuf>) -> BuildConfig {
    let matrix = dir.join("matrix.def");
    fs::write(&matrix, MATRIX).unwrap();
    BuildConfig {
        lex,
        matrix,
        out_dir: dir.join("out"),
        unk_pos: NOUN.to_string(),
        unk_cost: 10000,
        label: "test".to_string(),
    }
}

/// 順位 = 並びの位置。ほかは埋め草
fn vocab(words: &[(usize, &str)]) -> ChiveVocab {
    let mut keys: Vec<String> = (0..VOCAB_SIZE).map(|i| format!("k{i}")).collect();
    for (rank, word) in words {
        keys[*rank] = (*word).to_string();
    }
    ChiveVocab::from_keys(keys)
}

fn penalty(dir: &Path, reading: &str, surface: &str) -> Option<i16> {
    let dict = fs::read(dir.join(SYSTEM_DICT_FILE_NAME)).unwrap();
    let l = parse_dict_layout(&dict).unwrap();
    let (first, count) = unpack_index_value(index_get(&dict[l.index.clone()], reading).unwrap()?);
    (first..first + count).find_map(|i| {
        let e = read_raw_entry(&dict[l.entries.clone()], i).unwrap();
        (read_surface(&dict[l.strings.clone()], &e).unwrap() == surface).then_some(e.ime_penalty)
    })
}

fn build(
    lines: &[String],
    words: &[(usize, &str)],
) -> (tempfile::TempDir, ikari_dict_builder::BuildReport) {
    let tmp = tempfile::tempdir().unwrap();
    let lex = write(tmp.path(), "lex.csv", lines);
    let opts = BuildOptions {
        policy: Policy::parse(POLICY).unwrap(),
        chive_vocab: Some(vocab(words)),
        ..BuildOptions::default()
    };
    let report = build_with(&config(tmp.path(), vec![lex]), &opts).unwrap();
    (tmp, report)
}

#[test]
fn compounds_missing_from_chive_are_boosted_only_when_they_split_into_frequent_dictionary_words() {
    let lines = [
        row("異常", 6000, NOUN, "イジョウ"),
        row("気象", 7379, NOUN, "キショウ"),
        row("以上", 3000, NOUN, "イジョウ"),
        // chiVe にない 4 文字の熟語。異常 + 気象 に切れる
        row("異常気象", 15608, NOUN, "イジョウキショウ"),
        // 読みが部品の読みのつなぎと合わない（切れない）
        row("異常天気", 15000, NOUN, "イジョウキショウ"),
        // 部品の一方が辞書にない
        row("異常寒波", 15000, NOUN, "イジョウカンパ"),
    ];
    let (tmp, report) = build(&lines, &[(10, "異常"), (20, "気象"), (30, "以上")]);
    let out = tmp.path().join("out");
    // 異常気象 は当たる（lemma_rank の約 +2600 に -8000）
    let boosted = penalty(&out, "いじょうきしょう", "異常気象").unwrap();
    assert!(boosted < -2000, "{boosted}");
    assert_eq!(report.policy.compound_by_parts, 1);
    assert!(penalty(&out, "いじょうきしょう", "異常天気").unwrap() > 0);
    assert!(penalty(&out, "いじょうかんぱ", "異常寒波").unwrap() > 0);

    // 同じ読みに chiVe にある漢字の名詞（以上気象 40 位）があれば当てない（同音の守り）
    let mut guarded = lines.to_vec();
    guarded.push(row("以上気象", 9000, NOUN, "イジョウキショウ"));
    let (tmp, report) = build(
        &guarded,
        &[(10, "異常"), (20, "気象"), (30, "以上"), (40, "以上気象")],
    );
    assert!(penalty(&tmp.path().join("out"), "いじょうきしょう", "異常気象").unwrap() > 0);
    assert_eq!(report.policy.compound_by_parts, 0);
}

#[test]
fn the_most_frequent_homophone_with_a_default_cost_is_lowered_below_the_rarer_one() {
    let lines = [
        // 看守（一般の既定 7401）と 看取（サ変可能の既定 6519）
        row("看守", 7401, NOUN, "カンシュ"),
        row("看取", 6519, SAHEN, "カンシュ"),
        // 帯の外の同音語（Sudachi が高くした語）は比べない
        row("看手", 8691, NOUN, "カンシュ"),
    ];
    // 看守 10 位、看取 500 位（0.15 桁以上まれ）、看手 語彙になし
    let (tmp, _) = build(&lines, &[(10, "看守"), (500, "看取")]);
    let out = tmp.path().join("out");
    // 看取 の総コスト 6519（lemma_rank 0）− 600 = 5919 まで 看守 を下げる
    assert_eq!(penalty(&out, "かんしゅ", "看取"), Some(0));
    assert_eq!(penalty(&out, "かんしゅ", "看守"), Some(5919 - 7401));
    assert!(penalty(&out, "かんしゅ", "看手").unwrap() > 0);

    // 頻度の差が 0.15 桁に足りなければ何もしない（看取 13 位: log10(14) − log10(11) ≈ 0.10）
    let (tmp, _) = build(&lines, &[(10, "看守"), (13, "看取")]);
    assert_eq!(
        penalty(&tmp.path().join("out"), "かんしゅ", "看守"),
        Some(0)
    );
}
