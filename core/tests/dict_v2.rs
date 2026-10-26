//! 辞書形式 v2 の読み込み（表記の索引、lemma、penalty、活用規則、vectors.bin）のテスト。
//! 辞書は builder で fixture（testdata/sudachi-mini、testdata/chive-mini）から一時ディレクトリに作る。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ikari_core::{
    ConjRule, Dictionary, DictionarySet, Grammar, GrammarFile, SystemDictionary, WordEntry, WordId,
    open_dir, open_vectors,
};
use ikari_dict_builder::{
    BuildConfig, VectorsConfig, build, build_vectors, verify, verify_vectors,
};
use ikari_dict_format::{MAX_READING_CHARS, SYSTEM_DICT_FILE_NAME, VECTORS_FILE_NAME};

const FIXTURE_DIR: &str = "../testdata/sudachi-mini";
const CHIVE_FIXTURE: &str = "../testdata/chive-mini/vectors.txt";
const NOUN: &str = "名詞,普通名詞,一般,*,*,*";
const SHIMO_ICHIDAN_END: &str = "動詞,一般,*,*,下一段-マ行,終止形-一般";
const SHIMO_ICHIDAN_RENYOU: &str = "動詞,一般,*,*,下一段-マ行,連用形-一般";
const SHIMO_ICHIDAN_MIZEN: &str = "動詞,一般,*,*,下一段-マ行,未然形-一般";
const RA_END: &str = "動詞,一般,*,*,五段-ラ行,終止形-一般";
const RA_SOKUON: &str = "動詞,一般,*,*,五段-ラ行,連用形-促音便";

fn manifest(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn build_fixture(out: &Path) {
    let cfg = BuildConfig {
        lex: vec![manifest(FIXTURE_DIR).join("lex.csv")],
        matrix: manifest(FIXTURE_DIR).join("matrix.def"),
        out_dir: out.to_path_buf(),
        unk_pos: NOUN.to_string(),
        unk_cost: 10000,
        label: "sudachi-mini".to_string(),
    };
    build(&cfg).unwrap();
}

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        build_fixture(dir.path());
        Self { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn with_vectors(&self, clusters: u16) -> u64 {
        let mut cfg = VectorsConfig::new(
            manifest(CHIVE_FIXTURE),
            self.path().to_path_buf(),
            self.path().join(VECTORS_FILE_NAME),
        );
        cfg.clusters = clusters;
        build_vectors(&cfg).unwrap().dict_id
    }
}

/// 辞書の全エントリ（語ID の昇順）。
fn all_entries(dict: &SystemDictionary) -> Vec<WordEntry<'_>> {
    (0..dict.entry_count() as u32)
        .map(|index| dict.entry(WordId { dict: 0, index }).unwrap())
        .collect()
}

#[test]
fn builds_and_verifies_with_v2_sections() {
    let fx = Fixture::new();
    let report = verify(fx.path()).unwrap();
    assert!(report.lemmas > 50, "{report:?}");
    assert!(report.surfaces > 50, "{report:?}");
    assert!(report.conj_rules >= 4, "{report:?}");
}

#[test]
fn surface_search_matches_brute_force_over_every_prefix() {
    let fx = Fixture::new();
    let loaded = open_dir(fx.path()).unwrap();
    let dict = &loaded.dict;
    let entries = all_entries(dict);

    // 表記と、その接頭辞・延長・ずらしたものを問い合わせにする
    let mut probes: Vec<String> = Vec::new();
    for e in &entries {
        probes.push(e.surface.to_string());
        probes.push(format!("{}あ", e.surface));
        probes.push(format!("書き進めるのは{}", e.surface));
        if let Some(first) = e.surface.chars().next() {
            probes.push(first.to_string());
        }
    }
    probes.push(String::new());
    probes.push("存在しない表記".to_string());
    probes.push("あ".repeat(MAX_READING_CHARS + 5));
    probes.sort();
    probes.dedup();

    for probe in &probes {
        let mut got: Vec<(usize, WordId)> = Vec::new();
        dict.common_prefix_search_surface(probe, &mut |len, e| got.push((len, e.id)));

        // 総当たり: 表記が probe の接頭辞で、1〜MAX_READING_CHARS 文字
        let mut expected: Vec<(usize, i16, WordId)> = entries
            .iter()
            .filter(|e| {
                let chars = e.surface.chars().count();
                (1..=MAX_READING_CHARS).contains(&chars) && probe.starts_with(e.surface)
            })
            .map(|e| (e.surface.len(), e.cost, e.id))
            .collect();
        // 検索は短い表記から、同じ表記の中では (cost, index) の昇順
        expected.sort_by_key(|&(len, cost, id)| (len, cost, id.index));
        let expected: Vec<(usize, WordId)> =
            expected.into_iter().map(|(len, _, id)| (len, id)).collect();
        assert_eq!(got, expected, "probe {probe:?}");
    }
}

#[test]
fn surface_search_skips_surfaces_longer_than_the_limit() {
    let fx = Fixture::new();
    let loaded = open_dir(fx.path()).unwrap();
    let long = "鬱".repeat(MAX_READING_CHARS + 1);
    let mut hits = 0;
    loaded
        .dict
        .common_prefix_search_surface(&long, &mut |_, _| hits += 1);
    assert_eq!(hits, 0);
}

#[test]
fn lemma_and_dict_id_are_exposed() {
    let fx = Fixture::new();
    let loaded = open_dir(fx.path()).unwrap();
    let dict = &loaded.dict;
    assert!(dict.dict_id().is_some());

    let mut kou = None;
    dict.lookup_exact("こう", &mut |e| {
        if e.surface == "コウ" {
            kou = Some(e.id);
        }
    });
    let lemma = dict.lemma(kou.unwrap()).unwrap();
    assert_eq!(lemma.text, "こう");
    assert!(lemma.id.is_some());

    // lemma 番号は lemma ごとに一意（同じ lemma のエントリは同じ番号）
    let mut ids = std::collections::HashMap::new();
    for e in all_entries(dict) {
        let l = dict.lemma(e.id).unwrap();
        let prev = ids.insert(l.text.to_string(), l.id);
        assert!(prev.is_none() || prev == Some(l.id), "{}", l.text);
    }
    assert!(
        dict.lemma(WordId {
            dict: 0,
            index: u32::MAX
        })
        .is_none()
    );
    assert!(dict.lemma(WordId { dict: 1, index: 0 }).is_none());
}

#[test]
fn policy_penalties_land_in_the_entries() {
    let fx = Fixture::new();
    let loaded = open_dir(fx.path()).unwrap();
    let penalty_of = |reading: &str, surface: &str| {
        let mut found = Vec::new();
        loaded.dict.lookup_exact(reading, &mut |e| {
            if e.surface == surface {
                found.push(e.penalty);
            }
        });
        assert!(!found.is_empty(), "{reading} {surface}");
        found
    };
    // 表記ゆれ（カタカナ）
    assert_eq!(penalty_of("こう", "コウ"), [5000]);
    // 人名: 固有名詞 1000 + 人名 1000（lemma もカタカナなので表記ゆれではない）
    assert_eq!(penalty_of("せい", "セイ"), [2000]);
    // 数詞の数字 / 英小文字は除外せず、大きなペナルティで後ろに下げる（spec の基本原則）
    assert_eq!(penalty_of("よう", "4"), [10000]);
    assert_eq!(penalty_of("へんかん", "hennkann"), [10000]);
    // 英字を含む他の表記は +3000 に固有名詞 +1000
    assert_eq!(penalty_of("あいふぉーん", "iPhone"), [4000]);
    // 通常の語は 0
    assert_eq!(penalty_of("こうせい", "構成"), [0]);
    // 本来カタカナの語は対象外
    assert_eq!(penalty_of("らーめん", "ラーメン"), [0]);
    // 解析用の cost は変わらない
    let mut cost = None;
    loaded.dict.lookup_exact("こう", &mut |e| {
        if e.surface == "コウ" {
            cost = Some(e.cost);
        }
    });
    assert_eq!(cost, Some(2924));
}

#[test]
fn dictionary_set_forwards_surface_lemma_and_dict_id() {
    let fx = Fixture::new();
    let loaded = open_dir(fx.path()).unwrap();
    let mut set = DictionarySet::new();
    set.push(loaded.dict.clone()).unwrap();
    let mut user = ikari_core::InMemoryDictionary::new(set.grammar_id());
    user.insert("ゆーざ", "ユーザ", 1, 1, 100, 0);
    set.push(Arc::new(user)).unwrap();

    assert_eq!(set.dict_id(), loaded.dict.dict_id());

    let mut hits: Vec<(u8, usize)> = Vec::new();
    set.common_prefix_search_surface("ユーザーが", &mut |len, e| hits.push((e.id.dict, len)));
    // システム辞書の「ユーザー」と、ユーザー辞書の「ユーザ」
    assert!(hits.contains(&(0, "ユーザー".len())), "{hits:?}");
    assert!(hits.contains(&(1, "ユーザ".len())), "{hits:?}");

    // lemma 番号は slot 0 のものだけ Some
    let mut system_id = None;
    let mut user_id = None;
    set.common_prefix_search_surface("ユーザ", &mut |_, e| {
        if e.id.dict == 0 {
            system_id = Some(e.id);
        } else {
            user_id = Some(e.id);
        }
    });
    assert!(set.lemma(system_id.unwrap()).unwrap().id.is_some());
    let user_lemma = set.lemma(user_id.unwrap()).unwrap();
    assert_eq!(user_lemma.text, "ユーザ");
    assert!(user_lemma.id.is_none());
    assert!(set.lemma(WordId { dict: 9, index: 0 }).is_none());
}

/// 実際のユーザー辞書（活用の展開あり）を slot 1 に足した構成で、表記、lemma、dict_id、読みの検索を確かめる。
#[test]
fn dictionary_set_with_an_expanded_user_dictionary() {
    use ikari_core::UserDictionary;

    let fx = Fixture::new();
    let loaded = open_dir(fx.path()).unwrap();
    let user_dict = UserDictionary::parse(
        "#ikari-user-dict\tschema=1\nかきすすめる\t書き進める\t動詞:下一段-マ行\t\nゆーざ\tユーザ\t名詞\t\n",
    )
    .unwrap();
    let user = user_dict.build(&*loaded.grammar).unwrap();
    let mut set = DictionarySet::new();
    assert_eq!(set.push(loaded.dict.clone()).unwrap(), 0);
    assert_eq!(set.push(Arc::new(user)).unwrap(), 1);

    // dict_id はシステム辞書のもの（vectors.bin の照合に使う）
    assert_eq!(set.dict_id(), loaded.dict.dict_id());
    assert!(set.dict_id().is_some());

    // 読みの検索: 活用形の読みがユーザー辞書の slot で引ける
    let mut by_reading: Vec<(u8, String)> = Vec::new();
    set.lookup_exact("かきすすめ", &mut |e| {
        by_reading.push((e.id.dict, e.surface.to_string()));
    });
    assert!(
        by_reading.contains(&(1, "書き進め".to_string())),
        "{by_reading:?}"
    );
    let mut prefix: Vec<(u8, usize, String)> = Vec::new();
    set.common_prefix_search("かきすすめるよ", &mut |len, e| {
        prefix.push((e.id.dict, len, e.surface.to_string()));
    });
    assert!(
        prefix.contains(&(1, "かきすすめる".len(), "書き進める".to_string())),
        "{prefix:?}"
    );

    // 表記の検索: 活用形も引け、WordId は slot 1 で entry() に戻せる
    let mut forms: Vec<(usize, WordId, String)> = Vec::new();
    set.common_prefix_search_surface("書き進めた。", &mut |len, e| {
        forms.push((len, e.id, e.surface.to_string()));
    });
    let (len, id, surface) = forms
        .iter()
        .find(|(_, id, s)| id.dict == 1 && s == "書き進め")
        .cloned()
        .unwrap_or_else(|| panic!("no user-dict form: {forms:?}"));
    assert_eq!(len, surface.len());
    assert_eq!(set.entry(id).unwrap().surface, surface);

    // lemma: 活用形でも辞書形。lemma 番号（ベクトル用）はシステム辞書のものだけ
    let lemma = set.lemma(id).unwrap();
    assert_eq!(lemma.text, "書き進める");
    assert!(lemma.id.is_none());

    // システム辞書の語は slot 0 のまま lemma 番号を持つ
    let mut system: Option<WordId> = None;
    set.common_prefix_search_surface("ユーザー", &mut |_, e| {
        if e.id.dict == 0 {
            system = Some(e.id);
        }
    });
    let system_lemma = set.lemma(system.unwrap()).unwrap();
    assert!(system_lemma.id.is_some());
}

fn rule<'a>(g: &'a GrammarFile, base: &str, target: &str) -> Option<&'a ConjRule> {
    let (base, target) = (g.pos_id(base)?, g.pos_id(target)?);
    g.conj_rules()
        .iter()
        .find(|r| r.base_pos == base && r.target_pos == target)
}

/// 規則を終止形に適用する（末尾を `strip_chars` 文字削って足す）。
fn conjugate(rule: &ConjRule, base: &str) -> String {
    let keep = base.chars().count() - usize::from(rule.strip_chars);
    base.chars().take(keep).collect::<String>() + &rule.add_surface
}

#[test]
fn conj_rules_are_derived_from_the_fixture() {
    let fx = Fixture::new();
    let loaded = open_dir(fx.path()).unwrap();
    let g = &loaded.grammar;

    // 下一段-マ行: 書き進める -> 書き進め（連用形・未然形とも末尾の「る」を削るだけ）
    for target in [SHIMO_ICHIDAN_RENYOU, SHIMO_ICHIDAN_MIZEN] {
        let r = rule(g, SHIMO_ICHIDAN_END, target).unwrap_or_else(|| panic!("{target}"));
        assert_eq!(conjugate(r, "書き進める"), "書き進め");
        assert_eq!(conjugate(r, "かきすすめる"), "かきすすめ");
        assert_eq!(r.add_reading, "");
    }
    // 五段-ラ行: 乗る -> 乗っ（促音便）
    let r = rule(g, RA_END, RA_SOKUON).unwrap();
    assert_eq!(conjugate(r, "乗る"), "乗っ");
    assert_eq!(r.add_reading, "っ");
    assert_eq!(conjugate(r, "なる"), "なっ");
    // 規則は (base_pos, target_pos) の昇順で重複しない
    let keys: Vec<(u16, u16)> = g
        .conj_rules()
        .iter()
        .map(|r| (r.base_pos, r.target_pos))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(keys, sorted);
}

#[test]
fn pos_lookup_and_info() {
    let fx = Fixture::new();
    let loaded = open_dir(fx.path()).unwrap();
    let g = &loaded.grammar;
    for id in 0..g.pos_count() as u16 {
        let name = g.pos_name(id).unwrap();
        assert_eq!(g.pos_id(name), Some(id), "{name}");
        let info = g.pos_info(id).unwrap();
        assert!(info.rep_share_permille <= 1000);
    }
    assert_eq!(g.pos_id("存在しない品詞"), None);
    assert_eq!(g.pos_id(""), None);
    assert!(g.pos_info(u16::MAX).is_none());

    let noun = g.pos_info(g.pos_id(NOUN).unwrap()).unwrap();
    assert!(noun.rep_share_permille > 0);
    // 10パーセンタイルは、その品詞のコストの範囲に収まる
    assert!(noun.user_cost > -20000 && noun.user_cost < 20000);
}

#[test]
fn v1_style_old_files_are_rejected() {
    let fx = Fixture::new();
    let path = fx.path().join(SYSTEM_DICT_FILE_NAME);
    let mut bytes = fs::read(&path).unwrap();
    bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
    fs::write(&path, bytes).unwrap();
    assert!(SystemDictionary::open(&path).is_err());
}

#[test]
fn vectors_roundtrip_and_dict_id_check() {
    let fx = Fixture::new();
    let dict_id = fx.with_vectors(5);
    verify_vectors(fx.path()).unwrap();
    let loaded = open_dir(fx.path()).unwrap();
    assert_eq!(loaded.dict.dict_id(), Some(dict_id));

    let vectors = open_vectors(&fx.path().join(VECTORS_FILE_NAME), dict_id).unwrap();
    assert_eq!(vectors.dims(), 8);

    // lemma -> ベクトル。chiVe のテキストの順が行番号（頻度順位）
    let lemma_id = |text: &str| -> Option<u32> {
        let mut found = None;
        for e in all_entries(&loaded.dict) {
            let l = loaded.dict.lemma(e.id).unwrap();
            if l.text == text {
                found = l.id;
            }
        }
        found
    };
    let rank_of = |text: &str| vectors.rank(lemma_id(text).unwrap());
    assert_eq!(rank_of("品詞"), Some(0));
    assert_eq!(rank_of("厚生"), Some(1));
    assert_eq!(rank_of("変換"), Some(5)); // 存在しない語1 は行にならないので、テキストの 6 行目 -> 行 5
    assert_eq!(rank_of("ユーザー"), Some(17));
    // 辞書にあっても chiVe にない lemma は None
    assert!(vectors.rank(lemma_id("為る").unwrap()).is_none());
    assert!(vectors.vector(lemma_id("為る").unwrap()).is_none());
    assert!(vectors.vector(u32::MAX).is_none());

    // i8 に量子化した単位ベクトル（品詞 = [1.0, 0.3, 0 ...] を正規化 -> [0.958, 0.287, 0 ...]）
    let v = vectors.vector(lemma_id("品詞").unwrap()).unwrap();
    assert_eq!(v.len(), 8);
    assert_eq!(&v[..3], &[122, 36, 0]);

    // 同じ話題の語は同じクラスタ、違う話題は違うクラスタ
    let cluster = |text: &str| {
        let v = vectors.vector(lemma_id(text).unwrap()).unwrap();
        let f: Vec<f32> = v.iter().map(|&x| f32::from(x) / 127.0).collect();
        vectors.nearest_cluster(&f).unwrap()
    };
    assert_eq!(cluster("品詞"), cluster("変換"));
    assert_eq!(cluster("品詞"), cluster("ユーザー"));
    assert_eq!(cluster("厚生"), cluster("構成"));
    assert_eq!(cluster("回答"), cluster("校正"));
    assert_eq!(cluster("柿"), cluster("雀"));
    let groups = [
        cluster("品詞"),
        cluster("厚生"),
        cluster("回答"),
        cluster("柿"),
        cluster("書く"),
    ];
    let mut distinct = groups.to_vec();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 5, "{groups:?}");
    // 次元違い・零ベクトルは None
    assert!(vectors.nearest_cluster(&[1.0, 0.0]).is_none());
    assert!(vectors.nearest_cluster(&[0.0; 8]).is_none());

    // 別の辞書（dict_id が違う）では読み込めない
    assert!(open_vectors(&fx.path().join(VECTORS_FILE_NAME), dict_id ^ 1).is_err());
}

#[test]
fn vectors_build_is_deterministic_and_tied_to_the_dictionary() {
    let a = Fixture::new();
    let b = Fixture::new();
    a.with_vectors(5);
    b.with_vectors(5);
    let (da, db) = (
        fs::read(a.path().join(SYSTEM_DICT_FILE_NAME)).unwrap(),
        fs::read(b.path().join(SYSTEM_DICT_FILE_NAME)).unwrap(),
    );
    assert_eq!(da, db);
    assert_eq!(
        fs::read(a.path().join(VECTORS_FILE_NAME)).unwrap(),
        fs::read(b.path().join(VECTORS_FILE_NAME)).unwrap()
    );
    // 辞書を作り直して中身が変われば、古い vectors.bin は検証で弾かれる
    let other = tempfile::tempdir().unwrap();
    let mut cfg = BuildConfig {
        lex: vec![manifest(FIXTURE_DIR).join("lex.csv")],
        matrix: manifest(FIXTURE_DIR).join("matrix.def"),
        out_dir: other.path().to_path_buf(),
        unk_pos: NOUN.to_string(),
        unk_cost: 10000,
        label: "sudachi-mini".to_string(),
    };
    cfg.label = "other".to_string();
    build(&cfg).unwrap();
    // meta だけの違いでは dict_id は変わらない
    let id = |dir: &Path| {
        SystemDictionary::open(&dir.join(SYSTEM_DICT_FILE_NAME))
            .unwrap()
            .dict_id()
    };
    assert_eq!(id(a.path()), id(other.path()));
}

#[test]
fn broken_vectors_files_never_panic() {
    let fx = Fixture::new();
    let dict_id = fx.with_vectors(3);
    let path = fx.path().join(VECTORS_FILE_NAME);
    let bytes = fs::read(&path).unwrap();
    let scratch = fx.path().join("broken.bin");

    let exercise = |data: &[u8]| {
        fs::write(&scratch, data).unwrap();
        if let Ok(v) = open_vectors(&scratch, dict_id) {
            for id in 0..64 {
                let _ = v.vector(id);
                let _ = v.rank(id);
            }
            let _ = v.nearest_cluster(&vec![0.5; v.dims()]);
        }
    };
    for len in 0..bytes.len() {
        exercise(&bytes[..len]);
    }
    for i in 0..bytes.len() {
        for mask in [0x01u8, 0x80, 0xff] {
            let mut b = bytes.clone();
            b[i] ^= mask;
            exercise(&b);
        }
    }
}

#[test]
fn corrupt_v2_dictionary_headers_never_panic() {
    let fx = Fixture::new();
    let path = fx.path().join(SYSTEM_DICT_FILE_NAME);
    let bytes = fs::read(&path).unwrap();
    let scratch = fx.path().join("broken.dict");
    // ヘッダ（176 バイト）の全バイトを反転して開き、エントリと lemma を引く。
    // fst の走査は、セクションがずれた先の内容が fst として壊れていると fst crate 側で落ちうる（既知。
    // 全体の整合は builder の --verify で確かめる）ので、ここでは走査しない。
    for i in 0..ikari_dict_format::DICT_HEADER_LEN {
        for mask in [0x01u8, 0x10, 0x80, 0xff] {
            let mut b = bytes.clone();
            b[i] ^= mask;
            fs::write(&scratch, &b).unwrap();
            if let Ok(d) = SystemDictionary::open(&scratch) {
                for index in 0..8 {
                    let id = WordId { dict: 0, index };
                    let _ = d.lemma(id);
                    let _ = d.entry(id);
                }
            }
        }
    }
    // 切り詰め
    for len in (0..bytes.len()).step_by(97) {
        fs::write(&scratch, &bytes[..len]).unwrap();
        assert!(SystemDictionary::open(&scratch).is_err() || len == bytes.len());
    }
}
