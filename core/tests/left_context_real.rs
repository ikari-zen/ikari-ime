//! 実辞書での、左文脈の語との複合語の加点（ADR 0007 §10）。`#[ignore]` なので `--ignored` で明示したときだけ動き、
//! `IKARI_DICT_DIR` がなければ失敗する（黙って成功しない）。
//!
//! `IKARI_DICT_DIR=data/build/dict cargo test --release -p ikari-core --test left_context_real -- --ignored --nocapture`
//!
//! Ranker は LM なしの `full`（左文脈、かなの表記、文書内キャッシュ、学習（空）、プロファイル、話題）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ikari_core::{
    CompositeRanker, Context, Converter, Dictionary, DictionarySet, DocumentCacheConfig,
    DocumentCacheRanker, Grammar, KanaStyleConfig, KanaStyleRanker, KanaStyleTable,
    LatticeContextAnalyzer, LearningConfig, LearningRanker, LearningStore, LeftContextConfig,
    LeftContextRanker, ProfileConfig, ProfileRanker, RankInput, Ranker, TopicConfig, TopicRanker,
    UserDictionary, VECTORS_FILE_NAME, open_dir, open_vectors,
};

const DICT_DIR_ENV: &str = "IKARI_DICT_DIR";
/// 加点の段の時間を測る評価セット（読むだけ）
const SET_FILES: [&str; 2] = ["katakana-suffix.tsv", "homophones-ext.tsv"];
const REPEAT: usize = 5;
/// LeftContextRanker の 1回の p99 の上限（ADR 0007 §4 の測定は 24〜26 µs。変換の p99 予算 2 ms に対して十分小さいこと）
const RANK_P99_LIMIT: Duration = Duration::from_micros(200);

fn dict_dir() -> PathBuf {
    std::env::var_os(DICT_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            panic!("{DICT_DIR_ENV} is required (a real dictionary built by ikari-dict-builder)")
        })
}

/// LeftContextRanker の `rank` 1回の時間を記録する包み（名前は中身と同じなので、Guard の分類は変わらない）
struct Timed {
    inner: LeftContextRanker,
    times: Arc<Mutex<Vec<Duration>>>,
}

impl Ranker for Timed {
    fn rank(&self, input: &RankInput<'_>, cands: &mut [ikari_core::Candidate]) {
        let t = Instant::now();
        self.inner.rank(input, cands);
        self.times.lock().unwrap().push(t.elapsed());
    }

    fn name(&self) -> &str {
        self.inner.name()
    }
}

struct Setup {
    converter: Converter,
    times: Arc<Mutex<Vec<Duration>>>,
}

/// `user_dict` は ikari 形式のユーザー辞書の行（ヘッダなし）。空ならシステム辞書だけ
fn converter(compound_bonus: i32, user_dict: &str) -> Setup {
    let dir = dict_dir();
    let loaded = open_dir(&dir).unwrap();
    let dict_id = loaded.dict.dict_id().unwrap();
    let vectors = open_vectors(&dir.join(VECTORS_FILE_NAME), dict_id).unwrap();
    let grammar: Arc<dyn Grammar> = loaded.grammar;
    let dict: Arc<dyn Dictionary> = if user_dict.is_empty() {
        loaded.dict
    } else {
        let user = UserDictionary::parse(&format!("#ikari-user-dict\tschema=1\n{user_dict}"))
            .unwrap()
            .build(&*grammar)
            .unwrap();
        let mut set = DictionarySet::new();
        set.push(loaded.dict).unwrap();
        set.push(Arc::new(user)).unwrap();
        Arc::new(set)
    };
    // 学習は空（ファイルは保存しない。実機のユーザーデータには触れない）
    let store =
        LearningStore::open(&std::env::temp_dir().join("ikari-left-context-real-learning.tsv"))
            .unwrap();
    store.attach_grammar(Arc::clone(&grammar));
    let kv = BTreeMap::new();
    let times = Arc::new(Mutex::new(Vec::new()));
    let left = LeftContextRanker::new(LeftContextConfig {
        compound_bonus,
        ..LeftContextConfig::from_kv(&kv).unwrap()
    });
    let rankers: Vec<Arc<dyn Ranker>> = vec![
        Arc::new(Timed {
            inner: left,
            times: Arc::clone(&times),
        }),
        Arc::new(KanaStyleRanker::new(
            KanaStyleTable::builtin(),
            KanaStyleConfig::from_kv(&kv).unwrap(),
        )),
        Arc::new(DocumentCacheRanker::new(
            DocumentCacheConfig::from_kv(&kv).unwrap(),
        )),
        Arc::new(LearningRanker::new(
            Arc::new(store),
            Some(Arc::clone(&vectors)),
            LearningConfig::from_kv(&kv).unwrap(),
        )),
        Arc::new(ProfileRanker::new(
            Some(Arc::clone(&vectors)),
            ProfileConfig::from_kv(&kv).unwrap(),
        )),
        Arc::new(TopicRanker::new(
            Arc::clone(&vectors),
            TopicConfig::from_kv(&kv).unwrap(),
        )),
    ];
    let converter = Converter::builder(Arc::clone(&dict), Arc::clone(&grammar))
        .ranker(Arc::new(CompositeRanker::new(rankers)))
        .analyzer(Arc::new(LatticeContextAnalyzer::new(
            Arc::clone(&dict),
            grammar,
        )))
        .build()
        .unwrap();
    // 読みの逆引き表は、エンジンと同じく先に作っておく（最初の 1回の 0.1〜0.3 秒を測定に入れない）
    dict.warm_reading_index();
    Setup { converter, times }
}

fn top1(conv: &Converter, context: &str, reading: &str) -> String {
    let ctx = Context::new().with_left_text(context.to_owned());
    let mut conversion = conv.convert_until(reading, &ctx, None);
    conversion
        .page_until(0, None)
        .map(|p| p.candidates[0].surface.clone())
        .unwrap_or_default()
}

/// (文脈, 読み, 規則ありの 1位, 規則なしの 1位)
const CASES: [(&str, &str, &str, &str); 7] = [
    ("洗濯", "き", "機", "器"),
    ("最新", "ばん", "版", "板"),
    ("自動", "か", "化", "家"),
    ("京都", "し", "市", "史"),
    // 変わらないこと（子供たち と 子供達 は同じ lemma。犬都 は人名）
    ("子供", "たち", "たち", "たち"),
    ("犬", "と", "と", "と"),
    // 左が名詞でない（できる限り を当てない）
    ("できる", "かぎり", "かぎり", "かぎり"),
];

#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary built by ikari-dict-builder)"]
fn real_dictionary_compound_with_the_left_word() {
    let on = converter(LeftContextConfig::default().compound_bonus, "");
    let off = converter(0, "");
    let mut failures = Vec::new();
    for (context, reading, with_rule, without_rule) in CASES {
        let a = top1(&on.converter, context, reading);
        let b = top1(&off.converter, context, reading);
        eprintln!("{context} + {reading}: {b} -> {a}");
        if a != with_rule || b != without_rule {
            failures.push(format!(
                "{context} + {reading}: rule {a} (want {with_rule}), no rule {b} (want {without_rule})"
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary built by ikari-dict-builder)"]
fn real_dictionary_compound_from_the_user_dictionary() {
    // 日本語版 はシステム辞書にない（ADR 0007 §7.3）。ユーザー辞書に 名詞 で登録すると当たる
    let none = converter(LeftContextConfig::default().compound_bonus, "");
    assert_eq!(top1(&none.converter, "日本語", "ばん"), "板");
    let user = converter(
        LeftContextConfig::default().compound_bonus,
        "にほんごばん\t日本語版\t名詞\n",
    );
    assert_eq!(top1(&user.converter, "日本語", "ばん"), "版");
    // 人名で登録した語は当たらない（§12.1 Q4）
    let person = converter(
        LeftContextConfig::default().compound_bonus,
        "にほんごばん\t日本語版\t人名\n",
    );
    assert_eq!(top1(&person.converter, "日本語", "ばん"), "板");
}

/// LeftContextRanker の `rank` 1回の時間（評価セットの文脈つきのケース。ADR 0007 §4）
#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary built by ikari-dict-builder)"]
fn real_dictionary_left_context_rank_latency() {
    let cases = load_context_cases();
    for (label, bonus) in [
        ("compound_bonus 0", 0),
        ("default", LeftContextConfig::default().compound_bonus),
    ] {
        let setup = converter(bonus, "");
        for _ in 0..REPEAT {
            for (context, reading) in &cases {
                top1(&setup.converter, context, reading);
            }
        }
        let mut times = setup.times.lock().unwrap().clone();
        times.sort();
        let at = |q: f64| times[((times.len() - 1) as f64 * q).round() as usize];
        eprintln!(
            "LeftContextRanker::rank ({label}): n {} p50 {:?} p99 {:?} p99.9 {:?} max {:?}",
            times.len(),
            at(0.5),
            at(0.99),
            at(0.999),
            times.last().unwrap()
        );
        assert!(at(0.99) <= RANK_P99_LIMIT, "{label}: p99 {:?}", at(0.99));
    }
}

/// 評価セットのうち、左文脈のあるケースの（文脈, 読み）
fn load_context_cases() -> Vec<(String, String)> {
    let sets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../eval/sets");
    let mut cases = Vec::new();
    for name in SET_FILES {
        let text = std::fs::read_to_string(sets.join(name)).unwrap();
        let mut columns: Option<(usize, usize)> = None;
        for line in text.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            let Some((c, r)) = columns else {
                let find = |key: &str| fields.iter().position(|f| *f == key).unwrap();
                columns = Some((find("context"), find("reading")));
                continue;
            };
            let context = fields.get(c).copied().unwrap_or("");
            if !context.is_empty() {
                cases.push((context.to_owned(), fields[r].to_owned()));
            }
        }
    }
    assert!(
        cases.len() > 500,
        "evaluation sets not found ({})",
        cases.len()
    );
    cases
}
