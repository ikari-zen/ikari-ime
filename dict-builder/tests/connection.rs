//! 連接表の補正（ADR 0054）の結合テスト。小さい合成の辞書で、補正が 1 行 x 1 マスに閉じることを確かめる。

use std::fs;
use std::path::{Path, PathBuf};

use ikari_dict_builder::{BuildConfig, BuildError, BuildOptions, Policy, build_with};
use ikari_dict_format::{
    GRAMMAR_FILE_NAME, SYSTEM_DICT_FILE_NAME, index_get, matrix_cost, parse_dict_layout,
    parse_grammar_layout, read_raw_entry, read_surface, unpack_index_value,
};

const NOUN: &str = "名詞,普通名詞,一般,*,*";
const COUNTER_NOUN: &str = "名詞,普通名詞,助数詞可能,*,*";
const CASE_PARTICLE: &str = "助詞,格助詞,*,*,*";
const NOMINAL_PARTICLE: &str = "助詞,準体助詞,*,*,*";
const RENTAI: &str = "連体詞,*,*,*,*";

// ID の割り当て（cost(a, b) = a * 10 + b の連接表）
const SIZE: u16 = 7;
const NO_ID: u16 = 1;
const PREFECTURE_ID: u16 = 3;
const MATTER_ID: u16 = 4;
const RENTAI_ID: u16 = 5;
const OTHER_ID: u16 = 6;
const BOS_EOS: u16 = 0;
const NEW_LEFT: u16 = SIZE;
const MATTER_COST: i32 = 100;
const PREFECTURE_COST: i32 = 10;

fn row(surface: &str, left: u16, right: u16, cost: i32, pos: &str, reading: &str) -> String {
    format!("{surface},{left},{right},{cost},{surface},{pos},*,{reading},{surface},*,A,*,*,*,*")
}

fn matrix_text() -> String {
    let mut s = format!("{SIZE} {SIZE}\n");
    for a in 0..SIZE {
        for b in 0..SIZE {
            s.push_str(&format!("{a} {b} {}\n", i32::from(a) * 10 + i32::from(b)));
        }
    }
    s
}

/// 閾値 T = (の → 件 → EOS) − (の → 県 → EOS)
fn threshold(prev_right: u16) -> i32 {
    let path = |left: u16, right: u16, cost: i32| {
        i32::from(prev_right) * 10
            + i32::from(left)
            + cost
            + i32::from(right) * 10
            + i32::from(BOS_EOS)
    };
    path(MATTER_ID, MATTER_ID, MATTER_COST) - path(PREFECTURE_ID, PREFECTURE_ID, PREFECTURE_COST)
}

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn build(
        &self,
        name: &str,
        extra_rows: &[String],
        policy: &str,
        allow_weak: bool,
    ) -> Result<PathBuf, BuildError> {
        let mut rows = vec![
            row("の", NO_ID, NO_ID, 50, CASE_PARTICLE, "ノ"),
            row(
                "県",
                PREFECTURE_ID,
                PREFECTURE_ID,
                PREFECTURE_COST,
                NOUN,
                "ケン",
            ),
            row(
                "件",
                MATTER_ID,
                MATTER_ID,
                MATTER_COST,
                COUNTER_NOUN,
                "ケン",
            ),
            row("この", RENTAI_ID, RENTAI_ID, 30, RENTAI, "コノ"),
            row("語", OTHER_ID, OTHER_ID, 70, NOUN, "ゴ"),
        ];
        rows.extend_from_slice(extra_rows);
        let lex = self.dir.path().join(format!("{name}.csv"));
        fs::write(&lex, rows.join("\n") + "\n").unwrap();
        let matrix = self.dir.path().join("matrix.def");
        fs::write(&matrix, matrix_text()).unwrap();
        let out = self.dir.path().join(name);
        let cfg = BuildConfig {
            lex: vec![lex],
            matrix,
            out_dir: out.clone(),
            unk_pos: format!("{NOUN},*"),
            unk_cost: 10000,
            label: "test".to_string(),
        };
        let opts = BuildOptions {
            policy: Policy::parse(policy).unwrap(),
            allow_weak_connection: allow_weak,
            ..BuildOptions::default()
        };
        build_with(&cfg, &opts).map(|_| out)
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

    fn sizes(&self) -> (u16, u16) {
        let g = parse_grammar_layout(&self.grammar).unwrap();
        (g.prev_right_size, g.next_left_size)
    }

    fn cost(&self, prev_right: u16, next_left: u16) -> i16 {
        let g = parse_grammar_layout(&self.grammar).unwrap();
        matrix_cost(
            &self.grammar[g.matrix.clone()],
            g.prev_right_size,
            g.next_left_size,
            prev_right,
            next_left,
        )
    }

    /// (表記, left, right) を読みで引く
    fn lookup(&self, reading: &str) -> Vec<(String, u16, u16)> {
        let d = parse_dict_layout(&self.dict).unwrap();
        let Some(v) = index_get(&self.dict[d.index.clone()], reading).unwrap() else {
            return Vec::new();
        };
        let (first, count) = unpack_index_value(v);
        (first..first + count)
            .map(|i| {
                let e = read_raw_entry(&self.dict[d.entries.clone()], i).unwrap();
                let surface = read_surface(&self.dict[d.strings.clone()], &e).unwrap();
                (surface.to_string(), e.left_id, e.right_id)
            })
            .collect()
    }

    fn matter(&self) -> (u16, u16) {
        let rows = self.lookup("けん");
        let (_, left, right) = rows.iter().find(|(s, _, _)| s == "件").unwrap();
        (*left, *right)
    }
}

fn is_config_error(result: &Result<PathBuf, BuildError>) -> bool {
    matches!(result, Err(BuildError::Config(m)) if m.contains("ADR 0054"))
}

#[test]
fn zero_delta_changes_nothing() {
    let fx = Fixture::new();
    let base = Built::read(&fx.build("base", &[], "", false).unwrap());
    let zero = Built::read(
        &fx.build(
            "zero",
            &[],
            "matter_ken_after_no\t0\nmatter_ken_after_rentai\t0\n",
            false,
        )
        .unwrap(),
    );
    assert_eq!(zero.sizes(), (SIZE, SIZE));
    assert_eq!(base.grammar, zero.grammar);
    assert_eq!(base.matter(), (MATTER_ID, MATTER_ID));
}

#[test]
fn after_no_lowers_exactly_one_cell_of_a_copied_column() {
    let fx = Fixture::new();
    let delta = threshold(NO_ID) + 100;
    let base = Built::read(&fx.build("base", &[], "", false).unwrap());
    let built = Built::read(
        &fx.build("no", &[], &format!("matter_ken_after_no\t{delta}\n"), false)
            .unwrap(),
    );
    assert_eq!(built.sizes(), (SIZE, SIZE + 1));
    // 件の行だけが新しい左 ID を持ち、右 ID は変わらない
    assert_eq!(built.matter(), (NEW_LEFT, MATTER_ID));
    assert_eq!(
        built.lookup("けん").iter().find(|r| r.0 == "県"),
        Some(&("県".to_string(), PREFECTURE_ID, PREFECTURE_ID))
    );
    // 元の表はそのまま
    for a in 0..SIZE {
        for b in 0..SIZE {
            assert_eq!(built.cost(a, b), base.cost(a, b), "cost({a}, {b})");
        }
    }
    // 新しい列は元の件の列の写しで、の → 件 の 1 マスだけが δ 下がる
    for a in 0..SIZE {
        let expected = base.cost(a, MATTER_ID) - if a == NO_ID { delta as i16 } else { 0 };
        assert_eq!(built.cost(a, NEW_LEFT), expected, "cost({a}, new)");
    }
}

#[test]
fn delta_at_or_below_threshold_fails_unless_allowed() {
    let fx = Fixture::new();
    let t = threshold(NO_ID);
    let policy = format!("matter_ken_after_no\t{t}\n");
    assert!(is_config_error(&fx.build("weak", &[], &policy, false)));
    let ok = Built::read(&fx.build("weak-ok", &[], &policy, true).unwrap());
    assert_eq!(ok.sizes(), (SIZE, SIZE + 1));
    let above = format!("matter_ken_after_no\t{}\n", t + 1);
    assert!(fx.build("above", &[], &above, false).is_ok());
}

#[test]
fn matter_row_must_be_exactly_one() {
    let fx = Fixture::new();
    let policy = "matter_ken_after_no\t500\n";
    // 0 行（件の行がない）
    let lex_without = Fixture::new();
    let lex = lex_without.dir.path().join("lex.csv");
    let body = [
        row("の", NO_ID, NO_ID, 50, CASE_PARTICLE, "ノ"),
        row(
            "県",
            PREFECTURE_ID,
            PREFECTURE_ID,
            PREFECTURE_COST,
            NOUN,
            "ケン",
        ),
        row("語", OTHER_ID, OTHER_ID, 70, NOUN, "ゴ"),
    ]
    .join("\n");
    fs::write(&lex, body + "\n").unwrap();
    let matrix = lex_without.dir.path().join("matrix.def");
    fs::write(&matrix, matrix_text()).unwrap();
    let cfg = BuildConfig {
        lex: vec![lex],
        matrix,
        out_dir: lex_without.dir.path().join("out"),
        unk_pos: format!("{NOUN},*"),
        unk_cost: 10000,
        label: "test".to_string(),
    };
    let opts = BuildOptions {
        policy: Policy::parse(policy).unwrap(),
        ..BuildOptions::default()
    };
    assert!(matches!(
        build_with(&cfg, &opts),
        Err(BuildError::Config(m)) if m.contains("exactly one row")
    ));
    // 2 行
    let second = row("件", OTHER_ID, MATTER_ID, 200, COUNTER_NOUN, "ケン");
    assert!(is_config_error(&fx.build("two", &[second], policy, false)));
}

#[test]
fn right_id_shared_with_another_word_fails() {
    let fx = Fixture::new();
    let policy = "matter_ken_after_no\t500\n";
    // 準体助詞の「の」ではない別の語が、格助詞「の」の右 ID を使う
    let shared = row("に", NO_ID, NO_ID, 60, NOMINAL_PARTICLE, "ニ");
    assert!(is_config_error(&fx.build(
        "shared",
        &[shared],
        policy,
        false
    )));
    // 表記が「の」で終わる行（題名）は、右 ID が「の」で終わることを表すので許す
    let title = row(
        "ひとつ屋根の下の",
        OTHER_ID,
        NO_ID,
        900,
        NOUN,
        "ヒトツヤネノシタノ",
    );
    assert!(fx.build("title", &[title], policy, false).is_ok());
}

#[test]
fn after_rentai_is_a_separate_cell() {
    let fx = Fixture::new();
    let t_no = threshold(NO_ID);
    let t_rentai = threshold(RENTAI_ID);
    let (d_no, d_rentai) = (t_no + 50, t_rentai + 70);
    let base = Built::read(&fx.build("base", &[], "", false).unwrap());
    let both = Built::read(
        &fx.build(
            "both",
            &[],
            &format!("matter_ken_after_no\t{d_no}\nmatter_ken_after_rentai\t{d_rentai}\n"),
            false,
        )
        .unwrap(),
    );
    assert_eq!(
        both.cost(NO_ID, NEW_LEFT),
        base.cost(NO_ID, MATTER_ID) - d_no as i16
    );
    assert_eq!(
        both.cost(RENTAI_ID, NEW_LEFT),
        base.cost(RENTAI_ID, MATTER_ID) - d_rentai as i16
    );
    assert_eq!(
        both.cost(OTHER_ID, NEW_LEFT),
        base.cost(OTHER_ID, MATTER_ID)
    );
    // 連体詞だけ
    let only = Built::read(
        &fx.build(
            "rentai",
            &[],
            &format!("matter_ken_after_rentai\t{d_rentai}\n"),
            false,
        )
        .unwrap(),
    );
    assert_eq!(only.cost(NO_ID, NEW_LEFT), base.cost(NO_ID, MATTER_ID));
    assert_eq!(
        only.cost(RENTAI_ID, NEW_LEFT),
        base.cost(RENTAI_ID, MATTER_ID) - d_rentai as i16
    );
}

#[test]
fn negative_or_unknown_values_are_rejected_by_the_policy() {
    assert!(Policy::parse("matter_ken_after_no\t-1\n").is_err());
    assert!(Policy::parse("matter_ken_after_no\tx\n").is_err());
    assert!(Policy::parse("matter_ken_after_no\t1\nmatter_ken_after_no\t2\n").is_err());
    assert_eq!(
        Policy::parse("matter_ken_after_no\t9000\n")
            .unwrap()
            .connection()
            .matter_ken_after_no,
        9000
    );
    // 0 は書かないのと同じ（policy= の値が変わらない）
    assert_eq!(
        Policy::parse("matter_ken_after_no\t0\n")
            .unwrap()
            .fingerprint(),
        Policy::parse("").unwrap().fingerprint()
    );
}
