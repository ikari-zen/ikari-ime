//! 変換器の組み立て（辞書、ユーザー辞書、解析器、Ranker、プロファイル、学習）。ADR 0002 §14。
//!
//! 組み立て本体は `ikari-stack`（`AssemblyMode::Runtime`）にあり、ここは CLI の引数との対応と、
//! 警告の表示を持つ。方針（`ikari-stack` の説明も参照）:
//! - `--ranker cost` は単語コストと連接コストだけ（フェーズ0と同じ。ユーザー辞書は使う）。
//! - `--ranker full`（既定）は、**使える**ものだけを足す。文脈の解析器は、辞書が表記の索引を持つ（形式 v2）ときだけ。
//! - ユーザーデータが壊れているときは、黙って無視せずエラーにする（学習ファイルは上書きされうるため）。
//!   ユーザー辞書だけは、展開できなくても警告を出して変換を続ける。

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ikari_core::user::UserDir;
use ikari_core::{
    ConvertOptions, DateOptions, Dictionary, PostalOptions, UnicodeOptions, VECTORS_FILE_NAME,
    WordVectors,
};
use ikari_stack::{
    AssemblyMode, LmOverrides, ProfileSel, StackError, StackOptions, StackPaths,
    load_vectors as stack_load_vectors, read_ranker_config,
};

use crate::args::{CommonArgs, DICT_DIR_ENV, PROFILE_NONE, resolve_dict_dir_with};

pub use ikari_stack::{LoadedSet, Stack};

/// 組み立ての失敗を、CLI の従来のエラー文にする。
fn message(e: StackError) -> String {
    e.detail()
}

fn print_warnings(warnings: &[String]) {
    for w in warnings {
        eprintln!("warning: {w}");
    }
}

/// 辞書ディレクトリを決める（`--dict`、`$IKARI_DICT_DIR`、ユーザーデータの `dict/`、既定の順）。
fn resolve_dict_dir(dict_arg: Option<&Path>, user_dir: Option<&UserDir>) -> PathBuf {
    // ユーザーデータの dict/（install-data.sh の置き場所）があれば、環境変数の次に使う
    let user_dict = user_dir
        .cloned()
        .or_else(|| UserDir::resolve(None))
        .map(|d| d.dict_dir())
        .filter(|p| p.is_dir());
    resolve_dict_dir_with(dict_arg, std::env::var_os(DICT_DIR_ENV), user_dict)
}

/// 辞書ディレクトリとユーザー辞書を開く。ユーザー辞書の不具合は警告にとどめる。
pub fn load_dictionaries(
    dict_arg: Option<&Path>,
    user_dir: Option<&UserDir>,
) -> Result<LoadedSet, Box<dyn Error>> {
    let dict_dir = resolve_dict_dir(dict_arg, user_dir);
    let set = ikari_stack::load_dictionaries(&dict_dir, user_dir).map_err(message)?;
    print_warnings(&set.warnings);
    Ok(set)
}

/// 組み立ての注文。
pub struct StackRequest<'a> {
    pub common: &'a CommonArgs,
    pub page_size: usize,
    /// 学習ストアを開く（`--commit` やセッション）。`--ranker full` では常に開く
    pub open_learning: bool,
    /// 学習データが空でも `LearningRanker` を入れる（セッションの途中で学習が増えるため）
    pub always_learning_ranker: bool,
}

/// --lm-budget-ms が明示されたときは設定ファイルより優先する
fn lm_overrides(common: &CommonArgs) -> LmOverrides {
    #[cfg(feature = "lm")]
    return LmOverrides {
        budget_ms: common.lm_budget_ms,
        idle_unload_secs: None,
        idle_unload_gpu_secs: None,
        gpu: None,
    };
    #[cfg(not(feature = "lm"))]
    {
        let _ = common;
        LmOverrides::default()
    }
}

/// 辞書ディレクトリを決めて `ikari_stack::build_stack` で組み立てる。警告は標準エラーに出す。
pub fn build_stack(req: &StackRequest<'_>) -> Result<Stack, Box<dyn Error>> {
    let common = req.common;
    let user_dir = UserDir::resolve(common.user_dir.as_deref());
    let kv = match &common.ranker_config {
        Some(p) => read_ranker_config(p).map_err(message)?,
        None => BTreeMap::new(),
    };
    let paths = StackPaths {
        dict_dir: resolve_dict_dir(common.dict.as_deref(), user_dir.as_ref()),
        user_dir: user_dir.as_ref().map(|d| d.root().to_path_buf()),
        vectors: common.vectors.clone(),
        #[cfg(feature = "lm")]
        lm_model: common.lm.clone(),
        #[cfg(not(feature = "lm"))]
        lm_model: None,
        no_packs: common.no_packs,
    };
    let opts = StackOptions {
        ranker: common.ranker.into(),
        ranker_config: kv,
        lm: lm_overrides(common),
        profile: match common.profile.as_deref() {
            Some(PROFILE_NONE) => ProfileSel::None,
            Some(id) => ProfileSel::Named(id.to_owned()),
            None => ProfileSel::User,
        },
        convert: ConvertOptions {
            page_size: req.page_size,
            chunk_size: req.page_size,
            ..ConvertOptions::default()
        },
        mode: AssemblyMode::Runtime,
        no_analyzer: common.no_analyzer,
        open_learning: req.open_learning,
        always_learning_ranker: req.always_learning_ranker,
        clock: None,
        // 郵便番号の表は <dict>/postal.tsv（ADR 0032）
        postal: PostalOptions {
            te_prefix: common.postal_te_prefix,
            ..PostalOptions::default()
        },
        unicode: UnicodeOptions {
            notation: common.unicode_notation,
        },
        date: DateOptions {
            weekday_dates: common.date_weekday_dates,
        },
        ..StackOptions::default()
    };
    let stack = ikari_stack::build_stack(&paths, &opts).map_err(message)?;
    print_warnings(&stack.meta.warnings);
    Ok(stack)
}

/// 辞書に対応する vectors.bin を開く（`ingest` 用）。明示の指定が開けなければエラー。
pub fn load_vectors(
    common: &CommonArgs,
    set: &LoadedSet,
) -> Result<Option<Arc<dyn WordVectors>>, Box<dyn Error>> {
    let default = set.dict_dir.join(VECTORS_FILE_NAME);
    let mut warnings = Vec::new();
    let vectors = stack_load_vectors(
        set.system.dict_id(),
        common.vectors.as_deref(),
        Some(&default),
        AssemblyMode::Runtime,
        &mut warnings,
    )
    .map_err(message)?;
    print_warnings(&warnings);
    Ok(vectors)
}
