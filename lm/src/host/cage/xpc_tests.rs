//! 檻役と口の往復（macOS。プロセスの中の匿名の口で、檻役の芯とエンジンの側の口をつなぐ）。子は小さなスクリプト。
//! 署名要件は、通る（`always`）と通らない（`never`）の 2 つだけを使う（束と署名が要らない）。

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use super::client::request;
use super::xpc::{Connection, Obj};
use super::*;

/// 署名要件の言葉で、いつも満たす / いつも満たさない
const ANY_PEER: &str = "always";
const NO_PEER: &str = "never";
/// 親で、close-on-exec の印なしに開いておく fd の番号（os のテストの 200 と重ならない）
const PROBE_FD: i32 = 201;
/// 非同期の回収を待つ上限（テストが揺れない長さ）
const SETTLE: Duration = Duration::from_secs(5);
const REPLY: Duration = Duration::from_secs(5);

/// 0 から 4 と PROBE_FD のうち、開いている fd を 1 行に出す
const FDS_SCRIPT: &str =
    "for n in 0 1 2 3 4 201; do if [ -e /dev/fd/$n ]; then printf '%s ' $n; fi; done; echo";
/// 環境変数をそのまま出す
const ENV_SCRIPT: &str = "exec /usr/bin/env";
/// 標準入力が閉じるまで生きる
const CAT_SCRIPT: &str = "exec /bin/cat";
/// シェルが自分で足す環境変数（子に渡したものではない）
const SHELL_OWN_VARS: [&str; 4] = ["PWD", "SHLVL", "_", "OLDPWD"];

struct Rig {
    _dir: tempfile::TempDir,
    core: Arc<CageCore>,
    listener: CageListener,
    model: PathBuf,
}

impl Rig {
    fn new(body: &str) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("child.sh");
        std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let model = dir.path().join("model.gguf");
        std::fs::write(&model, b"gguf").unwrap();
        let core = Arc::new(CageCore::new(&program));
        let listener = CageListener::start(Arc::clone(&core), ANY_PEER).unwrap();
        Rig {
            _dir: dir,
            core,
            listener,
            model,
        }
    }

    fn client(&self) -> XpcCageClient {
        self.listener.client(ANY_PEER).unwrap()
    }

    /// 起動を頼む。返すのは、子の持ち手と、子の標準入力へ書く口と、標準出力を読む口
    fn launch(
        &self,
        client: &XpcCageClient,
    ) -> io::Result<(Box<dyn CageChild>, io::PipeWriter, io::PipeReader)> {
        let (child_in, to_child) = io::pipe()?;
        let (from_child, child_out) = io::pipe()?;
        let child = client.launch(CageLaunch {
            stdin: child_in,
            stdout: child_out,
            model: File::open(&self.model)?,
            gpu: CageGpu::Cpu,
        })?;
        Ok((child, to_child, from_child))
    }

    /// 生の接続（取り決めを手で送るため）
    fn raw(&self) -> Connection {
        let conn = Connection::to_endpoint(&self.listener.endpoint().unwrap()).unwrap();
        conn.on_event(|_| {});
        let cstr = std::ffi::CString::new(ANY_PEER).unwrap();
        conn.require_peer(&cstr).unwrap();
        conn.resume();
        conn
    }
}

fn message(op: &str, version: Option<u64>) -> Obj {
    let m = Obj::dictionary().unwrap();
    m.set_str(wire::KEY_OP, op);
    if let Some(v) = version {
        m.set_u64(wire::KEY_VERSION, v);
    }
    m
}

fn ask(conn: &Connection, m: &Obj) -> Option<CageReply> {
    let reply = request(conn, m, REPLY).unwrap();
    reply
        .get_str(wire::KEY_REPLY)
        .as_deref()
        .and_then(CageReply::from_word)
}

const V: Option<u64> = Some(CAGE_PROTOCOL_VERSION as u64);

/// 正しい 3 つの fd を付けた `launch`
fn launch_message(model: &File) -> (Obj, io::PipeWriter, io::PipeReader) {
    let (child_in, to_child) = io::pipe().unwrap();
    let (from_child, child_out) = io::pipe().unwrap();
    let m = message(wire::OP_LAUNCH, V);
    m.set_str(wire::KEY_GPU, CageGpu::Cpu.word());
    m.set_fd(wire::KEY_STDIN, child_in.as_fd());
    m.set_fd(wire::KEY_STDOUT, child_out.as_fd());
    m.set_fd(wire::KEY_MODEL, model.as_fd());
    (m, to_child, from_child)
}

fn read_all(mut from_child: io::PipeReader) -> String {
    let mut out = String::new();
    from_child.read_to_string(&mut out).unwrap();
    out
}

fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + SETTLE;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

/// その pid のプロセスがもう無い（回収済み）
fn gone(pid: u32) -> bool {
    let pid = libc::pid_t::try_from(pid).unwrap();
    // SAFETY: 状態を聞くだけ（signal 0）
    unsafe { libc::kill(pid, 0) == -1 }
}

#[test]
fn caged_child_gets_only_fds_0_1_3() {
    let file = tempfile::tempfile().unwrap();
    // SAFETY: 開いた fd を、使われていない番号に複製するだけ（`dup2` の結果には close-on-exec が付かない）
    let fd = unsafe { libc::dup2(file.as_raw_fd(), PROBE_FD) };
    assert_eq!(fd, PROBE_FD);
    // SAFETY: いま開いた fd で、ほかに持ち主はいない
    let _probe = unsafe { OwnedFd::from_raw_fd(fd) };

    let rig = Rig::new(FDS_SCRIPT);
    let (_child, to_child, from_child) = rig.launch(&rig.client()).unwrap();
    drop(to_child);
    assert_eq!(read_all(from_child), "0 1 3 \n");
}

#[test]
fn caged_child_has_an_empty_environment() {
    let rig = Rig::new(ENV_SCRIPT);
    let (_child, to_child, from_child) = rig.launch(&rig.client()).unwrap();
    drop(to_child);
    let out = read_all(from_child);
    for line in out.lines() {
        let name = line.split_once('=').map_or(line, |(k, _)| k);
        assert!(SHELL_OWN_VARS.contains(&name), "{line}");
    }
}

#[test]
fn the_child_gets_the_model_as_fd_3_and_the_fixed_arguments() {
    let rig = Rig::new("printf '%s ' \"$@\"; /bin/cat <&3");
    let (_child, to_child, from_child) = rig.launch(&rig.client()).unwrap();
    drop(to_child);
    assert_eq!(read_all(from_child), "--model /dev/fd/3 --gpu cpu gguf");
}

#[test]
fn a_round_trip_through_the_pipes_reaches_the_child() {
    let rig = Rig::new(CAT_SCRIPT);
    let (mut child, mut to_child, from_child) = rig.launch(&rig.client()).unwrap();
    assert!(child.pid() > 0);
    to_child.write_all(b"ping\n").unwrap();
    drop(to_child);
    assert_eq!(read_all(from_child), "ping\n");
    child.stop().unwrap();
}

#[test]
fn launches_beyond_the_total_limit_are_busy() {
    let rig = Rig::new(CAT_SCRIPT);
    let client = rig.client();
    let mut held = Vec::new();
    for _ in 0..MAX_CHILDREN_TOTAL {
        held.push(rig.launch(&client).unwrap());
    }
    assert_eq!(rig.core.live_children(), MAX_CHILDREN_TOTAL);
    let err = rig.launch(&client).err().unwrap();
    assert!(err.to_string().contains(CageReply::Busy.word()), "{err}");
    // 1 つ止めれば、また起動できる
    let (mut first, _, _) = held.remove(0);
    first.stop().unwrap();
    assert_eq!(rig.core.live_children(), MAX_CHILDREN_TOTAL - 1);
    held.push(rig.launch(&client).unwrap());
}

#[test]
fn one_child_per_connection() {
    let rig = Rig::new(CAT_SCRIPT);
    let model = File::open(&rig.model).unwrap();
    let conn = rig.raw();
    assert_eq!(ask(&conn, &message(wire::OP_HELLO, V)), Some(CageReply::Ok));
    let (first, _in1, _out1) = launch_message(&model);
    assert_eq!(ask(&conn, &first), Some(CageReply::Ok));
    let (second, _in2, _out2) = launch_message(&model);
    assert_eq!(ask(&conn, &second), Some(CageReply::Busy));
    assert_eq!(rig.core.live_children(), 1);
}

#[test]
fn stop_replies_after_kill_and_reap() {
    let rig = Rig::new(CAT_SCRIPT);
    let (mut child, _to_child, _from_child) = rig.launch(&rig.client()).unwrap();
    let pid = child.pid();
    assert!(!gone(pid));
    child.stop().unwrap();
    // 返事の時点で、もう回収されている
    assert!(gone(pid));
    assert_eq!(rig.core.live_children(), 0);
    // 2 回目は何もしない
    child.stop().unwrap();
}

#[test]
fn dropping_the_connection_reaps_the_child() {
    let rig = Rig::new(CAT_SCRIPT);
    let (child, _to_child, _from_child) = rig.launch(&rig.client()).unwrap();
    let pid = child.pid();
    drop(child);
    assert!(wait_until(|| rig.core.live_children() == 0));
    assert!(gone(pid));
}

#[test]
fn a_child_that_exits_by_itself_is_reaped_and_frees_its_slot() {
    let rig = Rig::new("exit 0");
    let (child, _to_child, _from_child) = rig.launch(&rig.client()).unwrap();
    let pid = child.pid();
    assert!(wait_until(|| rig.core.live_children() == 0));
    assert!(gone(pid));
}

#[test]
fn version_mismatch_is_refused_without_spawning() {
    let rig = Rig::new(CAT_SCRIPT);
    let model = File::open(&rig.model).unwrap();
    let other = Some(u64::from(CAGE_PROTOCOL_VERSION) + 1);
    let conn = rig.raw();
    assert_eq!(
        ask(&conn, &message(wire::OP_HELLO, other)),
        Some(CageReply::VersionMismatch)
    );
    assert_eq!(ask(&conn, &message(wire::OP_HELLO, V)), Some(CageReply::Ok));
    let (launch, _in, _out) = launch_message(&model);
    launch.set_u64(wire::KEY_VERSION, other.unwrap());
    assert_eq!(ask(&conn, &launch), Some(CageReply::VersionMismatch));
    assert_eq!(rig.core.live_children(), 0);
}

#[test]
fn xpc_hello_carries_no_fd_and_comes_first() {
    let rig = Rig::new(CAT_SCRIPT);
    let model = File::open(&rig.model).unwrap();
    // 檻役は、hello の前の launch を断る
    let conn = rig.raw();
    let (early, _in, _out) = launch_message(&model);
    assert_eq!(ask(&conn, &early), Some(CageReply::BadRequest));
    // fd つきの hello も断る（受けない）
    let hello = message(wire::OP_HELLO, V);
    hello.set_fd(wire::KEY_MODEL, model.as_fd());
    assert_eq!(ask(&conn, &hello), Some(CageReply::BadRequest));
    assert_eq!(rig.core.live_children(), 0);
    // エンジンの側の hello は、そのまま通る（fd を付けない）
    let answer = rig.client().hello().unwrap();
    assert_eq!(answer.peer, PeerCheck::Verified);
    assert_eq!(answer.reply, Some(CageReply::Ok));
    assert_eq!(answer.version, V);
}

#[test]
fn a_peer_that_fails_the_requirement_gets_no_launch() {
    // 相手（檻役）が署名要件を満たさないと、返事は落とされ、hello で止まる（fd を送らない）
    let rig = Rig::new(CAT_SCRIPT);
    let client = rig.listener.client(NO_PEER).unwrap();
    // 要件は付けられ（OS の誤りではなく）、返事が相手の確認で落とされる
    let dropped = client.hello().err().unwrap();
    assert_eq!(dropped.raw_os_error(), None, "{dropped}");
    assert_eq!(dropped.kind(), io::ErrorKind::PermissionDenied, "{dropped}");
    let slot = CageSlot::default();
    let err = slot.attach_unverified(Arc::new(client)).err().unwrap();
    assert_eq!(err.word(), "io");
    assert!(slot.current().is_none());
    let client = rig.listener.client(NO_PEER).unwrap();
    assert!(rig.launch(&client).is_err());
    assert_eq!(rig.core.live_children(), 0);
}

#[test]
fn the_cage_checks_received_fds_before_spawning() {
    let rig = Rig::new(CAT_SCRIPT);
    let model = File::open(&rig.model).unwrap();
    let writable = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&rig.model)
        .unwrap();
    let conn = rig.raw();
    assert_eq!(ask(&conn, &message(wire::OP_HELLO, V)), Some(CageReply::Ok));
    // モデルが書き込みでも開ける
    let (m, _in, _out) = launch_message(&model);
    m.set_fd(wire::KEY_MODEL, writable.as_fd());
    assert_eq!(ask(&conn, &m), Some(CageReply::BadRequest));
    // 標準入力と標準出力が逆
    let (child_in, to_child) = io::pipe().unwrap();
    let m = message(wire::OP_LAUNCH, V);
    m.set_str(wire::KEY_GPU, CageGpu::Cpu.word());
    m.set_fd(wire::KEY_STDIN, to_child.as_fd());
    m.set_fd(wire::KEY_STDOUT, child_in.as_fd());
    m.set_fd(wire::KEY_MODEL, model.as_fd());
    assert_eq!(ask(&conn, &m), Some(CageReply::BadRequest));
    // fd が足りない
    let m = message(wire::OP_LAUNCH, V);
    m.set_str(wire::KEY_GPU, CageGpu::Cpu.word());
    assert_eq!(ask(&conn, &m), Some(CageReply::BadRequest));
    // 知らない GPU の語
    let (m, _in2, _out2) = launch_message(&model);
    m.set_str(wire::KEY_GPU, "metal");
    assert_eq!(ask(&conn, &m), Some(CageReply::BadRequest));
    assert_eq!(rig.core.live_children(), 0);
    // 正しい形なら起動する
    let (m, _in3, _out3) = launch_message(&model);
    assert_eq!(ask(&conn, &m), Some(CageReply::Ok));
}

#[test]
fn unknown_words_are_bad_requests() {
    let rig = Rig::new(CAT_SCRIPT);
    let conn = rig.raw();
    assert_eq!(ask(&conn, &message("exec", V)), Some(CageReply::BadRequest));
    assert_eq!(
        ask(&conn, &message(wire::OP_STOP, V)),
        Some(CageReply::BadRequest)
    );
    // 名前の口の語は、匿名の口では受けない
    assert_eq!(
        ask(&conn, &message(wire::OP_ENDPOINT, V)),
        Some(CageReply::BadRequest)
    );
}

#[test]
fn a_relative_program_is_never_spawned() {
    let core = Arc::new(CageCore::new(std::path::Path::new("ikari-lm-host")));
    let listener = CageListener::start(Arc::clone(&core), ANY_PEER).unwrap();
    let rig = Rig::new(CAT_SCRIPT);
    let client = listener.client(ANY_PEER).unwrap();
    let err = rig.launch(&client).err().unwrap();
    assert!(
        err.to_string().contains(CageReply::SpawnFailed.word()),
        "{err}"
    );
    assert_eq!(core.live_children(), 0);
}
