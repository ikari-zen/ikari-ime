//! 診断ログのエンジン側（ADR 0034 §4）。殻の `DiagEvent` を許可リストで確かめて書き、
//! エンジン自身の事象（`lm_load` `lm_unload` `stat`）と、`begin_session` の `app=` を作る。
//!
//! 書くのは固定の語と数だけ（NFR-19）。例外は `app=` の Bundle ID（ADR 0034 §7、§9-1。ユーザー決定
//! 2026-10-25。2026-10-30 に `convert` の行へ広げた）で、`begin_session` と `convert` の行にだけ、
//! 文字の種類と長さを確かめて書く。secure は何も書かない（NFR-18）。
//! ディスクには触らない（`EngineLog::record_note` はキューに入れるだけ。ADR 0034 §3.1）。

use super::*;
use std::sync::{Once, Weak};

use ikari_runmark::PrevExit;

use crate::diag_schema::{self, Rejected};
use crate::model_check::ModelReject;

pub(crate) const KIND_DIAG: &str = "diag";
const KIND_DIAG_REJECT: &str = "diag_reject";
const KIND_LM_LOAD: &str = "lm_load";
const KIND_LM_UNLOAD: &str = "lm_unload";
const KIND_STAT: &str = "stat";
const KIND_PREV_EXIT: &str = "prev_exit";
const KIND_PANIC: &str = "panic";

const KIND_LM_REJECT: &str = "lm_reject";

/// `lm_load` の失敗の語
pub(crate) const ERR_LM_LOAD_FAILED: &str = "lm_load_failed";
/// `lm_reject` の語（読む前の照合に通らなかった）
const ERR_LM_REJECTED: &str = "lm_model_rejected";

/// `lm_unload` の理由（ADR 0034 §4.3）
#[derive(Clone, Copy)]
pub(crate) enum UnloadReason {
    Idle,
    Settings,
    Shutdown,
}

impl UnloadReason {
    fn as_word(self) -> &'static str {
        match self {
            UnloadReason::Idle => "idle",
            UnloadReason::Settings => "settings",
            UnloadReason::Shutdown => "shutdown",
        }
    }
}

/// `stat` を前回から要求があったときだけ、この間隔で書く（ADR 0034 §4.3）
pub(crate) const STAT_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Bundle ID に許す長さ（バイト。ADR 0034 §7 案 7-1）
const MAX_BUNDLE_ID_BYTES: usize = 128;
/// 外れた Bundle ID の代わりに書く語
const APP_INVALID: &str = "invalid";
/// Bundle ID が空のときの語
const APP_NONE: &str = "none";

/// 診断のための、エンジンの内側の状態（ログのスレッドに触らないものだけ）
pub(crate) struct DiagState {
    /// 処理した要求の数（`DiagEvent` は数えない）。`stat` を「要求があったときだけ」書くため
    requests: AtomicU64,
    /// 前回の `stat` を書いた時刻と、そのときの要求の数
    stat: Mutex<(Instant, u64)>,
    /// LM が読み込まれていると最後に見たか。解放（`lm_unload`）の検出に使う
    lm_loaded: Arc<AtomicBool>,
}

impl DiagState {
    pub(crate) fn new() -> DiagState {
        DiagState {
            requests: AtomicU64::new(0),
            stat: Mutex::new((Instant::now(), 0)),
            lm_loaded: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn count_request(&self) {
        self.requests.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn loaded_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.lm_loaded)
    }
}

/// `begin_session` と `convert` の行の `app=`。`[A-Za-z0-9.-]` だけ 128 バイトまで。外れたら `invalid`、空なら `none`。
/// 戻りに入れてよいのは、確かめ済みの文字列だけ
fn app_word(bundle_id: &str) -> &str {
    let valid = bundle_id.len() <= MAX_BUNDLE_ID_BYTES
        && bundle_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    match (valid, bundle_id.is_empty()) {
        (false, _) => APP_INVALID,
        (true, true) => APP_NONE,
        (true, false) => bundle_id,
    }
}

/// `begin_session` の行の注記。`session=` は `sh_episode` などの行と結ぶため（エンジンが
/// `DiagEvent` の行に同じ `session=` を足す）。secure は Bundle ID を書かない（NFR-18）
pub(crate) fn begin_session_note(b: &pb::BeginSession) -> String {
    match session_app(b) {
        None => format!("session={}", b.session_id),
        Some(app) => format!("session={} app={app}", b.session_id),
    }
}

/// セッションが持つ、ログに書いてよい `app=` の語。secure は None（Bundle ID を書かない。NFR-18）
pub(crate) fn session_app(b: &pb::BeginSession) -> Option<String> {
    (!b.secure).then(|| app_word(&b.client_bundle_id).to_owned())
}

/// `commit` / `commit_segments` の行に足す `rw=`。確定した候補のうち Rewriter のものの名前
/// （`Rewriter::name()` の固定の名前。重複なし、出た順、`,` でつなぐ）。なければ None
pub(crate) fn rewriter_note<'a>(
    origins: impl Iterator<Item = &'a CandidateOrigin>,
) -> Option<String> {
    let mut names: Vec<&'static str> = Vec::new();
    for origin in origins {
        if let CandidateOrigin::Rewriter { name, .. } = origin
            && !names.contains(name)
        {
            names.push(name);
        }
    }
    (!names.is_empty()).then(|| format!("rw={}", names.join(",")))
}

impl Inner {
    /// 殻からの `DiagEvent`。返事は常に Ack（殻は待たない）。
    /// secure は何も書かずに捨てる（`diag_reject` も書かない。NFR-18）。セッションがなければ
    /// `diag_reject reason=no_session` を数える（`UNKNOWN_SESSION` の行は書かない）
    pub(crate) fn diag(&self, event: &pb::DiagEvent) -> envelope::Body {
        let ack = envelope::Body::Ack(pb::Ack {});
        // 休止中（`EndSession` のあと）のセッションの事象も受ける: 合成の最後の `sh_episode` が
        // `EndSession` のあとに届いても、secure の判断は変わらない
        let secure = lock_mutex(&self.sessions)
            .get(&event.session_id)
            .map(|s| s.secure);
        match secure {
            Some(true) => return ack,
            None => {
                self.record_reject(&Rejected {
                    reason: diag_schema::RejectReason::NoSession,
                    kind: None,
                });
                return ack;
            }
            Some(false) => {}
        }
        match diag_schema::validate(event) {
            Ok(ok) => {
                let note = if ok.note.is_empty() {
                    format!("session={}", event.session_id)
                } else {
                    format!("session={} {}", event.session_id, ok.note)
                };
                self.log
                    .record_note(ok.kind, 0, self.lm_state_name(), None, Some(&note));
            }
            Err(rejected) => self.record_reject(&rejected),
        }
        ack
    }

    /// `diag_reject n=1 reason=<語> [kind=<許可リストの kind>]`。送られた文字列は書かない
    fn record_reject(&self, rejected: &Rejected) {
        let note = match rejected.kind {
            Some(kind) => format!("n=1 reason={} kind={kind}", rejected.reason.as_word()),
            None => format!("n=1 reason={}", rejected.reason.as_word()),
        };
        self.log
            .record_note(KIND_DIAG_REJECT, 0, self.lm_state_name(), None, Some(&note));
    }

    /// `lm_reject file=<モデルの名前> reason=<語>`。読む前の照合に通らず、LM なしで起動したとき
    pub(crate) fn record_lm_reject(&self, file_name: &str, reason: ModelReject) {
        let note = format!("file={file_name} reason={}", reason.as_word());
        self.log.record_note(
            KIND_LM_REJECT,
            0,
            self.lm_state_name(),
            Some(ERR_LM_REJECTED),
            Some(&note),
        );
    }

    pub(crate) fn record_lm_unload(&self, reason: UnloadReason) {
        let note = format!("reason={}", reason.as_word());
        self.log
            .record_note(KIND_LM_UNLOAD, 0, self.lm_state_name(), None, Some(&note));
    }

    /// 見回りのスレッドから 1 秒ごとに呼ぶ。アイドル解放の検出と、`stat`
    pub(crate) fn tick_diag(&self, now: Instant) {
        self.detect_idle_unload();
        self.maybe_record_stat(now);
    }

    /// 読み込み済みだった LM が読み込まれていなくなっていたら `lm_unload reason=idle`
    /// （Stack の差し替えは `rebuild_stack` が `settings` で書く）
    fn detect_idle_unload(&self) {
        let loaded = self.stack().lm.as_ref().is_some_and(|lm| lm.is_loaded());
        let was = self.diag.lm_loaded.swap(loaded, Ordering::AcqRel);
        if was && !loaded {
            self.record_lm_unload(UnloadReason::Idle);
        }
    }

    /// 前回の `stat` から `STAT_INTERVAL` が経ち、要求があったときだけ `stat rss_kb= sessions=`
    fn maybe_record_stat(&self, now: Instant) {
        let requests = self.diag.requests.load(Ordering::Relaxed);
        {
            let mut last = lock_mutex(&self.diag.stat);
            if now.saturating_duration_since(last.0) < STAT_INTERVAL || requests == last.1 {
                return;
            }
            *last = (now, requests);
        }
        let sessions = lock_mutex(&self.sessions).len();
        let note = match crate::rss::rss_kb() {
            Some(rss) => format!("rss_kb={rss} sessions={sessions}"),
            None => format!("sessions={sessions}"),
        };
        self.log
            .record_note(KIND_STAT, 0, self.lm_state_name(), None, Some(&note));
    }

    /// Stack の差し替えで LM が替わった（読み込み済みだったものが捨てられる）ときの `lm_unload`
    pub(crate) fn note_stack_swap(&self, old: &Stack, new: &Stack) {
        let same = match (&old.lm, &new.lm) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same && old.lm.as_ref().is_some_and(|lm| lm.is_loaded()) {
            self.record_lm_unload(UnloadReason::Settings);
        }
        let loaded = new.lm.as_ref().is_some_and(|lm| lm.is_loaded());
        self.diag.lm_loaded.store(loaded, Ordering::Release);
    }

    /// 前回の終わり方の行（`Engine::open` のあとの最初の行。ADR 0051 §5.1）。固定の語と数だけ
    pub(crate) fn note_prev_exit(&self, prev: &PrevExit) {
        let mut note = format!("exit={}", prev.exit.as_word());
        if let Some(sig) = prev.sig {
            note.push_str(&format!(" sig={sig}"));
        }
        if let Some(up_s) = prev.up_s {
            note.push_str(&format!(" up_s={up_s}"));
        }
        self.log
            .record_note(KIND_PREV_EXIT, 0, self.lm_state_name(), None, Some(&note));
    }

    /// `Engine` を閉じるとき、LM が読み込まれていたら `lm_unload reason=shutdown`
    pub(crate) fn note_shutdown(&self) {
        if self.stack().lm.as_ref().is_some_and(|lm| lm.is_loaded()) {
            self.record_lm_unload(UnloadReason::Shutdown);
        }
    }

    /// ワーカーが LM の読み込みを終えたときの `lm_load`（所要 us、失敗なら `err=lm_load_failed`）
    pub(crate) fn lm_load_line(
        log: &EngineLog,
        started: Instant,
        loaded: bool,
        device: ikari_stack::LmDevice,
    ) {
        let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        // 読めたときだけ装置を書く（失敗のときは付けない。ADR 0061 §5.4）
        let note = loaded.then(|| format!("dev={}", crate::convert::device_word(device)));
        log.record_note(
            KIND_LM_LOAD,
            micros,
            if loaded { "ready" } else { "failed" },
            (!loaded).then_some(ERR_LM_LOAD_FAILED),
            note.as_deref(),
        );
    }
}

// ---- panic の記録（ADR 0051 §5.1、§5.3）

/// panic の行を書く先（今のエンジンのログ）。エンジンを開き直しても hook は 1 つなので、差し替えられる場所を通す。
/// 弱い参照にして、エンジンが閉じたあとにログを生かし続けない
static PANIC_LOG: Mutex<Weak<EngineLog>> = Mutex::new(Weak::new());

/// panic の行の `thread` に書く語（スレッドの名前から決める。一覧の外は `other`）
const THREAD_MAIN: &str = "main";
const THREAD_OTHER: &str = "other";
const THREAD_WORDS: &[(&str, &str)] = &[
    ("main", THREAD_MAIN),
    ("ikari-log", "ikari-log"),
    ("ikari-prefetch", "prefetch"),
    ("ikari-lookahead", "lookahead"),
    ("ikari-janitor", "janitor"),
];

fn thread_word(name: &str) -> &'static str {
    THREAD_WORDS
        .iter()
        .find(|(raw, _)| *raw == name)
        .map_or(THREAD_OTHER, |(_, word)| word)
}

/// 今のエンジンのログを panic の行の書き先にする
pub(crate) fn set_panic_log(log: &Arc<EngineLog>) {
    *PANIC_LOG.lock().unwrap_or_else(|e| e.into_inner()) = Arc::downgrade(log);
}

/// panic の行を 1 行書く。キューに入れるだけ（hook の中でディスクに触らない）。
/// hook は panic の最中に呼ばれるので、ロックは待たない
fn record_panic(thread: &str, loc: &str) {
    let Ok(slot) = PANIC_LOG.try_lock() else {
        return;
    };
    if let Some(log) = slot.upgrade() {
        write_panic_line(&log, thread, loc);
    }
}

pub(crate) fn write_panic_line(log: &EngineLog, thread: &str, loc: &str) {
    let note = format!(
        "thread={} loc={}",
        thread_word(thread),
        diag_schema::loc_or_external(loc)
    );
    log.record_note(KIND_PANIC, 0, LM_UNKNOWN, None, Some(&note));
}

/// panic の行の `lm`（hook は LM の状態を知らない）
const LM_UNKNOWN: &str = "disabled";

/// 致命的なシグナルの処理と panic の hook を入れる。プロセスの入口（`ikari_engine_open`）から呼ぶ。
/// 2 回目以降は何もしない（エンジンを開き直しても 1 回だけ）。CLI とテストでは呼ばない
pub fn install_crash_handlers() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        ikari_runmark::install_signal_handlers();
        ikari_runmark::install_panic_hook(Box::new(record_panic));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_word_checks_charset_and_length() {
        assert_eq!(app_word("com.microsoft.VSCode"), "com.microsoft.VSCode");
        assert_eq!(app_word("net.my-app.X1"), "net.my-app.X1");
        assert_eq!(app_word(""), "none");
        assert_eq!(app_word("com.example.あ"), "invalid");
        assert_eq!(app_word("a b"), "invalid");
        assert_eq!(app_word("a=b"), "invalid");
        assert_eq!(
            app_word(&"a".repeat(MAX_BUNDLE_ID_BYTES)),
            "a".repeat(MAX_BUNDLE_ID_BYTES)
        );
        assert_eq!(app_word(&"a".repeat(MAX_BUNDLE_ID_BYTES + 1)), "invalid");
    }

    #[test]
    fn rewriter_note_lists_unique_names_in_order() {
        let r = |name| CandidateOrigin::Rewriter { name, format: 0 };
        let origins = [
            CandidateOrigin::Dictionary,
            r("postal"),
            r("date"),
            r("postal"),
        ];
        assert_eq!(
            rewriter_note(origins.iter()).as_deref(),
            Some("rw=postal,date")
        );
        assert_eq!(rewriter_note([CandidateOrigin::Katakana].iter()), None);
    }
}
