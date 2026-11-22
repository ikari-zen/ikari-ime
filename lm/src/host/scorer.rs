//! 親（エンジン）の側。子を起動して採点を頼む `LmScorer` と、それを作る `LmLoader`。
//!
//! 役割の分け方:
//! - [`HostScorer`] は、子 1 つの一生。子が死んだ・固まった・壊れた返事をしたら、子を回収して `alive() == false` になる。
//!   `ZenzRanker` のワーカーはそれを捨て、次の要求で loader を呼ぶ。
//! - loader（[`host_loader`]）は、子を起動して `Ready` を待つ。起動し直すか諦めるかは [`HostHealth`] が数える。
//!
//! 断念の前の試し直し（2026-11-15 ユーザー決定）: GPU を使ってよい設定で、連続失敗が上限に達したら、
//! すぐには諦めず、数を 0 に戻して、以後の子を CPU だけ（`--gpu cpu`）で起動する。CPU の子でも上限に達したら諦める。
//! 子が「GPU に載せていない」と知らせていたとき（GPU のない機械、設定がオフ）は、試し直さずに諦める。
//!
//! 数えるもの（連続失敗）: 起動できない、`Ready` が締め切りまでに来ない、版が違う、読み込みの失敗、
//! 子が終わった、返事が締め切りまでに来ない、壊れた返事。採点が 1 回成功したら 0 に戻す。
//! 数えないもの: 子が生きていて返した 1 件の採点の失敗（`Failed(Inference)`。長すぎるプロンプトなど、入力で決まる。
//! 同じプロセスで動かすときも、数えるだけで何もしない）。取り消し（ADR 0074。打っているだけで起きる）。
//!
//! 檻（ADR 0072 §3・§4）: 呼び出し側が [`HostCage`] で、モデルの出どころに合わせて選ぶ。プリセット（sha256 を表で
//! 確かめたもの）は段 1 か、段 2 を優先して張れなければ段 1 に落とす（`box=low` がログに出る）。自由指定のモデルは
//! 段 2 だけで、張れなければ起動せず失敗に数える（LM なし）。段 2 から段 1 に落とすときは、モデルに足した段 2 の
//! 許可を外してから起動する（許可が残ると、低い整合性の子は読めない。ADR 0072 §12.5）。開発用のビルド（`dev-knobs`）の
//! Windows だけ、環境変数（`placement::CAGE_ENV`）で上書きできる（VM での計測。#424）。
//!
//! 取り消し（ADR 0074 §3.1）: 返事を待つ間に印が立ったら、`Cancel { id }` を書き込みのスレッドに渡し、子の返事を
//! 待たずに `Cancelled` で戻る。取り消した id は覚えておき、あとで届いたその返事は読み捨てる。

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use crate::cancel::CancelToken;
use crate::error::LmError;
use crate::host::launch::HostLauncher;
use crate::host::model::ModelHandle;
use crate::host::os::{self, Cage, CageState, ChildProc};
use crate::host::proto::{
    FailKind, MAX_FRAME_BYTES, PROTOCOL_VERSION, Reply, Request, read_frame, write_frame,
};
use crate::host::{ARG_GPU, ARG_GPU_CPU, ARG_GPU_OFF, ARG_GPU_ON, ARG_MODEL};
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
/// 仮置き（同上）: 連続でこの回数失敗したら、子の起動を諦める。GPU を使う設定では、GPU の子でこの回数、
/// そのあと CPU の子でこの回数（合わせて 2 倍）
const DEFAULT_MAX_FAILURES: u32 = 3;

/// 檻役の口が無いときに `lm_load` のログへ足す固定の語
const NO_CAGE_NOTE: &str = "host_no_cage=1";

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
    /// 連続でこの回数失敗したら、以後は子を起動しない（1 以上）。GPU を使う設定では、先に CPU だけの子に
    /// 切り替えて、もう一度この回数まで試す
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
    /// モデル（GGUF）。照合のときに開いたハンドル（FC-3。`model` の説明）
    pub model: ModelHandle,
    /// GPU を使ってよいか（設定 `engine.lm_gpu`）
    pub use_gpu: bool,
    pub limits: HostLimits,
    /// 子にかける檻（モデルの出どころで選ぶ）
    pub cage: HostCage,
    /// だれが子を起動するか（OS と配置で選ぶ。ADR 0077。`launch` の説明）
    pub launcher: HostLauncher,
}

/// 子にかける檻の求め（ADR 0072 §4）
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HostCage {
    /// 段 1（プリセット。今の製品）
    #[default]
    Standard,
    /// 段 2 を優先し、張れなければ段 1 に落とす（プリセットだけに使う）
    PreferAppContainer,
    /// 段 2 だけ。張れなければ起動しない（自由指定のモデル。ADR 0072 Q1）
    RequireAppContainer,
}

impl HostCage {
    /// 開発用の上書きの語（`placement::CAGE_ENV` の値）
    #[cfg_attr(not(all(feature = "dev-knobs", windows)), allow(dead_code))]
    fn parse_override(value: &str) -> Option<HostCage> {
        match value {
            "standard" => Some(HostCage::Standard),
            "prefer" => Some(HostCage::PreferAppContainer),
            "appcontainer" => Some(HostCage::RequireAppContainer),
            _ => None,
        }
    }
}

/// 子の様子（数だけ）。loader と scorer が書き、エンジンのログとテストが読む
#[derive(Debug)]
pub struct HostHealth {
    max_failures: u32,
    consecutive_failures: AtomicU32,
    spawns: AtomicU64,
    /// GPU を使ってよい設定か（`HostSpec::use_gpu`）
    use_gpu: bool,
    /// 以後は CPU だけで起動する。子が GPU での読み込みに失敗して CPU で読み直した（ADR 0059 §2.4 と同じ扱い）か、
    /// GPU を使う子が連続で失敗して上限に達した
    gpu_failed: AtomicBool,
    /// `--gpu on` で起動した子が、GPU に載せていないと知らせた（GPU がない、環境変数でオフ）。
    /// CPU で試し直しても同じなので、上限に達したら諦める
    gpu_unused: AtomicBool,
    /// 今の子の pid（いなければ 0）
    child_id: AtomicU32,
    /// 最後に起動した子の檻（`CageState::ALL` の番号）
    cage: AtomicU8,
    /// 檻役の口が無くて起動しなかった（ADR 0077 §3.3 の「檻なし」。失敗には数えない）
    no_cage: AtomicBool,
}

impl HostHealth {
    fn new(max_failures: u32, use_gpu: bool) -> Self {
        Self {
            max_failures: max_failures.max(1),
            consecutive_failures: AtomicU32::new(0),
            spawns: AtomicU64::new(0),
            use_gpu,
            gpu_failed: AtomicBool::new(false),
            gpu_unused: AtomicBool::new(false),
            child_id: AtomicU32::new(0),
            cage: AtomicU8::new(cage_index(CageState::Unknown)),
            no_cage: AtomicBool::new(false),
        }
    }

    /// 檻役の口が無くて、子を起動しなかった状態か（口が届けば戻る）
    pub fn no_cage(&self) -> bool {
        self.no_cage.load(Ordering::Acquire)
    }

    /// 最後に起動した子の檻
    pub(crate) fn cage(&self) -> CageState {
        let i = usize::from(self.cage.load(Ordering::Acquire));
        CageState::ALL.get(i).copied().unwrap_or(CageState::Unknown)
    }

    fn set_cage(&self, cage: CageState) {
        self.cage.store(cage_index(cage), Ordering::Release);
    }

    /// 以後の子を CPU だけで起動する状態か（GPU の子の読み込みの失敗、または連続失敗のあと）
    pub fn gpu_disabled(&self) -> bool {
        self.gpu_failed.load(Ordering::Acquire)
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
        let note = format!(
            "host=on host_spawns={} host_failures={} host_gave_up={} host_gpu_off={} box={}",
            self.spawns(),
            self.consecutive_failures(),
            u8::from(self.gave_up()),
            u8::from(self.gpu_disabled()),
            self.cage().word()
        );
        if self.no_cage() {
            format!("{note} {NO_CAGE_NOTE}")
        } else {
            note
        }
    }

    /// 失敗を 1 つ数える。GPU を使う子で上限に達したら、諦める前に、CPU だけの子に切り替えて数を戻す。
    /// 呼ぶのは LM のワーカーのスレッドだけ（同時には呼ばれない）
    fn record_failure(&self) {
        let failures = self.consecutive_failures.fetch_add(1, Ordering::AcqRel) + 1;
        let gpu_in_use = self.use_gpu
            && !self.gpu_failed.load(Ordering::Acquire)
            && !self.gpu_unused.load(Ordering::Acquire);
        if failures >= self.max_failures && gpu_in_use {
            self.gpu_failed.store(true, Ordering::Release);
            self.consecutive_failures.store(0, Ordering::Release);
        }
    }

    fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Release);
    }
}

fn cage_index(cage: CageState) -> u8 {
    CageState::ALL
        .iter()
        .position(|c| *c == cage)
        .and_then(|i| u8::try_from(i).ok())
        .unwrap_or(0)
}

/// 子にかける檻の求め（`spec.cage`。開発用のビルドの Windows だけ、環境変数で上書きできる）
fn cage_for(spec: &HostSpec) -> HostCage {
    #[cfg(all(feature = "dev-knobs", windows))]
    if let Some(cage) = std::env::var(placement::CAGE_ENV)
        .ok()
        .and_then(|v| HostCage::parse_override(&v))
    {
        return cage;
    }
    spec.cage
}

/// 檻の求めに合わせて子を起動する（このファイルの先頭の説明）
fn spawn_caged(
    spec: &HostSpec,
    args: &[OsString],
) -> std::io::Result<(ChildProc, os::ToChild, os::FromChild)> {
    // 檻役に頼む（macOS の段 2。ADR 0077）。中身は D-3-mac（今は起動しない）
    if let HostLauncher::Cage(slot) = &spec.launcher {
        return os::spawn_via_cage(slot, &spec.model, args, cage_for(spec));
    }
    // 同梱の場所のモデルは、許可を足さずに読める（足す権限も無い。`ModelHandle::needs_cage_grant`）
    let grant = spec.model.needs_cage_grant();
    let container = Cage::AppContainer {
        readable: grant.then(|| spec.model.path()),
    };
    match cage_for(spec) {
        HostCage::Standard => os::spawn(&spec.program, args, Cage::Standard),
        HostCage::RequireAppContainer => os::spawn(&spec.program, args, container),
        HostCage::PreferAppContainer => match os::spawn(&spec.program, args, container) {
            Ok(child) => Ok(child),
            Err(_) => {
                // 許可を外せなくても段 1 を試す（読めなければ、読み込みの失敗に数えられる）
                if grant {
                    let _ = os::revoke_cage_read(spec.model.path());
                }
                os::spawn(&spec.program, args, Cage::Standard)
            }
        },
    }
}

/// 起動の口が檻役で、その口がまだ（または、もう）無いか
fn cage_missing(launcher: &HostLauncher) -> bool {
    matches!(launcher, HostLauncher::Cage(slot) if slot.current().is_none())
}

/// 子を起動して採点を頼む loader（`ZenzRanker::new` に渡す）と、子の様子。
/// 子は、loader が呼ばれたとき（最初の要求、アイドルの解放のあと、子が死んだあと）に起動する。
pub fn host_loader(spec: HostSpec) -> (LmLoader, Arc<HostHealth>) {
    let health = Arc::new(HostHealth::new(spec.limits.max_failures, spec.use_gpu));
    let loader: LmLoader = {
        let health = Arc::clone(&health);
        Arc::new(move || {
            let scorer = HostScorer::start(&spec, &health)?;
            Ok(Box::new(scorer) as Box<dyn LmScorer>)
        })
    };
    (loader, health)
}

/// 読み取りのスレッドと、取り消しの合図から、待っている側へ
enum Incoming {
    Reply(Reply),
    /// 読めない枠が来た（読み取りのスレッドは終わる）
    Malformed,
    /// 子の出力が閉じた
    Closed,
    /// 要求 `id` の待ちの間に、取り消しの印が立った
    Cancelled(u64),
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
    from_child: Receiver<Incoming>,
    /// 取り消しの合図で、待っている受け口を起こすための送り口。これを持つので、子の出力が閉じたことは
    /// 読み取りのスレッドが `Incoming::Closed` で知らせる
    wake: Sender<Incoming>,
}

impl Live {
    /// `gpu` は `--gpu` の値（[`ARG_GPU_ON`] / [`ARG_GPU_OFF`] / [`ARG_GPU_CPU`]）
    fn spawn(spec: &HostSpec, gpu: &str, health: &HostHealth) -> Result<Live, String> {
        let args: [OsString; 4] = [
            ARG_MODEL.into(),
            spec.model.path().as_os_str().to_owned(),
            ARG_GPU.into(),
            gpu.into(),
        ];
        let (proc, mut stdin, mut stdout) =
            spawn_caged(spec, &args).map_err(|e| format!("spawn: {e}"))?;
        health.set_cage(proc.cage());

        // パイプの読み書きには締め切りがないので、スレッドに任せて、チャネルの `recv_timeout` で待つ。
        // 書き込みが詰まっても（子が読まない）、返事が来ないので要求の締め切りで気づき、子を kill する。
        let (reply_tx, from_child) = mpsc::channel::<Incoming>();
        let wake = reply_tx.clone();
        let reader = thread::Builder::new()
            .name(READER_THREAD_NAME.to_owned())
            .spawn(move || {
                while let Ok(Some(frame)) = read_frame(&mut stdout) {
                    let Ok(reply) = Reply::decode(&frame) else {
                        let _ = reply_tx.send(Incoming::Malformed);
                        return;
                    };
                    if reply_tx.send(Incoming::Reply(reply)).is_err() {
                        return;
                    }
                }
                let _ = reply_tx.send(Incoming::Closed);
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
            wake,
        })
    }

    /// 子の返事か、取り消しの合図を待つ。取り消しの合図は `Ok(Err(番号))`
    fn recv_any(&self, deadline: Duration) -> Result<Result<Reply, u64>, RecvError> {
        match self.from_child.recv_timeout(deadline) {
            Ok(Incoming::Reply(reply)) => Ok(Ok(reply)),
            Ok(Incoming::Cancelled(id)) => Ok(Err(id)),
            Ok(Incoming::Malformed) => Err(RecvError::Malformed),
            Ok(Incoming::Closed) | Err(RecvTimeoutError::Disconnected) => Err(RecvError::Closed),
            Err(RecvTimeoutError::Timeout) => Err(RecvError::Timeout),
        }
    }

    /// 子の返事を待つ（`Ready` を待つとき。取り消しの合図は来ない）
    fn recv(&self, deadline: Duration) -> Result<Reply, RecvError> {
        let until = Instant::now() + deadline;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if let Ok(reply) = self.recv_any(left)? {
                return Ok(reply);
            }
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
    /// 取り消して、返事をまだ読んでいない id（届いたら読み捨てる）
    abandoned: HashSet<u64>,
}

impl HostScorer {
    /// 子を起動して `Ready` を待つ。諦めたあとは、起動せずに失敗を返す
    fn start(spec: &HostSpec, health: &Arc<HostHealth>) -> Result<HostScorer, LmError> {
        if health.gave_up() {
            return Err(LmError::Load("lm host gave up".to_owned()));
        }
        // 檻役の口が無いときは、段 1 に落とさず、起動しない（ADR 0077 §6）。口が来るのを待つ状態で、
        // 子の失敗ではないので数えない（数えると、口が届く前に断念してしまう）
        if cage_missing(&spec.launcher) {
            health.no_cage.store(true, Ordering::Release);
            return Err(LmError::Load("lm host: no cage".to_owned()));
        }
        health.no_cage.store(false, Ordering::Release);
        health.spawns.fetch_add(1, Ordering::AcqRel);
        let failed = |what: &str| {
            health.record_failure();
            LmError::Load(format!("lm host: {what}"))
        };
        let gpu_was_disabled = spec.use_gpu && health.gpu_disabled();
        let gpu_arg = match (spec.use_gpu, gpu_was_disabled) {
            (true, false) => ARG_GPU_ON,
            (true, true) => ARG_GPU_CPU,
            (false, _) => ARG_GPU_OFF,
        };
        let live = Live::spawn(spec, gpu_arg, health).map_err(|e| failed(&e))?;
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
                if gpu_arg == ARG_GPU_ON && device != LmDevice::Gpu {
                    health.gpu_unused.store(true, Ordering::Release);
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
                    abandoned: HashSet::new(),
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
    fn nll(
        &mut self,
        prompt: &str,
        candidates: &[String],
        cancel: &CancelToken,
    ) -> Result<Vec<f32>, LmError> {
        let Some(live) = self.live.as_ref() else {
            return Err(LmError::Inference("lm host: child is gone".to_owned()));
        };
        if cancel.is_cancelled() {
            return Err(LmError::Cancelled);
        }
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
        // 待ちの間だけ: 印が立ったら子に `Cancel` を送り、この受け口を起こす。どちらも詰まらない送り方にする
        // （印を立てるのはエンジンの別のスレッド。送れなければ、子は最後まで推論し、その返事を読み捨てる）
        let hook = {
            let to_child = live.to_child.clone();
            let wake = live.wake.clone();
            cancel.on_cancel(move || {
                let _ = to_child.try_send(Request::Cancel { id }.encode());
                let _ = wake.send(Incoming::Cancelled(id));
            })
        };
        let until = Instant::now() + self.request_deadline;
        let outcome = loop {
            let left = until.saturating_duration_since(Instant::now());
            match live.recv_any(left) {
                Ok(Err(got)) if got == id => break Err(None),
                // 前の要求の待ちの合図が遅れて届いた
                Ok(Err(_)) => {}
                Ok(Ok(Reply::Scores { id: got, .. } | Reply::Failed { id: got, .. }))
                    if self.abandoned.remove(&got) => {}
                Ok(Ok(reply)) => break Ok(reply),
                Err(e) => break Err(Some(e)),
            }
        };
        drop(hook);
        match outcome {
            Ok(Reply::Scores { id: got, nll }) if got == id && nll.len() == candidates.len() => {
                self.health.record_success();
                Ok(nll)
            }
            Ok(Reply::Failed {
                id: got,
                kind: FailKind::Inference,
                message,
            }) if got == id => Err(LmError::Inference(message)),
            // 子が先に気づいた（取り消しは失敗に数えない）
            Ok(Reply::Failed {
                id: got,
                kind: FailKind::Cancelled,
                ..
            }) if got == id => Err(LmError::Cancelled),
            Ok(_) => Err(self.fail("unexpected reply")),
            // 印が立った。子の返事は待たず、届いたら読み捨てる
            Err(None) => {
                self.abandoned.insert(id);
                Err(LmError::Cancelled)
            }
            Err(Some(e)) => Err(self.fail(e.word())),
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
    use crate::host::cage::CageSlot;

    #[test]
    fn the_cage_defaults_to_standard_and_parses_only_its_words() {
        assert_eq!(HostCage::default(), HostCage::Standard);
        assert_eq!(
            HostCage::parse_override("appcontainer"),
            Some(HostCage::RequireAppContainer)
        );
        assert_eq!(
            HostCage::parse_override("prefer"),
            Some(HostCage::PreferAppContainer)
        );
        assert_eq!(
            HostCage::parse_override("standard"),
            Some(HostCage::Standard)
        );
        assert_eq!(HostCage::parse_override("AppContainer"), None);
        assert_eq!(HostCage::parse_override(""), None);
    }

    #[test]
    fn defaults_are_the_provisional_values() {
        let l = HostLimits::default();
        assert_eq!(l.ready_deadline, Duration::from_secs(60));
        assert_eq!(l.request_deadline, Duration::from_secs(30));
        assert_eq!(l.max_failures, 3);
    }

    #[test]
    fn health_gives_up_at_the_limit_and_recovers_on_success() {
        let h = HostHealth::new(2, false);
        assert!(!h.gave_up());
        h.record_failure();
        assert!(!h.gave_up());
        h.record_success();
        h.record_failure();
        h.record_failure();
        assert!(h.gave_up());
        assert_eq!(
            h.log_note(),
            "host=on host_spawns=0 host_failures=2 host_gave_up=1 host_gpu_off=0 box=unknown"
        );
    }

    #[test]
    fn a_gpu_child_that_keeps_failing_gets_a_cpu_retry_before_giving_up() {
        let h = HostHealth::new(2, true);
        h.record_failure();
        h.record_failure();
        // GPU の子で上限: 諦めずに、CPU だけに切り替えて数を戻す
        assert!(!h.gave_up());
        assert!(h.gpu_disabled());
        assert_eq!(h.consecutive_failures(), 0);
        h.record_failure();
        h.record_failure();
        assert!(h.gave_up());
    }

    #[test]
    fn no_cpu_retry_when_the_child_was_not_on_the_gpu() {
        let h = HostHealth::new(2, true);
        h.gpu_unused.store(true, Ordering::Release);
        h.record_failure();
        h.record_failure();
        assert!(h.gave_up());
        assert!(!h.gpu_disabled());
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

    fn spec_with(launcher: HostLauncher) -> HostSpec {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.gguf");
        std::fs::write(&path, b"GGUF").unwrap();
        HostSpec {
            program: PathBuf::from("/nonexistent/ikari-lm-host"),
            model: ModelHandle::open(&path).unwrap(),
            use_gpu: false,
            limits: HostLimits::default(),
            cage: HostCage::Standard,
            launcher,
        }
    }

    #[test]
    fn without_a_cage_nothing_is_started_and_nothing_is_counted() {
        let spec = spec_with(HostLauncher::Cage(Arc::new(CageSlot::default())));
        let (loader, health) = host_loader(spec);
        // 口が無いあいだは何度頼んでも、起動も失敗の数え上げもしない（断念しない）
        for _ in 0..(DEFAULT_MAX_FAILURES * 3) {
            assert!(loader().is_err());
        }
        assert_eq!(health.spawns(), 0);
        assert_eq!(health.consecutive_failures(), 0);
        assert!(!health.gave_up());
        assert!(health.no_cage());
        assert!(health.log_note().ends_with(NO_CAGE_NOTE));
    }

    #[test]
    fn a_direct_launcher_is_never_without_a_cage() {
        // 直接の口（テスト、開発、Windows）は、檻役の口を見ない。起動に失敗すれば、今までどおり数える
        let (loader, health) = host_loader(spec_with(HostLauncher::Direct));
        assert!(loader().is_err());
        assert_eq!(health.spawns(), 1);
        assert_eq!(health.consecutive_failures(), 1);
        assert!(!health.no_cage());
        assert!(!health.log_note().contains(NO_CAGE_NOTE));
    }
}
