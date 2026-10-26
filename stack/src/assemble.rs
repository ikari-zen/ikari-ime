use super::*;
use crate::load::load_profile;

/// Rewriter（日付、郵便番号、数字、電卓、記号、顔文字、Unicode 表記、曜日の日付）の既定の組。
/// `PostalRewriter` は `NumberRewriter` より前（住所が数字の書き換えより先に出る。ADR 0032 §4）。
pub(super) fn default_rewriters(
    postal: Arc<PostalTable>,
    options: PostalOptions,
    unicode: UnicodeOptions,
    date: DateOptions,
) -> Vec<Arc<dyn Rewriter>> {
    let mut rewriters: Vec<Arc<dyn Rewriter>> = vec![
        Arc::new(DateRewriter::new()),
        Arc::new(PostalRewriter::new(postal, options)),
        Arc::new(NumberRewriter::new()),
        Arc::new(CalculatorRewriter::new()),
        Arc::new(SymbolRewriter::builtin()),
        Arc::new(EmoticonRewriter::builtin()),
    ];
    // 設定が off のときは組に入れない（構成の説明と学習の書式IDが、入れる前と変わらない）
    if unicode.notation {
        rewriters.push(Arc::new(UnicodeRewriter::new(unicode)));
    }
    if date.weekday_dates {
        rewriters.push(Arc::new(WeekdayRewriter::new(date)));
    }
    rewriters
}

/// 開いた辞書と文法（`assemble` の入力）。
pub struct StackInputs {
    pub dict: Arc<dyn Dictionary>,
    pub grammar: Arc<dyn Grammar>,
    /// 明示の vectors がないときに試す vectors.bin（なければ使わない）
    pub default_vectors: Option<PathBuf>,
    pub has_user_dict: bool,
    pub open_elapsed: Duration,
    pub dict_label: String,
    pub warnings: Vec<String>,
}

impl StackInputs {
    /// 辞書と文法だけを指定する（eval とテスト用）。
    pub fn new(dict: Arc<dyn Dictionary>, grammar: Arc<dyn Grammar>) -> StackInputs {
        StackInputs {
            dict_label: dict_label(dict.dict_id()),
            dict,
            grammar,
            default_vectors: None,
            has_user_dict: false,
            open_elapsed: Duration::ZERO,
            warnings: Vec::new(),
        }
    }
}

/// 開いた辞書と文法から組み立てる。`paths.dict_dir` は使わない。
pub fn assemble(
    inputs: StackInputs,
    paths: &StackPaths,
    opts: &StackOptions,
) -> Result<Stack, StackError> {
    assemble_with(inputs, paths, opts, &Reuse::default())
}

pub(super) fn assemble_with(
    inputs: StackInputs,
    paths: &StackPaths,
    opts: &StackOptions,
    reuse: &Reuse,
) -> Result<Stack, StackError> {
    let StackInputs {
        dict,
        grammar,
        default_vectors,
        has_user_dict,
        open_elapsed,
        dict_label,
        mut warnings,
    } = inputs;
    let set = opts.ranker;
    let eval = opts.mode == AssemblyMode::Eval;
    let cost = set == RankerSet::Cost;
    let full = set == RankerSet::Full;
    // 変換のラティスの辺の補正は `--ranker-config` の `lattice.adverb_noun.*` で上書きできる（ADR 0008 §3.2）
    let mut convert = opts.convert.clone();
    convert.adverb_noun = convert
        .adverb_noun
        .with_kv(&opts.ranker_config)
        .map_err(|e| config_err("lattice", e))?;
    // サ変でない名詞＋する の補正も同じ（`lattice.noun_suru.penalty`。ADR 0016 §7.1）
    convert.noun_suru = convert
        .noun_suru
        .with_kv(&opts.ranker_config)
        .map_err(|e| config_err("lattice", e))?;
    // 助詞＋しよう（意志形）の補正も同じ（`lattice.volitional.bonus`。ADR 0016 §10）
    convert.volitional = convert
        .volitional
        .with_kv(&opts.ranker_config)
        .map_err(|e| config_err("lattice", e))?;
    // 数詞 → 助数詞可能の名詞 の補正も同じ（`lattice.numeral_counter.bonus`。ADR 0019 §15）
    convert.numeral_counter = convert
        .numeral_counter
        .with_kv(&opts.ranker_config)
        .map_err(|e| config_err("lattice", e))?;
    // 未知語のカタカナの候補の補正（`katakana_like.*`。#271）
    convert.katakana_like = convert
        .katakana_like
        .with_kv(&opts.ranker_config)
        .map_err(|e| config_err("katakana_like", e))?;
    // 名詞 → 接尾辞 の補正も同じ（`lattice.noun_suffix.*`。ADR 0008 §13）
    convert.noun_suffix = convert
        .noun_suffix
        .with_kv(&opts.ranker_config)
        .map_err(|e| config_err("lattice", e))?;
    // 前の語 → 数詞「数」 の補正も同じ（`lattice.numeral_cover.*`。ADR 0019 規則 N）
    convert.numeral_cover = convert
        .numeral_cover
        .with_kv(&opts.ranker_config)
        .map_err(|e| config_err("lattice", e))?;

    // Cost は Ranker を足さず、ユーザーデータにも触らない（Eval）
    if cost && eval {
        if paths.lm_model.is_some() {
            return Err(StackError::Config(
                "LM needs a ranker other than cost (e.g. doc or full)".into(),
            ));
        }
        let mut builder = Converter::builder(dict.clone(), grammar.clone())
            .ranker(Arc::new(CostRanker))
            .options(convert.clone());
        if let Some(clock) = &opts.clock {
            builder = builder.clock(Arc::clone(clock));
        }
        let converter = Arc::new(builder.build().map_err(open_err)?);
        return Ok(Stack {
            converter,
            learning: None,
            lm: None,
            meta: StackMeta {
                description: "ranker=cost analyzer=null rewriters=none vectors=none lm=none".into(),
                dict_label,
                open_elapsed,
                has_user_dict,
                warnings,
            },
            vectors: None,
            profile: None,
            dict,
            grammar,
            lm_ranker: None,
            postal: None,
        });
    }

    let dict_id = dict.dict_id();
    let user_dir = paths.user_dir.as_ref().map(UserDir::new);
    // 解析器: Runtime は表記の索引（dict_id）があるときだけ。Cost では使わない
    let analyzer_active = !cost
        && !opts.no_analyzer
        && match opts.mode {
            AssemblyMode::Runtime => dict_id.is_some(),
            AssemblyMode::Eval => true,
        };

    // zenz.* 以外の Ranker には、zenz.* を渡さない
    let other_kv: BTreeMap<String, String> = opts
        .ranker_config
        .iter()
        .filter(|(k, _)| !k.starts_with(LM_KEY_PREFIX))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let vectors = if full || (eval && !cost) {
        load_vectors(
            dict_id,
            paths.vectors.as_deref(),
            default_vectors.as_deref(),
            opts.mode,
            &mut warnings,
        )?
    } else {
        None
    };
    if set == RankerSet::Topic && vectors.is_none() {
        return Err(StackError::MissingVectors);
    }

    let profile = if full && !eval {
        load_profile(&opts.profile, user_dir.as_ref(), &mut warnings)?
    } else {
        None
    };

    let learning = match reuse.learning.clone() {
        Some(store) => Some(store),
        None => match &user_dir {
            Some(ud) if full || opts.open_learning => {
                let store = LearningStore::open(&ud.learning_tsv())
                    .map_err(|e| open_err(format!("{}: {e}", ud.learning_tsv().display())))?;
                // 品詞名で特徴量を作る（辞書を作り直しても学習が壊れないように。ADR 0002 §8.1）
                store.attach_grammar(Arc::clone(&grammar));
                Some(Arc::new(store))
            }
            _ => None,
        },
    };

    let mut rankers: Vec<Arc<dyn Ranker>> = Vec::new();
    let mut names: Vec<&str> = Vec::new();
    if set.uses_doc() && analyzer_active {
        let left_cfg =
            LeftContextConfig::from_kv(&other_kv).map_err(|e| config_err("left_context", e))?;
        rankers.push(Arc::new(LeftContextRanker::new(left_cfg)));
        names.push("left_context");
        let kana_cfg =
            KanaStyleConfig::from_kv(&other_kv).map_err(|e| config_err("kana_style", e))?;
        rankers.push(Arc::new(KanaStyleRanker::new(
            KanaStyleTable::builtin(),
            kana_cfg,
        )));
        names.push("kana_style");
    }
    // 文書内キャッシュ: Runtime は解析器が前提、Eval は解析器の有無によらない
    if set.uses_doc() && (analyzer_active || eval) {
        let doc_cfg =
            DocumentCacheConfig::from_kv(&other_kv).map_err(|e| config_err("doc_cache", e))?;
        rankers.push(Arc::new(DocumentCacheRanker::new(doc_cfg)));
        names.push("doc_cache");
    }
    if full && let Some(store) = &learning {
        let wanted = eval || opts.always_learning_ranker || !store.is_empty();
        if wanted {
            let learn_cfg =
                LearningConfig::from_kv(&other_kv).map_err(|e| config_err("learning", e))?;
            rankers.push(Arc::new(LearningRanker::new(
                Arc::clone(store),
                vectors.clone(),
                learn_cfg,
            )));
            names.push("learning");
        }
    }
    if full && (eval || profile.is_some()) {
        let profile_cfg =
            ProfileConfig::from_kv(&other_kv).map_err(|e| config_err("profile", e))?;
        rankers.push(Arc::new(ProfileRanker::new(vectors.clone(), profile_cfg)));
        names.push("profile");
    }
    if set.uses_topic()
        && let Some(v) = &vectors
        && (analyzer_active || eval)
    {
        let topic_cfg = TopicConfig::from_kv(&other_kv).map_err(|e| config_err("topic", e))?;
        rankers.push(Arc::new(TopicRanker::new(Arc::clone(v), topic_cfg)));
        names.push("topic");
    }

    // LM: Runtime は Full のときだけ。Eval は Cost 以外で使える
    let mut lm: Option<Arc<dyn LmHandle>> = None;
    let mut lm_ranker: Option<Arc<dyn Ranker>> = None;
    let lm_wanted = paths.lm_model.is_some() && (full || eval);
    let mut lm_desc = "none".to_owned();
    if lm_wanted {
        let (ranker, handle) = match reuse.lm.clone() {
            Some(parts) => parts,
            None => {
                let path = paths.lm_model.as_deref().unwrap_or(Path::new(""));
                let mut lm_kv = opts.ranker_config.clone();
                opts.lm.apply(&mut lm_kv);
                lm::build(path, &lm_kv)?
            }
        };
        rankers.push(Arc::clone(&ranker));
        names.push("zenz");
        lm_ranker = Some(ranker);
        lm = Some(handle);
        lm_desc = paths
            .lm_model
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    }

    // B40（ADR 0024）: LM が規則 S の守った1位を覆さない。規則 S を止めたときは働かない
    let composite = |rankers| {
        Arc::new(CompositeRanker::new(rankers).keep_noun_suru(convert.noun_suru.is_enabled()))
    };
    let ranker: Arc<dyn Ranker> = if eval {
        composite(rankers)
    } else {
        match rankers.len() {
            0 => Arc::new(CostRanker),
            1 => rankers.remove(0),
            _ => composite(rankers),
        }
    };

    let analyzer: Arc<dyn ikari_core::ContextAnalyzer> = if analyzer_active {
        Arc::new(LatticeContextAnalyzer::new(
            Arc::clone(&dict),
            Arc::clone(&grammar),
        ))
    } else {
        Arc::new(NullAnalyzer)
    };
    // 表が壊れていても変換は止めない（警告にして空の表で続ける）
    let postal: Arc<PostalTable> = match (&reuse.postal, &opts.postal_tsv) {
        (Some(table), _) => Arc::clone(table),
        (None, Some(path)) => Arc::new(PostalTable::load(path).unwrap_or_else(|e| {
            warnings.push(e.to_string());
            PostalTable::empty()
        })),
        (None, None) => Arc::new(PostalTable::empty()),
    };
    let rewriters = default_rewriters(Arc::clone(&postal), opts.postal, opts.unicode, opts.date);
    let rewriter_count = rewriters.len();
    let mut builder = Converter::builder(dict.clone(), grammar.clone())
        .ranker(ranker)
        .analyzer(analyzer)
        .rewriters(rewriters)
        .options(convert.clone());
    if let Some(clock) = &opts.clock {
        builder = builder.clock(Arc::clone(clock));
    }
    let converter = Arc::new(builder.build().map_err(open_err)?);

    let description = if eval {
        format!(
            "ranker={}({}) analyzer={} rewriters={rewriter_count} vectors={} lm={lm_desc}",
            set.as_str(),
            names.join(","),
            if analyzer_active { "lattice" } else { "null" },
            if vectors.is_some() { "yes" } else { "none" },
        )
    } else {
        format!(
            "ranker={} rankers=[{}] analyzer={} vectors={} profile={} user_dict={}",
            if full { "full" } else { set.as_str() },
            names.join(","),
            if analyzer_active { "lattice" } else { "null" },
            if vectors.is_some() { "yes" } else { "no" },
            profile.as_ref().map_or("none".to_string(), |p| p
                .domains
                .iter()
                .map(|d| d.id.as_str())
                .collect::<Vec<_>>()
                .join("+")),
            if has_user_dict { "yes" } else { "no" },
        )
    };
    Ok(Stack {
        converter,
        learning,
        lm,
        meta: StackMeta {
            description,
            dict_label,
            open_elapsed,
            has_user_dict,
            warnings,
        },
        vectors,
        profile,
        dict,
        grammar,
        lm_ranker,
        postal: Some(postal),
    })
}
