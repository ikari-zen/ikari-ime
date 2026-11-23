//! ikari-engine: 常駐エンジン（セッション、先行計算、キャッシュ、protobuf の処理）。ADR 0004 §7。
//!
//! - `Engine::handle` は Envelope（protobuf のバイト列）を受けて Envelope を返す。決して panic せず、
//!   エラーも Envelope で返す。複数スレッドから同時に呼んでよい。
//! - `Prefetch` は読みが変わるたびに殻が投げる。セッションごとに最新の 1 件だけを別スレッドで計算し、
//!   キャッシュに置く。`Convert` はキャッシュに当たればすぐ返し、計算中なら締め切りまで待ち、
//!   間に合わなければ LM を使わずにその場で変換する。
//! - 返したページは差し替えない（NFR-05。`Conversion` の性質のまま）。
//! - 学習はメモリに記録し、5 秒ごとと `EndSession`、`Shutdown`、`Drop` で保存する。
//! - データのディレクトリは `engine.lock` を OS のファイルロック（Unix は flock、Windows は `LockFileExW`）で持つ（CLI の学習ファイルの書き込みとの排他）。
//! - 管理の要求（ユーザー辞書、学習、エクスポート／インポート、プロファイル。ADR 0005 §8）は `admin.rs`。
//!
//! 構成: 入口は `dispatch`、接続は `connection`（ADR 0064）、セッションは `session`、変換の要求は `convert`、LM の状態は `lm`、
//! 檻役とのつなぎは `lm_cage`（ADR 0077）、データの監視と保存は `data`、データの場所は `data_location`（ADR 0080）、
//! モデルの選択・取り込み・検査は `model_select`・`model_import`・`model_check`（ADR 0058、0060）、設定は `settings`、
//! 裏のスレッドは `workers`、文節は `segments`。このファイルは `Engine`、`Inner`、上限の定数。

mod admin;
mod cache;
mod connection;
pub mod context_limits;
mod convert;
mod ctx_trace;
mod data;
pub mod data_location;
mod diag;
mod diag_schema;
mod dispatch;
mod lm;
mod lm_cage;
mod lock;
mod log;
#[cfg(test)]
mod mock_seam;
mod model_check;
mod model_import;
mod model_select;
mod rss;
mod segments;
mod session;
#[cfg(test)]
use crate::convert::DEFAULT_CONVERT_WAIT;
mod settings;
mod workers;

use std::collections::HashMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use ikari_core::user::{UserDir, today_unix_day};
use ikari_core::{
    CandidateOrigin, Context, ConvertOptions, DateOptions, LmBudget, PostalOptions, UnicodeOptions,
};
use ikari_proto::SCHEMA_VERSION;
use ikari_proto::v1 as pb;
use ikari_proto::v1::{Envelope, ErrorCode, ErrorReply, LmState, envelope};
use ikari_runmark::RunMark;
use ikari_stack::lm_cage::CageSlot;
use ikari_stack::{
    DictSetReject, LmIsolation, LmOverrides, LmStats, ProfileSel, RankerSet, Reuse, Stack,
    StackError, StackOptions, StackPaths, build_stack, build_stack_with, check_dictionary_set,
    lm_host_exe_name,
};
use prost::Message;

use crate::cache::{
    CacheEntry, CacheKey, CacheOutcome, ConversionTable, Job, MissReason, PrefetchCache,
    PrefetchContext, PrefetchQueue, SharedConversion, lock as lock_mutex,
};
use crate::connection::{ConnectionIds, SessionKey};
use crate::context_limits::ContextLimits;
use crate::convert::candidate_kind;
use crate::data::{DataWatch, SaveState, load_emoji_reply};
use crate::data_location::{DataLocations, DataStatus, LocationKind};
use crate::log::{EngineLog, LOG_DIR_NAME, RUN_MARK_FILE_NAME};
use crate::segments::{LookJob, LookaheadQueue, RealSeam, SegEntry, SegmentSeam, layout_reply};
use crate::session::Session;

pub use crate::connection::{Connection, ConnectionId};
pub use crate::diag::install_crash_handlers;
pub use crate::lock::{EngineLock, LockError};
pub use crate::settings::{SETTINGS_FILE_NAME, Settings};
pub use ikari_proto::v1::EngineConfig;

/// データのディレクトリの中の配置（ADR 0004 §2）
pub const MODELS_DIR_NAME: &str = "models";
pub const MODEL_FILE_NAME: &str = "zenz-v3.2-small-Q5_K_M.gguf";

/// 読みの上限（文字数）
pub const MAX_READING_CHARS: usize = 256;
/// 文脈の上限（文字数。超えた分は古い側を切る）
pub const MAX_CONTEXT_CHARS: usize = 2000;
/// 確定履歴の 1 件の表記の上限（文字数）
const MAX_SURFACE_CHARS: usize = 2000;
/// 要求 1 つの上限（バイト）。読み 256 文字と文脈 2000 文字×2 が十分に入る大きさ。
/// `ImportData` 以外はこれを超えると `BAD_REQUEST`（上限は 2 段。ADR 0005 §8.3）
pub const MAX_REQUEST_BYTES: usize = 256 * 1024;
/// 受け付ける要求の上限（バイト）。`ImportData`（エクスポートのファイルを運ぶ）だけがこの大きさまで許される
pub const MAX_ADMIN_REQUEST_BYTES: usize = 64 * 1024 * 1024;
/// 1 ページの件数（番号ラベル 1〜9）
pub const MAX_PAGE_SIZE: u32 = 9;
/// 要求できるページの最大の番号（際限のない生成を避ける）
const MAX_PAGE_INDEX: usize = 1000;
/// セッションの数の上限（古いものから捨てる）
const SESSION_LIMIT: usize = 256;
/// 学習を保存する間隔
const SAVE_INTERVAL: Duration = Duration::from_secs(5);
/// 後始末のスレッドの周期
const JANITOR_TICK: Duration = Duration::from_secs(1);
/// ユーザー辞書とプロファイルの更新を見る間隔
const DATA_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// `idle_unload_secs == 0`（解放しない）を LM の設定に渡すときの値（10 年）
const NEVER_UNLOAD_SECS: u64 = 315_360_000;
/// どの場所の辞書とモデルを使っているかの行（`data_location dict=<語> model=<語>`。ADR 0080 §3.2）
const KIND_DATA_LOCATION: &str = "data_location";
/// 辞書の組を検査に落とした行（`dict_reject location=<語> reason=<語>`。次の場所を使った）
const KIND_DICT_REJECT: &str = "dict_reject";
/// `data_location` の行で、LM を読まないときの語
const NO_MODEL_WORD: &str = "none";

/// エンジン本体。複数スレッドから `handle` を同時に呼んでよい（`Send + Sync`）。
pub struct Engine {
    inner: Arc<Inner>,
    workers: Vec<JoinHandle<()>>,
    /// 起動中の印（ログが有効なときだけ。ADR 0051 §3.4）。`Drop` の最後に `exit=clean` を書く
    run_mark: Option<RunMark>,
    // 最後に解放する（学習の保存が済んでから、別のエンジンに譲る）
    _lock: Option<EngineLock>,
}

impl fmt::Debug for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Engine").finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum EngineError {
    /// 設定が不正（データのディレクトリがないなど）
    Config(String),
    /// 辞書やユーザーデータを開けない、または別のエンジンが動いている
    Open(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::Config(m) => write!(f, "config: {m}"),
            EngineError::Open(m) => write!(f, "open: {m}"),
        }
    }
}

impl std::error::Error for EngineError {}

/// ユーザー辞書やプロファイルの更新で Stack を組み立て直す関数。学習ストアと LM は使い回す。
pub type StackBuilder = Box<dyn Fn(&Reuse) -> Result<Stack, StackError> + Send + Sync>;

/// `Engine::from_parts` の部品（モックの Stack でテストするため。`open` もこれを使う）。
pub struct EngineParts {
    pub stack: Stack,
    /// None ならユーザー辞書とプロファイルを読み直さない
    pub rebuild: Option<StackBuilder>,
    /// ユーザー辞書とプロファイルの更新を見る場所、ログを書く場所
    pub data_dir: Option<PathBuf>,
    /// LM を使うはずか（使えなければ `LM_FAILED`、使わない設定なら `LM_DISABLED`）
    pub lm_requested: bool,
    pub lm_budget: LmBudget,
    /// Space で計算中の先行計算を待つ上限（`engine.convert_wait_ms`。既定は `DEFAULT_CONVERT_WAIT`）
    pub convert_wait: Duration,
    pub lock: Option<EngineLock>,
}

/// LM の推論をどこで動かすか（ADR 0060 Q4）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LmHostChoice {
    /// エンジンの実行ファイルと同じフォルダの `ikari-lm-host` を、別プロセスとして使う（製品）。
    /// そこに無ければ、同じプロセスで動かす（feature `lm` のビルドだけ。なければ LM なし）
    #[default]
    Beside,
    /// 指定の実行ファイルを、別プロセスとして使う（テスト、計測）
    Exe(PathBuf),
    /// 同じプロセスで動かす（feature `lm` が要る。テスト、計測）
    InProcess,
}

impl LmHostChoice {
    /// 別プロセスとして起動する実行ファイル（絶対パス。PATH からは探さない）。None なら同じプロセス
    fn resolve(&self) -> Option<PathBuf> {
        match self {
            LmHostChoice::Beside => {
                let exe = std::env::current_exe().ok()?;
                // macOS の製品は、子が檻役の束にある（ADR 0077 §3.4。`lm_cage`）
                lm_cage::caged_host_program(&exe).or_else(|| {
                    exe.parent()
                        .map(|dir| dir.join(lm_host_exe_name()))
                        .filter(|path| path.is_file())
                })
            }
            LmHostChoice::Exe(path) => Some(path.clone()),
            LmHostChoice::InProcess => None,
        }
    }
}

/// エンジンの Stack の設定。`idle_unload_secs == 0` は解放しない。
pub fn stack_options(idle_unload_secs: u32, budget: LmBudget) -> StackOptions {
    let unload = if idle_unload_secs == 0 {
        NEVER_UNLOAD_SECS
    } else {
        u64::from(idle_unload_secs)
    };
    StackOptions {
        ranker: RankerSet::Full,
        lm: LmOverrides {
            // 待ちの長さは変換 1 回ごとの締め切りで決まる。ZenzRanker の `budget` は上限として最大に合わせる
            budget_ms: Some(budget.max.as_millis() as u64),
            idle_unload_secs: Some(unload),
            idle_unload_gpu_secs: None,
            gpu: None,
        },
        profile: ProfileSel::User,
        convert: ConvertOptions {
            page_size: MAX_PAGE_SIZE as usize,
            chunk_size: MAX_PAGE_SIZE as usize,
            lm_budget: budget,
            ..ConvertOptions::default()
        },
        // セッションの途中で学習が増えるので、空でも LearningRanker を入れる
        always_learning_ranker: true,
        // 壊れた学習ファイルで日本語が打てなくなるのを避ける（#271）
        quarantine_learning: true,
        ..StackOptions::default()
    }
}

impl Engine {
    /// 辞書、vectors、学習、LM を読み込み、LM の warm_up を別スレッドで始める。
    ///
    /// `data_dir`（ADR 0004 §2 の配置）の `dict/` を開く。`settings.tsv` の `engine.lm` と
    /// `engine.idle_unload_secs`、`engine.lm_deadline_ms`、`engine.convert_wait_ms` があれば `cfg` より優先する。LM のモデル（`models/` の GGUF）がない、
    /// またはこのビルドに LM がないときは、LM なしで動く（`HelloReply.lm = LM_FAILED`）。
    ///
    /// LM の推論は、エンジンの実行ファイルの隣の `ikari-lm-host` に別プロセスで頼む（[`LmHostChoice::Beside`]）。
    pub fn open(cfg: &EngineConfig) -> Result<Engine, EngineError> {
        Engine::open_with(cfg, &LmHostChoice::default())
    }

    /// [`Engine::open`] と同じ。LM の推論の場所を選ぶ（テストと計測のため）。
    pub fn open_with(cfg: &EngineConfig, lm_host: &LmHostChoice) -> Result<Engine, EngineError> {
        Engine::open_in(cfg, lm_host, data_location::bundled_data_dir().as_deref())
    }

    /// [`Engine::open_with`] の本体。`bundled` は同梱の場所（製品は実行ファイルのパスから決める。テストは渡す）。
    ///
    /// 辞書とモデルは、利用者のデータの場所 → 同梱の場所の順に、検査に通った最初のものを使う（ADR 0080 §3.2）。
    /// 辞書の組は最後の候補だけ検査せずに開き、開けなければ今までどおり起動に失敗する（どちらも無い・落ちるとき）
    pub(crate) fn open_in(
        cfg: &EngineConfig,
        lm_host: &LmHostChoice,
        bundled: Option<&Path>,
    ) -> Result<Engine, EngineError> {
        if cfg.data_dir.is_empty() {
            return Err(EngineError::Config("data_dir is empty".into()));
        }
        let data_dir = PathBuf::from(&cfg.data_dir);
        if !data_dir.is_dir() {
            return Err(EngineError::Config("data_dir is not a directory".into()));
        }
        secure_data_dir(&data_dir);
        let locations = DataLocations::new(&data_dir, bundled);
        let settings = settings::load(&data_dir);
        let lm_requested = settings.lm.unwrap_or(cfg.enable_lm);
        let idle_unload_secs = settings.idle_unload_secs.unwrap_or(cfg.idle_unload_secs);

        let lock = EngineLock::acquire(&data_dir).map_err(|e| EngineError::Open(e.to_string()))?;
        // inbox のモデルの束を取り込む（D-4 の条件 1。モデルのハンドルを開く前に、ここ 1 か所で）
        let imported = model_import::import_models(&data_dir, &model_select::format_words());
        // 別プロセスの子があれば、このビルドに llama.cpp が無くても LM を使える
        let lm_host = lm_host.resolve();
        // どのモデルを、どの檻の段で読むか（ADR 0060 §7、ADR 0072 §12.8 の 1。`model_select`）。
        // 自由指定は、段 2 のある構成でだけ読む（同じプロセスで動かすときは読まない）
        let resolved = model_select::resolve_model(
            lm_requested,
            &locations,
            &settings,
            model_select::user_models_offered(lm_host.is_some()),
        );
        // 照合したハンドルは、エンジンの寿命の間持ち、子に渡す（FC-3）。読まないビルドでは持たない
        let lm_handle = resolved
            .model
            .filter(|_| lm_host.is_some() || cfg!(feature = "lm"));
        // 読み込みの印は、子を起こす前に書く（D-4 の条件 5。最初に読み込めたら消す）
        let loading_mark = Arc::new(model_select::LoadingMark::default());
        if let Some(id) = resolved.loading.filter(|_| lm_handle.is_some()) {
            loading_mark.arm(&data_dir, id);
        }
        let lm_model = lm_handle.as_ref().map(|h| h.path().to_owned());
        // 子の起動の口（ADR 0077。`lm_cage`）。檻役の口は、開いたあとに `attach_lm_cage` で入る
        let lm_cage_slot = Arc::new(CageSlot::default());
        let lm_isolation = LmIsolation {
            cage: model_select::cage_for(resolved.origin),
            launcher: lm_cage::product_launcher(&lm_cage_slot),
            model: lm_handle,
        };
        // GPU のときだけ解放を遅くする対象か（Vulkan。Metal の挙動は変えない）。別プロセスのときは、
        // このビルドの feature ではなく OS で見る（子が Vulkan のビルドかは、エンジンからは分からない。
        // GPU に載らなければ、遅くする値は使われない）
        let slow_gpu_unload = cfg!(feature = "lm-vulkan") || (cfg!(windows) && lm_host.is_some());
        let dict = choose_dictionary_set(&locations);
        let paths = StackPaths {
            dict_dir: dict.dir.clone(),
            user_dir: Some(data_dir.clone()),
            vectors: None,
            lm_model,
            lm_host,
            no_packs: settings.pack_it == Some(false),
        };
        let lm_budget = LmBudget {
            max: settings.lm_deadline_or_default(),
            ..LmBudget::default()
        };
        let mut opts = stack_options(idle_unload_secs, lm_budget);
        opts.lm.gpu = Some(settings.lm_gpu_or_default());
        opts.lm_isolation = lm_isolation;
        opts.lm_format = resolved.format;
        if slow_gpu_unload {
            opts.lm.idle_unload_gpu_secs = Some(
                settings
                    .idle_unload_gpu_secs()
                    .map_or(NEVER_UNLOAD_SECS, u64::from),
            );
        }
        // 住所 ↔ 郵便番号の設定（表は <dict>/postal.tsv。ADR 0032）
        opts.postal = PostalOptions {
            japanese: settings.postal_japanese_or_default(),
            te_prefix: settings.postal_te_prefix_or_default(),
        };
        opts.unicode = UnicodeOptions {
            notation: settings.unicode_notation_or_default(),
        };
        opts.date = DateOptions {
            weekday_dates: settings.date_weekday_dates_or_default(),
        };
        // 郵便番号の表も、上から順に置かれている最初の場所（§3.2）。組み立て直すたびに選び直す
        // （利用者が後から置いた・消した表に合わせる）
        opts.postal_tsv = Some(locations.table_path(ikari_core::POSTAL_TSV_FILE_NAME));
        let stack = build_stack(&paths, &opts).map_err(|e| EngineError::Open(e.to_string()))?;
        let rebuild_locations = locations.clone();
        let rebuild: StackBuilder = Box::new(move |reuse| {
            let mut opts = opts.clone();
            opts.postal_tsv = Some(rebuild_locations.table_path(ikari_core::POSTAL_TSV_FILE_NAME));
            build_stack_with(&paths, &opts, reuse)
        });
        let status = DataStatus {
            dict: dict.kind,
            model: resolved.location,
        };
        let engine = Engine::from_parts_with(
            EngineParts {
                stack,
                rebuild: Some(rebuild),
                data_dir: Some(data_dir),
                lm_requested,
                lm_budget,
                convert_wait: settings.convert_wait_or_default(),
                lock: Some(lock),
            },
            None,
            lm_cage_slot,
            loading_mark,
            Some(OpenedData { locations, status }),
        );
        for (kind, reason) in &dict.rejects {
            engine.inner.record_dict_reject(*kind, *reason);
        }
        engine.inner.note_data_location(status);
        for outcome in &imported {
            engine.inner.record_model_import(outcome);
        }
        for (file_name, reason) in &resolved.rejects {
            engine.inner.record_lm_reject(file_name, *reason);
        }
        Ok(engine)
    }

    /// 組み立て済みの部品から作る。スレッドを起動し、LM があれば warm_up を始める。
    pub fn from_parts(parts: EngineParts) -> Engine {
        Engine::from_parts_seam(parts, None)
    }

    /// `from_parts` と同じ。`seam` を渡すと、文節の区切りにコアの代わりにそれを使う（テスト用）
    pub(crate) fn from_parts_seam(
        parts: EngineParts,
        seam_override: Option<Arc<dyn SegmentSeam>>,
    ) -> Engine {
        Engine::from_parts_with(parts, seam_override, Arc::default(), Arc::default(), None)
    }

    /// `from_parts_seam` と同じ。檻役の口を入れる場所（`lm_cage`）を、組み立てに渡したものと同じにする。
    /// `loading_mark` は、組み立ての前に書いた読み込みの印（D-4 の条件 5）。`opened` は `open` が選んだ場所
    /// （無ければ利用者のデータの場所だけ。`from_parts`）
    fn from_parts_with(
        parts: EngineParts,
        seam_override: Option<Arc<dyn SegmentSeam>>,
        lm_cage: Arc<CageSlot>,
        loading_mark: Arc<model_select::LoadingMark>,
        opened: Option<OpenedData>,
    ) -> Engine {
        let EngineParts {
            stack,
            rebuild,
            data_dir,
            lm_requested,
            lm_budget,
            convert_wait,
            lock,
        } = parts;
        let log = data_dir
            .as_deref()
            .map_or_else(EngineLog::disabled, EngineLog::open);
        // ロックを取れたあとの、ログが有効なときだけ、前回の印を読んで作り直す（CLI とテストの
        // `EngineLog::disabled` では印を作らない。ロックを取れなかったエンジンはここまで来ない）
        let (run_mark, prev_exit) = match data_dir.as_deref().filter(|_| log.is_enabled()) {
            Some(dir) => {
                let path = dir.join(LOG_DIR_NAME).join(RUN_MARK_FILE_NAME);
                let (mark, prev) = RunMark::begin(&path);
                (Some(mark), prev)
            }
            None => (None, None),
        };
        let watch = data_dir.as_deref().map(DataWatch::new).map(Mutex::new);
        let (locations, data_status) = opened.map(|o| (o.locations, o.status)).unzip();
        let emoji_path = match &locations {
            Some(locations) => Some(locations.table_path(ikari_core::EMOJI_TSV_FILE_NAME)),
            None => data_dir.as_deref().map(dispatch::emoji_tsv_path_in),
        };
        let emoji = load_emoji_reply(emoji_path.as_deref());
        let admin_dir = data_dir.clone();
        let learning_quarantined = AtomicBool::new(stack.meta.learning_quarantined.is_some());
        let inner = Arc::new(Inner {
            stack: RwLock::new(Arc::new(stack)),
            rebuild,
            watch,
            emoji: RwLock::new(Arc::new(emoji)),
            sessions: Mutex::new(HashMap::new()),
            limits: RwLock::new(ContextLimits::DEFAULT),
            conversions: Mutex::new(ConversionTable::default()),
            cache: Mutex::new(PrefetchCache::default()),
            ctx_trace: Mutex::new(ctx_trace::ContextTrace::default()),
            queue: PrefetchQueue::default(),
            lookahead: LookaheadQueue::default(),
            save: Mutex::new(SaveState {
                dirty: false,
                last_save: Instant::now(),
            }),
            stop: (Mutex::new(false), Condvar::new()),
            next_conversion: AtomicU64::new(1),
            connections: ConnectionIds::new(),
            lm_requested,
            lm_budget,
            convert_wait,
            warming: Arc::new(AtomicBool::new(false)),
            log: Arc::new(log),
            diag: diag::DiagState::new(),
            data_dir: admin_dir,
            admin: Mutex::new(()),
            save_io: Mutex::new(()),
            learning_quarantined,
            seam_override,
            lm_cage,
            loading_mark,
            locations,
            data_status,
        });
        diag::set_panic_log(&inner.log);
        // 前回の終わり方は、このエンジンの最初の行にする（ワーカーを起こす前に書く）
        if let Some(prev) = &prev_exit {
            inner.note_prev_exit(prev);
        }
        let mut workers = Vec::new();
        for (name, run) in [
            ("ikari-prefetch", Inner::prefetch_loop as fn(&Inner)),
            ("ikari-lookahead", Inner::lookahead_loop as fn(&Inner)),
            ("ikari-janitor", Inner::janitor_loop as fn(&Inner)),
        ] {
            let inner = Arc::clone(&inner);
            if let Ok(handle) = std::thread::Builder::new()
                .name(name.to_owned())
                .spawn(move || run(&inner))
            {
                workers.push(handle);
            }
        }
        inner.ensure_warm();
        inner.warm_dictionary();
        Engine {
            inner,
            workers,
            run_mark,
            _lock: lock,
        }
    }

    /// Envelope（protobuf のバイト列）を受けて Envelope を返す。決して panic せず、エラーも Envelope で返す。
    pub fn handle(&self, req: &[u8]) -> Vec<u8> {
        self.inner.handle_frame(ConnectionId::DEFAULT, req)
    }

    /// どちらの場所の辞書とモデルを使っているか（ADR 0080 §3.2 の状態）。`open` で開いたエンジンだけ Some
    pub fn data_status(&self) -> Option<DataStatus> {
        self.inner.data_status
    }
}

/// `open` が選んだ場所（`from_parts_with` に渡す）
struct OpenedData {
    locations: DataLocations,
    status: DataStatus,
}

/// 選んだ辞書の組
struct DictChoice {
    kind: LocationKind,
    dir: PathBuf,
    /// 検査に落とした場所と理由（置かれていないだけの場所は含めない）
    rejects: Vec<(LocationKind, DictSetReject)>,
}

/// 辞書の組を、上から順に、検査に通った最初の場所から選ぶ（ADR 0080 §3.2）。最後の候補は検査しない
/// （組み立てが開いて確かめる。開けなければ今までどおり起動に失敗する。候補が 1 つの開発のビルドは今と同じ）
fn choose_dictionary_set(locations: &DataLocations) -> DictChoice {
    let mut rejects = Vec::new();
    let mut candidates = locations.iter().peekable();
    while let Some(loc) = candidates.next() {
        let dir = loc.dict_dir();
        let is_last = candidates.peek().is_none();
        let passed = is_last || {
            match check_dictionary_set(&dir) {
                Ok(()) => true,
                Err(DictSetReject::Missing) => false,
                Err(reason) => {
                    rejects.push((loc.kind, reason));
                    false
                }
            }
        };
        if passed {
            return DictChoice {
                kind: loc.kind,
                dir,
                rejects,
            };
        }
    }
    // 利用者のデータの場所はいつも候補にある（ここには来ない）
    DictChoice {
        kind: LocationKind::User,
        dir: locations.user().dict_dir(),
        rejects,
    }
}

impl Inner {
    /// `Engine::handle` と `Connection::handle` の共通の本体。セッションは `conn` の名前空間で引く（ADR 0064 D1）。
    /// `Engine` が落ちたあと（`Connection` だけが残っている）は、処理もログもせず `ENGINE_NOT_READY` を返す
    fn handle_frame(&self, conn: ConnectionId, req: &[u8]) -> Vec<u8> {
        if self.is_stopped() {
            return engine_stopped_reply(req);
        }
        let started = Instant::now();
        let done = self.process(conn, req);
        let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        // 成功した `DiagEvent` は、`diag` が事象そのものの行を書く（要求の行は重ねて書かない）
        if done.kind != diag::KIND_DIAG {
            self.diag.count_request();
        }
        if !done.silent && (done.kind != diag::KIND_DIAG || done.error.is_some()) {
            self.log.record_note(
                done.kind,
                micros,
                self.lm_state_name(),
                done.error,
                done.note.as_deref(),
            );
        }
        done.reply
    }
}

impl Engine {
    /// 常駐の殻の出来事（ウィンドウの作成、ログオフの通知など）を `engine.log` に 1 行書く（`host <note>`）。
    /// `note` は固定の語と数だけにすること（ユーザーの文字を入れない）。キューに入れるだけ。
    pub fn record_host_event(&self, note: &str) {
        self.inner.note_host(note);
    }

    /// 新しい接続の口を作る。接続の番号はエンジンが振る（殻は知らない。ADR 0064 §4.1 の K1）。
    /// engine-host は、パイプの接続ごとにこれを 1 つ取り、その接続の枠をすべて `Connection::handle` に渡す。
    /// セッションのキーは（接続, `session_id`）なので、別の接続が同じ `session_id` を使っても混ざらない。
    pub fn connect(&self) -> Connection {
        let id = self.inner.connections.next();
        self.inner.note_connection("connect", id);
        Connection::new(Arc::clone(&self.inner), id)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.inner.stop();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        self.inner.save_now();
        self.inner.note_shutdown();
        // 学習の保存の行まで書いてから、ログのスレッドを止めて残りを flush する
        self.inner.log.shutdown();
        // 最後に、正常に終わったと印に書く（ここまで来なければ unknown / term / crash のまま）
        if let Some(mark) = &self.run_mark {
            mark.finish_clean();
        }
    }
}

/// `ErrorReply` を入れた Envelope をエンコードする。`message` にユーザーの文字列を入れないこと。
pub fn error_envelope(request_id: u64, code: ErrorCode, message: &str) -> Vec<u8> {
    Envelope {
        schema_version: SCHEMA_VERSION,
        request_id,
        body: Some(envelope::Body::Error(ErrorReply {
            code: code as i32,
            message: message.to_owned(),
        })),
    }
    .encode_to_vec()
}

/// `Engine` が落ちたあとの要求への返事。`request_id` は読めれば返す（読めなければ 0）
fn engine_stopped_reply(req: &[u8]) -> Vec<u8> {
    not_ready_envelope(req, "engine stopped")
}

/// エンジンがまだ開けていないときの返事（Windows の engine-host が、開けないまま常駐する間に返す。
/// ADR 0062 §5.4 の決定 1。macOS の `IkariEngineService` の `notReady` と同じ）。`request_id` は読めれば返す（読めなければ 0）
pub fn not_ready_reply(req: &[u8]) -> Vec<u8> {
    not_ready_envelope(req, "engine not ready")
}

fn not_ready_envelope(req: &[u8], message: &str) -> Vec<u8> {
    let request_id = if req.len() > MAX_ADMIN_REQUEST_BYTES {
        0
    } else {
        Envelope::decode(req).map_or(0, |env| env.request_id)
    };
    error_envelope(request_id, ErrorCode::EngineNotReady, message)
}

/// 要求の失敗（`ErrorReply` になる）。`message` は固定の文だけ。
struct Fail {
    code: ErrorCode,
    message: &'static str,
}

impl Fail {
    const fn new(code: ErrorCode, message: &'static str) -> Fail {
        Fail { code, message }
    }

    const fn bad(message: &'static str) -> Fail {
        Fail::new(ErrorCode::BadRequest, message)
    }
}

const UNKNOWN_SESSION: Fail = Fail::new(ErrorCode::UnknownSession, "unknown session");
const UNKNOWN_CONVERSION: Fail = Fail::new(ErrorCode::UnknownConversion, "unknown conversion");

struct Inner {
    stack: RwLock<Arc<Stack>>,
    rebuild: Option<StackBuilder>,
    watch: Option<Mutex<DataWatch>>,
    /// 絵文字のショートコードの表（読み込み済みの返事。`emoji.tsv` の更新で入れ替える）
    emoji: RwLock<Arc<pb::EmojiTableReply>>,
    /// セッション（キーは（接続, `session_id`）。ADR 0064 D1）
    sessions: Mutex<HashMap<SessionKey, Session>>,
    /// 文脈の量と時間（本番は既定値。テストだけが短くする）
    limits: RwLock<ContextLimits>,
    conversions: Mutex<ConversionTable>,
    cache: Mutex<PrefetchCache>,
    /// 文脈の番号が進んだ理由のログ用（ADR 0021 L0）。周辺テキストはメモリだけで、書かない
    ctx_trace: Mutex<ctx_trace::ContextTrace>,
    queue: PrefetchQueue,
    /// 文節の先読み（ADR 0006 §3.4）
    lookahead: LookaheadQueue,
    save: Mutex<SaveState>,
    stop: (Mutex<bool>, Condvar),
    next_conversion: AtomicU64,
    /// 接続の番号の払い出し（ADR 0064 §4）
    connections: ConnectionIds,
    lm_requested: bool,
    lm_budget: LmBudget,
    /// Space で計算中の先行計算を待つ上限
    convert_wait: Duration,
    warming: Arc<AtomicBool>,
    log: Arc<EngineLog>,
    diag: diag::DiagState,
    /// ユーザーデータの置き場所（管理の要求に要る。なければ管理は `ENGINE_NOT_READY`）
    data_dir: Option<PathBuf>,
    /// 管理の要求を 1 つずつ処理する（変換の経路は取らない）
    admin: Mutex<()>,
    /// 学習ファイルの書き込みを 1 つずつにする（`save` の状態とは別。書き込みの間、状態を持たない）
    save_io: Mutex<()>,
    /// 起動時に不正な学習ファイルを退避した記録。Hello の返事に入れたら下ろして、1 回だけ伝える（#271）
    learning_quarantined: AtomicBool,
    /// 文節の区切りをコアに頼む口。None なら本番（`RealSeam`）。テストだけがモックを入れる
    seam_override: Option<Arc<dyn SegmentSeam>>,
    /// 檻役の口（macOS の段 2。ADR 0077。`lm_cage`）。組み立ての `HostLauncher::Cage` と同じもの
    lm_cage: Arc<CageSlot>,
    /// 自由指定のモデルの読み込みの印（`model_select`。最初に読み込めたら消す）
    loading_mark: Arc<model_select::LoadingMark>,
    /// 辞書と表を読む場所の候補（`open` のときだけ。ADR 0080 §3.2）。None なら利用者のデータの場所だけ
    locations: Option<DataLocations>,
    /// どちらの場所の辞書とモデルを使っているか（`open` のときだけ）
    data_status: Option<DataStatus>,
}

/// 起動時にデータ用フォルダ（本体、backups、logs）を 0700、中のファイルを 0600 にする。
/// 失敗しても起動は止めない（権限を変えられない場所でも動く）。
/// Windows ではユーザーのフォルダの ACL に任せ、何もしない。
#[cfg(unix)]
fn secure_data_dir(data_dir: &Path) {
    for dir in [
        data_dir.to_path_buf(),
        data_dir.join(admin::BACKUP_DIR_NAME),
        data_dir.join(log::LOG_DIR_NAME),
    ] {
        if dir.is_dir() {
            let _ = std::fs::set_permissions(
                &dir,
                std::os::unix::fs::PermissionsExt::from_mode(ikari_core::user::PRIVATE_DIR_MODE),
            );
            ikari_core::user::tighten_files(&dir);
        }
    }
}

#[cfg(not(unix))]
fn secure_data_dir(_data_dir: &Path) {}

fn char_count_exceeds(s: &str, limit: usize) -> bool {
    s.chars().nth(limit).is_some()
}

/// 末尾の `n` 文字。
fn tail_chars(s: &str, n: usize) -> &str {
    let count = s.chars().count();
    if count <= n {
        return s;
    }
    let skip = count - n;
    s.char_indices().nth(skip).map_or("", |(i, _)| &s[i..])
}

/// 先頭の `n` 文字。
fn head_chars(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map_or(s, |(i, _)| &s[..i])
}

impl Inner {
    /// 絵文字の表を読み直す場所。`watched` は見張っている利用者のデータの場所の `emoji.tsv`（§3.2: 置かれていれば
    /// それ、無ければ同梱の場所の物）
    pub(crate) fn emoji_table_path(&self, watched: &Path) -> PathBuf {
        self.locations.as_ref().map_or_else(
            || watched.to_path_buf(),
            |l| l.table_path(ikari_core::EMOJI_TSV_FILE_NAME),
        )
    }

    /// `dict_reject location=<語> reason=<語>`（パスは書かない）
    fn record_dict_reject(&self, kind: LocationKind, reason: DictSetReject) {
        let note = format!("location={} reason={}", kind.as_word(), reason.as_word());
        self.log
            .record_note(KIND_DICT_REJECT, 0, self.lm_state_name(), None, Some(&note));
    }

    /// `data_location dict=<語> model=<語>`
    fn note_data_location(&self, status: DataStatus) {
        let model = status.model.map_or(NO_MODEL_WORD, LocationKind::as_word);
        let note = format!("dict={} model={model}", status.dict.as_word());
        self.log.record_note(
            KIND_DATA_LOCATION,
            0,
            self.lm_state_name(),
            None,
            Some(&note),
        );
    }

    fn limits(&self) -> ContextLimits {
        *self.limits.read().unwrap_or_else(|e| e.into_inner())
    }

    fn stack(&self) -> Arc<Stack> {
        Arc::clone(&self.stack.read().unwrap_or_else(|e| e.into_inner()))
    }
}

fn micros_u32(since: Instant) -> u32 {
    u32::try_from(since.elapsed().as_micros()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests;
