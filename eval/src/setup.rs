//! Converter の構成を組み立てる（`--ranker`、`--no-analyzer`、`--vectors`、`--ranker-config`、`--lm`。
//! ADR 0002 §15.1）。組み立て本体は `ikari-stack`（`AssemblyMode::Eval`）にあり、ここは eval の引数との対応と、
//! eval 固有の部分（一時ディレクトリ、LM の集計）だけを持つ。

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ikari_core::{
    Clock, ConvertError, Converter, Dictionary, Grammar, LearningStore, POSTAL_TSV_FILE_NAME,
    Profile, UserDir, VECTORS_FILE_NAME, WordVectors,
};
use ikari_stack::{
    AssemblyMode, LmOverrides, LoadOptions, RankerSet, StackError, StackInputs, StackOptions,
    StackPaths, assemble, layer_dictionaries, read_ranker_config,
};

use crate::LmReport;

const TEMP_DIR_PREFIX: &str = "ikari-eval";

/// `--profile` の値
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ProfileChoice {
    /// ケースの profile 列に従う（既定。full のときだけ有効）
    #[default]
    PerCase,
    /// プロファイルを一切使わない（`--profile none`）
    None,
    /// 全ケースにこのプリセットを使う
    Named(String),
}

impl ProfileChoice {
    pub fn parse(s: &str) -> ProfileChoice {
        match s {
            "none" => ProfileChoice::None,
            name => ProfileChoice::Named(name.to_owned()),
        }
    }
}

#[derive(Clone, Default)]
pub struct SetupOptions {
    pub ranker: Option<RankerSet>,
    pub no_analyzer: bool,
    /// 明示した vectors.bin
    pub vectors: Option<PathBuf>,
    /// 明示がないときに試す vectors.bin（`<dict>/vectors.bin`）。なければ使わない
    pub default_vectors: Option<PathBuf>,
    /// `key=value` の行の設定
    pub ranker_config: Option<PathBuf>,
    /// zenz の GGUF（feature `lm` が要る）
    pub lm: Option<PathBuf>,
    pub lm_budget_ms: Option<u64>,
    /// 学習ストアの置き場所。None なら一時ディレクトリ（利用者のデータを評価に混ぜない）
    pub user_dir: Option<PathBuf>,
    /// Rewriter の時計。None なら SystemClock
    pub clock: Option<Arc<dyn Clock>>,
    /// 同梱の語彙集（IT）を重ねる。エンジンと同じ並び（システム辞書、語彙集、ユーザー辞書）になる。
    /// 既定は false（モックの辞書の単体テストのため）。`ikari-eval` の実行は `--no-packs` がなければ true にする
    pub packs: bool,
    /// 住所 ↔ 郵便番号の表（postal.tsv。ADR 0032）。None なら表なし（郵便番号の候補は出ない）。
    /// `ikari-eval` の実行は `--postal` か `<dict>/postal.tsv`
    pub postal_tsv: Option<PathBuf>,
}

#[derive(Debug)]
pub enum SetupError {
    Config(String),
    Convert(ConvertError),
    Io(String),
}

impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetupError::Config(m) => write!(f, "{m}"),
            SetupError::Convert(e) => write!(f, "{e}"),
            SetupError::Io(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for SetupError {}

impl From<ConvertError> for SetupError {
    fn from(e: ConvertError) -> Self {
        SetupError::Convert(e)
    }
}

impl From<StackError> for SetupError {
    fn from(e: StackError) -> Self {
        match e {
            StackError::Open(m) => SetupError::Io(m),
            other => SetupError::Config(other.detail()),
        }
    }
}

/// 終了時に消す一時ディレクトリ
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// LM の動作の集計を返す関数
pub type LmReporter = Arc<dyn Fn() -> LmReport + Send + Sync>;

pub struct Setup {
    pub converter: Arc<Converter>,
    /// 構成の1行（レポートに出す）
    pub description: String,
    pub ranker_name: String,
    /// 学習ストア（full のとき。シナリオの commit が記録する先）
    pub store: Option<Arc<LearningStore>>,
    pub vectors: Option<Arc<dyn WordVectors>>,
    /// LM の動作の集計（`--lm` のとき）
    pub lm_report: Option<LmReporter>,
    _temp: Option<TempDir>,
}

/// 同梱のプリセットから `Profile` を作る（`it` など）。未知の名前は None。
pub fn preset_profile(name: &str) -> Option<Arc<Profile>> {
    Profile::presets()
        .into_iter()
        .find(|d| d.id == name)
        .map(|d| Arc::new(Profile::new(vec![d])))
}

fn temp_user_dir() -> Result<TempDir, SetupError> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir =
        std::env::temp_dir().join(format!("{TEMP_DIR_PREFIX}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .map_err(|e| SetupError::Io(format!("cannot create {}: {e}", dir.display())))?;
    Ok(TempDir(dir))
}

/// 辞書と文法から、指定の構成の Converter を組み立てる。
///
/// `Cost` は `Converter::new` と同じ（解析器なし、Rewriter なし）。`Full` は解析器（`--no-analyzer` で外す）、
/// 5つの Rewriter、DocumentCache → Learning → Profile → Topic（vectors があるとき）→ Zenz（`--lm`）。
pub fn build(
    dict: Arc<dyn Dictionary>,
    grammar: Arc<dyn Grammar>,
    opts: &SetupOptions,
) -> Result<Setup, SetupError> {
    let set = opts.ranker.unwrap_or(RankerSet::Cost);
    let kv = match &opts.ranker_config {
        Some(path) => read_ranker_config(path)?,
        None => BTreeMap::new(),
    };

    if set == RankerSet::Cost && opts.lm.is_some() {
        return Err(SetupError::Config(
            "--lm needs a ranker other than cost (e.g. --ranker doc or full)".into(),
        ));
    }

    // 学習ストアは full のときだけ。user_dir がなければ一時ディレクトリ（利用者の学習を評価に混ぜない）
    let mut temp = None;
    let user_dir = if set == RankerSet::Full {
        Some(match &opts.user_dir {
            Some(p) => p.clone(),
            None => {
                let t = temp_user_dir()?;
                let p = t.0.clone();
                temp = Some(t);
                p
            }
        })
    } else {
        None
    };
    // UserDir を通して置き場所の決め方を揃える
    let user_dir = user_dir.map(|p| UserDir::new(p).root().to_path_buf());

    let paths = StackPaths {
        dict_dir: PathBuf::new(),
        user_dir: user_dir.clone(),
        vectors: opts.vectors.clone(),
        lm_model: opts.lm.clone(),
        no_packs: !opts.packs,
    };
    let stack_opts = StackOptions {
        ranker: set,
        ranker_config: kv,
        // --lm-budget-ms が明示されたときは設定ファイルより優先する。
        // 省略時は ZenzConfig の既定（上限 300ms）。実際の待ちは変換1回の締め切りで決まる
        lm: LmOverrides {
            budget_ms: opts.lm_budget_ms,
            idle_unload_secs: None,
        },
        mode: AssemblyMode::Eval,
        no_analyzer: opts.no_analyzer,
        clock: opts.clock.clone(),
        postal_tsv: opts.postal_tsv.clone(),
        ..StackOptions::default()
    };
    // 語彙集と、`--user-dir` を明示したときのユーザー辞書を重ねる（一時ディレクトリにはユーザー辞書がない）
    let mut warnings = Vec::new();
    let explicit_user = opts.user_dir.as_ref().map(|p| UserDir::new(p.clone()));
    let layered = layer_dictionaries(
        dict,
        &*grammar,
        explicit_user.as_ref(),
        LoadOptions {
            it_pack: opts.packs,
        },
        &mut warnings,
    )?;
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    let mut inputs = StackInputs::new(layered.dict, grammar);
    inputs.has_user_dict = layered.has_user_dict;
    inputs.default_vectors = opts.default_vectors.clone();
    let stack = assemble(inputs, &paths, &stack_opts).map_err(map_stack_error)?;
    // 最初の採点にモデルの読み込みと Metal の初期化が混ざらないよう、測る前に暖める
    // （ADR 0021 L3。実機のエンジンは起動時に暖める）。統計（呼び出し、時間切れ）には入らない。
    if let Some(lm) = stack.lm.as_ref() {
        lm.warm_up();
    }

    let lm_report = stack.lm.clone().map(|lm| -> LmReporter {
        Arc::new(move || {
            let s = lm.stats();
            LmReport {
                calls: s.calls,
                timeouts: s.timeouts,
                late_hits: s.late_hits,
            }
        })
    });
    let ranker_name = set.as_str();
    Ok(Setup {
        converter: stack.converter,
        description: stack.meta.description,
        ranker_name: ranker_name.into(),
        store: stack.learning,
        vectors: stack.vectors,
        lm_report,
        _temp: temp,
    })
}

/// 引数名を含むエラー文にそろえる。
fn map_stack_error(e: StackError) -> SetupError {
    match e {
        StackError::MissingVectors => SetupError::Config(
            "--ranker topic needs vectors.bin (--vectors, or <dict>/vectors.bin)".into(),
        ),
        StackError::LmNotBuilt => SetupError::Config(
            "--lm needs a build with the `lm` feature: cargo build -p ikari-eval --features lm-metal (requires cmake)"
                .into(),
        ),
        StackError::LmModelNotFound(path) => {
            SetupError::Config(format!("--lm: {} does not exist", path.display()))
        }
        other => other.into(),
    }
}

/// 郵便番号の表の置き場所。指定がなければ `<dict_dir>/postal.tsv`
pub fn default_postal_path(dict_dir: &Path) -> PathBuf {
    dict_dir.join(POSTAL_TSV_FILE_NAME)
}

/// 指定がなければ `<dict_dir>/vectors.bin`
pub fn default_vectors_path(dict_dir: &Path) -> PathBuf {
    dict_dir.join(VECTORS_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::mock_grammar;
    use ikari_core::InMemoryDictionary;

    fn mock() -> (Arc<dyn Dictionary>, Arc<dyn Grammar>) {
        let g = mock_grammar();
        let mut d = InMemoryDictionary::new(g.grammar_id());
        d.insert("ひんし", "品詞", 1, 1, 100, 0);
        d.insert("ひんし", "瀕死", 1, 1, 200, 0);
        (Arc::new(d), Arc::new(g))
    }

    #[test]
    fn profile_choice_parses() {
        assert_eq!(ProfileChoice::parse("none"), ProfileChoice::None);
        assert_eq!(
            ProfileChoice::parse("it"),
            ProfileChoice::Named("it".into())
        );
        assert_eq!(ProfileChoice::default(), ProfileChoice::PerCase);
    }

    #[test]
    fn cost_setup_is_the_phase0_converter() {
        let (d, g) = mock();
        let setup = build(d, g, &SetupOptions::default()).unwrap();
        assert_eq!(setup.ranker_name, "cost");
        assert!(setup.description.contains("ranker=cost"));
        assert!(setup.store.is_none() && setup.lm_report.is_none());
        let mut conv = setup
            .converter
            .convert("ひんし", &ikari_core::Context::new());
        assert_eq!(conv.page(0).unwrap().candidates[0].surface, "品詞");
    }

    #[test]
    fn ranker_name_follows_the_requested_set() {
        for set in [RankerSet::Analyzer, RankerSet::Doc, RankerSet::Full] {
            let (d, g) = mock();
            let opts = SetupOptions {
                ranker: Some(set),
                ..Default::default()
            };
            let setup = build(d, g, &opts).unwrap();
            assert_eq!(setup.ranker_name, set.as_str());
        }
    }

    #[test]
    fn lm_requires_the_full_ranker() {
        let (d, g) = mock();
        let opts = SetupOptions {
            lm: Some(PathBuf::from("/nonexistent.gguf")),
            ..Default::default()
        };
        let e = build(d, g, &opts).err().expect("must fail");
        assert!(e.to_string().contains("other than cost"), "{e}");
    }

    #[test]
    fn unreadable_ranker_config_is_a_config_error() {
        let (d, g) = mock();
        let opts = SetupOptions {
            ranker: Some(RankerSet::Full),
            ranker_config: Some(PathBuf::from("/nonexistent/ranker.conf")),
            ..Default::default()
        };
        let e = build(d, g, &opts).err().expect("must fail");
        assert!(e.to_string().contains("cannot read"), "{e}");
    }

    #[test]
    fn default_vectors_path_is_inside_the_dict_dir() {
        assert_eq!(
            default_vectors_path(Path::new("/d")),
            PathBuf::from("/d/vectors.bin")
        );
    }

    #[cfg(not(feature = "lm"))]
    #[test]
    fn lm_without_the_feature_explains_how_to_build() {
        let (d, g) = mock();
        let opts = SetupOptions {
            ranker: Some(RankerSet::Doc),
            lm: Some(PathBuf::from("x.gguf")),
            ..Default::default()
        };
        let e = build(d, g, &opts).err().expect("must fail");
        assert!(e.to_string().contains("lm-metal"), "{e}");
    }

    /// full 構成の組み立てには、レーンU（LearningStore）、レーンB（解析器と Ranker）が要る。
    #[test]
    fn full_setup_builds_and_converts() {
        let (d, g) = mock();
        let opts = SetupOptions {
            ranker: Some(RankerSet::Full),
            ..Default::default()
        };
        let setup = build(d, g, &opts).unwrap();
        assert!(
            setup
                .description
                .contains("ranker=full(left_context,kana_style,doc_cache,learning,profile")
        );
        assert!(setup.store.is_some());
        let mut conv = setup
            .converter
            .convert("ひんし", &ikari_core::Context::new());
        assert!(conv.page(0).is_some());
    }

    #[test]
    fn presets_resolve_by_id() {
        assert!(preset_profile("it").is_some());
        assert!(preset_profile("no-such-domain").is_none());
    }
}
