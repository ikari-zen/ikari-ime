use ikari_core::{Context, DenseGrammar, InMemoryDictionary, UnknownParams};

use super::*;

mod postal;

fn mock() -> (Arc<dyn Dictionary>, Arc<dyn Grammar>) {
    let mut g = DenseGrammar::new(
        2,
        2,
        0,
        UnknownParams {
            left_id: 1,
            right_id: 1,
            pos_id: 0,
            cost_per_char: 5000,
        },
    );
    g.add_pos("名詞,普通名詞,一般,*,*,*");
    let mut d = InMemoryDictionary::new(g.grammar_id());
    d.insert("ひんし", "品詞", 1, 1, 100, 0);
    d.insert("ひんし", "瀕死", 1, 1, 200, 0);
    (Arc::new(d), Arc::new(g))
}

fn eval_opts(ranker: RankerSet) -> StackOptions {
    StackOptions {
        ranker,
        mode: AssemblyMode::Eval,
        ..Default::default()
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ikari-stack-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn ranker_set_parses_and_prints() {
    for s in ["cost", "analyzer", "doc", "topic", "full"] {
        assert_eq!(RankerSet::parse(s).unwrap().as_str(), s);
    }
    assert_eq!(RankerSet::parse("x"), None);
    for set in RankerSet::ALL {
        assert_eq!(RankerSet::parse(set.as_str()), Some(set));
    }
}

#[test]
fn ranker_config_reads_lines() {
    let kv =
        parse_ranker_config("# c\n\nsurface_bonus = 1200\nzenz.alpha=0.5\n  k=v=w \n").unwrap();
    assert_eq!(kv["surface_bonus"], "1200");
    assert_eq!(kv["zenz.alpha"], "0.5");
    assert_eq!(kv["k"], "v=w");
    assert!(parse_ranker_config("novalue\n").is_err());
    assert!(parse_ranker_config("=1\n").is_err());
    assert!(parse_ranker_config("").unwrap().is_empty());
}

#[test]
fn unreadable_ranker_config_is_a_config_error() {
    let e = read_ranker_config(Path::new("/nonexistent/ranker.conf")).unwrap_err();
    assert!(
        matches!(&e, StackError::Config(m) if m.contains("cannot read")),
        "{e}"
    );
}

#[test]
fn lm_overrides_win_over_the_config_file() {
    let mut kv = BTreeMap::from([
        ("zenz.budget_ms".to_owned(), "10".to_owned()),
        ("zenz.alpha".to_owned(), "0.5".to_owned()),
    ]);
    LmOverrides {
        budget_ms: Some(300),
        idle_unload_secs: Some(60),
        idle_unload_gpu_secs: Some(1800),
        gpu: Some(false),
    }
    .apply(&mut kv);
    assert_eq!(kv["zenz.budget_ms"], "300");
    assert_eq!(kv["zenz.idle_unload_secs"], "60");
    assert_eq!(kv["zenz.gpu"], "off");
    assert_eq!(kv["zenz.idle_unload_gpu_secs"], "1800");
    assert_eq!(kv["zenz.alpha"], "0.5");
    let mut untouched = kv.clone();
    LmOverrides::default().apply(&mut untouched);
    assert_eq!(untouched, kv);
}

#[test]
fn eval_cost_is_the_phase0_converter() {
    let (d, g) = mock();
    let stack = assemble(
        StackInputs::new(d, g),
        &StackPaths::default(),
        &eval_opts(RankerSet::Cost),
    )
    .unwrap();
    assert!(stack.meta.description.contains("ranker=cost"));
    assert!(stack.learning.is_none() && stack.lm.is_none());
    let mut conv = stack.converter.convert("ひんし", &Context::new());
    assert_eq!(conv.page(0).unwrap().candidates[0].surface, "品詞");
}

#[test]
fn eval_cost_rejects_an_lm() {
    let (d, g) = mock();
    let paths = StackPaths {
        lm_model: Some(PathBuf::from("/nonexistent.gguf")),
        ..Default::default()
    };
    let e = assemble(StackInputs::new(d, g), &paths, &eval_opts(RankerSet::Cost))
        .err()
        .expect("must fail");
    assert!(e.to_string().contains("other than cost"), "{e}");
}

#[test]
fn eval_full_keeps_the_ranker_order() {
    let (d, g) = mock();
    let dir = temp_dir("eval-full");
    let paths = StackPaths {
        user_dir: Some(dir.clone()),
        ..Default::default()
    };
    let stack = assemble(StackInputs::new(d, g), &paths, &eval_opts(RankerSet::Full)).unwrap();
    assert!(
        stack
            .meta
            .description
            .contains("ranker=full(left_context,kana_style,doc_cache,learning,profile"),
        "{}",
        stack.meta.description
    );
    assert!(stack.learning.is_some());
    let mut conv = stack.converter.convert("ひんし", &Context::new());
    assert!(conv.page(0).is_some());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn runtime_without_an_index_skips_the_analyzer_rankers() {
    // モックの辞書は dict_id を持たない（表記の索引なし）
    let (d, g) = mock();
    let stack = assemble(
        StackInputs::new(d, g),
        &StackPaths::default(),
        &StackOptions::default(),
    )
    .unwrap();
    assert!(
        stack.meta.description.contains("analyzer=null"),
        "{}",
        stack.meta.description
    );
    assert!(stack.learning.is_none());
    let mut conv = stack.converter.convert("ひんし", &Context::new());
    assert_eq!(conv.page(0).unwrap().candidates[0].surface, "品詞");
}

#[test]
fn runtime_adds_learning_only_when_wanted() {
    let dir = temp_dir("runtime-learning");
    let paths = StackPaths {
        user_dir: Some(dir.clone()),
        ..Default::default()
    };
    let (d, g) = mock();
    let plain = assemble(
        StackInputs::new(d.clone(), g.clone()),
        &paths,
        &StackOptions::default(),
    )
    .unwrap();
    assert!(plain.learning.is_some(), "Full always opens the store");
    assert!(!plain.meta.description.contains("learning"));
    let always = assemble(
        StackInputs::new(d, g),
        &paths,
        &StackOptions {
            always_learning_ranker: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(always.meta.description.contains("learning"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn runtime_quarantines_a_broken_learning_file_and_starts_empty() {
    let dir = temp_dir("quarantine");
    let tsv = dir.join("learning.tsv");
    std::fs::write(&tsv, "broken\n").unwrap();
    let paths = StackPaths {
        user_dir: Some(dir.clone()),
        ..Default::default()
    };
    let (d, g) = mock();
    let off = assemble(
        StackInputs::new(d.clone(), g.clone()),
        &paths,
        &StackOptions::default(),
    );
    assert!(off.is_err(), "not moved unless asked");
    assert!(tsv.exists());
    let opts = StackOptions {
        quarantine_learning: true,
        ..Default::default()
    };
    let stack = assemble(StackInputs::new(d, g), &paths, &opts).unwrap();
    assert_eq!(
        stack.meta.learning_quarantined.as_deref(),
        Some(dir.join("learning.tsv.bad").as_path())
    );
    assert_eq!(
        std::fs::read(dir.join("learning.tsv.bad")).unwrap(),
        b"broken\n"
    );
    assert!(stack.learning.as_ref().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn eval_does_not_move_a_broken_learning_file() {
    let dir = temp_dir("quarantine-eval");
    std::fs::write(dir.join("learning.tsv"), "broken\n").unwrap();
    let paths = StackPaths {
        user_dir: Some(dir.clone()),
        ..Default::default()
    };
    let (d, g) = mock();
    let r = assemble(StackInputs::new(d, g), &paths, &eval_opts(RankerSet::Full));
    assert!(r.is_err());
    assert!(dir.join("learning.tsv").exists());
    assert!(!dir.join("learning.tsv.bad").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reuse_keeps_the_same_learning_store() {
    let dir = temp_dir("reuse");
    let paths = StackPaths {
        user_dir: Some(dir.clone()),
        ..Default::default()
    };
    let (d, g) = mock();
    let first = assemble(
        StackInputs::new(d.clone(), g.clone()),
        &paths,
        &StackOptions::default(),
    )
    .unwrap();
    let second = assemble_with(
        StackInputs::new(d, g),
        &paths,
        &StackOptions::default(),
        &Reuse::from_stack(&first),
    )
    .unwrap();
    assert!(Arc::ptr_eq(
        first.learning.as_ref().unwrap(),
        second.learning.as_ref().unwrap()
    ));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn without_learning_reopens_the_store_from_the_file() {
    let dir = temp_dir("without-learning");
    let paths = StackPaths {
        user_dir: Some(dir.clone()),
        ..Default::default()
    };
    let (d, g) = mock();
    let first = assemble(
        StackInputs::new(d.clone(), g.clone()),
        &paths,
        &StackOptions::default(),
    )
    .unwrap();
    // ファイルだけが差し替わった（メモリの first は古いまま）
    let source = UserDir::new(&dir);
    let other = LearningStore::open(&dir.join("other.tsv")).unwrap();
    other.save().unwrap();
    std::fs::copy(dir.join("other.tsv"), source.learning_tsv()).unwrap();
    let second = assemble_with(
        StackInputs::new(d, g),
        &paths,
        &StackOptions::default(),
        &Reuse::without_learning(&first),
    )
    .unwrap();
    let (old, new) = (
        first.learning.as_ref().unwrap(),
        second.learning.as_ref().unwrap(),
    );
    assert!(!Arc::ptr_eq(old, new));
    assert_eq!(new.salt(), other.salt());
    assert_ne!(old.salt(), other.salt());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn topic_needs_vectors() {
    let (d, g) = mock();
    let e = assemble(
        StackInputs::new(d, g),
        &StackPaths::default(),
        &eval_opts(RankerSet::Topic),
    )
    .err()
    .expect("must fail");
    assert!(matches!(e, StackError::MissingVectors), "{e}");
    assert!(e.to_string().contains("vectors.bin"), "{e}");
}

#[test]
fn explicit_vectors_must_open() {
    let (d, g) = mock();
    let paths = StackPaths {
        vectors: Some(PathBuf::from("/nonexistent/vectors.bin")),
        ..Default::default()
    };
    // dict_id のない辞書では、明示した vectors.bin は使えない
    assert!(assemble(StackInputs::new(d, g), &paths, &StackOptions::default()).is_err());
}

#[test]
fn missing_dictionary_is_an_open_error() {
    let paths = StackPaths {
        dict_dir: PathBuf::from("/nonexistent/dict"),
        ..Default::default()
    };
    let e = build_stack(&paths, &StackOptions::default())
        .err()
        .expect("must fail");
    assert!(matches!(e, StackError::Open(_)), "{e}");
}

#[cfg(not(feature = "lm"))]
#[test]
fn lm_without_the_feature_explains_how_to_build() {
    let (d, g) = mock();
    let paths = StackPaths {
        lm_model: Some(PathBuf::from("x.gguf")),
        ..Default::default()
    };
    let e = assemble(StackInputs::new(d, g), &paths, &StackOptions::default())
        .err()
        .expect("must fail");
    assert!(matches!(e, StackError::LmNotBuilt), "{e}");
    assert!(e.to_string().contains("lm-metal"), "{e}");
}

#[test]
fn unknown_config_key_is_a_config_error() {
    let (d, g) = mock();
    let mut opts = StackOptions::default();
    opts.ranker_config
        .insert("doc_cache.no_such_key".into(), "1".into());
    // 解析器を使わない構成では読まれないので、doc 以上の構成で確かめる
    let result = assemble(
        StackInputs::new(d, g),
        &StackPaths::default(),
        &eval_opts(RankerSet::Doc),
    );
    assert!(result.is_ok());
    let (d, g) = mock();
    let e = assemble(
        StackInputs::new(d, g),
        &StackPaths::default(),
        &StackOptions {
            ranker_config: opts.ranker_config,
            ..eval_opts(RankerSet::Doc)
        },
    )
    .err();
    assert!(matches!(e, Some(StackError::Config(_))), "{e:?}");
}

/// 副詞 そう、名詞 時期、それを覆う名詞 掃除機 の辞書（ADR 0008）
fn adverb_mock() -> (Arc<dyn Dictionary>, Arc<dyn Grammar>) {
    let mut g = DenseGrammar::new(
        2,
        2,
        0,
        UnknownParams {
            left_id: 1,
            right_id: 1,
            pos_id: 0,
            cost_per_char: 5000,
        },
    );
    let noun = g.add_pos("名詞,普通名詞,一般,*,*,*");
    let adverb = g.add_pos("副詞,*,*,*,*,*");
    let mut d = InMemoryDictionary::new(g.grammar_id());
    d.insert("そう", "そう", 1, 1, 1000, adverb);
    d.insert("じき", "時期", 1, 1, 1000, noun);
    d.insert("そうじき", "掃除機", 1, 1, 4000, noun);
    (Arc::new(d), Arc::new(g))
}

fn top_of(opts: &StackOptions, reading: &str) -> Result<String, StackError> {
    let (d, g) = adverb_mock();
    let stack = assemble(StackInputs::new(d, g), &StackPaths::default(), opts)?;
    let mut conv = stack.converter.convert(reading, &Context::new());
    Ok(conv.page(0).unwrap().candidates[0].surface.clone())
}

#[test]
fn adverb_noun_rule_is_on_by_default_and_read_from_the_config() {
    for set in [RankerSet::Cost, RankerSet::Doc] {
        // 既定: そう → 時期 の辺に +6000 で、掃除機 が1位
        assert_eq!(top_of(&eval_opts(set), "そうじき").unwrap(), "掃除機");
        let with = |k: &str, v: &str| StackOptions {
            ranker_config: [(k.to_owned(), v.to_owned())].into_iter().collect(),
            ..eval_opts(set)
        };
        // penalty = 0 で止まる
        let off = with("lattice.adverb_noun.penalty", "0");
        assert_eq!(top_of(&off, "そうじき").unwrap(), "そう時期");
        // 覆う名詞の上限が 掃除機 のコストより低ければ、加算しない
        let low_cap = with("lattice.adverb_noun.cover_max_cost", "3999");
        assert_eq!(top_of(&low_cap, "そうじき").unwrap(), "そう時期");
        // 未知のキーと負の値は設定の誤り
        for (k, v) in [
            ("lattice.adverb_noun.no_such_key", "1"),
            ("lattice.adverb_noun.penalty", "-1"),
        ] {
            let e = top_of(&with(k, v), "そうじき").err();
            assert!(matches!(e, Some(StackError::Config(_))), "{k}={v}: {e:?}");
        }
    }
}

#[test]
fn noun_suru_rule_is_read_from_the_config() {
    for set in [RankerSet::Cost, RankerSet::Doc] {
        let with = |k: &str, v: &str| StackOptions {
            ranker_config: [(k.to_owned(), v.to_owned())].into_iter().collect(),
            ..eval_opts(set)
        };
        // 値の上書きと、止める値（0）は受け付ける
        for v in ["0", "5000"] {
            let opts = with("lattice.noun_suru.penalty", v);
            assert_eq!(top_of(&opts, "そうじき").unwrap(), "掃除機", "{v}");
        }
        // 未知のキーと負の値は設定の誤り
        for (k, v) in [
            ("lattice.noun_suru.no_such_key", "1"),
            ("lattice.noun_suru.penalty", "-1"),
        ] {
            let e = top_of(&with(k, v), "そうじき").err();
            assert!(matches!(e, Some(StackError::Config(_))), "{k}={v}: {e:?}");
        }
    }
}

#[test]
fn noun_suffix_rule_is_read_from_the_config() {
    for set in [RankerSet::Cost, RankerSet::Doc] {
        let with = |k: &str, v: &str| StackOptions {
            ranker_config: [(k.to_owned(), v.to_owned())].into_iter().collect(),
            ..eval_opts(set)
        };
        // 値の上書きと、止める値（0）、上限は受け付ける
        for (k, v) in [
            ("lattice.noun_suffix.penalty", "0"),
            ("lattice.noun_suffix.penalty", "5000"),
            ("lattice.noun_suffix.cover_max_cost", "9000"),
        ] {
            let opts = with(k, v);
            assert_eq!(top_of(&opts, "そうじき").unwrap(), "掃除機", "{k}={v}");
        }
        // 未知のキーと負の値は設定の誤り
        for (k, v) in [
            ("lattice.noun_suffix.no_such_key", "1"),
            ("lattice.noun_suffix.penalty", "-1"),
        ] {
            let e = top_of(&with(k, v), "そうじき").err();
            assert!(matches!(e, Some(StackError::Config(_))), "{k}={v}: {e:?}");
        }
    }
}

#[test]
fn numeral_cover_rule_is_read_from_the_config() {
    for set in [RankerSet::Cost, RankerSet::Doc] {
        let with = |k: &str, v: &str| StackOptions {
            ranker_config: [(k.to_owned(), v.to_owned())].into_iter().collect(),
            ..eval_opts(set)
        };
        // 値の上書きと、止める値（0）、上限は受け付ける
        for (k, v) in [
            ("lattice.numeral_cover.penalty", "0"),
            ("lattice.numeral_cover.penalty", "5000"),
            ("lattice.numeral_cover.cover_max_cost", "9000"),
        ] {
            let opts = with(k, v);
            assert_eq!(top_of(&opts, "そうじき").unwrap(), "掃除機", "{k}={v}");
        }
        // 未知のキーと負の値は設定の誤り
        for (k, v) in [
            ("lattice.numeral_cover.no_such_key", "1"),
            ("lattice.numeral_cover.penalty", "-1"),
        ] {
            let e = top_of(&with(k, v), "そうじき").err();
            assert!(matches!(e, Some(StackError::Config(_))), "{k}={v}: {e:?}");
        }
    }
}

// ---- 語彙集とユーザー辞書の重ね方（ADR 0012 §2.1、§4） ----

const NOUN_POS: &str = "名詞,普通名詞,一般,*,*,*";
const PROPER_POS: &str = "名詞,固有名詞,一般,*,*,*";
const PACK_READING: &str = "くばねてぃす";
const PACK_KATAKANA: &str = "クバネティス";
const PACK_LATIN: &str = "Kubernetes";

/// 語彙集とユーザー辞書の品詞（名詞、固有名詞）を持つ文法と、小さなシステム辞書
fn layered_mock() -> (Arc<dyn Dictionary>, DenseGrammar) {
    let mut g = DenseGrammar::new(
        4,
        4,
        0,
        UnknownParams {
            left_id: 1,
            right_id: 1,
            pos_id: 0,
            cost_per_char: 5000,
        },
    );
    for name in [NOUN_POS, PROPER_POS] {
        let id = g.add_pos(name);
        g.set_pos_info(
            id,
            ikari_core::PosInfo {
                rep_left_id: 1,
                rep_right_id: 1,
                user_cost: 3000,
                rep_share_permille: 990,
            },
        );
    }
    let mut d = InMemoryDictionary::new(g.grammar_id());
    d.insert("ひんし", "品詞", 1, 1, 100, 0);
    (Arc::new(d), g)
}

/// (slot, 表記) を、安い順に
fn found(dict: &dyn Dictionary, reading: &str) -> Vec<(u8, String)> {
    let mut out = Vec::new();
    dict.lookup_exact(reading, &mut |e| {
        out.push((e.id.dict, e.surface.to_string(), e.cost))
    });
    out.sort_by_key(|(_, _, c)| *c);
    out.into_iter().map(|(d, s, _)| (d, s)).collect()
}

#[test]
fn the_it_pack_is_layered_after_the_system_dictionary_by_default() {
    let (system, g) = layered_mock();
    let mut warnings = Vec::new();
    let layered =
        layer_dictionaries(system, &g, None, LoadOptions::default(), &mut warnings).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(layered.packs, ["it"]);
    assert!(!layered.has_user_dict);
    assert_eq!(
        found(&*layered.dict, PACK_READING),
        [(1, PACK_KATAKANA.to_owned()), (1, PACK_LATIN.to_owned())]
    );
    // システム辞書は slot 0 のまま
    assert_eq!(found(&*layered.dict, "ひんし"), [(0, "品詞".to_owned())]);
}

#[test]
fn the_it_pack_can_be_turned_off() {
    let (system, g) = layered_mock();
    let mut warnings = Vec::new();
    let off = LoadOptions { it_pack: false };
    let layered = layer_dictionaries(system, &g, None, off, &mut warnings).unwrap();
    assert!(layered.packs.is_empty());
    assert!(found(&*layered.dict, PACK_READING).is_empty());
    assert_eq!(found(&*layered.dict, "ひんし"), [(0, "品詞".to_owned())]);
}

#[test]
fn the_user_dictionary_comes_after_the_pack() {
    let (system, g) = layered_mock();
    let dir = temp_dir("layer-user");
    let user_dir = UserDir::new(&dir);
    let mut user = UserDictionary::default();
    user.add(ikari_core::user::UserDictEntry {
        reading: "ひんし".into(),
        surface: "品死".into(),
        pos: "名詞".into(),
        comment: String::new(),
    })
    .unwrap();
    user.save(&user_dir.user_dict_tsv()).unwrap();
    let mut warnings = Vec::new();
    let layered = layer_dictionaries(
        system,
        &g,
        Some(&user_dir),
        LoadOptions::default(),
        &mut warnings,
    )
    .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(layered.has_user_dict);
    // システム辞書 slot 0、語彙集 slot 1、ユーザー辞書 slot 2
    assert_eq!(found(&*layered.dict, PACK_READING)[0].0, 1);
    let user_hit = found(&*layered.dict, "ひんし");
    assert!(user_hit.contains(&(2, "品死".to_owned())), "{user_hit:?}");
    // 語彙集を切ると、ユーザー辞書は slot 1
    let (system, g) = layered_mock();
    let off = LoadOptions { it_pack: false };
    let layered = layer_dictionaries(system, &g, Some(&user_dir), off, &mut warnings).unwrap();
    assert!(found(&*layered.dict, "ひんし").contains(&(1, "品死".to_owned())));
}

#[test]
fn a_grammar_without_the_pack_parts_of_speech_only_warns() {
    let (system, g) = mock();
    let mut warnings = Vec::new();
    let layered =
        layer_dictionaries(system, &*g, None, LoadOptions::default(), &mut warnings).unwrap();
    assert!(layered.packs.is_empty());
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("語彙集"), "{warnings:?}");
}
