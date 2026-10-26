//! `ikari_dict_builder::build` の結合テスト（インラインの CSV と fixture）。ADR 0001 §10.1 B。

use std::fs;
use std::path::{Path, PathBuf};

use ikari_dict_builder::{
    BuildConfig, BuildError, SKIP_DUPLICATE, SKIP_LEFT_ID_MINUS1, SKIP_READING_NOT_INDEXABLE,
    SKIP_READING_TOO_LONG, SKIP_SYMBOL_POS, build, verify,
};
use ikari_dict_format::{
    GRAMMAR_FILE_NAME, SYSTEM_DICT_FILE_NAME, index_get, matrix_cost, parse_dict_layout,
    parse_grammar_layout, read_pos_record, read_raw_entry, read_surface, unpack_index_value,
};

const NOUN: &str = "名詞,普通名詞,一般,*,*";
const SYMBOL: &str = "補助記号,読点,*,*,*";

/// 19列の Sudachi 形式の行
fn row(surface: &str, left: i32, right: i32, cost: i32, pos: &str, reading: &str) -> String {
    format!("{surface},{left},{right},{cost},{surface},{pos},*,{reading},{surface},*,A,*,*,*,*")
}

/// `a b cost` の全組み合わせ。cost = a * 10 + b（転置すると値が変わる）
fn matrix_text(prev_right_size: u16, next_left_size: u16) -> String {
    let mut s = format!("{prev_right_size} {next_left_size}\n");
    for a in 0..prev_right_size {
        for b in 0..next_left_size {
            s.push_str(&format!("{a} {b} {}\n", i32::from(a) * 10 + i32::from(b)));
        }
    }
    s
}

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn write(&self, name: &str, content: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        path
    }

    fn out(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn config(&self, lex: Vec<PathBuf>, matrix: PathBuf, out: &str) -> BuildConfig {
        BuildConfig {
            lex,
            matrix,
            out_dir: self.out(out),
            unk_pos: format!("{NOUN},*"),
            unk_cost: 10000,
            label: "test".to_string(),
        }
    }
}

struct Built {
    grammar: Vec<u8>,
    dict: Vec<u8>,
}

impl Built {
    fn read(dir: &Path) -> Self {
        Self {
            grammar: fs::read(dir.join(GRAMMAR_FILE_NAME)).unwrap(),
            dict: fs::read(dir.join(SYSTEM_DICT_FILE_NAME)).unwrap(),
        }
    }

    /// (表記, left, right, cost, 品詞名)
    fn lookup(&self, reading: &str) -> Vec<(String, u16, u16, i16, String)> {
        let g = parse_grammar_layout(&self.grammar).unwrap();
        let d = parse_dict_layout(&self.dict).unwrap();
        let Some(v) = index_get(&self.dict[d.index.clone()], reading).unwrap() else {
            return Vec::new();
        };
        let (first, count) = unpack_index_value(v);
        (first..first + count)
            .map(|i| {
                let e = read_raw_entry(&self.dict[d.entries.clone()], i).unwrap();
                let surface = read_surface(&self.dict[d.strings.clone()], &e).unwrap();
                let pos = read_pos_record(&self.grammar[g.pos.clone()], g.pos_count, e.pos_id)
                    .unwrap()
                    .name
                    .to_string();
                (surface.to_string(), e.left_id, e.right_id, e.cost, pos)
            })
            .collect()
    }
}

#[test]
fn non_square_matrix_survives_build_without_transposition() {
    let env = Env::new();
    // prev_right_size=2, next_left_size=3: left は 0..3、right は 0..2
    let matrix = env.write("matrix.def", &matrix_text(2, 3));
    let lex = env.write("lex.csv", &(row("語", 2, 1, 100, NOUN, "ゴ") + "\n"));
    let report = build(&env.config(vec![lex], matrix, "out")).unwrap();
    assert_eq!(report.entries, 1);

    let built = Built::read(&env.out("out"));
    let g = parse_grammar_layout(&built.grammar).unwrap();
    assert_eq!((g.prev_right_size, g.next_left_size), (2, 3));
    let m = &built.grammar[g.matrix.clone()];
    for prev_right in 0..2u16 {
        for next_left in 0..3u16 {
            assert_eq!(
                matrix_cost(m, 2, 3, prev_right, next_left),
                (prev_right * 10 + next_left) as i16,
                "cost({prev_right} -> {next_left})"
            );
        }
    }
    // 範囲外は i16::MAX
    assert_eq!(matrix_cost(m, 2, 3, 2, 0), i16::MAX);
    assert_eq!(matrix_cost(m, 2, 3, 0, 3), i16::MAX);
}

#[test]
fn lex_ids_are_checked_against_the_matrix_ranges() {
    let env = Env::new();
    let matrix = env.write("matrix.def", &matrix_text(2, 3));
    // right=2 は prev_right_size=2 の範囲外（left=2 は next_left_size=3 の範囲内）
    let lex = env.write("lex.csv", &(row("語", 2, 2, 100, NOUN, "ゴ") + "\n"));
    let err = build(&env.config(vec![lex], matrix, "out")).unwrap_err();
    assert!(matches!(err, BuildError::Input { line: 1, .. }), "{err}");
    assert!(!env.out("out").join(GRAMMAR_FILE_NAME).exists());
}

#[test]
fn all_filters_end_to_end() {
    let env = Env::new();
    let matrix = env.write("matrix.def", &matrix_text(4, 4));
    let long = "ア".repeat(33);
    let lines = [
        row("品詞", 1, 1, 7392, NOUN, "ヒンシ"),
        row("瀕死", 1, 1, 7403, NOUN, "ヒンシ"),
        row("瀕死", 1, 1, 9999, NOUN, "ヒンシ"), // 重複: 小さい方を残す
        row("分割用", -1, -1, 0, NOUN, "ブンカツ"),
        row("、", 2, 2, -2049, SYMBOL, "、"),
        row("北斗", 1, 1, 100, NOUN, "ホ・ク＝ト"), // 区切りを除去
        row("1\\u002c2塁", 1, 1, 200, NOUN, "イチニルイ"), // 表記の \uXXXX
        row("profile", 1, 1, 300, NOUN, "profile"),
        row("長い", 1, 1, 300, NOUN, &long),
        row("カナ", 3, 3, 50, "名詞,固有名詞,一般,*,*", "カタカナ"),
    ];
    let lex = env.write("lex.csv", &(lines.join("\n") + "\n"));
    let report = build(&env.config(vec![lex], matrix, "out")).unwrap();

    assert_eq!(report.rows_read, 10);
    assert_eq!(report.skipped.get(SKIP_LEFT_ID_MINUS1), Some(&1));
    assert_eq!(report.skipped.get(SKIP_SYMBOL_POS), Some(&1));
    assert_eq!(report.skipped.get(SKIP_READING_NOT_INDEXABLE), Some(&1));
    assert_eq!(report.skipped.get(SKIP_READING_TOO_LONG), Some(&1));
    assert_eq!(report.skipped.get(SKIP_DUPLICATE), Some(&1));
    assert_eq!(report.entries, 5);
    assert_eq!(report.readings, 4); // ひんし, ほくと, いちにるい, かたかな
    // 品詞は除外した行(分割用は名詞,補助記号は別)も含めて 名詞/補助記号/固有名詞 の3つ
    assert_eq!(report.pos_count, 3);

    let built = Built::read(&env.out("out"));
    let hits = built.lookup("ひんし");
    assert_eq!(hits.len(), 2);
    assert_eq!((hits[0].0.as_str(), hits[0].3), ("品詞", 7392));
    assert_eq!((hits[1].0.as_str(), hits[1].3), ("瀕死", 7403));
    assert_eq!(hits[0].4, "名詞,普通名詞,一般,*,*,*");
    assert_eq!(built.lookup("ほくと")[0].0, "北斗");
    assert_eq!(built.lookup("いちにるい")[0].0, "1,2塁");
    assert_eq!(built.lookup("かたかな")[0].4, "名詞,固有名詞,一般,*,*,*");
    // カタカナの読みはひらがなで引く。除外した行は引けない
    assert!(built.lookup("ヒンシ").is_empty());
    assert!(built.lookup("ぶんかつ").is_empty());
    assert!(built.lookup("てん").is_empty());
    assert!(built.lookup("profile").is_empty());
}

#[test]
fn unknown_word_parameters_come_from_unk_pos() {
    let env = Env::new();
    let matrix = env.write("matrix.def", &matrix_text(4, 4));
    let lex = env.write(
        "lex.csv",
        &[
            row("a", 2, 3, 1, NOUN, "ア"),
            row("b", 2, 3, 1, NOUN, "イ"),
            row("c", 1, 1, 1, NOUN, "ウ"),
            row("d", 1, 2, 1, "動詞,一般,*,*,*", "エ"),
        ]
        .join("\n"),
    );
    let mut cfg = env.config(vec![lex], matrix, "out");
    cfg.unk_cost = -123;
    let report = build(&cfg).unwrap();
    assert_eq!(report.pos_count, 2);
    let built = Built::read(&env.out("out"));
    let g = parse_grammar_layout(&built.grammar).unwrap();
    // 名詞の代表は最頻の (left=2, right=3)
    assert_eq!((g.unk_left_id, g.unk_right_id), (2, 3));
    assert_eq!(g.unk_cost, -123);
    assert_eq!(g.bos_eos_id, 0);
    let rec = read_pos_record(&built.grammar[g.pos.clone()], g.pos_count, g.unk_pos_id).unwrap();
    assert_eq!(rec.name, "名詞,普通名詞,一般,*,*,*");
    assert_eq!((rec.rep_left_id, rec.rep_right_id), (2, 3));
}

#[test]
fn missing_unk_pos_fails_without_output() {
    let env = Env::new();
    let matrix = env.write("matrix.def", &matrix_text(2, 2));
    let lex = env.write("lex.csv", &(row("a", 1, 1, 1, NOUN, "ア") + "\n"));
    let mut cfg = env.config(vec![lex], matrix, "out");
    cfg.unk_pos = "存在しない,*,*,*,*,*".to_string();
    let err = build(&cfg).unwrap_err();
    assert!(matches!(err, BuildError::Config(_)), "{err}");
    assert!(!env.out("out").exists() || fs::read_dir(env.out("out")).unwrap().next().is_none());
}

#[test]
fn failed_build_keeps_existing_files_and_leaves_no_tmp() {
    let env = Env::new();
    let matrix = env.write("matrix.def", &matrix_text(2, 2));
    let good = env.write("good.csv", &(row("a", 1, 1, 1, NOUN, "ア") + "\n"));
    build(&env.config(vec![good.clone()], matrix.clone(), "out")).unwrap();
    let before = Built::read(&env.out("out"));

    let bad = env.write("bad.csv", "これは,壊れた,行\n");
    let err = build(&env.config(vec![bad], matrix, "out")).unwrap_err();
    assert!(matches!(err, BuildError::Input { line: 1, .. }), "{err}");

    let after = Built::read(&env.out("out"));
    assert_eq!(before.grammar, after.grammar);
    assert_eq!(before.dict, after.dict);
    let names: Vec<String> = fs::read_dir(env.out("out"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
}

#[test]
fn successful_build_leaves_only_final_files() {
    let env = Env::new();
    let matrix = env.write("matrix.def", &matrix_text(2, 2));
    let lex = env.write("lex.csv", &(row("a", 1, 1, 1, NOUN, "ア") + "\n"));
    build(&env.config(vec![lex], matrix, "nested/out")).unwrap();
    let mut names: Vec<String> = fs::read_dir(env.out("nested/out"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec![GRAMMAR_FILE_NAME, SYSTEM_DICT_FILE_NAME]);
}

#[test]
fn build_is_deterministic_and_independent_of_row_order() {
    let env = Env::new();
    let matrix = env.write("matrix.def", &matrix_text(5, 5));
    let mut lines = [
        row("品詞", 1, 1, 7392, NOUN, "ヒンシ"),
        row("瀕死", 2, 2, 7403, NOUN, "ヒンシ"),
        row("瀕し", 3, 3, 7907, "動詞,一般,*,*,*", "ヒンシ"),
        row("の", 4, 4, 3360, "助詞,格助詞,*,*,*", "ノ"),
        row("野", 1, 1, 8901, NOUN, "ノ"),
    ];
    let lex_a = env.write("a/lex.csv", &(lines.join("\n") + "\n"));
    lines.reverse();
    let lex_b = env.write("b/lex.csv", &(lines.join("\n") + "\n"));

    build(&env.config(vec![lex_a.clone()], matrix.clone(), "out1")).unwrap();
    build(&env.config(vec![lex_a], matrix.clone(), "out2")).unwrap();
    build(&env.config(vec![lex_b], matrix, "out3")).unwrap();
    let (o1, o2, o3) = (
        Built::read(&env.out("out1")),
        Built::read(&env.out("out2")),
        Built::read(&env.out("out3")),
    );
    assert_eq!(o1.grammar, o2.grammar);
    assert_eq!(o1.dict, o2.dict);
    assert_eq!(o1.grammar, o3.grammar);
    assert_eq!(o1.dict, o3.dict);
}

#[test]
fn multiple_lex_files_are_merged_and_deduplicated() {
    let env = Env::new();
    let matrix = env.write("matrix.def", &matrix_text(3, 3));
    let small = env.write(
        "small.csv",
        &(row("品詞", 1, 1, 500, NOUN, "ヒンシ") + "\n"),
    );
    let core = env.write(
        "core.csv",
        &(row("品詞", 1, 1, 400, NOUN, "ヒンシ")
            + "\n"
            + &row("瀕死", 2, 2, 600, NOUN, "ヒンシ")
            + "\n"),
    );
    let report = build(&env.config(vec![small, core], matrix, "out")).unwrap();
    assert_eq!(report.rows_read, 3);
    assert_eq!(report.entries, 2);
    assert_eq!(report.skipped.get(SKIP_DUPLICATE), Some(&1));
    let built = Built::read(&env.out("out"));
    let hits = built.lookup("ひんし");
    assert_eq!(
        hits.iter().map(|h| (h.0.as_str(), h.3)).collect::<Vec<_>>(),
        vec![("品詞", 400), ("瀕死", 600)]
    );
}

#[test]
fn no_lex_files_is_a_config_error() {
    let env = Env::new();
    let matrix = env.write("matrix.def", &matrix_text(2, 2));
    let err = build(&env.config(vec![], matrix, "out")).unwrap_err();
    assert!(matches!(err, BuildError::Config(_)));
}

#[test]
fn missing_input_files_are_io_errors() {
    let env = Env::new();
    let matrix = env.write("matrix.def", &matrix_text(2, 2));
    let err = build(&env.config(vec![env.out("nope.csv")], matrix, "out")).unwrap_err();
    assert!(matches!(err, BuildError::Io(_)), "{err}");
    let lex = env.write("lex.csv", &(row("a", 1, 1, 1, NOUN, "ア") + "\n"));
    let err = build(&env.config(vec![lex], env.out("nope.def"), "out")).unwrap_err();
    assert!(matches!(err, BuildError::Io(_)), "{err}");
}

#[test]
fn invalid_utf8_input_is_an_error_with_line_number() {
    let env = Env::new();
    let matrix = env.write("matrix.def", &matrix_text(2, 2));
    let mut bytes = (row("a", 1, 1, 1, NOUN, "ア") + "\n").into_bytes();
    bytes.extend_from_slice(b"\xff\xfe,1,1,1\n");
    let path = env.out("lex.csv");
    fs::write(&path, bytes).unwrap();
    let err = build(&env.config(vec![path], matrix, "out")).unwrap_err();
    assert!(matches!(err, BuildError::Input { line: 2, .. }), "{err}");
}

#[test]
fn verify_accepts_a_fresh_build_and_detects_damage() {
    let env = Env::new();
    let matrix = env.write("matrix.def", &matrix_text(3, 3));
    let lex = env.write(
        "lex.csv",
        &[
            row("品詞", 1, 1, 7392, NOUN, "ヒンシ"),
            row("瀕死", 2, 2, 7403, NOUN, "ヒンシ"),
            row("の", 1, 2, 3360, "助詞,格助詞,*,*,*", "ノ"),
        ]
        .join("\n"),
    );
    let report = build(&env.config(vec![lex], matrix, "out")).unwrap();
    let ok = verify(&env.out("out")).unwrap();
    assert_eq!(
        (ok.entries, ok.readings, ok.pos_count),
        (report.entries, report.readings, report.pos_count)
    );

    // エントリ部の pos_id を範囲外にする
    let dir = env.out("out");
    let dict_path = dir.join(SYSTEM_DICT_FILE_NAME);
    let original = fs::read(&dict_path).unwrap();
    let layout = parse_dict_layout(&original).unwrap();
    let mut damaged = original.clone();
    let pos_id_at = layout.entries.start + 12;
    damaged[pos_id_at..pos_id_at + 2].copy_from_slice(&999u16.to_le_bytes());
    fs::write(&dict_path, &damaged).unwrap();
    assert!(verify(&dir).is_err());
    fs::write(&dict_path, &original).unwrap();
    assert!(verify(&dir).is_ok());

    // 連接表を書き換える -> grammar_id の不一致
    let grammar_path = dir.join(GRAMMAR_FILE_NAME);
    let original_g = fs::read(&grammar_path).unwrap();
    let g = parse_grammar_layout(&original_g).unwrap();
    let mut damaged = original_g.clone();
    damaged[g.matrix.start] ^= 0x7f;
    fs::write(&grammar_path, &damaged).unwrap();
    assert!(verify(&dir).is_err());
    fs::write(&grammar_path, &original_g).unwrap();

    // 切り詰め
    fs::write(&dict_path, &original[..original.len() / 2]).unwrap();
    assert!(verify(&dir).is_err());

    // ファイルがない
    fs::remove_file(&dict_path).unwrap();
    assert!(matches!(verify(&dir), Err(BuildError::Io(_))));
}

#[test]
fn verify_rejects_dict_built_from_another_grammar() {
    let env = Env::new();
    let lex = env.write("lex.csv", &(row("a", 1, 1, 1, NOUN, "ア") + "\n"));
    build(&env.config(
        vec![lex.clone()],
        env.write("m1.def", &matrix_text(2, 2)),
        "one",
    ))
    .unwrap();
    build(&env.config(vec![lex], env.write("m2.def", &matrix_text(3, 3)), "two")).unwrap();
    fs::copy(
        env.out("two").join(GRAMMAR_FILE_NAME),
        env.out("one").join(GRAMMAR_FILE_NAME),
    )
    .unwrap();
    assert!(verify(&env.out("one")).is_err());
}

/// リポジトリ同梱の fixture（testdata/sudachi-mini）でビルドできる。
#[test]
fn fixture_builds_and_contains_hinshi() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../testdata/sudachi-mini");
    let env = Env::new();
    let cfg = BuildConfig {
        lex: vec![root.join("lex.csv")],
        matrix: root.join("matrix.def"),
        out_dir: env.out("fixture"),
        unk_pos: format!("{NOUN},*"),
        unk_cost: 10000,
        label: "sudachi-mini".to_string(),
    };
    let report = build(&cfg).unwrap();
    assert!(report.entries >= 50, "{report}");
    // 除外規則が実際に働く行を fixture に含めている
    assert!(
        report
            .skipped
            .get(SKIP_LEFT_ID_MINUS1)
            .is_some_and(|&n| n >= 1)
    );
    assert!(report.skipped.get(SKIP_SYMBOL_POS).is_some_and(|&n| n >= 2));
    assert!(
        report
            .skipped
            .get(SKIP_READING_NOT_INDEXABLE)
            .is_some_and(|&n| n >= 1)
    );

    let built = Built::read(&env.out("fixture"));
    let hits = built.lookup("ひんし");
    let surfaces: Vec<&str> = hits.iter().map(|h| h.0.as_str()).collect();
    assert!(surfaces.contains(&"品詞"), "{surfaces:?}");
    assert!(surfaces.contains(&"瀕死"), "{surfaces:?}");
    verify(&env.out("fixture")).unwrap();
}
