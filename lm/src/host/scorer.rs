//! 親（エンジン）の側。子を起動して採点を頼む `LmScorer` と、それを作る `LmLoader`。
//!
//! 役割の分け方:
//! - [`HostScorer`] は、子 1 つの一生。子が死んだ・固まった・壊れた返事をしたら、子を回収して `alive() == false` になる。
//!   `ZenzRanker` のワーカーはそれを捨て、次の要求で loader を呼ぶ。
//! - loader（[`host_loader`]）は、子を起動して `Ready` を待つ。起動し直すか諦めるかは [`HostHealth`] が数える。
//!
//! 数えるもの（連続失敗）: 起動できない、`Ready` が締め切りまでに来ない、版が違う、読み込みの失敗、
//! 子が終わった、返事が締め切りまでに来ない、壊れた返事。採点が 1 回成功したら 0 に戻す。
//! 数えないもの: 子が生きていて返した 1 件の採点の失敗（`Failed(Inference)`。長すぎるプロンプトなど、入力で決まる。
//! 同じプロセスで動かすときも、数えるだけで何もしない）。

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread;
use std::time::Duration;

use crate::error::LmError;
use crate::host::os::{self, ChildProc};
use crate::host::proto::{
    FailKind, MAX_FRAME_BYTES, PROTOCOL_VERSION, Reply, Request, read_frame, write_frame,
};
use crate::host::{ARG_GPU, ARG_GPU_OFF, ARG_GPU_ON, ARG_MODEL};
use crate::placement::{self, GpuState};
use crate::scorer::{LmDevice, LmLoader, LmScorer};

/// 仮置き（計測で決めた値ではない。ADR 0060 Q4 の (3)）: 起動から `Ready` までの上限。
/// モデルの読み込みを含む。N150 の VM の Vulkan で読み込みが約 12 秒だった（ADR 0059）ので、それより十分長くする。
/// 既存の `warm_up` の待ちの上限（60 秒）に合わせた
const DEFAULT_READY_DEADLINE: Duration = Duration::from_secs(60);
/// 仮置き（同上）: 採点 1 回（書き込みから返事まで）の上限。固まった子を見つけるためのもので、変換の待ち
/// （`ZenzConfig::budget`。300ms まで）とは別。N150 の VM の Vulkan で最初の 1 回が約 6 秒だった（ADR 0059）ので、
/// それより十分長くする
const DEFAULT_REQUEST_DEADLINE: Duration = Duration::from_secs(30);
/// 仮置き（同上）: 連続でこの回数失敗したら、子の起動を諦める
const DEFAULT_MAX_FAILURES: u32 = 3;

const READER_THREAD_NAME: &str = "ikari-lm-host-read";
const WRITER_THREAD_NAME: &str = "ikari-lm-host-write";
/// ログに出すデバイスの要約の上限（バイト）と、通す文字。子から来た文字列をそのままログに書かない（NFR-19）
const MAX_DEVICES_BYTES: usize = 128;
const INVALID_DEVICES: &str = "invalid";

/// 締め切りと、諦めるまでの回数
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostLimits {
    /// 起動から `Ready`（モデルの読み込みが済んだ）までの上限
    pub ready_deadline: Duration,
    /// 採点 1 回の上限（要求の書き込みと、返事の読みを含む）
    pub request_deadline: Duration,
    /// 連続でこの回数失敗したら、以後は子を起動しない（1 以上）
    pub max_failures: u32,
}

impl Default for HostLimits {
    fn default() -> Self {
        Self {
            ready_deadline: DEFAULT_READY_DEADLINE,
            request_deadline: DEFAULT_REQUEST_DEADLINE,
            max_failures: DEFAULT_MAX_FAILURES,
        }
    }
}

/// 子の起動のしかた
#[derive(Clone, Debug)]
pub struct HostSpec {
    /// 子の実行ファイル。絶対パスで渡す（PATH から探さない）
    pub program: PathBuf,
    /// モデル（GGUF）
    pub model: PathBuf,
    /// GPU を使ってよいか（設定 `engine.lm_gpu`）
    pub use_gpu: bool,
    pub limits: HostLimits,
}

/// 子の様子（数だけ）。loader と scorer が書き、エンジンのログとテストが読む
#[derive(Debug)]
pub struct HostHealth {
    max_failures: u32,
    consecutive_failures: AtomicU32,
    spawns: AtomicU64,
    /// 子が GPU での読み込みに失敗して CPU で読み直した。以後は CPU だけで起動する（ADR 0059 §2.4 と同じ扱い）
    gpu_failed: AtomicBool,
    /// 今の子の pid（いなければ 0）
    child_id: AtomicU32,
}

impl HostHealth {
    fn new(max_failures: u32) -> Self {
        Self {
            max_failures: max_failures.max(1),
            consecutive_failures: AtomicU32::new(0),
            spawns: AtomicU64::new(0),
            gpu_failed: AtomicBool::new(false),
            child_id: AtomicU32::new(0),
        }
    }

    /// 連続失敗が上限に達し、もう子を起動しない状態か（エンジンを起動し直すまで続く）
    pub fn gave_up(&self) -> bool {
        self.consecutive_failures() >= self.max_failures
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures.load(Ordering::Acquire)
    }

    /// 子を起動しようとした回数（起動に失敗したものも含む）
    pub fn spawns(&self) -> u64 {
        self.spawns.load(Ordering::Acquire)
    }

    /// 今の子の pid
    pub fn child_id(&self) -> Option<u32> {
        match self.child_id.load(Ordering::Acquire) {
            0 => None,
            id => Some(id),
        }
    }

    /// `lm_load` のログに足す固定の語（数だけ）
    pub fn log_note(&self) -> String {
        format!(
            "host=on host_spawns={} host_failures={} host_gave_up={}",
            self.spawns(),
            self.consecutive_failures(),
            u8::from(self.gave_up())
        )
    }

    fn record_failure(&self) {
        self.consecutive_failures.fetch_add(1, Ordering::AcqRel);
    }

    fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Release);
    }
}

/// 子を起動して採点を頼む loader（`ZenzRanker::new` に渡す）と、子の様子。
/// 子は、loader が呼ばれたとき（最初の要求、アイドルの解放のあと、子が死んだあと）に起動する。
pub fn host_loader(spec: HostSpec) -> (LmLoader, Arc<HostHealth>) {
    let health = Arc::new(HostHealth::new(spec.limits.max_failures));
    let loader: LmLoader = {
        let health = Arc::clone(&health);
        Arc::new(move || {
            let scorer = HostScorer::start(&spec, &health)?;
            Ok(Box::new(scorer) as Box<dyn LmScorer>)
        })
    };
    (loader, health)
}

/// 返事を待った結果の失敗
enum RecvError {
    Timeout,
    /// 子が終わった（出力が閉じた）
    Closed,
    Malformed,
}

impl RecvError {
    fn word(&self) -> &'static str {
        match self {
            RecvError::Timeout => "deadline",
            RecvError::Closed => "child exited",
            RecvError::Malformed => "malformed reply",
        }
    }
}

/// 生きている子と、そのパイプ。捨てると子を kill して wait する（`ChildProc` の Drop）。
/// 書き込みのスレッドは送り口が閉じて、読み取りのスレッドは子の出力が閉じて終わる。
struct Live {
    // 宣言順 = drop 順。先に子を回収すると、パイプが閉じて 2 つのスレッドが終わる
    proc: ChildProc,
    to_child: SyncSender<Vec<u8>>,
    from_child: Receiver<Option<Reply>>,
}

impl Live {
    fn spawn(spec: &HostSpec, use_gpu: bool) -> Result<Live, String> {
        let gpu = if use_gpu { ARG_GPU_ON } else { ARG_GPU_OFF };
        let args: [OsString; 4] = [
            ARG_MODEL.into(),
            spec.model.clone().into_os_string(),
            ARG_GPU.into(),
            gpu.into(),
        ];
        let (proc, mut stdin, mut stdout) =
            os::spawn(&spec.program, &args).map_err(|e| format!("spawn: {e}"))?;

        // パイプの読み書きには締め切りがないので、スレッドに任せて、チャネルの `recv_timeout` で待つ。
        // 書き込みが詰まっても（子が読まない）、返事が来ないので要求の締め切りで気づき、子を kill する。
        let (reply_tx, from_child) = mpsc::channel::<Option<Reply>>();
        let reader = thread::Builder::new()
            .name(READER_THREAD_NAME.to_owned())
            .spawn(move || {
                while let Ok(Some(frame)) = read_frame(&mut stdout) {
                    let reply = Reply::decode(&frame).ok();
                    let malformed = reply.is_none();
                    if reply_tx.send(reply).is_err() || malformed {
                        return;
                    }
                }
            });
        let (to_child, frames) = mpsc::sync_channel::<Vec<u8>>(1);
        let writer = thread::Builder::new()
            .name(WRITER_THREAD_NAME.to_owned())
            .spawn(move || {
                for frame in frames {
                    if write_frame(&mut stdin, &frame).is_err() {
                        return;
                    }
                }
            });
        // スレッドを作れなかったときは、`proc` の Drop が子を回収する
        if reader.is_err() || writer.is_err() {
            return Err("io thread".to_owned());
        }
        Ok(Live {
            proc,
            to_child,
            from_child,
        })
    }

    fn recv(&self, deadline: Duration) -> Result<Reply, RecvError> {
        match self.from_child.recv_timeout(deadline) {
            Ok(Some(reply)) => Ok(reply),
            Ok(None) => Err(RecvError::Malformed),
            Err(RecvTimeoutError::Timeout) => Err(RecvError::Timeout),
            Err(RecvTimeoutError::Disconnected) => Err(RecvError::Closed),
        }
    }
}

/// 子 1 つに採点を頼む `LmScorer`
struct HostScorer {
    /// 子が死んだ・回収したあとは None
    live: Option<Live>,
    health: Arc<HostHealth>,
    device: LmDevice,
    request_deadline: Duration,
    next_id: u64,
}

impl HostScorer {
    /// 子を起動して `Ready` を待つ。諦めたあとは、起動せずに失敗を返す
    fn start(spec: &HostSpec, health: &Arc<HostHealth>) -> Result<HostScorer, LmError> {
        if health.gave_up() {
            return Err(LmError::Load("lm host gave up".to_owned()));
        }
        health.spawns.fetch_add(1, Ordering::AcqRel);
        let failed = |what: &str| {
            health.record_failure();
            LmError::Load(format!("lm host: {what}"))
        };
        let gpu_was_disabled = spec.use_gpu && health.gpu_failed.load(Ordering::Acquire);
        let live = Live::spawn(spec, spec.use_gpu && !gpu_was_disabled).map_err(|e| failed(&e))?;
        // ここから先の失敗では、`live` を捨てて子を回収する
        match live.recv(spec.limits.ready_deadline) {
            Ok(Reply::Ready {
                version: PROTOCOL_VERSION,
                device,
                gpu,
                devices,
            }) => {
                if gpu == GpuState::FallbackLoadFailed {
                    health.gpu_failed.store(true, Ordering::Release);
                }
                // 同じプロセスで読んだときと同じ記録を残す（`lm_load` のログ、ADR 0061 の表示が読む）
                placement::set_devices(sanitize_devices(&devices));
                placement::set_gpu(if gpu_was_disabled {
                    GpuState::OffAfterFailure
                } else {
                    gpu
                });
                health.child_id.store(live.proc.id(), Ordering::Release);
                Ok(HostScorer {
                    live: Some(live),
                    health: Arc::clone(health),
                    device,
                    request_deadline: spec.limits.request_deadline,
                    next_id: 0,
                })
            }
            Ok(Reply::Ready { .. }) => Err(failed("protocol version mismatch")),
            Ok(Reply::Failed {
                kind: FailKind::Load,
                ..
            }) => Err(failed("model load failed")),
            Ok(_) => Err(failed("unexpected first reply")),
            Err(e) => Err(failed(e.word())),
        }
    }

    /// 子を回収して、連続失敗に数える
    fn fail(&mut self, what: &str) -> LmError {
        self.drop_child();
        self.health.record_failure();
        LmError::Inference(format!("lm host: {what}"))
    }

    fn drop_child(&mut self) {
        if self.live.take().is_some() {
            self.health.child_id.store(0, Ordering::Release);
        }
    }
}

impl LmScorer for HostScorer {
    fn nll(&mut self, prompt: &str, candidates: &[String]) -> Result<Vec<f32>, LmError> {
        let Some(live) = self.live.as_ref() else {
            return Err(LmError::Inference("lm host: child is gone".to_owned()));
        };
        self.next_id += 1;
        let id = self.next_id;
        let frame = Request::Score {
            id,
            prompt: prompt.to_owned(),
            candidates: candidates.to_vec(),
        }
        .encode();
        // 大きすぎる要求は入力で決まる。子のせいではないので、送らず、数えない
        if frame.len() > MAX_FRAME_BYTES {
            return Err(LmError::Inference("lm host: request too large".to_owned()));
        }
        if live.to_child.try_send(frame).is_err() {
            return Err(self.fail("write"));
        }
        match live.recv(self.request_deadline) {
            Ok(Reply::Scores { id: got, nll }) if got == id && nll.len() == candidates.len() => {
                self.health.record_success();
                Ok(nll)
            }
            Ok(Reply::Failed {
                id: got,
                kind: FailKind::Inference,
                message,
            }) if got == id => Err(LmError::Inference(message)),
            Ok(_) => Err(self.fail("unexpected reply")),
            Err(e) => Err(self.fail(e.word())),
        }
    }

    fn device(&self) -> LmDevice {
        self.device
    }

    fn alive(&self) -> bool {
        self.live.is_some()
    }
}

impl Drop for HostScorer {
    fn drop(&mut self) {
        self.drop_child();
    }
}

/// デバイスの要約を、ログに書いてよい形にする（英数字と `,:_-` だけ、[`MAX_DEVICES_BYTES`] まで）。
/// 外れていれば `invalid`
fn sanitize_devices(devices: &str) -> String {
    let ok = !devices.is_empty()
        && devices.len() <= MAX_DEVICES_BYTES
        && devices
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b',' | b':' | b'_' | b'-'));
    if ok {
        devices.to_owned()
    } else {
        INVALID_DEVICES.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_provisional_values() {
        let l = HostLimits::default();
        assert_eq!(l.ready_deadline, Duration::from_secs(60));
        assert_eq!(l.request_deadline, Duration::from_secs(30));
        assert_eq!(l.max_failures, 3);
    }

    #[test]
    fn health_gives_up_at_the_limit_and_recovers_on_success() {
        let h = HostHealth::new(2);
        assert!(!h.gave_up());
        h.record_failure();
        assert!(!h.gave_up());
        h.record_success();
        h.record_failure();
        h.record_failure();
        assert!(h.gave_up());
        assert_eq!(
            h.log_note(),
            "host=on host_spawns=0 host_failures=2 host_gave_up=1"
        );
    }

    #[test]
    fn devices_from_the_child_are_checked_before_logging() {
        assert_eq!(
            sanitize_devices("cpu,vulkan:IntegratedGpu"),
            "cpu,vulkan:IntegratedGpu"
        );
        assert_eq!(sanitize_devices("cpu MTL0"), "invalid");
        assert_eq!(sanitize_devices("秘密"), "invalid");
        assert_eq!(sanitize_devices(""), "invalid");
        assert_eq!(sanitize_devices(&"a".repeat(129)), "invalid");
    }
}
