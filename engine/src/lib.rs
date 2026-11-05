//! ikari-engine: 常駐エンジン（セッション、先行計算、キャッシュ、protobuf の処理）。ADR 0004 §7。
//!
//! - `Engine::handle` は Envelope（protobuf のバイト列）を受けて Envelope を返す。決して panic せず、
//!   エラーも Envelope で返す。複数スレッドから同時に呼んでよい。
//! - `Prefetch` は読みが変わるたびに殻が投げる。セッションごとに最新の 1 件だけを別スレッドで計算し、
//!   キャッシュに置く。`Convert` はキャッシュに当たればすぐ返し、計算中なら締め切りまで待ち、
//!   間に合わなければ LM を使わずにその場で変換する。
//! - 返したページは差し替えない（NFR-05。`Conversion` の性質のまま）。
//! - 学習はメモリに記録し、5 秒ごとと `EndSession`、`Shutdown`、`Drop` で保存する。
//! - データのディレクトリは `engine.lock` を flock で持つ（CLI の学習ファイルの書き込みとの排他）。
//! - 管理の要求（ユーザー辞書、学習、エクスポート／インポート、プロファイル。ADR 0005 §8）は `admin.rs`。
//!
//! 構成: 入口は `dispatch`、セッションは `session`、変換の要求は `convert`、LM の状態は `lm`、データの監視と
//! 保存は `data`、裏のスレッドは `workers`、文節は `segments`。このファイルは `Engine`、`Inner`、上限の定数。

mod admin;
mod cache;
pub mod context_limits;
mod convert;
mod ctx_trace;
mod data;
mod diag;
mod diag_schema;
mod dispatch;
mod lm;
mod lock;
mod log;
#[cfg(test)]
mod mock_seam;
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
use ikari_stack::{
    LmOverrides, LmStats, ProfileSel, RankerSet, Reuse, Stack, StackError, StackOptions,
    StackPaths, build_stack, build_stack_with,
};
use prost::Message;

use crate::cache::{
    CacheEntry, CacheKey, CacheOutcome, ConversionTable, Job, MissReason, PrefetchCache,
    PrefetchContext, PrefetchQueue, SharedConversion, lock as lock_mutex,
};
use crate::context_limits::ContextLimits;
use crate::convert::candidate_kind;
use crate::data::{DataWatch, SaveState, load_emoji_reply};
use crate::log::{EngineLog, LOG_DIR_NAME, RUN_MARK_FILE_NAME};
use crate::segments::{LookJob, LookaheadQueue, RealSeam, SegEntry, SegmentSeam, layout_reply};
use crate::session::Session;

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
        ..StackOptions::default()
    }
}

impl Engine {
    /// 辞書、vectors、学習、LM を読み込み、LM の warm_up を別スレッドで始める。
    ///
    /// `data_dir`（ADR 0004 §2 の配置）の `dict/` を開く。`settings.tsv` の `engine.lm` と
    /// `engine.idle_unload_secs`、`engine.lm_deadline_ms`、`engine.convert_wait_ms` があれば `cfg` より優先する。LM のモデル（`models/` の GGUF）がない、
    /// またはこのビルドに LM がないときは、LM なしで動く（`HelloReply.lm = LM_FAILED`）。
    pub fn open(cfg: &EngineConfig) -> Result<Engine, EngineError> {
        if cfg.data_dir.is_empty() {
            return Err(EngineError::Config("data_dir is empty".into()));
        }
        let data_dir = PathBuf::from(&cfg.data_dir);
        if !data_dir.is_dir() {
            return Err(EngineError::Config("data_dir is not a directory".into()));
        }
        secure_data_dir(&data_dir);
        let settings = settings::load(&data_dir);
        let lm_requested = settings.lm.unwrap_or(cfg.enable_lm);
        let idle_unload_secs = settings.idle_unload_secs.unwrap_or(cfg.idle_unload_secs);

        let lock = EngineLock::acquire(&data_dir).map_err(|e| EngineError::Open(e.to_string()))?;
        let model = data_dir.join(MODELS_DIR_NAME).join(MODEL_FILE_NAME);
        let lm_model = (lm_requested && cfg!(feature = "lm") && model.is_file()).then_some(model);
        let paths = StackPaths {
            dict_dir: UserDir::new(&data_dir).dict_dir(),
            user_dir: Some(data_dir.clone()),
            vectors: None,
            lm_model,
            no_packs: settings.pack_it == Some(false),
        };
        let lm_budget = LmBudget {
            max: settings.lm_deadline_or_default(),
            ..LmBudget::default()
        };
        let mut opts = stack_options(idle_unload_secs, lm_budget);
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
        let stack = build_stack(&paths, &opts).map_err(|e| EngineError::Open(e.to_string()))?;
        let rebuild: StackBuilder = Box::new(move |reuse| build_stack_with(&paths, &opts, reuse));
        Ok(Engine::from_parts(EngineParts {
            stack,
            rebuild: Some(rebuild),
            data_dir: Some(data_dir),
            lm_requested,
            lm_budget,
            convert_wait: settings.convert_wait_or_default(),
            lock: Some(lock),
        }))
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
        let emoji = load_emoji_reply(
            data_dir
                .as_deref()
                .map(dispatch::emoji_tsv_path_in)
                .as_deref(),
        );
        let admin_dir = data_dir.clone();
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
            lm_requested,
            lm_budget,
            convert_wait,
            warming: Arc::new(AtomicBool::new(false)),
            log: Arc::new(log),
            diag: diag::DiagState::new(),
            data_dir: admin_dir,
            admin: Mutex::new(()),
            save_io: Mutex::new(()),
            seam_override,
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
        let started = Instant::now();
        let done = self.inner.process(req);
        let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        // 成功した `DiagEvent` は、`diag` が事象そのものの行を書く（要求の行は重ねて書かない）
        if done.kind != diag::KIND_DIAG {
            self.inner.diag.count_request();
        }
        if done.kind != diag::KIND_DIAG || done.error.is_some() {
            self.inner.log.record_note(
                done.kind,
                micros,
                self.inner.lm_state_name(),
                done.error,
                done.note.as_deref(),
            );
        }
        done.reply
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
    sessions: Mutex<HashMap<u64, Session>>,
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
    /// 文節の区切りをコアに頼む口。None なら本番（`RealSeam`）。テストだけがモックを入れる
    seam_override: Option<Arc<dyn SegmentSeam>>,
}

/// 起動時にデータ用フォルダ（本体、backups、logs）を 0700、中のファイルを 0600 にする。
/// 失敗しても起動は止めない（権限を変えられない場所でも動く）。
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
