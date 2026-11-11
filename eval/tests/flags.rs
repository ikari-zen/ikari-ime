//! ADR 0002 §15 の eval のフラグの統合テスト。fixture の辞書（`testdata/sudachi-mini`）で `ikari-eval` を実行し、
//! 終了コードと出力を確かめる。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ikari_dict_builder::{BuildConfig, build};

const UNK_POS: &str = "名詞,普通名詞,一般,*,*,*";
const UNK_COST: i16 = 10_000;
const EXIT_OK: i32 = 0;
const EXIT_GATE_FAILED: i32 = 1;
const EXIT_ERROR: i32 = 2;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("eval/ has a parent")
        .to_path_buf()
}

struct Env {
    root: PathBuf,
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let root = workspace_root();
        let fixture = root.join("testdata/sudachi-mini");
        let dir = tempfile::tempdir().unwrap();
        build(&BuildConfig {
            lex: vec![fixture.join("lex.csv")],
            matrix: fixture.join("matrix.def"),
            out_dir: dir.path().join("dict"),
            unk_pos: UNK_POS.to_owned(),
            unk_cost: UNK_COST,
            label: "sudachi-mini".to_owned(),
        })
        .expect("build fixture dictionary");
        Self { root, dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let p = self.path(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    fn homophones(&self) -> PathBuf {
        self.root.join("eval/sets/homophones.tsv")
    }

    /// 共通の引数（辞書と繰り返し回数）つきで実行する
    fn eval(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ikari-eval"))
            .arg("--dict")
            .arg(self.path("dict"))
            .args(["--repeat", "1"])
            .args(args)
            .output()
            .expect("run ikari-eval")
    }

    /// 辞書を使わないサブコマンド用
    fn raw(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ikari-eval"))
            .args(args)
            .output()
            .expect("run ikari-eval")
    }
}

fn code(o: &Output) -> i32 {
    o.status.code().expect("exited normally")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn path_str(p: &Path) -> &str {
    p.to_str().unwrap()
}

// fixture の辞書での homophones.tsv は 14 件中 4 件が1位（h-01, h-05, h-09, a-01）= 35.7%

#[test]
fn min_top1_gates_the_exit_code() {
    let env = Env::new();
    let set = env.homophones();
    let set = path_str(&set);

    let ok = env.eval(&["--set", set, "--min-top1", "0.35"]);
    assert_eq!(code(&ok), EXIT_OK, "{}", stderr(&ok));

    let boundary = env.eval(&["--set", set, "--min-top1", "0"]);
    assert_eq!(code(&boundary), EXIT_OK);

    let fail = env.eval(&["--set", set, "--min-top1", "0.4"]);
    assert_eq!(code(&fail), EXIT_GATE_FAILED);
    let err = stderr(&fail);
    assert!(err.contains("min-top1") && err.contains("35.7%"), "{err}");
    // 失敗してもレポートは出る
    assert!(stdout(&fail).contains("## 正解率"));
}

#[test]
fn min_top1_rejects_out_of_range_values() {
    let env = Env::new();
    for bad in ["1.5", "-0.1", "abc", "NaN"] {
        let o = env.eval(&["--min-top1", bad]);
        assert_eq!(code(&o), EXIT_ERROR, "{bad}: {}", stderr(&o));
    }
}

#[test]
fn baseline_regressions_are_listed_and_fail_only_when_asked() {
    let env = Env::new();
    let set = env.homophones();
    let set = path_str(&set);
    let base = env.path("base.tsv");

    let first = env.eval(&["--set", set, "--out-cases", path_str(&base)]);
    assert_eq!(code(&first), EXIT_OK, "{}", stderr(&first));

    // 自分自身との比較: 回帰なし
    let same = env.eval(&[
        "--set",
        set,
        "--baseline",
        path_str(&base),
        "--fail-on-regression",
    ]);
    assert_eq!(code(&same), EXIT_OK, "{}", stderr(&same));
    assert!(stdout(&same).contains("回帰なし"), "{}", stdout(&same));

    // 基準で h-02（いまは3位）が1位だったことにする -> 回帰
    let tampered: String = std::fs::read_to_string(&base)
        .unwrap()
        .lines()
        .map(|l| {
            if l.starts_with("h-02\t") {
                // rank 列（7列目）を 1 にする
                let mut cells: Vec<&str> = l.split('\t').collect();
                cells[6] = "1";
                cells.join("\t")
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let tampered_path = env.write("tampered.tsv", &tampered);

    let listed = env.eval(&["--set", set, "--baseline", path_str(&tampered_path)]);
    assert_eq!(
        code(&listed),
        EXIT_OK,
        "without --fail-on-regression it only lists"
    );
    let out = stdout(&listed);
    assert!(out.contains("回帰: 1 件"), "{out}");
    assert!(out.contains("| h-02 |"), "{out}");

    let failed = env.eval(&[
        "--set",
        set,
        "--baseline",
        path_str(&tampered_path),
        "--fail-on-regression",
    ]);
    assert_eq!(code(&failed), EXIT_GATE_FAILED);
    assert!(stderr(&failed).contains("h-02"), "{}", stderr(&failed));
}

#[test]
fn fail_on_regression_requires_a_baseline() {
    let env = Env::new();
    let o = env.eval(&["--fail-on-regression"]);
    assert_eq!(code(&o), EXIT_ERROR);
    assert!(stderr(&o).contains("baseline"), "{}", stderr(&o));
}

#[test]
fn unreadable_baseline_is_an_error() {
    let env = Env::new();
    let o = env.eval(&["--baseline", "/nonexistent/base.tsv"]);
    assert_eq!(code(&o), EXIT_ERROR);
    let bad = env.write("bad.tsv", "kind\trank\nk\t1\n");
    let o = env.eval(&["--baseline", path_str(&bad)]);
    assert_eq!(code(&o), EXIT_ERROR);
    assert!(stderr(&o).contains("missing column `id`"), "{}", stderr(&o));
}

const SPLIT_SET: &str = "\
id\tkind\ttags\tcontext\treading\texpected\tsplit
d1\t話題\tllm\t\tひんし\t品詞\tdev
d2\t話題\tllm,unreachable\t\tひんし\tありえない\tdev
t1\t話題\tllm\t\tひんし\t瀕死\ttest
";

#[test]
fn split_and_unreachable_filter_the_cases() {
    let env = Env::new();
    let set = env.write("split.tsv", SPLIT_SET);
    let set = path_str(&set);

    let dev = stdout(&env.eval(&["--set", set, "--split", "dev"]));
    assert!(dev.contains("(1 cases)"), "{dev}");
    assert!(
        dev.contains("- split: dev / 除外（unreachable）: 1 件"),
        "{dev}"
    );

    let dev_all = stdout(&env.eval(&["--set", set, "--split", "dev", "--include-unreachable"]));
    assert!(dev_all.contains("(2 cases)"), "{dev_all}");

    let test = stdout(&env.eval(&["--set", set, "--split", "test"]));
    assert!(test.contains("(1 cases)"), "{test}");
    assert!(test.contains("| t1 |"), "{test}");

    let all = stdout(&env.eval(&["--set", set]));
    assert!(all.contains("(2 cases)"), "{all}");
}

#[test]
fn bad_split_value_is_rejected_by_the_parser() {
    let env = Env::new();
    let o = env.eval(&["--split", "validation"]);
    assert_eq!(code(&o), EXIT_ERROR);
}

#[test]
fn alternatives_in_expected_are_accepted() {
    let env = Env::new();
    let set = env.write(
        "alt.tsv",
        "id\tkind\tcontext\treading\texpected\na1\t話題\t\tひんし\t存在しない|品詞\n",
    );
    let o = env.eval(&["--set", path_str(&set), "--min-top1", "1"]);
    assert_eq!(code(&o), EXIT_OK, "{}", stderr(&o));
}

#[test]
fn ajimee_format_is_read_and_converted() {
    let env = Env::new();
    // 入力はカタカナ。ひらがなに直して変換し、expected_output のどれかと一致すれば正解
    let json = r#"[
      {"index":"1","context_text":"","input":"ヒンシ","expected_output":["品詞","瀕死"],"original_text":"x","splitted_input_for_limited_input_length":[]},
      {"index":"2","context_text":"システムの","input":"コウセイ","expected_output":["構成"],"original_text":"y","splitted_input_for_limited_input_length":[]}
    ]"#;
    let path = env.write("ajimee.json", json);
    let o = env.eval(&["--set", path_str(&path), "--set-format", "ajimee"]);
    assert_eq!(code(&o), EXIT_OK, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("(2 cases)"), "{out}");
    assert!(out.contains("| kind:ajimee | 1 |"), "{out}");
    assert!(out.contains("| kind:ajimee-文脈 | 1 |"), "{out}");
    assert!(out.contains("| tag:ajimee | 2 |"), "{out}");
    // 別解のどちらかが1位なら正解（表のセルでは `|` がエスケープされる）
    assert!(
        out.contains("| ajimee-1 | ajimee |  | ひんし | 品詞\\|瀕死 | 品詞 | 1 |"),
        "{out}"
    );
}

#[test]
fn ajimee_format_rejects_broken_json() {
    let env = Env::new();
    let path = env.write("broken.json", "[{\"input\":");
    let o = env.eval(&["--set", path_str(&path), "--set-format", "ajimee"]);
    assert_eq!(code(&o), EXIT_ERROR);
    assert!(stderr(&o).contains("json error"), "{}", stderr(&o));
}

#[test]
fn compare_subcommand_prints_rank_differences() {
    let env = Env::new();
    let a = env.write(
        "a.tsv",
        "id\tkind\texpected\ttop1\trank\nx\tk\te\tf\t3\ny\tk\te\te\t1\n",
    );
    let b = env.write(
        "b.tsv",
        "id\tkind\texpected\ttop1\trank\nx\tk\te\te\t1\ny\tk\te\te\t1\n",
    );
    let o = env.raw(&["compare", path_str(&a), path_str(&b)]);
    assert_eq!(code(&o), EXIT_OK, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("| x | k | e | 3 | 1 |"), "{out}");
    assert!(out.contains("改善（b が上位）1"), "{out}");
    assert!(!out.contains("| y |"), "{out}");

    let missing = env.raw(&["compare", path_str(&a), "/nonexistent.tsv"]);
    assert_eq!(code(&missing), EXIT_ERROR);
}

#[test]
fn reach_subcommand_marks_unreachable_cases() {
    let env = Env::new();
    let set = env.write(
        "reach.tsv",
        "# keep me\nid\tkind\ttags\tcontext\treading\texpected\nok\t話題\tllm\t\tひんし\t品詞\nng\t話題\tllm\t\tひんし\t存在しない表記\n",
    );
    // --write なしでは変更しない
    let o = env.raw(&[
        "reach",
        "--set",
        path_str(&set),
        "--dict",
        path_str(&env.path("dict")),
    ]);
    assert_eq!(code(&o), EXIT_OK, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("1 of 2 cases"), "{out}");
    assert!(out.contains("ng\tひんし\t存在しない表記"), "{out}");
    assert!(
        !std::fs::read_to_string(&set)
            .unwrap()
            .contains("unreachable")
    );

    let o = env.raw(&[
        "reach",
        "--set",
        path_str(&set),
        "--dict",
        path_str(&env.path("dict")),
        "--write",
    ]);
    assert_eq!(code(&o), EXIT_OK, "{}", stderr(&o));
    let written = std::fs::read_to_string(&set).unwrap();
    assert_eq!(
        written,
        "# keep me\nid\tkind\ttags\tcontext\treading\texpected\nok\t話題\tllm\t\tひんし\t品詞\nng\t話題\tllm,unreachable\t\tひんし\t存在しない表記\n"
    );
    // 印を付けたあとは、既定では評価から除かれる
    let after = stdout(&env.eval(&["--set", path_str(&set)]));
    assert!(after.contains("(1 cases)"), "{after}");
}

#[test]
fn profile_ab_without_profile_cases_is_empty_and_succeeds() {
    let env = Env::new();
    let set = env.write(
        "noprofile.tsv",
        "id\tkind\tcontext\treading\texpected\na\t否定\t\tひんし\t品詞\n",
    );
    let o = env.eval(&["--set", path_str(&set), "--profile-ab"]);
    assert_eq!(code(&o), EXIT_OK, "{}", stderr(&o));
    assert!(
        stdout(&o).contains("対象のケースがありません"),
        "{}",
        stdout(&o)
    );
}

#[test]
fn report_shows_config_and_rss() {
    let env = Env::new();
    let o = env.eval(&["--set", path_str(&env.homophones())]);
    let out = stdout(&o);
    assert!(out.contains("- 構成: ranker=cost analyzer=null"), "{out}");
    assert!(out.contains("最大RSS: "), "{out}");
    // RSS の計測は Unix だけ（eval/src/rss.rs。Windows は「未計測」と出す）
    #[cfg(unix)]
    assert!(!out.contains("最大RSS: 未計測"), "{out}");
}

#[test]
fn ingest_holdout_without_a_ledger_gives_no_cases() {
    let env = Env::new();
    let user = env.path("user");
    std::fs::create_dir_all(&user).unwrap();
    let o = env.eval(&["--ingest-holdout", path_str(&user)]);
    assert_eq!(code(&o), EXIT_OK, "{}{}", stdout(&o), stderr(&o));
    assert!(stdout(&o).contains("ingest-holdout:"), "{}", stdout(&o));
}

#[cfg(not(feature = "lm"))]
#[test]
fn lm_flag_without_the_feature_is_an_error() {
    let env = Env::new();
    let o = env.eval(&[
        "--set",
        path_str(&env.homophones()),
        "--lm",
        "/nonexistent.gguf",
    ]);
    assert_eq!(code(&o), EXIT_ERROR);
    assert!(stderr(&o).contains("other than cost"), "{}", stderr(&o));
}

#[test]
fn unknown_ranker_is_rejected() {
    let env = Env::new();
    let o = env.eval(&["--ranker", "magic"]);
    assert_eq!(code(&o), EXIT_ERROR);
}

#[test]
fn analyzer_doc_and_topic_rankers_run_on_the_fixture() {
    let env = Env::new();
    let set = env.homophones();
    for (ranker, expect) in [
        ("analyzer", "ranker=analyzer()"),
        ("doc", "ranker=doc(left_context,kana_style,doc_cache)"),
    ] {
        let o = env.eval(&["--set", path_str(&set), "--ranker", ranker]);
        assert_eq!(code(&o), EXIT_OK, "{ranker}: {}", stderr(&o));
        let out = stdout(&o);
        assert!(out.contains(expect), "{out}");
        assert!(out.contains("analyzer=lattice rewriters=6"), "{out}");
        assert!(out.contains("(14 cases)"), "{out}");
    }
    // vectors がなければ topic は組み立てられない
    let o = env.eval(&["--set", path_str(&set), "--ranker", "topic"]);
    assert_eq!(code(&o), EXIT_ERROR);
    assert!(stderr(&o).contains("vectors.bin"), "{}", stderr(&o));
    // --no-analyzer は構成の行に出る
    let o = env.eval(&["--set", path_str(&set), "--ranker", "doc", "--no-analyzer"]);
    assert!(stdout(&o).contains("analyzer=null"), "{}", stdout(&o));
}

/// 実辞書（v2、vectors.bin つき。`IKARI_DICT_DIR`）で R1〜R3 を通す。未設定なら失敗する。
#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary)"]
fn real_dictionary_runs_r1_to_r3() {
    let dir = std::env::var_os("IKARI_DICT_DIR")
        .map(PathBuf::from)
        .expect("IKARI_DICT_DIR is not set (required by #[ignore] tests)");
    assert!(
        dir.join("system.dict").exists(),
        "no real dictionary at {}",
        dir.display()
    );
    let env = Env::new();
    let set = env.homophones();
    let mut rankers = vec!["cost", "analyzer", "doc"];
    if dir.join("vectors.bin").exists() {
        rankers.push("topic");
    }
    for ranker in rankers {
        let o = Command::new(env!("CARGO_BIN_EXE_ikari-eval"))
            .arg("--dict")
            .arg(&dir)
            .args(["--repeat", "1", "--set", path_str(&set), "--ranker", ranker])
            .output()
            .unwrap();
        assert_eq!(code(&o), EXIT_OK, "{ranker}: {}", stderr(&o));
        assert!(stdout(&o).contains("(14 cases)"), "{ranker}");
    }
}

// --- 他レーンの本体が要るテスト（マージ後に有効にする）

#[test]
fn full_ranker_runs_the_bundled_set() {
    let env = Env::new();
    let o = env.eval(&["--set", path_str(&env.homophones()), "--ranker", "full"]);
    assert_eq!(code(&o), EXIT_OK, "{}", stderr(&o));
    assert!(stdout(&o).contains("ranker=full(left_context,kana_style,doc_cache,learning,profile"));
}

#[test]
fn profile_ab_runs_with_real_presets() {
    let env = Env::new();
    let o = env.eval(&[
        "--set",
        path_str(&env.homophones()),
        "--ranker",
        "full",
        "--profile-ab",
    ]);
    let out = stdout(&o);
    assert!(out.contains("プロファイルあり・なしの差分"), "{out}");
    assert!(
        code(&o) == EXIT_OK || stderr(&o).contains("profile-ab"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn scenario_commit_then_probe() {
    let env = Env::new();
    let scenario = env.write(
        "scenario.tsv",
        "step\tcontext\treading\tsurface\nprobe\t技術文書で\tひんし\t品詞\ncommit\t技術文書で\tひんし\t瀕死\nprobe\t技術文書で\tひんし\t瀕死\n",
    );
    let o = env.eval(&["--scenario", path_str(&scenario), "--ranker", "full"]);
    assert_eq!(code(&o), EXIT_OK, "{}{}", stdout(&o), stderr(&o));
}

/// 取り込み → 評価用の 2 割の読み出し → ケース化までがつながっていること
#[test]
fn ingest_holdout_builds_cases_from_an_ingested_file() {
    use ikari_core::{LearningStore, open_dir, user::UserDir};
    use ikari_import::{Env as ImportEnv, RunOptions, is_holdout, run_file};

    let env = Env::new();
    let user = env.path("user");
    std::fs::create_dir_all(&user).unwrap();
    let mut lines = vec![
        r#"{"type":"header","format":"ikari-ingest","version":1,"prompt_version":"ingest-prompt-v1"}"#
            .to_owned(),
    ];
    let mut holdout_sentences = 0;
    for n in 1..60 {
        let surface = format!("ひん死{}", "の".repeat(n));
        let reading = format!("ひんし{}", "の".repeat(n));
        if is_holdout(&surface) {
            holdout_sentences += 1;
        }
        lines.push(format!(
            r#"{{"type":"sentence","surface":"{surface}","reading":"{reading}"}}"#
        ));
    }
    assert!(holdout_sentences > 0);
    let file = env.write("ingest.jsonl", &(lines.join("\n") + "\n"));

    let loaded = open_dir(&env.path("dict")).unwrap();
    let import_env = ImportEnv {
        dict: &*loaded.dict,
        grammar: &*loaded.grammar,
        vectors: None,
    };
    let store = LearningStore::open(&user.join("learning.tsv")).unwrap();
    store.attach_grammar(loaded.grammar.clone());
    run_file(
        &UserDir::new(&user),
        &file,
        &import_env,
        &store,
        RunOptions::now(false, false),
    )
    .expect("ingest");

    let o = env.eval(&["--ingest-holdout", path_str(&user)]);
    assert_eq!(code(&o), EXIT_OK, "{}{}", stdout(&o), stderr(&o));
    assert!(stdout(&o).contains("holdout-0-0"), "{}", stdout(&o));
}

/// 辞書の除外をやめ、順位を後ろに下げるだけにした（spec の基本原則）。
/// fixture には `hennkann`（ascii_lower）と `4`（読み ヨウ、numeric_surface）がある。
#[test]
fn penalized_entries_stay_as_lower_candidates() {
    let env = Env::new();
    let set = env.write(
        "penalized.tsv",
        "id\tkind\tcontext\treading\texpected\n\
         p-1\t話題\t\tへんかん\thennkann\n\
         p-2\t話題\t\tよう\t4\n",
    );
    let out_cases = env.path("penalized-cases.tsv");
    let o = env.eval(&[
        "--set",
        path_str(&set),
        "--include-unreachable",
        "--out-cases",
        path_str(&out_cases),
    ]);
    assert_eq!(code(&o), EXIT_OK, "{}", stderr(&o));
    let cases = std::fs::read_to_string(&out_cases).unwrap();
    for id in ["p-1", "p-2"] {
        let line = cases.lines().find(|l| l.starts_with(id)).expect(id);
        let cols: Vec<&str> = line.split('\t').collect();
        // id kind context reading expected top1 rank ...
        let rank: usize = cols[6]
            .parse()
            .unwrap_or_else(|_| panic!("{id} must stay a candidate: {line}"));
        // 同じ読みに普通の候補がある `hennkann` は上位に出ない（`4` の読みには他に数字の候補がない）
        if id == "p-1" {
            assert!(rank > 1, "{id} must not be first: {line}");
            assert_ne!(cols[5], cols[4], "{id}: {line}");
        }
    }
}

/// `--user-dir` に user_dict.tsv があれば、ユーザー辞書として重ねる（今までは読まなかった。ADR 0012 §10-6）。
/// `--user-dir` がなければ（一時ディレクトリ）重ねない。
#[test]
fn user_dictionary_in_the_user_dir_is_layered() {
    use ikari_core::user::{UserDictEntry, UserDictionary, UserDir};

    let env = Env::new();
    let user = env.path("user");
    std::fs::create_dir_all(&user).unwrap();
    let mut dict = UserDictionary::default();
    dict.add(UserDictEntry {
        reading: "ひんし".into(),
        surface: "品死".into(),
        pos: "名詞".into(),
        comment: String::new(),
    })
    .unwrap();
    dict.save(&UserDir::new(&user).user_dict_tsv()).unwrap();
    let set = env.write(
        "user-dict.tsv",
        "id\tkind\tcontext\treading\texpected\nu-1\t話題\t\tひんし\t品死\n",
    );
    let top1_of = |args: &[&str]| {
        let out_cases = env.path("user-dict-cases.tsv");
        let mut all = vec![
            "--set",
            path_str(&set),
            "--include-unreachable",
            "--out-cases",
        ];
        all.push(path_str(&out_cases));
        all.extend_from_slice(args);
        let o = env.eval(&all);
        assert!(
            code(&o) == EXIT_OK || code(&o) == EXIT_GATE_FAILED,
            "{}",
            stderr(&o)
        );
        let cases = std::fs::read_to_string(&out_cases).unwrap();
        let line = cases
            .lines()
            .find(|l| l.starts_with("u-1"))
            .unwrap()
            .to_owned();
        line.split('\t').nth(5).unwrap().to_owned()
    };
    assert_eq!(
        top1_of(&["--ranker", "full", "--user-dir", path_str(&user)]),
        "品死"
    );
    assert_ne!(top1_of(&["--ranker", "full"]), "品死");
}
