//! フェーズ1（レーン U）の CLI 統合テスト。fixture 辞書を作って `ikari` のサブコマンドを起動する。
//!
//! 他レーンの実装がマージされるまで動かないものには `#[ignore = "requires phase1 lane X"]` を付けている
//! （lane A: 辞書 v2 と Grammar の新メソッド、lane B: Converter の配線と各 Ranker）。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ikari_core::LearningStore;
use ikari_dict_builder::{BuildConfig, build};

const UNK_POS: &str = "名詞,普通名詞,一般,*,*,*";
const UNK_COST: i16 = 10_000;
const HEADER: &str =
    r#"{"type":"header","format":"ikari-ingest","version":1,"prompt_version":"ingest-prompt-v1"}"#;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../testdata/sudachi-mini")
}

fn build_fixture_dict(out_dir: &Path) {
    let cfg = BuildConfig {
        lex: vec![fixture_dir().join("lex.csv")],
        matrix: fixture_dir().join("matrix.def"),
        out_dir: out_dir.to_path_buf(),
        unk_pos: UNK_POS.to_string(),
        unk_cost: UNK_COST,
        label: "phase1-test".to_string(),
    };
    build(&cfg).expect("build fixture dictionary");
}

/// 一時ディレクトリの中に、辞書とユーザーデータの置き場所を持つ。
struct Env {
    tmp: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        build_fixture_dict(&tmp.path().join("dict"));
        Self { tmp }
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn dict(&self) -> PathBuf {
        self.root().join("dict")
    }

    fn user(&self) -> PathBuf {
        self.root().join("user")
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root().join(name)
    }

    /// `--user-dir` と `--dict` を付けずに、環境変数で渡して起動する（実際の HOME に触れない）。
    fn run(&self, args: &[&str]) -> Output {
        self.run_in(&self.user(), args)
    }

    fn run_in(&self, user: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ikari"))
            .env("IKARI_USER_DIR", user)
            .env("IKARI_DICT_DIR", self.dict())
            .args(args)
            .output()
            .expect("spawn ikari")
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "args {args:?}\nstderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn fails(&self, args: &[&str]) -> (Option<i32>, String) {
        let out = self.run(args);
        assert!(
            !out.status.success(),
            "args {args:?} unexpectedly succeeded"
        );
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn read_user(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.user().join(name)).ok()
    }
}

fn jsonl(sentences: &[(&str, &str)]) -> String {
    let mut out = format!("{HEADER}\n");
    for (s, r) in sentences {
        out.push_str(&sentence_line(s, r));
        out.push('\n');
    }
    out
}

/// テストの文に引用符やバックスラッシュは含めない
fn sentence_line(surface: &str, reading: &str) -> String {
    format!(r#"{{"type":"sentence","surface":"{surface}","reading":"{reading}"}}"#)
}

// ---- 引数 ----

#[test]
fn help_lists_the_subcommands() {
    let env = Env::new();
    let out = env.run(&["--help"]);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    for name in ["session", "user-dict", "learn", "profile", "ingest", "data"] {
        assert!(text.contains(name), "{name} missing in help");
    }
}

#[test]
fn subcommand_usage_errors_exit_with_code_2() {
    let env = Env::new();
    for args in [
        vec!["user-dict"],
        vec!["learn", "reset"],
        vec!["data", "export"],
        vec!["ingest", "remove", "x"],
        vec!["profile", "set"],
    ] {
        assert_eq!(env.fails(&args).0, Some(2), "{args:?}");
    }
}

#[test]
fn user_data_commands_need_a_resolvable_user_dir() {
    let out = Command::new(env!("CARGO_BIN_EXE_ikari"))
        .env_remove("IKARI_USER_DIR")
        .env_remove("HOME")
        .env_remove("XDG_DATA_HOME")
        // Windows の置き場所（%APPDATA%）も外す
        .env_remove("APPDATA")
        .args(["user-dict", "list"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("置き場所"));
}

// ---- ユーザー辞書 ----

#[test]
fn user_dict_add_list_remove() {
    let env = Env::new();
    env.ok(&[
        "user-dict",
        "add",
        "イカリ",
        "ikari-ime",
        "短縮よみ",
        "--comment",
        "自作IME",
    ]);
    env.ok(&[
        "user-dict",
        "add",
        "かきすすめる",
        "書き進める",
        "動詞:下一段-マ行",
    ]);
    let list = env.ok(&["user-dict", "list"]);
    assert!(
        list.contains("いかり\tikari-ime\t短縮よみ\t自作IME"),
        "{list}"
    );
    assert!(
        list.contains("かきすすめる\t書き進める\t動詞:下一段-マ行\t"),
        "{list}"
    );
    let file = env.read_user("user_dict.tsv").unwrap();
    assert!(file.starts_with("#ikari-user-dict\tschema=1\n"));

    // 同じ (読み, 表記) は上書き
    env.ok(&["user-dict", "add", "いかり", "ikari-ime", "名詞"]);
    assert_eq!(env.ok(&["user-dict", "list"]).lines().count(), 2);

    env.ok(&["user-dict", "remove", "いかり", "ikari-ime"]);
    assert_eq!(env.ok(&["user-dict", "list"]).lines().count(), 1);
    let (code, stderr) = env.fails(&["user-dict", "remove", "いかり", "ikari-ime"]);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("登録されていません"));
}

#[test]
fn user_dict_add_rejects_bad_entries_and_changes_nothing() {
    let env = Env::new();
    env.ok(&["user-dict", "add", "あ", "亜", "名詞"]);
    let before = env.read_user("user_dict.tsv");
    for args in [
        vec!["user-dict", "add", "abc", "x", "名詞"],
        vec!["user-dict", "add", "あ", "亜", "謎の品詞"],
        vec!["user-dict", "add", "かく", "書く", "動詞:五段-ラ行"],
        vec!["user-dict", "add", "かく", "書く", "動詞"],
    ] {
        assert_eq!(env.fails(&args).0, Some(1), "{args:?}");
    }
    assert_eq!(env.read_user("user_dict.tsv"), before);
}

#[test]
fn user_dict_import_mozc_maps_parts_of_speech_and_reports_skips() {
    let env = Env::new();
    let mozc = env.path("mozc.txt");
    fs::write(
        &mozc,
        "!Microsoft IME Dictionary Tool\n\
         いかり\tikari-ime\t名詞\tコメント\n\
         たべる\t食べる\t動詞一段\t\n\
         にっこり\t:-)\t顔文字\t\n\
         かく\t書く\t動詞カ行五段\t\n",
    )
    .unwrap();
    let out = env.run(&[
        "user-dict",
        "import",
        "--format",
        "mozc",
        mozc.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("line 4") && stderr.contains("顔文字"),
        "{stderr}"
    );
    let list = env.ok(&["user-dict", "list"]);
    assert!(list.contains("いかり\tikari-ime\t名詞\tコメント"), "{list}");
    assert!(list.contains("たべる\t食べる\t動詞:下一段-バ行"), "{list}");
    assert!(list.contains("かく\t書く\t動詞:五段-カ行"), "{list}");
    assert_eq!(list.lines().count(), 3);
}

#[test]
fn user_dict_import_is_all_or_nothing_for_the_ikari_format() {
    let env = Env::new();
    env.ok(&["user-dict", "add", "あ", "亜", "名詞"]);
    let before = env.read_user("user_dict.tsv");
    let bad = env.path("bad.tsv");
    fs::write(
        &bad,
        "#ikari-user-dict\tschema=1\nい\t伊\t名詞\t\nう\t宇\t謎\t\n",
    )
    .unwrap();
    assert_eq!(
        env.fails(&["user-dict", "import", bad.to_str().unwrap()]).0,
        Some(1)
    );
    assert_eq!(env.read_user("user_dict.tsv"), before);
    let good = env.path("good.tsv");
    fs::write(&good, "#ikari-user-dict\tschema=1\nい\t伊\t名詞\t\n").unwrap();
    env.ok(&["user-dict", "import", good.to_str().unwrap()]);
    assert_eq!(env.ok(&["user-dict", "list"]).lines().count(), 2);
}

#[test]
fn user_dict_check_expands_registered_verbs_against_the_v2_grammar() {
    let env = Env::new();
    env.ok(&[
        "user-dict",
        "add",
        "かきすすめる",
        "書き進める",
        "動詞:下一段-マ行",
    ]);
    // fixture の文法には 形状詞 / 形容詞 の品詞がないので、検査全体は失敗するが、
    // 登録した動詞は活用規則で展開できる
    let (code, _) = env.fails(&["user-dict", "check"]);
    assert_eq!(code, Some(1));
    let out = env.run(&["user-dict", "check"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("形状詞"), "{stdout}");
    assert!(!stdout.contains("ユーザー辞書を展開できません"), "{stdout}");
}

#[test]
fn user_dict_check_needs_a_readable_dictionary() {
    let env = Env::new();
    let missing = env.path("nothing");
    let (code, stderr) = env.fails(&["user-dict", "check", "--dict", missing.to_str().unwrap()]);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("nothing"), "{stderr}");
}

#[test]
fn user_dict_words_are_used_in_conversion() {
    let env = Env::new();
    env.ok(&["user-dict", "add", "いかり", "ikari-ime", "短縮よみ"]);
    let out = env.ok(&["--ranker", "cost", "--format", "tsv", "いかり"]);
    assert!(out.contains("ikari-ime"), "{out}");
}

// ---- プロファイル ----

#[test]
fn profile_presets_set_show_clear() {
    let env = Env::new();
    let presets = env.ok(&["profile", "presets"]);
    for id in ["it", "infra", "novel", "business"] {
        assert!(presets.contains(id), "{presets}");
    }
    assert!(presets.contains("下書き"));
    assert!(env.ok(&["profile", "show"]).contains("設定なし"));
    env.ok(&["profile", "set", "it=often", "novel=sometimes"]);
    let shown = env.ok(&["profile", "show"]);
    assert!(
        shown.contains("it\toften") && shown.contains("novel\tsometimes"),
        "{shown}"
    );
    assert_eq!(env.fails(&["profile", "set", "golf=often"]).0, Some(1));
    assert_eq!(env.fails(&["profile", "set", "it=always"]).0, Some(1));
    assert!(env.ok(&["profile", "show"]).contains("it\toften"));
    env.ok(&["profile", "clear"]);
    assert!(env.ok(&["profile", "show"]).contains("設定なし"));
}

#[test]
fn unknown_profile_name_is_an_error() {
    let env = Env::new();
    let (code, stderr) = env.fails(&["--profile", "golf", "ひんし"]);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("golf"), "{stderr}");
}

// ---- 学習 ----

fn seed_learning(env: &Env) {
    let store = LearningStore::open(&env.user().join("learning.tsv")).unwrap();
    let feats =
        ikari_core::user::extract(&ikari_core::AnalyzedContext::default(), None, store.salt());
    let seg = |s: &str, r: &str| ikari_core::user::SegmentKey {
        surface: s.into(),
        reading: r.into(),
        pos_name: "名詞,普通名詞,一般,*,*,*".into(),
    };
    store.add_initial(1, "ひんし", &[seg("品詞", "ひんし")], &feats, 20_000);
    store.add_initial(2, "かき", &[seg("牡蠣", "かき")], &feats, 20_000);
    store.save().unwrap();
}

#[test]
fn learn_show_and_reset_by_layer() {
    let env = Env::new();
    // 実際の使用の記録を足す（commit の代わりに、ストアに直接）
    seed_learning(&env);
    let shown = env.ok(&["learn", "show"]);
    assert!(shown.contains("# usage=0 initial=2"), "{shown}");
    assert!(shown.contains("initial\t1\tword\tひんし\t品詞"), "{shown}");
    let one = env.ok(&["learn", "show", "--reading", "ヒンシ"]);
    assert!(one.contains("品詞") && !one.contains("牡蠣"), "{one}");

    env.ok(&["learn", "reset", "--layer", "usage"]);
    assert!(env.ok(&["learn", "show"]).contains("initial=2"));
    env.ok(&["learn", "reset", "--layer", "initial"]);
    assert!(env.ok(&["learn", "show"]).contains("initial=0"));
}

#[test]
fn corrupt_learning_file_is_an_error_and_is_left_alone() {
    let env = Env::new();
    fs::create_dir_all(env.user()).unwrap();
    fs::write(env.user().join("learning.tsv"), "garbage\n").unwrap();
    assert_eq!(env.fails(&["learn", "show"]).0, Some(1));
    assert_eq!(env.fails(&["learn", "reset", "--layer", "all"]).0, Some(1));
    assert_eq!(env.fails(&["ひんし"]).0, Some(1));
    assert_eq!(env.read_user("learning.tsv").as_deref(), Some("garbage\n"));
}

#[test]
fn secure_commit_does_not_touch_the_learning_file() {
    let env = Env::new();
    // 学習ファイルがない状態
    let out = env.run(&[
        "--secure",
        "--commit",
        "1",
        "--context",
        "秘密の文脈",
        "ひんし",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("学習しません"));
    assert!(env.read_user("learning.tsv").is_none());
    assert!(!env.user().exists());

    // 学習ファイルがある状態
    seed_learning(&env);
    let before = env.read_user("learning.tsv");
    assert!(
        env.run(&["--secure", "--commit", "2", "ひんし"])
            .status
            .success()
    );
    assert_eq!(env.read_user("learning.tsv"), before);
}

#[test]
fn secure_session_does_not_touch_the_learning_file() {
    let env = Env::new();
    seed_learning(&env);
    let before = env.read_user("learning.tsv");
    let script = env.path("s.tsv");
    fs::write(&script, "ひんし\t瀕死\n").unwrap();
    let out = env.run(&["session", "--secure", "--script", script.to_str().unwrap()]);
    // 確定の記録は行わない（Conversion::commit も呼ばない）。候補の探索は secure でも行える
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(env.read_user("learning.tsv"), before);
}

#[test]
fn session_probe_lines_show_the_top_candidate_without_learning() {
    let env = Env::new();
    let script = env.path("probe.tsv");
    fs::write(&script, "# コメント\nひんし\nhinshi\t\n").unwrap();
    let out = env.ok(&["session", "--script", script.to_str().unwrap()]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    for l in &lines {
        let cols: Vec<&str> = l.split('\t').collect();
        assert_eq!(cols.len(), 4, "{l}");
        assert_eq!(cols[0], "ひんし");
        assert!(!cols[1].is_empty());
        assert_eq!((cols[2], cols[3]), ("", ""));
    }
    // probe は何も保存しない
    assert!(env.read_user("learning.tsv").is_none());
}

#[test]
fn commit_is_learned_and_session_stacks_the_history() {
    let env = Env::new();
    let out = env.run(&[
        "--commit",
        "2",
        "--context",
        "仕様を",
        "--format",
        "tsv",
        "ひんし",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let shown = env.ok(&["learn", "show", "--reading", "ひんし"]);
    assert!(shown.contains("usage"), "{shown}");
    // 文を保存していない: 左文脈の語は出てこない
    assert!(!shown.contains("仕様を"));

    let script = env.path("s.tsv");
    fs::write(&script, "ひんし\t瀕死\nひんし\t\n").unwrap();
    let out = env.ok(&["session", "--script", script.to_str().unwrap()]);
    let rows: Vec<Vec<&str>> = out.lines().map(|l| l.split('\t').collect()).collect();
    assert_eq!(rows[0][2], "瀕死");
    assert!(!rows[0][3].is_empty());
}

// ---- データのエクスポートとインポート ----

#[test]
fn data_export_import_roundtrip_overwrites() {
    let env = Env::new();
    env.ok(&["user-dict", "add", "いかり", "ikari-ime", "短縮よみ"]);
    env.ok(&["profile", "set", "it=often"]);
    seed_learning(&env);
    let file = env.path("export.txt");
    env.ok(&["data", "export", file.to_str().unwrap()]);
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.starts_with("#ikari-export\tschema=1\n"));

    // 別のユーザーディレクトリへ
    let other = env.path("user2");
    let out = env.run_in(&other, &["user-dict", "add", "ほか", "他", "名詞"]);
    assert!(out.status.success());
    let out = env.run_in(&other, &["data", "import", file.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let listed = env.run_in(&other, &["user-dict", "list"]);
    let listed = String::from_utf8(listed.stdout).unwrap();
    assert!(
        listed.contains("ikari-ime") && !listed.contains("他"),
        "{listed}"
    );
    for name in ["learning.tsv", "profile.tsv"] {
        assert_eq!(
            fs::read_to_string(other.join(name)).unwrap(),
            fs::read_to_string(env.user().join(name)).unwrap(),
            "{name}"
        );
    }
    // 再エクスポートが同じ
    let file2 = env.path("export2.txt");
    let out = env.run_in(&other, &["data", "export", file2.to_str().unwrap()]);
    assert!(out.status.success());
    assert_eq!(fs::read_to_string(&file2).unwrap(), text);
}

#[test]
fn data_import_of_an_invalid_file_changes_nothing() {
    let env = Env::new();
    env.ok(&["user-dict", "add", "あ", "亜", "名詞"]);
    seed_learning(&env);
    let names = ["user_dict.tsv", "learning.tsv"];
    let before: Vec<_> = names.iter().map(|n| env.read_user(n)).collect();

    let good = env.path("good.txt");
    env.ok(&["data", "export", good.to_str().unwrap()]);
    let good_text = fs::read_to_string(&good).unwrap();
    let bad_cases = [
        String::new(),
        "not an export\n".to_string(),
        good_text.replace("#ikari-export\tschema=1", "#ikari-export\tschema=7"),
        good_text.replace("あ\t亜\t名詞", "あ\t亜\t存在しない品詞"),
        good_text.replace("[learning]", "[learning]\n#extra junk"),
        good_text.replacen("[ingest_sources]", "[unknown_section]", 1),
    ];
    for (i, bad) in bad_cases.iter().enumerate() {
        let f = env.path(&format!("bad{i}.txt"));
        fs::write(&f, bad).unwrap();
        let (code, _) = env.fails(&["data", "import", f.to_str().unwrap()]);
        assert_eq!(code, Some(1), "case {i}");
        let after: Vec<_> = names.iter().map(|n| env.read_user(n)).collect();
        assert_eq!(after, before, "case {i} changed files");
    }
}

// ---- 取り込み ----

#[test]
fn ingest_prompt_shows_version_and_the_business_document_warning() {
    let env = Env::new();
    let out = env.run(&["ingest", "prompt"]);
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("ingest-prompt-v1"));
    assert!(stdout.contains("業務文書"));
    // 画面（stderr）にも注意を出す
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("業務文書"), "{stderr}");
}

fn sample_ingest(env: &Env) -> PathBuf {
    let path = env.path("my-writing.jsonl");
    // fixture 辞書の語だけで書いた文（変換、品詞、を、に、の、は）と、未知語（怒里）を含む文、読み違いの文
    fs::write(
        &path,
        jsonl(&[
            ("品詞を変換", "ひんしをへんかん"),
            ("瀕死の変換", "ひんしのへんかん"),
            ("怒里を変換", "いかりをへんかん"),
            ("怒里の変換", "いかりのへんかん"),
            ("変換に品詞", "へんかんにひんし"),
            ("変換を品詞", "へんかんをひんし"),
            ("品詞は変換", "ひんしはへんかん"),
            ("品詞を変換", "へんかんをひんし"),
        ]),
    )
    .unwrap();
    path
}

#[test]
fn ingest_run_list_candidates_accept_remove() {
    let env = Env::new();
    let file = sample_ingest(&env);

    // dry-run は何も作らない
    let out = env.ok(&["ingest", "run", file.to_str().unwrap(), "--dry-run"]);
    assert!(out.contains("dry-run"), "{out}");
    assert!(!env.user().exists());

    let out = env.ok(&["ingest", "run", file.to_str().unwrap()]);
    assert!(out.contains("id=1"), "{out}");
    assert!(out.contains("ingest-prompt-v1"), "{out}");
    assert!(out.contains("有効な文: 8"), "{out}");
    // 読み違いの文は捨てられる
    assert!(out.contains("辞書で読みを再現できない"), "{out}");

    let list = env.ok(&["ingest", "list"]);
    assert!(
        list.starts_with("1\t") && list.contains("my-writing.jsonl"),
        "{list}"
    );
    assert!(list.contains("ingest-prompt-v1"));

    // 同じファイルは二重に取り込まない
    let (code, stderr) = env.fails(&["ingest", "run", file.to_str().unwrap()]);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("取り込み済み"), "{stderr}");

    // 初期学習は initial 層だけ
    let shown = env.ok(&["learn", "show", "--limit", "1000"]);
    assert!(shown.contains("initial\t1\tword"), "{shown}");
    assert!(!shown.contains("usage\t"), "{shown}");

    // 登録候補
    let cands = env.ok(&["ingest", "candidates"]);
    assert!(cands.contains("1\tいかり\t怒里\t"), "{cands}");
    env.ok(&["ingest", "accept", "1"]);
    assert!(
        env.ok(&["user-dict", "list"])
            .contains("いかり\t怒里\t名詞")
    );
    assert!(env.ok(&["ingest", "candidates"]).contains("登録候補なし"));
    assert_eq!(env.fails(&["ingest", "accept", "5"]).0, Some(1));

    // 取り込み元を消すと、その initial 記録も消える。ファイルは残る
    env.ok(&["ingest", "remove", "1"]);
    assert!(env.ok(&["learn", "show"]).contains("initial=0"));
    assert!(env.ok(&["ingest", "list"]).contains("取り込みなし"));
    assert!(file.exists());
    assert_eq!(env.fails(&["ingest", "remove", "1"]).0, Some(1));
}

#[test]
fn ingest_reset_keeps_the_usage_layer() {
    let env = Env::new();
    let file = sample_ingest(&env);
    env.ok(&["ingest", "run", file.to_str().unwrap()]);
    env.ok(&["ingest", "reset"]);
    assert!(env.ok(&["learn", "show"]).contains("initial=0"));
    assert!(env.ok(&["ingest", "list"]).contains("取り込みなし"));
}

#[test]
fn ingest_rejects_invalid_files_and_changes_nothing() {
    let env = Env::new();
    let cases = [
        ("empty.jsonl", String::new()),
        (
            "nohdr.jsonl",
            format!("{}\n", sentence_line("品詞", "ひんし")),
        ),
        (
            "oldprompt.jsonl",
            HEADER.replace("ingest-prompt-v1", "ingest-prompt-v0") + "\n",
        ),
        (
            "badver.jsonl",
            HEADER.replace("\"version\":1", "\"version\":2") + "\n",
        ),
    ];
    for (name, text) in cases {
        let f = env.path(name);
        fs::write(&f, text).unwrap();
        let (code, _) = env.fails(&["ingest", "run", f.to_str().unwrap()]);
        assert_eq!(code, Some(1), "{name}");
        assert!(!env.user().exists(), "{name} created user data");
    }
    let (code, _) = env.fails(&["ingest", "run", env.path("missing.jsonl").to_str().unwrap()]);
    assert_eq!(code, Some(1));
}

#[test]
fn ingest_never_copies_sentences_into_learning_or_export() {
    let env = Env::new();
    let file = sample_ingest(&env);
    env.ok(&["ingest", "run", file.to_str().unwrap()]);
    let export = env.path("export.txt");
    env.ok(&["data", "export", export.to_str().unwrap()]);
    let learning = env.read_user("learning.tsv").unwrap();
    let export = fs::read_to_string(&export).unwrap();
    for sentence in [
        "品詞を変換",
        "瀕死の変換",
        "怒里を変換",
        "怒里の変換",
        "変換に品詞",
        "変換を品詞",
        "品詞は変換",
    ] {
        for text in [&learning, &export] {
            assert!(!text.contains(sentence), "{sentence} がそのまま入っている");
        }
    }
    // 評価用の文も、ユーザーデータのどのファイルにも複製されない
    for entry in fs::read_dir(env.user()).unwrap() {
        let p = entry.unwrap().path();
        let text = fs::read_to_string(&p).unwrap();
        assert!(!text.contains("品詞を変換"), "{}", p.display());
    }
}

#[test]
fn ingest_apply_profile_writes_profile_tsv_only_when_asked() {
    let env = Env::new();
    let file = env.path("it.jsonl");
    // fixture 辞書には IT の種語に当たる語がほとんどない（辞書、変換 など）。当たらなければ書かない
    fs::write(&file, jsonl(&[("品詞を変換", "ひんしをへんかん")])).unwrap();
    let out = env.ok(&["ingest", "run", file.to_str().unwrap()]);
    assert!(out.contains("分野の推定"), "{out}");
    assert!(env.read_user("profile.tsv").is_none());
}
