//! Full の notcore を辞書の入力に足すビルドの結合テスト（ADR 0012 §11.3）。
//!
//! 守ること: 品詞の統計と活用規則は Core の行だけで数える（grammar.bin が Core だけのときと同じ）、
//! 固有名詞は d2s で絞る、新語の `lemma_rank` を外して `extension_peer` を足す、分解のベクトルを付ける。

use std::fs;
use std::path::{Path, PathBuf};

use ikari_dict_builder::{
    BuildConfig, BuildOptions, ChiveVocab, Policy, SKIP_NOTCORE_PROPER_HOMOPHONE,
    SKIP_NOTCORE_PROPER_SHORT, SKIP_NOTCORE_UNKNOWN_POS, VectorsConfig, build_vectors, build_with,
    verify, verify_vectors,
};
use ikari_dict_format::{
    GRAMMAR_FILE_NAME, SYSTEM_DICT_FILE_NAME, VECTORS_FILE_NAME, index_get, parse_dict_layout,
    parse_vectors_layout, read_lemma, read_lemma_row, read_raw_entry, read_surface,
    unpack_index_value,
};

const NOUN: &str = "名詞,普通名詞,一般,*,*,*";
const PROPER: &str = "名詞,固有名詞,一般,*,*,*";
const VERB_END: &str = "動詞,一般,*,*,五段-ラ行,終止形-一般";
const VERB_PAST: &str = "動詞,一般,*,*,五段-ラ行,連用形-促音便";
const MATRIX: &str = "3 3\n0 0 0\n0 1 0\n0 2 0\n1 0 0\n1 1 0\n1 2 0\n2 0 0\n2 1 0\n2 2 0\n";

/// 19 列の行。`pos` は品詞1〜活用形の 6 列をカンマで結んだもの。`splits` は (A 単位, B 単位)
fn line(
    surface: &str,
    pos: &str,
    reading: &str,
    lemma: &str,
    dict_form: &str,
    splits: (&str, &str),
    cost: i32,
) -> String {
    format!(
        "{surface},1,1,{cost},{surface},{pos},{reading},{lemma},{dict_form},A,{},{},*,*",
        splits.0, splits.1
    )
}

fn simple(surface: &str, cost: i32, pos: &str, reading: &str) -> String {
    line(surface, pos, reading, surface, "*", ("*", "*"), cost)
}

fn write(dir: &Path, name: &str, lines: &[String]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    path
}

fn config(dir: &Path, lex: Vec<PathBuf>, out: &str) -> BuildConfig {
    let matrix = dir.join("matrix.def");
    fs::write(&matrix, MATRIX).unwrap();
    BuildConfig {
        lex,
        matrix,
        out_dir: dir.join(out),
        unk_pos: NOUN.to_string(),
        unk_cost: 10000,
        label: "test".to_string(),
    }
}

/// Core: 名詞 3 行（固有名詞を含む）と、動詞の活用 2 組（行番号 0..=6）
fn core_lines() -> Vec<String> {
    let mut lines = vec![
        simple("体制", 5712, NOUN, "タイセイ"),
        simple("台風", 5000, NOUN, "タイフウ"),
        simple("大阪", 4000, PROPER, "オオサカ"),
    ];
    for (id, (base, past, base_reading, past_reading)) in [
        ("取る", "取っ", "トル", "トッ"),
        ("売る", "売っ", "ウル", "ウッ"),
    ]
    .into_iter()
    .enumerate()
    {
        let base_id = (3 + id * 2).to_string();
        lines.push(line(
            base,
            VERB_END,
            base_reading,
            base,
            &base_id,
            ("*", "*"),
            6000,
        ));
        lines.push(line(
            past,
            VERB_PAST,
            past_reading,
            base,
            &base_id,
            ("*", "*"),
            6500,
        ));
    }
    lines
}

fn entries_of(dir: &Path, reading: &str) -> Vec<(String, i16)> {
    let dict = fs::read(dir.join(SYSTEM_DICT_FILE_NAME)).unwrap();
    let l = parse_dict_layout(&dict).unwrap();
    let Some(v) = index_get(&dict[l.index.clone()], reading).unwrap() else {
        return Vec::new();
    };
    let (first, count) = unpack_index_value(v);
    (first..first + count)
        .map(|i| {
            let e = read_raw_entry(&dict[l.entries.clone()], i).unwrap();
            let s = read_surface(&dict[l.strings.clone()], &e).unwrap();
            (s.to_string(), e.ime_penalty)
        })
        .collect()
}

fn penalty(dir: &Path, reading: &str, surface: &str) -> Option<i16> {
    entries_of(dir, reading)
        .into_iter()
        .find(|(s, _)| s == surface)
        .map(|(_, p)| p)
}

#[test]
fn notcore_rows_do_not_change_grammar_bin() {
    let tmp = tempfile::tempdir().unwrap();
    let core = write(tmp.path(), "core.csv", &core_lines());
    // notcore: 安い名詞、極端なコストの動詞の活用、Core にない品詞の行、固有名詞
    let notcore = write(
        tmp.path(),
        "notcore.csv",
        &[
            simple("沼", 1, NOUN, "ヌマ"),
            line("沼る", VERB_END, "ヌマル", "沼る", "8", ("*", "*"), 100),
            line("沼っ", VERB_PAST, "ヌマッ", "沼る", "8", ("*", "*"), 200),
            simple("珍", 5, "感動詞,フィラー,*,*,*,*", "チン"),
            simple("横浜山手", 3, PROPER, "ヨコハマヤマテ"),
        ],
    );
    let cfg_core = config(tmp.path(), vec![core.clone()], "core-only");
    let core_report = build_with(&cfg_core, &BuildOptions::default()).unwrap();
    let cfg_full = config(tmp.path(), vec![core], "full");
    let opts = BuildOptions {
        notcore_lex: vec![notcore],
        ..BuildOptions::default()
    };
    let report = build_with(&cfg_full, &opts).unwrap();

    assert_eq!(
        fs::read(tmp.path().join("core-only").join(GRAMMAR_FILE_NAME)).unwrap(),
        fs::read(tmp.path().join("full").join(GRAMMAR_FILE_NAME)).unwrap(),
        "grammar.bin must be byte-identical to the Core-only build"
    );
    // Core にない品詞の行は捨てる。残りは辞書に入る
    assert_eq!(report.skipped.get(SKIP_NOTCORE_UNKNOWN_POS), Some(&1));
    assert_eq!(report.notcore_entries, 4);
    assert_eq!(report.entries, core_lines().len() + 4);
    assert_eq!(report.rows_read, core_lines().len() + 5);
    assert_eq!(entries_of(&tmp.path().join("full"), "ぬまる").len(), 1);
    verify(&tmp.path().join("full")).unwrap();
    // 活用規則は Core の行だけで導く（notcore の活用形は数えない）
    assert_eq!(report.conj_rules, core_report.conj_rules);
    assert_eq!(report.conj_stats, core_report.conj_stats);
}

#[test]
fn proper_nouns_are_filtered_by_d2s() {
    let tmp = tempfile::tempdir().unwrap();
    let core = write(tmp.path(), "core.csv", &core_lines());
    let notcore = write(
        tmp.path(),
        "notcore.csv",
        &[
            // 同じ読みに固有名詞以外の語（Core の 体制）がある → 外す
            simple("大西", 3000, PROPER, "タイセイ"),
            // 同じ読みに notcore の普通名詞がある → 外す
            simple("鯱", 3000, PROPER, "シャチ"),
            simple("シャチ", 3000, NOUN, "シャチ"),
            // 同じ読みに動詞の活用形（取っ）がある → 外す
            simple("トッ", 3000, PROPER, "トッ"),
            // 読みが 2 文字以下は外す。3 文字は残す
            simple("桃", 3000, PROPER, "モモ"),
            simple("桃太", 3000, PROPER, "モモタ"),
            // 表記が ASCII の名前は、読み 3 文字以下を外す。4 文字は残す
            simple("KAZ", 3000, PROPER, "カズキ"),
            simple("KAZE", 3000, PROPER, "カゼノキ"),
            // 固有名詞どうしだけの同じ読みは残す（Core の 大阪 とも）
            simple("大坂", 3000, PROPER, "オオサカ"),
            simple("大谷翔平", 3000, PROPER, "オオタニショウヘイ"),
            simple("沼る", 3000, NOUN, "ヌマル"),
        ],
    );
    let opts = BuildOptions {
        notcore_lex: vec![notcore],
        ..BuildOptions::default()
    };
    let report = build_with(&config(tmp.path(), vec![core], "out"), &opts).unwrap();
    let out = tmp.path().join("out");
    assert_eq!(report.skipped.get(SKIP_NOTCORE_PROPER_HOMOPHONE), Some(&3));
    assert_eq!(report.skipped.get(SKIP_NOTCORE_PROPER_SHORT), Some(&2));
    let kept = |reading: &str, surface: &str| penalty(&out, reading, surface).is_some();
    assert!(!kept("たいせい", "大西"));
    assert!(!kept("しゃち", "鯱") && kept("しゃち", "シャチ"));
    assert!(!kept("とっ", "トッ"));
    assert!(!kept("もも", "桃") && kept("ももた", "桃太"));
    assert!(!kept("かずき", "KAZ") && kept("かぜのき", "KAZE"));
    assert!(kept("おおさか", "大坂") && kept("おおさか", "大阪"));
    assert!(kept("おおたにしょうへい", "大谷翔平"));
    assert!(kept("ぬまる", "沼る"));
    // 外した行も行番号には数える（Sudachi の分割の ID が参照する）
    assert_eq!(report.rows_read, core_lines().len() + 11);
    assert_eq!(report.notcore_entries, 6);
}

#[test]
fn new_words_skip_lemma_rank_and_get_extension_peer_only_beside_common_core_words() {
    let tmp = tempfile::tempdir().unwrap();
    let core = write(tmp.path(), "core.csv", &core_lines());
    let notcore = write(
        tmp.path(),
        "notcore.csv",
        &[
            // 同じ読みの Core の語（体制）が chiVe の上位にある新語 → extension_peer
            simple("タイ製", 4000, NOUN, "タイセイ"),
            // 同じ読みの Core の語がない新語 → 何も足さない（lemma_rank も当てない）
            simple("沼", 4000, NOUN, "ヌマ"),
            // lemma が chiVe にある notcore の語 → lemma_rank は Core の語と同じ。同じ読みの Core の語（台風）があるので extension_peer
            line("颱風", NOUN, "タイフウ", "台風", "*", ("*", "*"), 4000),
            // 固有名詞は chiVe になくても lemma_rank を当てる（Core の固有名詞と同じ）
            simple("沼田", 4000, PROPER, "ヌマタ"),
            // 同じ読みの Core の語が chiVe の 3 万位より下 → extension_peer は足さない
            simple("売", 4000, NOUN, "ウル"),
        ],
    );
    // chiVe の語彙: 体制（0 位）、台風、ダミー 6 万語、売る（6 万位より下）
    let mut keys = vec!["体制".to_string(), "台風".to_string()];
    keys.extend((0..60_000).map(|i| format!("ダミー{i}")));
    keys.push("売る".to_string());
    let vocab_size = keys.len();
    let policy = Policy::parse("lemma_rank\t2000\nextension_peer\t3000\n").unwrap();
    let opts = BuildOptions {
        policy,
        chive_vocab: Some(ChiveVocab::from_keys(keys)),
        chive_vocab_label: Some("test".into()),
        notcore_lex: vec![notcore],
        chain_lex: Vec::new(),
    };
    let report = build_with(&config(tmp.path(), vec![core], "out"), &opts).unwrap();
    let out = tmp.path().join("out");

    assert_eq!(penalty(&out, "たいせい", "体制"), Some(0));
    assert_eq!(penalty(&out, "たいせい", "タイ製"), Some(3000));
    assert_eq!(penalty(&out, "ぬま", "沼"), Some(0));
    assert_eq!(penalty(&out, "たいふう", "颱風"), Some(3000));
    // 固有名詞: lemma_rank（語彙にない → 語彙の最後より後ろ。log10(60004) - 3 = 1.78 桁 × 2000）
    let proper = penalty(&out, "ぬまた", "沼田").unwrap();
    assert!((3500..3600).contains(&proper), "{proper}");
    // 売る は 6 万位より下 → extension_peer なし。売 は chiVe にない新語なので lemma_rank もなし
    assert_eq!(penalty(&out, "うる", "売"), Some(0));

    let hits = report.policy.hits_by_predicate();
    assert_eq!(hits["extension_peer"], 2);
    assert_eq!(report.policy.lemma_rank_exempt, 3); // タイ製、沼、売
    let md = report.policy.to_markdown(&opts.policy, Some(vocab_size));
    assert!(md.contains("| `extension_peer` | +3000 | 2 |"), "{md}");
    assert!(md.contains("`lemma_rank` を当てなかった行: 3"), "{md}");
    assert!(report.notcore_markdown().contains("品詞の統計"));
}

#[test]
fn core_only_build_is_unaffected_by_the_notcore_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let core = write(tmp.path(), "core.csv", &core_lines());
    let policy = Policy::parse("lemma_rank\t2000\nextension_peer\t3000\n").unwrap();
    let opts = BuildOptions {
        policy,
        chive_vocab: Some(ChiveVocab::from_keys(["体制".to_string()])),
        ..BuildOptions::default()
    };
    let report = build_with(&config(tmp.path(), vec![core], "out"), &opts).unwrap();
    assert_eq!(report.policy.hits_by_predicate()["extension_peer"], 0);
    assert_eq!(report.policy.lemma_rank_exempt, 0);
    assert_eq!(report.notcore_entries, 0);
    assert_eq!(report.notcore_markdown(), "");
}

#[test]
fn decomposition_adds_mean_vectors_for_lemmas_missing_from_chive() {
    let tmp = tempfile::tempdir().unwrap();
    // 行番号: 0 コイン 1 ロッカー 2 コインロッカー | 3 沼る 4 ガ 5 チャ 6 親 7 猫 8 ガチャ 9 親ガチャ 10 猫ロッカー
    let core = write(
        tmp.path(),
        "core.csv",
        &[
            simple("コイン", 5000, NOUN, "コイン"),
            simple("ロッカー", 5000, NOUN, "ロッカー"),
            line(
                "コインロッカー",
                NOUN,
                "コインロッカー",
                "コインロッカー",
                "*",
                ("0/1", "0/1"),
                6000,
            ),
        ],
    );
    let notcore = write(
        tmp.path(),
        "notcore.csv",
        &[
            // 分割のない新語には付かない
            simple("沼る", 6000, NOUN, "ヌマル"),
            simple("ガ", 5000, NOUN, "ガ"),
            simple("チャ", 5000, NOUN, "チャ"),
            simple("親", 5000, NOUN, "オヤ"),
            simple("猫", 5000, NOUN, "ネコ"),
            // A 分割だけ
            line("ガチャ", NOUN, "ガチャ", "ガチャ", "*", ("4/5", "*"), 6000),
            // 部分の ガチャ は chiVe になく、1 段下の ガ + チャ をたどる
            line(
                "親ガチャ",
                NOUN,
                "オヤガチャ",
                "親ガチャ",
                "*",
                ("6/8", "*"),
                6000,
            ),
            // 部分の 猫 は chiVe になく、分割もない → 付けない
            line(
                "猫ロッカー",
                NOUN,
                "ネコロッカー",
                "猫ロッカー",
                "*",
                ("7/1", "*"),
                6000,
            ),
        ],
    );
    // 辞書は行を読みで並べるので、ここでの行番号は入力のファイルでの順
    let opts = BuildOptions {
        notcore_lex: vec![notcore.clone()],
        ..BuildOptions::default()
    };
    let dict_dir = tmp.path().join("out");
    build_with(&config(tmp.path(), vec![core.clone()], "out"), &opts).unwrap();

    // chiVe（2 次元）: コイン (1,0)、ロッカー (0,1)、ガ (1,0)、チャ (0,1)、親 (1,1)
    let chive = tmp.path().join("chive.txt");
    fs::write(
        &chive,
        "5 2\nコイン 1 0\nロッカー 0 1\nガ 1 0\nチャ 0 1\n親 1 1\n",
    )
    .unwrap();
    let out = dict_dir.join(VECTORS_FILE_NAME);
    let mut cfg = VectorsConfig::new(chive, dict_dir.clone(), out.clone());
    cfg.clusters = 2;
    cfg.lex = vec![core, notcore];
    let report = build_vectors(&cfg).unwrap();
    // chiVe に完全一致する 5 語 + 分解 3 語（コインロッカー、ガチャ、親ガチャ）
    assert_eq!(report.matched, 5, "{report}");
    assert_eq!(report.decomposed, 3, "{report}");
    verify_vectors(&dict_dir).unwrap();

    let dict = fs::read(dict_dir.join(SYSTEM_DICT_FILE_NAME)).unwrap();
    let dl = parse_dict_layout(&dict).unwrap();
    let vectors = fs::read(&out).unwrap();
    let vl = parse_vectors_layout(&vectors).unwrap();
    let lemma_table = &dict[dl.lemma_table.clone()];
    let vector_of = |lemma: &str| -> Option<Vec<i8>> {
        let id = (0..dl.lemma_count)
            .find(|&i| read_lemma(lemma_table, dl.lemma_count, i) == Some(lemma))?;
        let row = read_lemma_row(&vectors[vl.lemma_rows.clone()], id)? as usize;
        let dims = usize::from(vl.dims);
        let bytes = &vectors[vl.rows.clone()][row * dims..(row + 1) * dims];
        Some(bytes.iter().map(|&b| b as i8).collect())
    };
    // どれも (1,1) 方向の単位ベクトル × 127 = 90, 90
    assert_eq!(vector_of("コインロッカー"), Some(vec![90, 90]));
    assert_eq!(vector_of("ガチャ"), Some(vec![90, 90]));
    assert_eq!(vector_of("親ガチャ"), Some(vec![90, 90]));
    // コイン は chiVe のベクトルのまま
    assert_eq!(vector_of("コイン"), Some(vec![127, 0]));
    // 分割のない語、部分が欠ける語には付かない
    assert_eq!(vector_of("沼る"), None);
    assert_eq!(vector_of("猫ロッカー"), None);

    // 分割を渡さなければ分解しない
    cfg.lex = Vec::new();
    let plain = build_vectors(&cfg).unwrap();
    assert_eq!((plain.matched, plain.decomposed), (5, 0));
}
