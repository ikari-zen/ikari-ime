//! 主スレッドを止めないエンジンへの問い合わせ（今の `AppContext` の `engineQueue`・`emojiQueue` と、郵便番号の
//! `DispatchQueue.global`）。結果は `Shell` が次の `dispatch` の終わりか、見回りのタイマー（`Timer`）で受け取る。
//!
//! 本番は別のスレッドで流す。シナリオとテストは、決まった順で結果を受け取れるよう、その場で流す（`Mode::Inline`）。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use ikari_proto::v1;

use crate::bridge::{Frame, RelayJob};
use crate::engine::{EngineAdmin, EngineLink, lm_state};
use crate::menu::{EngineLmState, ProfileDomain, ProfileWeight};
use crate::postal::PostalLookupRequest;
use crate::timeouts;
use crate::types::ConnId;

/// 絵文字のショートコードの表の待ち（主スレッドの外で待つ。ADR 0027 §8.2）。
pub const EMOJI_TABLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
/// 殻とエンジンの版の照合のための `Hello` の待ち（エンジンの起動の前倒しが主な目的）。
pub const HELLO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// 流し方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundMode {
    /// 別のスレッド（本番）
    Thread,
    /// その場で（シナリオ、テスト、性能の計測。結果の順が決まる）
    Inline,
}

/// 頼む仕事。
#[derive(Debug)]
pub(crate) enum Job {
    EmojiTable,
    /// `Hello` で起動を前倒しし、LM の状態を取る。`attempt == 0` なら分野も取る。`shutdown_first` は設定の変更の起こし直し
    EngineInfo {
        attempt: u32,
        generation: u64,
        shutdown_first: bool,
    },
    Postal(PostalLookupRequest),
    SetProfileDomain {
        id: String,
        included: bool,
    },
    /// 設定アプリの管理の要求をエンジンへ中継する（L-E）
    Relay {
        conn: ConnId,
        job: RelayJob,
    },
}

/// 仕事の結果。
#[derive(Debug)]
pub(crate) enum JobResult {
    EmojiTable(Option<v1::EmojiTableReply>),
    EngineInfo {
        attempt: u32,
        generation: u64,
        lm: Option<EngineLmState>,
        profile: Option<Vec<ProfileDomain>>,
    },
    Postal {
        request: PostalLookupRequest,
        addresses: Vec<String>,
    },
    Profile(Option<Vec<ProfileDomain>>),
    /// 中継の返事の枠
    Relay {
        conn: ConnId,
        frame: Frame,
    },
}

/// 仕事の出し入れ。
pub(crate) struct Background {
    mode: BackgroundMode,
    shell_version: String,
    results: Arc<Mutex<Vec<JobResult>>>,
    outstanding: Arc<AtomicUsize>,
}

fn lock(results: &Mutex<Vec<JobResult>>) -> MutexGuard<'_, Vec<JobResult>> {
    results
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Background {
    pub(crate) fn new(mode: BackgroundMode, shell_version: String) -> Self {
        Self {
            mode,
            shell_version,
            results: Arc::new(Mutex::new(Vec::new())),
            outstanding: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn set_mode(&mut self, mode: BackgroundMode) {
        self.mode = mode;
    }

    /// まだ結果が来ていない仕事があるか。
    pub(crate) fn has_outstanding(&self) -> bool {
        self.outstanding.load(Ordering::Acquire) > 0
    }

    /// 来た結果を取り出す（来た順）。
    pub(crate) fn drain(&self) -> Vec<JobResult> {
        std::mem::take(&mut *lock(&self.results))
    }

    pub(crate) fn spawn(&self, engine: &Arc<EngineLink>, job: Job) {
        let engine = Arc::clone(engine);
        let results = Arc::clone(&self.results);
        let outstanding = Arc::clone(&self.outstanding);
        let shell_version = self.shell_version.clone();
        outstanding.fetch_add(1, Ordering::AcqRel);
        let work = move || {
            let result = run(&engine, &shell_version, job);
            lock(&results).push(result);
            outstanding.fetch_sub(1, Ordering::AcqRel);
        };
        match self.mode {
            BackgroundMode::Inline => work(),
            BackgroundMode::Thread => {
                // スレッドを作れないとき（資源の枯渇）は、その場で流す（結果を落とさない）
                let fallback = std::sync::Arc::new(std::sync::Mutex::new(Some(work)));
                let in_thread = Arc::clone(&fallback);
                let spawned = std::thread::Builder::new()
                    .name("ikari-shell-background".into())
                    .spawn(move || {
                        if let Some(work) = in_thread
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .take()
                        {
                            work();
                        }
                    });
                if spawned.is_err()
                    && let Some(work) = fallback
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take()
                {
                    work();
                }
            }
        }
    }
}

fn run(engine: &EngineLink, shell_version: &str, job: Job) -> JobResult {
    match job {
        Job::EmojiTable => JobResult::EmojiTable(engine.emoji_table(EMOJI_TABLE_TIMEOUT).ok()),
        Job::EngineInfo {
            attempt,
            generation,
            shutdown_first,
        } => {
            if shutdown_first {
                engine.shutdown(v1::ShutdownReason::SettingsChanged);
            }
            match engine.hello(shell_version, HELLO_TIMEOUT) {
                Ok(hello) => JobResult::EngineInfo {
                    attempt,
                    generation,
                    lm: Some(lm_state(&hello)),
                    profile: if attempt == 0 {
                        EngineAdmin::new(engine).profile().ok()
                    } else {
                        None
                    },
                },
                Err(_) => JobResult::EngineInfo {
                    attempt,
                    generation,
                    lm: None,
                    profile: None,
                },
            }
        }
        Job::Postal(request) => {
            let addresses = engine
                .postal_lookup(&request.code, timeouts::POSTAL_LOOKUP)
                .unwrap_or_default();
            JobResult::Postal { request, addresses }
        }
        Job::SetProfileDomain { id, included } => {
            let weight = if included {
                ProfileWeight::Often
            } else {
                ProfileWeight::Off
            };
            JobResult::Profile(EngineAdmin::new(engine).set_weight(weight, &id).ok())
        }
        Job::Relay { conn, job } => JobResult::Relay {
            conn,
            frame: job.run(engine),
        },
    }
}
