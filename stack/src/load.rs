use super::*;

/// 辞書の読み込みの設定。
#[derive(Clone, Copy, Debug)]
pub struct LoadOptions {
    /// 同梱の IT の語彙集を使う（既定は true。ADR 0012 B20-4）
    pub it_pack: bool,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self { it_pack: true }
    }
}

/// 開いた辞書。語彙集かユーザー辞書があれば `dict` は `DictionarySet`。
/// slot の並びは システム辞書、語彙集（使うとき）、ユーザー辞書（語があるとき）の順で、
/// slot 0 だけが固定（lemma 番号とベクトルの対応が有効。`SYSTEM_SLOT`）。
pub struct LoadedSet {
    pub system: Arc<SystemDictionary>,
    pub grammar: Arc<GrammarFile>,
    pub dict: Arc<dyn Dictionary>,
    pub dict_dir: PathBuf,
    pub open_elapsed: Duration,
    /// ユーザー辞書の語が1つ以上入っている
    pub has_user_dict: bool,
    /// 読み込んだ語彙集の id
    pub packs: Vec<String>,
    /// ユーザー辞書や語彙集を使えなかったときの理由（変換は続ける）
    pub warnings: Vec<String>,
}

impl LoadedSet {
    pub fn grammar_dyn(&self) -> Arc<dyn Grammar> {
        self.grammar.clone()
    }
}

/// 辞書ディレクトリとユーザー辞書を開く。ユーザー辞書の不具合は警告にとどめる。
pub fn load_dictionaries(
    dict_dir: &Path,
    user_dir: Option<&UserDir>,
) -> Result<LoadedSet, StackError> {
    load_dictionaries_with(dict_dir, user_dir, LoadOptions::default())
}

/// `load_dictionaries` に設定を渡す版。
pub fn load_dictionaries_with(
    dict_dir: &Path,
    user_dir: Option<&UserDir>,
    options: LoadOptions,
) -> Result<LoadedSet, StackError> {
    let t = Instant::now();
    let loaded =
        open_dir(dict_dir).map_err(|e| open_err(format!("{}: {e}", dict_dir.display())))?;
    let system = loaded.dict;
    let grammar = loaded.grammar;

    let mut warnings = Vec::new();
    let layered = layer_dictionaries(system.clone(), &*grammar, user_dir, options, &mut warnings)?;
    Ok(LoadedSet {
        system,
        grammar,
        dict: layered.dict,
        dict_dir: dict_dir.to_path_buf(),
        open_elapsed: t.elapsed(),
        has_user_dict: layered.has_user_dict,
        packs: layered.packs,
        warnings,
    })
}

/// システム辞書に語彙集とユーザー辞書を重ねたもの。
pub struct Layered {
    pub dict: Arc<dyn Dictionary>,
    /// 読み込んだ語彙集の id
    pub packs: Vec<String>,
    /// ユーザー辞書の語が1つ以上入っている
    pub has_user_dict: bool,
}

/// システム辞書に、語彙集（`options`）とユーザー辞書（`user_dir` の中身）を、この順の slot で重ねる。
/// どちらも、使えないときは理由を `warnings` に足して続ける。何も重ならなければ `system` をそのまま返す。
/// eval も同じ重ね方を使う（開いた辞書を受け取る）。
pub fn layer_dictionaries(
    system: Arc<dyn Dictionary>,
    grammar: &dyn Grammar,
    user_dir: Option<&UserDir>,
    options: LoadOptions,
    warnings: &mut Vec<String>,
) -> Result<Layered, StackError> {
    let user_dict = user_dir.and_then(|ud| match load_user_dictionary(ud, grammar) {
        Ok(d) => d,
        Err(e) => {
            warnings.push(format!("ユーザー辞書を使えません: {e}"));
            None
        }
    });
    let mut packs = Vec::new();
    let mut pack_dicts = Vec::new();
    if options.it_pack {
        match load_it_pack(grammar, &*system) {
            Ok(d) => {
                packs.push(IT_PACK_ID.to_owned());
                pack_dicts.push(d);
            }
            Err(e) => warnings.push(format!("語彙集 {IT_PACK_ID} を使えません: {e}")),
        }
    }
    let has_user_dict = user_dict.is_some();
    let dict: Arc<dyn Dictionary> = if pack_dicts.is_empty() && user_dict.is_none() {
        system
    } else {
        let mut set = DictionarySet::new();
        set.push(system).map_err(open_err)?;
        for pack in pack_dicts {
            set.push(Arc::new(pack)).map_err(open_err)?;
        }
        if let Some(user) = user_dict {
            set.push(Arc::new(user)).map_err(open_err)?;
        }
        Arc::new(set)
    };
    Ok(Layered {
        dict,
        packs,
        has_user_dict,
    })
}

/// 同梱の IT の語彙集を、ユーザー辞書と同じ作り方で辞書にする。
fn load_it_pack(
    grammar: &dyn Grammar,
    system: &dyn Dictionary,
) -> Result<ikari_core::InMemoryDictionary, ikari_core::UserDataError> {
    builtin_it_pack()?.build(grammar, system)
}

/// ユーザー辞書を開いて展開する。空、またはファイルがなければ None。
fn load_user_dictionary(
    dir: &UserDir,
    grammar: &dyn Grammar,
) -> Result<Option<ikari_core::InMemoryDictionary>, ikari_core::UserDataError> {
    let user = UserDictionary::open(&dir.user_dict_tsv())?;
    if user.is_empty() {
        return Ok(None);
    }
    Ok(Some(user.build(grammar)?))
}

/// vectors.bin を開く。
/// - 明示（`explicit`）: 開けなければエラー。辞書が vectors.bin に対応していなくてもエラー。
/// - 既定（`default`）: なければ None。開けないときは Runtime なら警告にとどめ、Eval ならエラー。
pub fn load_vectors(
    dict_id: Option<u64>,
    explicit: Option<&Path>,
    default: Option<&Path>,
    mode: AssemblyMode,
    warnings: &mut Vec<String>,
) -> Result<Option<Arc<dyn WordVectors>>, StackError> {
    let (path, is_explicit) = match (explicit, default) {
        (Some(p), _) => (p, true),
        (None, Some(p)) if p.exists() => (p, false),
        _ => return Ok(None),
    };
    let Some(dict_id) = dict_id else {
        if is_explicit || mode == AssemblyMode::Eval {
            return Err(StackError::Config(
                "この辞書は vectors.bin に対応していません（dict_id が必要です。形式 v2）".into(),
            ));
        }
        return Ok(None);
    };
    match open_vectors(path, dict_id) {
        Ok(v) => Ok(Some(v)),
        Err(e) if is_explicit || mode == AssemblyMode::Eval => {
            Err(open_err(format!("{}: {e}", path.display())))
        }
        Err(e) => {
            warnings.push(format!("{} を使えません: {e}", path.display()));
            Ok(None)
        }
    }
}

pub(super) fn load_profile(
    sel: &ProfileSel,
    user_dir: Option<&UserDir>,
    warnings: &mut Vec<String>,
) -> Result<Option<Arc<Profile>>, StackError> {
    match sel {
        ProfileSel::None => Ok(None),
        ProfileSel::Named(id) => {
            let p = Profile::from_settings(&[(id.clone(), ProfileWeight::OFTEN)])
                .map_err(|e| config_err("profile", e))?;
            Ok(Some(Arc::new(p)))
        }
        ProfileSel::User => match user_dir {
            None => Ok(None),
            Some(ud) => match Profile::load(ud) {
                Ok(p) => Ok(p.map(Arc::new)),
                Err(e) => {
                    warnings.push(format!("プロファイルを使えません: {e}"));
                    Ok(None)
                }
            },
        },
    }
}
