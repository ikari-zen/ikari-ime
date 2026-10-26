//! mmap 辞書の読み込み（SystemDictionary / GrammarFile / DictionarySet / open_dir）のテスト。
//! ADR 0001 §10.1 B。辞書は builder と format crate で一時ディレクトリに作る。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ikari_core::{
    DictError, Dictionary, DictionarySet, Grammar, GrammarFile, SystemDictionary, WordId, open_dir,
};
use ikari_dict_builder::{BuildConfig, build};
use ikari_dict_format::{
    DICT_KIND_SYSTEM, DictEntryInput, DictSpec, FormatError, GRAMMAR_FILE_NAME, GrammarSpec,
    MAX_READING_CHARS, PosRecord, SYSTEM_DICT_FILE_NAME, encode_dict, encode_grammar,
    parse_dict_layout,
};

const FIXTURE_DIR: &str = "../testdata/sudachi-mini";
const NOUN: &str = "名詞,普通名詞,一般,*,*,*";

// ---------------------------------------------------------------------------
// 補助
// ---------------------------------------------------------------------------

/// 外部 crate なしの擬似乱数（xorshift64*）。テストを決定的にする。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURE_DIR)
        .join(name)
}

fn build_fixture(out: &Path) {
    let cfg = BuildConfig {
        lex: vec![fixture_path("lex.csv")],
        matrix: fixture_path("matrix.def"),
        out_dir: out.to_path_buf(),
        unk_pos: NOUN.to_string(),
        unk_cost: 10000,
        label: "sudachi-mini".to_string(),
    };
    build(&cfg).unwrap();
}

#[derive(Clone, Debug)]
struct TestEntry {
    reading: String,
    surface: String,
    left: u16,
    right: u16,
    cost: i16,
    pos: u16,
}

/// 非正方の 3x4 文法（prev_right_size=3, next_left_size=4）と、品詞2つ。
fn write_grammar(dir: &Path, matrix_seed: i16) -> u64 {
    let matrix: Vec<i16> = (0..12).map(|i| matrix_seed + i as i16 * 3 - 7).collect();
    let pos = [
        PosRecord {
            rep_left_id: 1,
            rep_right_id: 2,
            user_cost: 0,
            rep_share_permille: 0,
            name: "助詞,格助詞,*,*,*,*",
        },
        PosRecord {
            rep_left_id: 3,
            rep_right_id: 0,
            user_cost: 0,
            rep_share_permille: 0,
            name: NOUN,
        },
    ];
    let bytes = encode_grammar(&GrammarSpec {
        prev_right_size: 3,
        next_left_size: 4,
        bos_eos_id: 0,
        unk_left_id: 3,
        unk_right_id: 2,
        unk_pos_id: 1,
        unk_cost: 777,
        matrix: &matrix,
        pos: &pos,
        conj: &[],
    })
    .unwrap();
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(GRAMMAR_FILE_NAME), &bytes).unwrap();
    ikari_dict_format::parse_grammar_layout(&bytes)
        .unwrap()
        .grammar_id
}

fn write_dict(dir: &Path, grammar_id: u64, entries: &[TestEntry], meta: &str) {
    let inputs: Vec<DictEntryInput<'_>> = entries
        .iter()
        .map(|e| DictEntryInput {
            reading: &e.reading,
            surface: &e.surface,
            left_id: e.left,
            right_id: e.right,
            cost: e.cost,
            pos_id: e.pos,
            ime_penalty: 0,
            lemma: &e.surface,
        })
        .collect();
    let bytes = encode_dict(&DictSpec {
        grammar_id,
        kind: DICT_KIND_SYSTEM,
        entries: &inputs,
        meta,
    })
    .unwrap();
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(SYSTEM_DICT_FILE_NAME), bytes).unwrap();
}

/// 小さな文字集合から、接頭辞の関係が多く出るように読みを作る。
fn random_entries(rng: &mut Rng, count: usize) -> Vec<TestEntry> {
    const ALPHABET: [&str; 5] = ["あ", "い", "う", "ー", "ゔ"];
    (0..count)
        .map(|n| {
            let len = if n % 25 == 0 {
                MAX_READING_CHARS // 最大長の読みも入れる
            } else {
                1 + rng.below(6)
            };
            let reading: String = (0..len)
                .map(|_| ALPHABET[rng.below(ALPHABET.len())])
                .collect();
            TestEntry {
                reading,
                surface: format!("語{}", rng.below(40)),
                left: rng.below(4) as u16,
                right: rng.below(3) as u16,
                cost: rng.below(2000) as i16 - 1000,
                pos: rng.below(2) as u16,
            }
        })
        .collect()
}

type Hit = (usize, String, u16, u16, i16, u16);

fn collect_prefix(dict: &dyn Dictionary, key: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    dict.common_prefix_search(key, &mut |len, e| {
        hits.push((
            len,
            e.surface.to_string(),
            e.left_id,
            e.right_id,
            e.cost,
            e.pos_id,
        ));
    });
    hits
}

/// 全数探索: key の各文字境界の接頭辞について、読みが一致するエントリを格納順（コスト昇順）に並べる。
fn brute_force(entries: &[TestEntry], key: &str) -> Vec<Hit> {
    let mut sorted: Vec<&TestEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| {
        (&a.reading, a.cost, &a.surface, a.left, a.right, a.pos)
            .cmp(&(&b.reading, b.cost, &b.surface, b.left, b.right, b.pos))
    });
    let mut hits = Vec::new();
    for (chars, (start, ch)) in key.char_indices().enumerate() {
        if chars >= MAX_READING_CHARS {
            break;
        }
        let end = start + ch.len_utf8();
        for e in sorted.iter().filter(|e| e.reading == key[..end]) {
            hits.push((end, e.surface.clone(), e.left, e.right, e.cost, e.pos));
        }
    }
    hits
}

// ---------------------------------------------------------------------------
// common_prefix_search
// ---------------------------------------------------------------------------

#[test]
fn common_prefix_search_matches_brute_force_on_random_dictionary() {
    let dir = tempfile::tempdir().unwrap();
    let gid = write_grammar(dir.path(), 0);
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let entries = random_entries(&mut rng, 600);
    write_dict(dir.path(), gid, &entries, "source=random\n");

    let loaded = open_dir(dir.path()).unwrap();
    assert_eq!(loaded.dict.entry_count(), 600);
    assert_eq!(loaded.dict.meta(), "source=random\n");

    let mut keys: Vec<String> = entries.iter().map(|e| e.reading.clone()).collect();
    // 登録済みの読みの続き、まったく別の文字、空、長すぎるキーも試す
    keys.extend(
        entries
            .iter()
            .take(80)
            .map(|e| format!("{}あいう", e.reading)),
    );
    keys.extend(
        [
            "",
            "x",
            "あ",
            "ああああああ",
            "ひんし",
            "あ".repeat(40).as_str(),
            "ー".repeat(33).as_str(),
        ]
        .map(String::from),
    );
    for _ in 0..200 {
        let len = 1 + rng.below(40);
        keys.push(
            (0..len)
                .map(|_| ["あ", "い", "う", "ー", "ゔ", "漢"][rng.below(6)])
                .collect(),
        );
    }
    let mut total_hits = 0;
    for key in &keys {
        let got = collect_prefix(&*loaded.dict, key);
        let expected = brute_force(&entries, key);
        assert_eq!(got, expected, "key {key:?}");
        total_hits += got.len();
    }
    assert!(
        total_hits > 1000,
        "the test should exercise many hits ({total_hits})"
    );
}

#[test]
fn prefix_search_reports_byte_lengths_and_cost_order() {
    let dir = tempfile::tempdir().unwrap();
    let gid = write_grammar(dir.path(), 0);
    let e = |reading: &str, surface: &str, cost: i16| TestEntry {
        reading: reading.into(),
        surface: surface.into(),
        left: 1,
        right: 1,
        cost,
        pos: 0,
    };
    let entries = vec![
        e("ひ", "日", 10),
        e("ひん", "品", 20),
        e("ひんし", "瀕死", 300),
        e("ひんし", "品詞", 100),
        e("ひんし", "浜死", 200),
    ];
    write_dict(dir.path(), gid, &entries, "");
    let dict = open_dir(dir.path()).unwrap().dict;
    let hits = collect_prefix(&*dict, "ひんしのこうせい");
    let summary: Vec<(usize, &str)> = hits.iter().map(|h| (h.0, h.1.as_str())).collect();
    // 長さは UTF-8 のバイト数（ひらがな1文字 = 3バイト）。同じ読みの中ではコスト昇順
    assert_eq!(
        summary,
        vec![(3, "日"), (6, "品"), (9, "品詞"), (9, "浜死"), (9, "瀕死")]
    );
}

#[test]
fn search_stops_at_max_reading_chars() {
    let dir = tempfile::tempdir().unwrap();
    let gid = write_grammar(dir.path(), 0);
    let at_max = "あ".repeat(MAX_READING_CHARS);
    let entries = vec![TestEntry {
        reading: at_max.clone(),
        surface: "最大".into(),
        left: 0,
        right: 0,
        cost: 0,
        pos: 0,
    }];
    write_dict(dir.path(), gid, &entries, "");
    let dict = open_dir(dir.path()).unwrap().dict;
    let key = format!("{at_max}あああ");
    let hits = collect_prefix(&*dict, &key);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, MAX_READING_CHARS * 3);
}

#[test]
fn fixture_prefix_search_agrees_with_lookup_exact() {
    let dir = tempfile::tempdir().unwrap();
    build_fixture(dir.path());
    let dict = open_dir(dir.path()).unwrap().dict;

    let keys = [
        "ひんし",
        "ひんしのこうせい",
        "こうせいをかいとう",
        "しようしよう",
        "へんかんする",
        "かきすすめる",
        "このひんしはなった",
        "ひんしひんしひんし",
        "なったのは",
        "xひんし",
        "",
    ];
    for key in keys {
        let got = collect_prefix(&*dict, key);
        // 各接頭辞を lookup_exact で引いた結果を連結したものと一致する
        let mut expected = Vec::new();
        for (start, ch) in key.char_indices() {
            let end = start + ch.len_utf8();
            dict.lookup_exact(&key[..end], &mut |e| {
                expected.push((
                    end,
                    e.surface.to_string(),
                    e.left_id,
                    e.right_id,
                    e.cost,
                    e.pos_id,
                ));
            });
        }
        assert_eq!(got, expected, "key {key:?}");
    }
    assert!(!collect_prefix(&*dict, "ひんしのこうせい").is_empty());
}

// ---------------------------------------------------------------------------
// 実体: fixture から作った辞書
// ---------------------------------------------------------------------------

#[test]
fn fixture_dictionary_finds_hinshi_with_part_of_speech() {
    let dir = tempfile::tempdir().unwrap();
    build_fixture(dir.path());
    let loaded = open_dir(dir.path()).unwrap();

    let mut hits = Vec::new();
    loaded.dict.lookup_exact("ひんし", &mut |e| {
        hits.push((
            e.surface.to_string(),
            e.cost,
            loaded.grammar.pos_name(e.pos_id).map(String::from),
        ));
    });
    let find = |s: &str| {
        hits.iter()
            .find(|h| h.0 == s)
            .unwrap_or_else(|| panic!("{s} not found: {hits:?}"))
    };
    assert_eq!(find("品詞").1, 7392);
    assert_eq!(find("瀕死").1, 7403);
    assert_eq!(find("品詞").2.as_deref(), Some(NOUN));
    // コスト昇順
    assert!(hits.windows(2).all(|w| w[0].1 <= w[1].1), "{hits:?}");

    // カタカナで引いても何も出ない（クエリ側で normalize_key を通す契約）
    let mut katakana = 0;
    loaded.dict.lookup_exact("ヒンシ", &mut |_| katakana += 1);
    assert_eq!(katakana, 0);
    assert!(loaded.dict.meta().contains("source=sudachi-mini"));
}

#[test]
fn entry_roundtrips_ids_and_rejects_bad_ids() {
    let dir = tempfile::tempdir().unwrap();
    build_fixture(dir.path());
    let dict = open_dir(dir.path()).unwrap().dict;

    let mut ids = Vec::new();
    dict.common_prefix_search("ひんしのこうせい", &mut |_, e| {
        ids.push((e.id, e.surface.to_string(), e.cost))
    });
    assert!(!ids.is_empty());
    for (id, surface, cost) in ids {
        assert_eq!(id.dict, 0);
        let e = dict.entry(id).unwrap();
        assert_eq!((e.id, e.surface, e.cost), (id, surface.as_str(), cost));
    }
    let n = dict.entry_count() as u32;
    assert!(
        dict.entry(WordId {
            dict: 0,
            index: n - 1
        })
        .is_some()
    );
    assert!(dict.entry(WordId { dict: 0, index: n }).is_none());
    assert!(
        dict.entry(WordId {
            dict: 0,
            index: u32::MAX
        })
        .is_none()
    );
    assert!(dict.entry(WordId { dict: 1, index: 0 }).is_none());
}

#[test]
fn builds_are_deterministic() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    build_fixture(a.path());
    build_fixture(b.path());
    for name in [GRAMMAR_FILE_NAME, SYSTEM_DICT_FILE_NAME] {
        assert_eq!(
            fs::read(a.path().join(name)).unwrap(),
            fs::read(b.path().join(name)).unwrap(),
            "{name}"
        );
    }
}

// ---------------------------------------------------------------------------
// GrammarFile
// ---------------------------------------------------------------------------

#[test]
fn grammar_file_exposes_matrix_pos_and_unknown_params() {
    let dir = tempfile::tempdir().unwrap();
    let gid = write_grammar(dir.path(), 100);
    let grammar = GrammarFile::open(&dir.path().join(GRAMMAR_FILE_NAME)).unwrap();
    assert_eq!(grammar.grammar_id(), gid);
    assert_eq!(grammar.bos_eos_id(), 0);
    assert_eq!(grammar.pos_count(), 2);

    // matrix[next_left * 3 + prev_right] = 100 + idx*3 - 7。非正方なので転置すると値が合わない
    for next_left in 0..4u16 {
        for prev_right in 0..3u16 {
            let idx = i16::try_from(next_left * 3 + prev_right).unwrap();
            assert_eq!(
                grammar.connection_cost(prev_right, next_left),
                100 + idx * 3 - 7
            );
        }
    }
    // 範囲外は i16::MAX（パニックしない）
    assert_eq!(grammar.connection_cost(3, 0), i16::MAX);
    assert_eq!(grammar.connection_cost(0, 4), i16::MAX);
    assert_eq!(grammar.connection_cost(u16::MAX, u16::MAX), i16::MAX);

    assert_eq!(grammar.pos_name(0), Some("助詞,格助詞,*,*,*,*"));
    assert_eq!(grammar.pos_name(1), Some(NOUN));
    assert_eq!(grammar.pos_name(2), None);
    assert_eq!(grammar.pos_name(u16::MAX), None);

    let unk = grammar.unknown();
    assert_eq!(
        (unk.left_id, unk.right_id, unk.pos_id, unk.cost_per_char),
        (3, 2, 1, 777)
    );
}

// ---------------------------------------------------------------------------
// エラー: マジック、バージョン、切り詰め、grammar_id
// ---------------------------------------------------------------------------

fn format_error(result: Result<impl Sized, DictError>) -> FormatError {
    match result {
        Err(DictError::Format(e)) => e,
        Err(other) => panic!("expected a format error, got {other}"),
        Ok(_) => panic!("expected an error"),
    }
}

#[test]
fn broken_files_are_errors_not_panics() {
    let dir = tempfile::tempdir().unwrap();
    let gid = write_grammar(dir.path(), 0);
    let mut rng = Rng(7);
    write_dict(dir.path(), gid, &random_entries(&mut rng, 50), "m=1\n");
    let dict_path = dir.path().join(SYSTEM_DICT_FILE_NAME);
    let grammar_path = dir.path().join(GRAMMAR_FILE_NAME);
    let dict_bytes = fs::read(&dict_path).unwrap();
    let grammar_bytes = fs::read(&grammar_path).unwrap();

    let work = tempfile::tempdir().unwrap();
    let dict_copy = work.path().join("d.dict");
    let grammar_copy = work.path().join("g.bin");

    // マジック違い
    let mut b = dict_bytes.clone();
    b[0] = b'X';
    fs::write(&dict_copy, &b).unwrap();
    assert!(matches!(
        format_error(SystemDictionary::open(&dict_copy)),
        FormatError::BadMagic { .. }
    ));
    let mut b = grammar_bytes.clone();
    b[7] = b'X';
    fs::write(&grammar_copy, &b).unwrap();
    assert!(matches!(
        format_error(GrammarFile::open(&grammar_copy)),
        FormatError::BadMagic { .. }
    ));

    // grammar.bin を system.dict として開く
    assert!(matches!(
        format_error(SystemDictionary::open(&grammar_path)),
        FormatError::BadMagic { .. }
    ));

    // バージョン違い
    let mut b = dict_bytes.clone();
    b[8..12].copy_from_slice(&1u32.to_le_bytes());
    fs::write(&dict_copy, &b).unwrap();
    assert!(matches!(
        format_error(SystemDictionary::open(&dict_copy)),
        FormatError::UnsupportedVersion { found: 1, .. }
    ));
    let mut b = grammar_bytes.clone();
    b[8..12].copy_from_slice(&0u32.to_le_bytes());
    fs::write(&grammar_copy, &b).unwrap();
    assert!(matches!(
        format_error(GrammarFile::open(&grammar_copy)),
        FormatError::UnsupportedVersion { found: 0, .. }
    ));

    // 切り詰め（使う長さより短いものはすべてエラー）
    let used = parse_dict_layout(&dict_bytes).unwrap().meta.end;
    for len in (0..used).step_by(7).chain([used - 1]) {
        fs::write(&dict_copy, &dict_bytes[..len]).unwrap();
        assert!(
            SystemDictionary::open(&dict_copy).is_err(),
            "dict len {len}"
        );
    }
    for len in (0..grammar_bytes.len() - 8).step_by(5) {
        fs::write(&grammar_copy, &grammar_bytes[..len]).unwrap();
        assert!(
            GrammarFile::open(&grammar_copy).is_err(),
            "grammar len {len}"
        );
    }

    // 範囲外のオフセット（strings_offset = 巨大）
    let mut b = dict_bytes.clone();
    b[64..72].copy_from_slice(&(u64::MAX - 7).to_le_bytes());
    fs::write(&dict_copy, &b).unwrap();
    assert!(SystemDictionary::open(&dict_copy).is_err());

    // 存在しないファイルは Io
    assert!(matches!(
        SystemDictionary::open(&work.path().join("nope")),
        Err(DictError::Io(_))
    ));
    assert!(matches!(open_dir(work.path()), Err(DictError::Io(_))));
    // 空ファイルは形式エラー（mmap できない環境でもパニックしない）
    fs::write(&dict_copy, b"").unwrap();
    assert!(matches!(
        format_error(SystemDictionary::open(&dict_copy)),
        FormatError::Truncated { .. }
    ));
}

#[test]
fn open_dir_rejects_mismatched_grammar_id() {
    let dir = tempfile::tempdir().unwrap();
    let gid = write_grammar(dir.path(), 0);
    let mut rng = Rng(11);
    write_dict(dir.path(), gid, &random_entries(&mut rng, 20), "");
    open_dir(dir.path()).unwrap();

    // 別の連接表（seed が違う）の grammar.bin に差し替える
    let other = tempfile::tempdir().unwrap();
    let other_gid = write_grammar(other.path(), 5);
    assert_ne!(gid, other_gid);
    fs::copy(
        other.path().join(GRAMMAR_FILE_NAME),
        dir.path().join(GRAMMAR_FILE_NAME),
    )
    .unwrap();
    match open_dir(dir.path()) {
        Err(DictError::GrammarMismatch { dict, grammar }) => {
            assert_eq!((dict, grammar), (gid, other_gid));
        }
        Err(other) => panic!("unexpected error {other}"),
        Ok(_) => panic!("mismatch must be rejected"),
    }
}

/// 壊れたエントリ（範囲外の表記位置）は飛ばし、ほかのエントリは返す。
#[test]
fn corrupt_entries_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let gid = write_grammar(dir.path(), 0);
    let e = |surface: &str, cost: i16| TestEntry {
        reading: "ひんし".into(),
        surface: surface.into(),
        left: 1,
        right: 1,
        cost,
        pos: 0,
    };
    write_dict(
        dir.path(),
        gid,
        &[e("品詞", 1), e("瀕死", 2), e("浜死", 3)],
        "",
    );
    let path = dir.path().join(SYSTEM_DICT_FILE_NAME);
    let mut bytes = fs::read(&path).unwrap();
    let layout = parse_dict_layout(&bytes).unwrap();
    // 2番目のエントリの surface_offset を範囲外に、3番目の surface_len を文字の途中にする
    let second = layout.entries.start + 16;
    bytes[second..second + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    let third = layout.entries.start + 32;
    bytes[third + 4..third + 6].copy_from_slice(&2u16.to_le_bytes());
    fs::write(&path, &bytes).unwrap();

    let dict = SystemDictionary::open(&path).unwrap();
    let mut seen = Vec::new();
    dict.lookup_exact("ひんし", &mut |e| seen.push(e.surface.to_string()));
    assert_eq!(seen, vec!["品詞"]);
    assert_eq!(collect_prefix(&dict, "ひんし").len(), 1);
}

/// ヘッダ・エントリ・文字列部のバイト破壊で、open/検索がパニックしない（索引部は fst 任せ）。
#[test]
fn byte_flips_outside_index_never_panic() {
    let dir = tempfile::tempdir().unwrap();
    let gid = write_grammar(dir.path(), 0);
    let mut rng = Rng(99);
    write_dict(dir.path(), gid, &random_entries(&mut rng, 40), "m=1\n");
    let path = dir.path().join(SYSTEM_DICT_FILE_NAME);
    let original = fs::read(&path).unwrap();
    let layout = parse_dict_layout(&original).unwrap();

    let work = tempfile::tempdir().unwrap();
    let copy = work.path().join("d.dict");
    let targets: Vec<usize> = (0..layout.index.start)
        .chain(layout.entries.clone())
        .chain(layout.strings.clone())
        .collect();
    for pos in targets {
        let mut b = original.clone();
        b[pos] ^= 0xa5;
        fs::write(&copy, &b).unwrap();
        if let Ok(dict) = SystemDictionary::open(&copy) {
            for key in ["あ", "あいう", "ううううう", "ー"] {
                let _ = collect_prefix(&dict, key);
                dict.lookup_exact(key, &mut |_| {});
            }
            let _ = dict.entry(WordId { dict: 0, index: 3 });
            let _ = dict.meta();
        }
    }
}

// ---------------------------------------------------------------------------
// DictionarySet
// ---------------------------------------------------------------------------

fn open_pair(
    entries_a: &[TestEntry],
    entries_b: &[TestEntry],
) -> (
    tempfile::TempDir,
    Arc<SystemDictionary>,
    Arc<SystemDictionary>,
) {
    let dir = tempfile::tempdir().unwrap();
    let gid = write_grammar(dir.path(), 0);
    write_dict(&dir.path().join("a"), gid, entries_a, "name=a\n");
    write_dict(&dir.path().join("b"), gid, entries_b, "name=b\n");
    let a = SystemDictionary::open(&dir.path().join("a").join(SYSTEM_DICT_FILE_NAME)).unwrap();
    let b = SystemDictionary::open(&dir.path().join("b").join(SYSTEM_DICT_FILE_NAME)).unwrap();
    (dir, Arc::new(a), Arc::new(b))
}

fn te(reading: &str, surface: &str, cost: i16) -> TestEntry {
    TestEntry {
        reading: reading.into(),
        surface: surface.into(),
        left: 1,
        right: 1,
        cost,
        pos: 0,
    }
}

#[test]
fn dictionary_set_merges_results_and_relabels_word_ids() {
    let (_dir, a, b) = open_pair(
        &[te("ひんし", "品詞", 10), te("ひん", "品", 5)],
        &[te("ひんし", "瀕死", 20), te("ひ", "日", 1)],
    );
    let mut set = DictionarySet::new();
    assert!(set.is_empty());
    assert_eq!(set.push(a.clone()).unwrap(), 0);
    assert_eq!(set.push(b.clone()).unwrap(), 1);
    assert_eq!(set.len(), 2);
    assert_eq!(set.grammar_id(), a.grammar_id());

    let mut hits = Vec::new();
    set.common_prefix_search("ひんし", &mut |len, e| {
        hits.push((len, e.id, e.surface.to_string()))
    });
    // 辞書 a の結果、続いて辞書 b の結果
    let summary: Vec<(usize, u8, &str)> =
        hits.iter().map(|h| (h.0, h.1.dict, h.2.as_str())).collect();
    assert_eq!(
        summary,
        vec![(6, 0, "品"), (9, 0, "品詞"), (3, 1, "日"), (9, 1, "瀕死")]
    );

    let mut exact = Vec::new();
    set.lookup_exact("ひんし", &mut |e| {
        exact.push((e.id.dict, e.surface.to_string()))
    });
    assert_eq!(
        exact,
        vec![(0, "品詞".to_string()), (1, "瀕死".to_string())]
    );

    // WordId で引き直せる
    for (_, id, surface) in &hits {
        assert_eq!(set.entry(*id).unwrap().surface, surface);
        assert_eq!(set.entry(*id).unwrap().id, *id);
    }
    assert!(set.entry(WordId { dict: 2, index: 0 }).is_none());
    assert!(set.entry(WordId { dict: 1, index: 99 }).is_none());
    assert!(
        set.entry(WordId {
            dict: 255,
            index: 0
        })
        .is_none()
    );
}

#[test]
fn dictionary_set_rejects_different_grammar() {
    let (_dir, a, _b) = open_pair(&[te("あ", "亜", 1)], &[te("い", "伊", 1)]);
    let other_dir = tempfile::tempdir().unwrap();
    let other_gid = write_grammar(other_dir.path(), 9);
    write_dict(other_dir.path(), other_gid, &[te("う", "宇", 1)], "");
    let other =
        Arc::new(SystemDictionary::open(&other_dir.path().join(SYSTEM_DICT_FILE_NAME)).unwrap());

    let mut set = DictionarySet::new();
    set.push(a.clone()).unwrap();
    match set.push(other.clone()) {
        Err(DictError::GrammarMismatch { dict, grammar }) => {
            assert_eq!(dict, other.grammar_id());
            assert_eq!(grammar, a.grammar_id());
        }
        Err(e) => panic!("unexpected error {e}"),
        Ok(id) => panic!("must be rejected, got id {id}"),
    }
    assert_eq!(set.len(), 1); // 失敗しても状態は変わらない
}

#[test]
fn dictionary_set_limits_to_256_dictionaries() {
    let (_dir, a, _b) = open_pair(&[te("あ", "亜", 1)], &[te("い", "伊", 1)]);
    let mut set = DictionarySet::default();
    for expected in 0..=u8::MAX {
        assert_eq!(set.push(a.clone()).unwrap(), expected);
    }
    assert!(set.push(a.clone()).is_err());
    assert_eq!(set.len(), 256);
    assert_eq!(
        set.entry(WordId {
            dict: 255,
            index: 0
        })
        .unwrap()
        .surface,
        "亜"
    );
}

#[test]
fn empty_dictionary_set_finds_nothing() {
    let set = DictionarySet::new();
    assert!(collect_prefix(&set, "あいう").is_empty());
    set.lookup_exact("あ", &mut |_| panic!("no entries expected"));
    assert!(set.entry(WordId { dict: 0, index: 0 }).is_none());
    assert_eq!(set.grammar_id(), 0);
}

#[test]
fn dictionaries_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SystemDictionary>();
    assert_send_sync::<GrammarFile>();
    assert_send_sync::<DictionarySet>();

    // 複数スレッドから同時に引ける
    let (_dir, a, _b) = open_pair(&[te("ひんし", "品詞", 1)], &[te("あ", "亜", 1)]);
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let dict = Arc::clone(&a);
            std::thread::spawn(move || {
                (0..200)
                    .map(|_| collect_prefix(&*dict, "ひんし").len())
                    .sum::<usize>()
            })
        })
        .collect();
    for h in handles {
        assert_eq!(h.join().unwrap(), 200);
    }
}

// ---------------------------------------------------------------------------
// reading（読みの逆引き。ADR 0006 §9.3）
// ---------------------------------------------------------------------------

/// 不変条件: 全エントリで、`lookup_exact(reading(id))` が id を含む。読みは格納した読みと一致する
fn assert_reading_roundtrip(dict: &dyn Dictionary, count: u32) {
    for index in 0..count {
        let id = WordId { dict: 0, index };
        let reading = dict
            .reading(id)
            .unwrap_or_else(|| panic!("no reading for entry {index}"))
            .into_owned();
        let mut found = false;
        dict.lookup_exact(&reading, &mut |e| found |= e.id == id);
        assert!(found, "entry {index} not found by its reading {reading:?}");
    }
}

#[test]
fn reading_inverts_lookup_exact_on_random_dictionary() {
    let dir = tempfile::tempdir().unwrap();
    let gid = write_grammar(dir.path(), 0);
    let mut rng = Rng(0x0bad_cafe_1234_5678);
    let entries = random_entries(&mut rng, 800);
    write_dict(dir.path(), gid, &entries, "source=random\n");
    let loaded = open_dir(dir.path()).unwrap();
    assert_reading_roundtrip(&*loaded.dict, 800);
    // 格納した順（コスト昇順など）は builder が決めるので、読みの多重集合が一致することを見る
    let mut stored: Vec<String> = entries.iter().map(|e| e.reading.clone()).collect();
    let mut got: Vec<String> = (0..800)
        .map(|index| {
            loaded
                .dict
                .reading(WordId { dict: 0, index })
                .unwrap()
                .into_owned()
        })
        .collect();
    stored.sort();
    got.sort();
    assert_eq!(stored, got);
}

#[test]
fn reading_inverts_lookup_exact_on_fixture_dictionary() {
    let dir = tempfile::tempdir().unwrap();
    build_fixture(dir.path());
    let loaded = open_dir(dir.path()).unwrap();
    let count = u32::try_from(loaded.dict.entry_count()).unwrap();
    assert!(count > 0);
    assert_reading_roundtrip(&*loaded.dict, count);
}

#[test]
fn reading_rejects_bad_ids_and_warming_is_idempotent() {
    let (_dir, a, _b) = open_pair(&[te("ひんし", "品詞", 10), te("ひ", "日", 1)], &[]);
    a.warm_reading_index();
    a.warm_reading_index();
    let count = u32::try_from(a.entry_count()).unwrap();
    assert!(
        a.reading(WordId {
            dict: 0,
            index: count
        })
        .is_none()
    );
    assert!(a.reading(WordId { dict: 1, index: 0 }).is_none());
    assert!(
        a.reading(WordId {
            dict: 0,
            index: u32::MAX
        })
        .is_none()
    );
}

#[test]
fn prefault_leaves_lookups_unchanged() {
    let (_dir, a, _b) = open_pair(&[te("ひんし", "品詞", 10), te("ひ", "日", 1)], &[]);
    let mut before = Vec::new();
    a.lookup_exact("ひんし", &mut |e| before.push(e.id));
    a.prefault();
    a.prefault();
    let mut after = Vec::new();
    a.lookup_exact("ひんし", &mut |e| after.push(e.id));
    assert!(!before.is_empty());
    assert_eq!(before, after);
}

#[test]
fn dictionary_set_reads_the_reading_of_each_member() {
    let (_dir, a, b) = open_pair(
        &[te("ひんし", "品詞", 10)],
        &[te("ひんし", "瀕死", 20), te("ひ", "日", 1)],
    );
    let mut set = DictionarySet::new();
    set.push(a).unwrap();
    set.push(b).unwrap();
    set.warm_reading_index();
    let mut checked = 0;
    set.common_prefix_search("ひんし", &mut |_, e| {
        let reading = set.reading(e.id).unwrap();
        let mut found = false;
        set.lookup_exact(&reading, &mut |x| found |= x.id == e.id);
        assert!(found);
        checked += 1;
    });
    assert_eq!(checked, 3);
    assert!(set.reading(WordId { dict: 2, index: 0 }).is_none());
    assert!(set.reading(WordId { dict: 1, index: 99 }).is_none());
}
