//! 檻役の側（bin `ikari-lm-cage`。macOS。ADR 0077 §3.1）。判断を持たず、決まった子を決まった引数で起動し、
//! 止めて回収するだけ。
//!
//! - 名前の口（XPC サービス。殻と smoke だけ）: [`wire::OP_ENDPOINT`] に、匿名の口を返す。
//! - 匿名の口（エンジンだけ）: 接続ごとに [`CageSession`] を持ち、`hello` のあとでだけ `launch` を受ける。
//!   受けた fd は [`probe_fd`] と [`check_launch_fds`] で確かめてから起動する。子は接続ごとに 1 つ、全体で
//!   [`MAX_CHILDREN_TOTAL`] まで。接続が無効になったら、その接続の子を止めて回収する。
//! - 相手の確認は、口ごとの署名要件（束の Info.plist。無ければ、その口の接続をすべて断る）。

use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use super::client::XpcCageClient;
use super::spawn::{self, CagedChild};
use super::xpc::{self, Connection, Kind, Obj, RawObj};
use super::{
    CAGE_PROTOCOL_VERSION, CageGpu, CageReply, CageSession, INFO_KEY_ENGINE_PEER_REQUIREMENT,
    INFO_KEY_SHELL_PEER_REQUIREMENT, MAX_CHILDREN_TOTAL, check_launch_fds, main_bundle_string,
    probe_fd, wire,
};
use crate::host::HOST_EXE_STEM;

/// 檻役の判断を持たない芯（子の起動、数え方、回収）。テストはプロセスの中の匿名の口でこれを動かす
#[derive(Debug)]
pub struct CageCore {
    /// 起動する実行ファイル（絶対パス。無ければ起動を断る）
    program: Option<PathBuf>,
    /// 動いている子の数（全体）
    live: Arc<AtomicUsize>,
    /// 子がいる間、XPC サービスを終わらせない（サービスとして動くときだけ）
    transactions: bool,
}

/// 接続 1 本の状態
#[derive(Debug, Default)]
struct PeerState {
    session: CageSession,
    child: Option<Arc<CagedChild>>,
}

/// 返事 1 つ
struct Answer {
    reply: CageReply,
    pid: Option<u32>,
}

impl Answer {
    fn of(reply: CageReply) -> Answer {
        Answer { reply, pid: None }
    }
}

impl CageCore {
    /// `program` を子として起動する芯（`program` は絶対パス。相対なら起動を断る）
    pub fn new(program: &Path) -> CageCore {
        CageCore {
            program: program.is_absolute().then(|| program.to_owned()),
            live: Arc::new(AtomicUsize::new(0)),
            transactions: false,
        }
    }

    /// 動いている子の数（全体）
    pub fn live_children(&self) -> usize {
        self.live.load(Ordering::Acquire)
    }

    fn reserve(&self) -> bool {
        self.live
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_CHILDREN_TOTAL).then_some(n + 1)
            })
            .is_ok()
    }

    fn released(&self) -> spawn::OnReaped {
        let live = Arc::clone(&self.live);
        let transactions = self.transactions;
        Box::new(move || {
            live.fetch_sub(1, Ordering::AcqRel);
            if transactions {
                xpc::transaction_end();
            }
        })
    }

    fn handle(&self, state: &mut PeerState, message: &Obj) -> Answer {
        let version = message.get_u64(wire::KEY_VERSION);
        match message.get_str(wire::KEY_OP).as_deref() {
            Some(wire::OP_HELLO) => {
                // hello は fd を付けない（FC-1）。付いていれば形の違い
                if [wire::KEY_STDIN, wire::KEY_STDOUT, wire::KEY_MODEL]
                    .iter()
                    .any(|k| message.has(k))
                {
                    return Answer::of(CageReply::BadRequest);
                }
                Answer::of(state.session.on_hello(version))
            }
            Some(wire::OP_LAUNCH) => self.launch(state, version, message),
            Some(wire::OP_STOP) => match state.session.admit_launch(version) {
                Err(reply) => Answer::of(reply),
                Ok(()) => {
                    if let Some(child) = state.child.take() {
                        child.kill_and_reap();
                    }
                    Answer::of(CageReply::Ok)
                }
            },
            _ => Answer::of(CageReply::BadRequest),
        }
    }

    fn launch(&self, state: &mut PeerState, version: Option<u64>, message: &Obj) -> Answer {
        if let Err(reply) = state.session.admit_launch(version) {
            return Answer::of(reply);
        }
        if state.child.as_ref().is_some_and(|c| !c.is_reaped()) {
            return Answer::of(CageReply::Busy);
        }
        let Some(gpu) = message
            .get_str(wire::KEY_GPU)
            .as_deref()
            .and_then(CageGpu::from_word)
        else {
            return Answer::of(CageReply::BadRequest);
        };
        let fds = (
            message.dup_fd(wire::KEY_STDIN),
            message.dup_fd(wire::KEY_STDOUT),
            message.dup_fd(wire::KEY_MODEL),
        );
        let (Some(stdin), Some(stdout), Some(model)) = fds else {
            return Answer::of(CageReply::BadRequest);
        };
        if !fds_are_allowed(&stdin, &stdout, &model) {
            return Answer::of(CageReply::BadRequest);
        }
        let Some(program) = self.program.as_deref() else {
            return Answer::of(CageReply::SpawnFailed);
        };
        if !self.reserve() {
            return Answer::of(CageReply::Busy);
        }
        if self.transactions {
            xpc::transaction_begin();
        }
        let released = self.released();
        match spawn::spawn(
            program,
            gpu,
            stdin.as_fd(),
            stdout.as_fd(),
            model.as_fd(),
            released,
        ) {
            Ok(child) => {
                let pid = child.pid();
                state.child = Some(child);
                Answer {
                    reply: CageReply::Ok,
                    pid: Some(pid),
                }
            }
            Err(_) => {
                // 起動しなかったので、数とトランザクションを戻す
                (self.released())();
                Answer::of(CageReply::SpawnFailed)
            }
        }
    }

    /// 接続が無効になった: その接続の子を止めて回収する
    fn closed(&self, state: &mut PeerState) {
        if let Some(child) = state.child.take() {
            child.kill_and_reap();
        }
    }
}

/// 受けた 3 つの fd の種類と開き方を確かめる（FC-2）。取れなければ通さない
fn fds_are_allowed(stdin: &OwnedFd, stdout: &OwnedFd, model: &OwnedFd) -> bool {
    match (
        probe_fd(stdin.as_fd()),
        probe_fd(stdout.as_fd()),
        probe_fd(model.as_fd()),
    ) {
        (Ok(i), Ok(o), Ok(m)) => check_launch_fds(i, o, m).is_ok(),
        _ => false,
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 匿名の受け口（エンジンだけが使う）。捨てると受け口を取り消す
#[derive(Debug)]
pub struct CageListener {
    listener: Connection,
}

impl CageListener {
    /// 受け口を作って始める。`requirement` は、つないでくる相手（エンジン）の署名要件
    pub fn start(core: Arc<CageCore>, requirement: &str) -> io::Result<CageListener> {
        let requirement =
            Arc::new(CString::new(requirement).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "nul in peer requirement")
            })?);
        let listener = Connection::anonymous_listener()?;
        listener.on_event(move |event| {
            if xpc::kind_of(event) == Kind::Connection {
                accept_engine(Arc::clone(&core), &requirement, event);
            }
        });
        listener.resume();
        Ok(CageListener { listener })
    }

    /// この受け口への匿名の口（殻がエンジンに渡す）
    pub(super) fn endpoint(&self) -> io::Result<Obj> {
        self.listener.endpoint()
    }

    /// この受け口へつなぐエンジンの側の口（テストと開発用。製品は殻が仲立ちした口を
    /// [`XpcCageClient::from_endpoint`] で受ける）。`requirement` は檻役の側の署名要件
    pub fn client(&self, requirement: &str) -> io::Result<XpcCageClient> {
        XpcCageClient::from_obj(self.endpoint()?, requirement)
    }
}

/// 受けない接続を閉じる（始めずに放すと libxpc が誤りにするので、空のハンドラで始めてから取り消す）
pub(super) fn refuse(peer: &Obj) {
    peer.on_event(|_| {});
    peer.resume();
    peer.cancel();
}

/// 受けた接続に、要件とハンドラを付けて始める。ハンドラは返事を送るために接続を持ち、最後の知らせ
/// （無効）で放す（ハンドラとの循環を切る）
fn serve_peer(
    event: RawObj,
    requirement: &CStr,
    on_message: impl Fn(&Obj) -> Option<Obj> + Send + Sync + 'static,
    on_closed: impl Fn() + Send + Sync + 'static,
) {
    let Some(peer) = Obj::retain(event) else {
        return;
    };
    if peer.require_peer(requirement).is_err() {
        refuse(&peer);
        return;
    }
    let holder = Mutex::new(Some(peer.clone()));
    peer.on_event(move |event| match xpc::kind_of(event) {
        Kind::Dictionary => {
            let Some(message) = Obj::retain(event) else {
                return;
            };
            if let (Some(reply), Some(conn)) = (on_message(&message), lock(&holder).as_ref()) {
                conn.send(&reply);
            }
        }
        Kind::ConnectionInvalid => {
            on_closed();
            lock(&holder).take();
        }
        _ => {}
    });
    peer.resume();
}

/// エンジンからの接続 1 本を受ける
fn accept_engine(core: Arc<CageCore>, requirement: &CStr, event: RawObj) {
    let state = Arc::new(Mutex::new(PeerState::default()));
    let closing = Arc::clone(&state);
    let closing_core = Arc::clone(&core);
    serve_peer(
        event,
        requirement,
        move |message| {
            let answer = core.handle(&mut lock(&state), message);
            let reply = message.reply_to()?;
            write_answer(&reply, &answer);
            Some(reply)
        },
        move || closing_core.closed(&mut lock(&closing)),
    );
}

fn write_answer(reply: &Obj, answer: &Answer) {
    reply.set_str(wire::KEY_REPLY, answer.reply.word());
    reply.set_u64(wire::KEY_VERSION, u64::from(CAGE_PROTOCOL_VERSION));
    if let Some(pid) = answer.pid {
        reply.set_u64(wire::KEY_PID, u64::from(pid));
    }
}

/// 名前の口への要求（殻、smoke）: 匿名の口を返す。ほかの要求は `bad-request`
fn endpoint_reply(listener: Option<&CageListener>, message: &Obj) -> Option<Obj> {
    let reply = message.reply_to()?;
    let asked = message.get_str(wire::KEY_OP).as_deref() == Some(wire::OP_ENDPOINT);
    let version = message.get_u64(wire::KEY_VERSION);
    let answer = if !asked {
        CageReply::BadRequest
    } else if version != Some(u64::from(CAGE_PROTOCOL_VERSION)) {
        CageReply::VersionMismatch
    } else {
        match listener.map(CageListener::endpoint) {
            Some(Ok(endpoint)) => {
                reply.set_value(wire::KEY_ENDPOINT, &endpoint);
                CageReply::Ok
            }
            _ => CageReply::SpawnFailed,
        }
    };
    reply.set_str(wire::KEY_REPLY, answer.word());
    reply.set_u64(wire::KEY_VERSION, u64::from(CAGE_PROTOCOL_VERSION));
    Some(reply)
}

/// XPC サービスとしての檻役（bin の中身）
struct Service {
    shell_requirement: Option<CString>,
    listener: Option<CageListener>,
}

static SERVICE: OnceLock<Service> = OnceLock::new();

/// 檻役の XPC サービスとして動く（戻らない）。実行ファイルは自分の隣の `ikari-lm-host`。
/// 署名要件は自分の束の Info.plist（無い口は、すべての接続を断る）
pub fn run_service() -> ! {
    let program = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(HOST_EXE_STEM)));
    let mut core = CageCore::new(program.as_deref().unwrap_or(Path::new("")));
    core.transactions = true;
    let core = Arc::new(core);
    let shell_requirement =
        main_bundle_string(INFO_KEY_SHELL_PEER_REQUIREMENT).and_then(|r| CString::new(r).ok());
    let listener = main_bundle_string(INFO_KEY_ENGINE_PEER_REQUIREMENT)
        .and_then(|r| CageListener::start(Arc::clone(&core), &r).ok());
    let _ = SERVICE.set(Service {
        shell_requirement,
        listener,
    });
    xpc::run_main(on_named_peer)
}

/// 名前の口への接続（殻、smoke）
extern "C" fn on_named_peer(event: RawObj) {
    let Some(service) = SERVICE.get() else {
        return;
    };
    let Some(requirement) = service.shell_requirement.as_deref() else {
        // 要件が分からないときは誰も受けない
        if let Some(peer) = Obj::retain(event) {
            refuse(&peer);
        }
        return;
    };
    serve_peer(
        event,
        requirement,
        move |message| endpoint_reply(service.listener.as_ref(), message),
        || {},
    );
}
