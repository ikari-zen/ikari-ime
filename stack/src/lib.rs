//! ikari-stack: Converter、Ranker、LM の共有の組み立て（ADR 0004 §3.2）。
//!
//! `cli/src/stack.rs` と `eval/src/setup.rs` の共通部分。エンジン、CLI、eval が同じ組み立てを使う。
//! 移すときに挙動を変えない（確認は eval の ext test の `--out-cases` がビット単位で一致すること）。
//!
//! 使い分け:
//! - `build_stack`: 辞書ディレクトリから開いて組み立てる（エンジンと CLI）。
//! - `assemble`: 開いた辞書と文法から組み立てる（eval。モックの辞書でも使える）。
//!
//! CLI と eval で組み立ての規則が少し違う（学習やプロファイルの Ranker を常に入れるか、解析器に
//! 表記の索引を要求するか、など）。その違いは [`AssemblyMode`] に集めた。

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ikari_core::user::{IT_PACK_ID, UserDictionary, UserDir, builtin_it_pack};
use ikari_core::{
    CalculatorRewriter, Clock, CompositeRanker, ConvertOptions, Converter, CostRanker, DateOptions,
    DateRewriter, Dictionary, DictionarySet, DocumentCacheConfig, DocumentCacheRanker,
    EmoticonRewriter, Grammar, GrammarFile, KanaStyleConfig, KanaStyleRanker, KanaStyleTable,
    LatticeContextAnalyzer, LearningConfig, LearningRanker, LearningStore, LeftContextConfig,
    LeftContextRanker, NullAnalyzer, NumberRewriter, POSTAL_TSV_FILE_NAME, PostalOptions,
    PostalRewriter, PostalTable, Profile, ProfileConfig, ProfileRanker, ProfileWeight, Ranker,
    Rewriter, SymbolRewriter, SystemDictionary, TopicConfig, TopicRanker, UnicodeOptions,
    UnicodeRewriter, VECTORS_FILE_NAME, WeekdayRewriter, WordVectors, open_dir, open_vectors,
};

mod assemble;
mod lm;
mod load;
#[cfg(test)]
mod tests;

use assemble::assemble_with;
pub use assemble::{StackInputs, assemble};
pub use load::{
    Layered, LoadOptions, LoadedSet, layer_dictionaries, load_dictionaries, load_dictionaries_with,
    load_vectors,
};

/// LM 用のキー（`zenz.` で始まる）。他の Ranker の設定には渡さない
const LM_KEY_PREFIX: &str = "zenz.";
const KEY_LM_BUDGET_MS: &str = "zenz.budget_ms";
const KEY_LM_IDLE_UNLOAD_SECS: &str = "zenz.idle_unload_secs";

/// 組み立てに使うパス。
#[derive(Clone, Debug, Default)]
pub struct StackPaths {
    pub dict_dir: PathBuf,
    /// ユーザーデータ（ユーザー辞書、学習、プロファイル）。None なら使わない
    pub user_dir: Option<PathBuf>,
    /// 明示した vectors.bin。None なら `dict_dir` の既定の vectors.bin を探す（なければ使わない）
    pub vectors: Option<PathBuf>,
    /// zenz の GGUF。None なら LM を使わない
    pub lm_model: Option<PathBuf>,
    /// 同梱の語彙集（IT）を使わない。既定は使う（ADR 0012 B20-4。設定 `dict.pack_it`）
    pub no_packs: bool,
}

/// eval の `--ranker` と同じ意味。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RankerSet {
    /// 単語コストと連接コストだけ（フェーズ0。R0）
    Cost,
    /// R1: cost + 文脈の解析器 + Rewriter（Ranker は CostRanker のまま）
    Analyzer,
    /// R2: R1 + 左文脈、かなの表記、DocumentCache
    Doc,
    /// R3: R2 + Topic（vectors.bin が要る）
    Topic,
    /// 解析器、Rewriter、使える Ranker すべて（学習とプロファイルを含む）
    #[default]
    Full,
}

impl RankerSet {
    /// 受け付ける値の一覧（CLI の候補表示用）
    pub const ALL: [RankerSet; 5] = [
        RankerSet::Cost,
        RankerSet::Analyzer,
        RankerSet::Doc,
        RankerSet::Topic,
        RankerSet::Full,
    ];

    pub fn parse(s: &str) -> Option<RankerSet> {
        match s {
            "cost" => Some(RankerSet::Cost),
            "analyzer" => Some(RankerSet::Analyzer),
            "doc" => Some(RankerSet::Doc),
            "topic" => Some(RankerSet::Topic),
            "full" => Some(RankerSet::Full),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RankerSet::Cost => "cost",
            RankerSet::Analyzer => "analyzer",
            RankerSet::Doc => "doc",
            RankerSet::Topic => "topic",
            RankerSet::Full => "full",
        }
    }

    fn uses_doc(self) -> bool {
        matches!(self, RankerSet::Doc | RankerSet::Topic | RankerSet::Full)
    }

    fn uses_topic(self) -> bool {
        matches!(self, RankerSet::Topic | RankerSet::Full)
    }
}

/// プロファイルの選び方（eval の `ProfileChoice` と同じ意味）。
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ProfileSel {
    /// ユーザーのプロファイルファイルに従う
    #[default]
    User,
    /// プロファイルを使わない
    None,
    /// 名前付きのプリセット
    Named(String),
}

/// 組み立ての規則の違い（CLI／エンジンと eval で、既存の挙動を保つため）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AssemblyMode {
    /// CLI とエンジン。使えるものだけを足す。
    /// - 文脈の解析器は、辞書が表記の索引を持つ（`dict_id` がある）ときだけ
    /// - 学習とプロファイルの Ranker は、データがあるときだけ
    /// - 既定の vectors.bin が壊れていれば警告にとどめる
    /// - Cost でも Rewriter を入れる
    #[default]
    Runtime,
    /// eval。構成を固定して比べるための組み立て。
    /// - 解析器は `no_analyzer` でなければ常に入れる
    /// - Full なら学習とプロファイルの Ranker を常に入れる（プロファイルはケースの文脈で渡る）
    /// - vectors.bin が開けなければエラー
    /// - Cost は Rewriter も解析器もなし。Cost 以外は Ranker を常に `CompositeRanker` で包む
    Eval,
}

/// 呼び出し側が明示した LM の設定。設定ファイル（`ranker_config` の `zenz.*`）より優先する。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LmOverrides {
    /// LM の待ち時間の上限（`zenz.budget_ms`）
    pub budget_ms: Option<u64>,
    /// 使われなくなってからモデルを解放するまでの秒数（`zenz.idle_unload_secs`）
    pub idle_unload_secs: Option<u64>,
}

impl LmOverrides {
    /// 設定ファイルの値の上に、明示した値を重ねる。
    fn apply(self, kv: &mut BTreeMap<String, String>) {
        if let Some(ms) = self.budget_ms {
            kv.insert(KEY_LM_BUDGET_MS.to_owned(), ms.to_string());
        }
        if let Some(secs) = self.idle_unload_secs {
            kv.insert(KEY_LM_IDLE_UNLOAD_SECS.to_owned(), secs.to_string());
        }
    }
}

#[derive(Clone, Default)]
pub struct StackOptions {
    pub ranker: RankerSet,
    /// `--ranker-config` の key=value。`zenz.*` は LM にだけ渡す
    pub ranker_config: BTreeMap<String, String>,
    /// 明示した LM の設定（`ranker_config` より優先）
    pub lm: LmOverrides,
    pub profile: ProfileSel,
    pub convert: ConvertOptions,
    pub mode: AssemblyMode,
    /// 文脈の解析器を使わない（`--no-analyzer`）
    pub no_analyzer: bool,
    /// Ranker が Cost でも学習ストアを開く（`--commit` やセッション）
    pub open_learning: bool,
    /// 学習データが空でも `LearningRanker` を入れる（セッションの途中で学習が増えるため。Runtime のみ）
    pub always_learning_ranker: bool,
    /// Rewriter の時計。None なら SystemClock
    pub clock: Option<Arc<dyn Clock>>,
    /// 住所 ↔ 郵便番号の表（`postal.tsv`。ADR 0032）。None なら `build_stack_with` が
    /// `<dict_dir>/postal.tsv` を使う（`assemble` 単体では表なし = 何も出さない）。ファイルがなければ空の表
    pub postal_tsv: Option<PathBuf>,
    /// 住所 ↔ 郵便番号の動作（設定 `postal.japanese`、`postal.te_prefix`）
    pub postal: PostalOptions,
    /// Unicode 表記の動作（設定 `unicode.notation`）
    pub unicode: UnicodeOptions,
    /// 日付の動作（設定 `date.weekday_dates`）
    pub date: DateOptions,
}

impl fmt::Debug for StackOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StackOptions")
            .field("ranker", &self.ranker)
            .field("ranker_config", &self.ranker_config)
            .field("lm", &self.lm)
            .field("profile", &self.profile)
            .field("convert", &self.convert)
            .field("mode", &self.mode)
            .field("no_analyzer", &self.no_analyzer)
            .field("open_learning", &self.open_learning)
            .field("always_learning_ranker", &self.always_learning_ranker)
            .field("clock", &self.clock.is_some())
            .field("postal_tsv", &self.postal_tsv)
            .field("postal", &self.postal)
            .field("unicode", &self.unicode)
            .field("date", &self.date)
            .finish()
    }
}

/// 構成の説明（`--timing` やエンジンの `HelloReply.dict_label` 用）。
#[derive(Clone, Debug, Default)]
pub struct StackMeta {
    pub description: String,
    pub dict_label: String,
    /// 辞書とユーザーデータを開くのにかかった時間
    pub open_elapsed: Duration,
    /// ユーザー辞書の語が1つ以上入っている
    pub has_user_dict: bool,
    /// 続行できた不具合（ユーザー辞書、プロファイル、既定の vectors.bin）。ユーザーの文字列は含まない
    pub warnings: Vec<String>,
    /// 学習ファイルが不正で退避したとき、退避先（この組み立てで 1 回だけ Some。殻への通知に使う）
    pub learning_quarantined: Option<PathBuf>,
}

/// LM の統計（ログと eval の欄用）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LmStats {
    /// 採点の対象があった `rank()` の呼び出し数
    pub calls: u64,
    pub timeouts: u64,
    /// 遅れて届いた結果を使った回数
    pub late_hits: u64,
    pub loads: u64,
    pub load_failures: u64,
}

/// LM への操作の口（`ikari-lm` を知らずにエンジンが使うため）。
pub trait LmHandle: Send + Sync {
    /// ダミーの短い採点を1回実行し、Metal の初期化とモデルの読み込みを済ませる（`ZenzRanker::warm_up`）
    fn warm_up(&self);
    fn stats(&self) -> LmStats;
    fn is_loaded(&self) -> bool;
}

pub struct Stack {
    pub converter: Arc<Converter>,
    pub learning: Option<Arc<LearningStore>>,
    pub lm: Option<Arc<dyn LmHandle>>,
    pub meta: StackMeta,
    /// 確定の学習（`LearningStore::record`）に渡す
    pub vectors: Option<Arc<dyn WordVectors>>,
    pub profile: Option<Arc<Profile>>,
    pub dict: Arc<dyn Dictionary>,
    pub grammar: Arc<dyn Grammar>,
    /// LM の Ranker（組み立て直すときに使い回す）
    pub lm_ranker: Option<Arc<dyn Ranker>>,
    /// 住所 ↔ 郵便番号の表（組み立て直すときに使い回す。読み込みが重いので、ファイルが変わるまで読み直さない）
    pub postal: Option<Arc<PostalTable>>,
}

#[derive(Debug)]
pub enum StackError {
    /// 辞書、vectors、ユーザーデータ、LM のどれかを開けない
    Open(String),
    /// 設定（ranker_config など）が不正
    Config(String),
    /// `RankerSet::Topic` なのに vectors.bin がない
    MissingVectors,
    /// LM を求められたが、このビルドに `lm` feature がない
    LmNotBuilt,
    /// 指定された LM のモデルのファイルがない
    LmModelNotFound(PathBuf),
}

impl StackError {
    /// 種類の接頭辞を除いた説明（CLI が自分の書式で出すため）。
    pub fn detail(&self) -> String {
        match self {
            StackError::Open(m) | StackError::Config(m) => m.clone(),
            StackError::MissingVectors => {
                "ranker topic needs vectors.bin (explicit path, or <dict>/vectors.bin)".into()
            }
            StackError::LmNotBuilt => {
                "LM needs a build with the `lm` feature (cargo build --features lm-metal; requires cmake)"
                    .into()
            }
            StackError::LmModelNotFound(p) => format!("LM model not found: {}", p.display()),
        }
    }
}

impl fmt::Display for StackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StackError::Open(_) | StackError::LmModelNotFound(_) => {
                write!(f, "open: {}", self.detail())
            }
            StackError::Config(_) | StackError::MissingVectors | StackError::LmNotBuilt => {
                write!(f, "config: {}", self.detail())
            }
        }
    }
}

impl std::error::Error for StackError {}

fn open_err(e: impl fmt::Display) -> StackError {
    StackError::Open(e.to_string())
}

fn config_err(name: &str, e: impl fmt::Display) -> StackError {
    StackError::Config(format!("{name}: {e}"))
}

/// 設定ファイルの行頭がこれなら、コメントとして読み飛ばす
const CONFIG_COMMENT_PREFIX: char = '#';

/// `--ranker-config` の `key=value` の行（空行と `#` の行は読み飛ばす）を読む。
/// `=` のない行と、キーが空の行はエラー。
pub fn parse_ranker_config(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut kv = BTreeMap::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(CONFIG_COMMENT_PREFIX) {
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .ok_or_else(|| format!("line {}: expected `key=value`", i + 1))?;
        let k = k.trim();
        if k.is_empty() {
            return Err(format!("line {}: empty key", i + 1));
        }
        kv.insert(k.to_owned(), v.trim().to_owned());
    }
    Ok(kv)
}

/// `--ranker-config` のファイルを読む。
pub fn read_ranker_config(path: &Path) -> Result<BTreeMap<String, String>, StackError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| StackError::Config(format!("cannot read {}: {e}", path.display())))?;
    parse_ranker_config(&text).map_err(|e| StackError::Config(format!("{}: {e}", path.display())))
}

/// LM の Ranker と、その操作の口。
pub type LmParts = (Arc<dyn Ranker>, Arc<dyn LmHandle>);

/// 組み立て直すときに使い回すもの（LM のモデルを読み直さず、学習のメモリを失わないため）。
#[derive(Default, Clone)]
pub struct Reuse {
    pub learning: Option<Arc<LearningStore>>,
    pub lm: Option<LmParts>,
    /// 読み込み済みの郵便番号の表。`postal.tsv` が変わったときは None にして読み直させる
    pub postal: Option<Arc<PostalTable>>,
}

impl Reuse {
    /// 作った Stack から取り出す。
    pub fn from_stack(stack: &Stack) -> Reuse {
        Reuse {
            learning: stack.learning.clone(),
            lm: stack.lm_ranker.clone().zip(stack.lm.clone()),
            postal: stack.postal.clone(),
        }
    }

    /// LM だけを使い回す。学習ストアはファイルから開き直す（インポートで学習ファイルを差し替えたあと、
    /// 古いメモリが次の保存でファイルを上書きしないように。ADR 0005 §8.3）。
    pub fn without_learning(stack: &Stack) -> Reuse {
        Reuse {
            learning: None,
            ..Reuse::from_stack(stack)
        }
    }
}

/// 辞書ディレクトリから開いて組み立てる。
pub fn build_stack(paths: &StackPaths, opts: &StackOptions) -> Result<Stack, StackError> {
    build_stack_with(paths, opts, &Reuse::default())
}

/// `build_stack` の、学習ストアと LM を使い回す版（ユーザー辞書の更新で組み立て直すとき）。
pub fn build_stack_with(
    paths: &StackPaths,
    opts: &StackOptions,
    reuse: &Reuse,
) -> Result<Stack, StackError> {
    let user_dir = paths.user_dir.as_ref().map(UserDir::new);
    let load = LoadOptions {
        it_pack: !paths.no_packs,
    };
    let set = load_dictionaries_with(&paths.dict_dir, user_dir.as_ref(), load)?;
    let inputs = StackInputs {
        dict: set.dict.clone(),
        grammar: set.grammar_dyn(),
        default_vectors: Some(paths.dict_dir.join(VECTORS_FILE_NAME)),
        has_user_dict: set.has_user_dict,
        open_elapsed: set.open_elapsed,
        dict_label: dict_label(set.system.dict_id()),
        warnings: set.warnings,
    };
    // 郵便番号の表は辞書と同じ dict/ に置く（なければ空の表。emoji.tsv と同じ流儀）
    let mut opts = opts.clone();
    if opts.postal_tsv.is_none() {
        opts.postal_tsv = Some(paths.dict_dir.join(POSTAL_TSV_FILE_NAME));
    }
    assemble_with(inputs, paths, &opts, reuse)
}

fn dict_label(dict_id: Option<u64>) -> String {
    dict_id.map_or_else(|| "unknown".to_owned(), |id| format!("{id:016x}"))
}
