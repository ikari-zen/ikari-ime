//! macOS の檻役を通した LM の子（ADR 0077）: プロセスの中の匿名の口で、檻役の芯（`CageCore`）とエンジンの側の口
//! （`XpcCageClient`）をつなぎ、偽の子（`ikari-lm-host-fake`）で採点が往復することと、檻役が落ちたときに子が
//! 自分で終わることを確かめる。束も署名も要らない（署名要件は `always`）。
#![cfg(target_vendor = "apple")]

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use ikari_core::{
    AnalyzedContext, Candidate, CandidateOrigin, Context, DenseGrammar, InMemoryDictionary,
    RankInput, Ranker, UnknownParams,
};
use ikari_lm::host::cage::{CageCore, CageListener, CageSlot, PARENT_WATCH_INTERVAL};
use ikari_lm::host::{HostCage, HostLauncher, HostLimits, HostSpec, ModelHandle, host_loader};
use ikari_lm::{ZenzConfig, ZenzRanker};

const FAKE_HOST: &str = env!("CARGO_BIN_EXE_ikari-lm-host-fake");
/// 署名要件の言葉で、いつも満たす
const ANY_PEER: &str = "always";
const RANK_BUDGET_MS: u64 = 3000;
const GOOD: &str = "良";
const BAD: &str = "悪";
const BAD_BASE: i32 = 5000;
const GOOD_BASE: i32 = 5200;
const HEALTHY: &str = "nll.良=0\nnll.悪=20\n";
/// 檻役の代わりの親が、子を起動してから生きている長さ（見張りの間隔より長い）
const PARENT_ALIVE: Duration = Duration::from_secs(3);

fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + ikari_testtime::scaled(Duration::from_secs(5));
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        thread::sleep(ikari_testtime::ms(10));
    }
    false
}

fn pid_alive(pid: u32) -> bool {
    let pid = libc::pid_t::try_from(pid).unwrap();
    // SAFETY: 状態を聞くだけ（signal 0）
    unsafe { libc::kill(pid, 0) == 0 }
}

struct Rig {
    _dir: tempfile::TempDir,
    core: Arc<CageCore>,
    _listener: CageListener,
    ranker: ZenzRanker,
    health: Arc<ikari_lm::host::HostHealth>,
}

impl Rig {
    fn new(attach: bool) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("script.txt");
        std::fs::write(&script, HEALTHY).unwrap();
        let core = Arc::new(CageCore::new(&PathBuf::from(FAKE_HOST)));
        let listener = CageListener::start(Arc::clone(&core), ANY_PEER).unwrap();
        let slot = Arc::new(CageSlot::default());
        if attach {
            let client = listener.client(ANY_PEER).unwrap();
            slot.attach_unverified(Arc::new(client)).unwrap();
        }
        let (loader, health) = host_loader(HostSpec {
            // 檻役の口では使わない（檻役が自分の実行ファイルを起動する）
            program: PathBuf::from("/nonexistent/ikari-lm-host"),
            model: ModelHandle::open(&script).unwrap(),
            use_gpu: false,
            limits: HostLimits::default(),
            cage: HostCage::Standard,
            launcher: HostLauncher::Cage(slot),
        });
        let cfg = ZenzConfig {
            budget: ikari_testtime::ms(RANK_BUDGET_MS),
            load_retry: Duration::ZERO,
            ..ZenzConfig::default()
        };
        Rig {
            _dir: dir,
            core,
            _listener: listener,
            ranker: ZenzRanker::new(loader, cfg),
            health,
        }
    }

    fn first(&self, reading: &str) -> String {
        let grammar = DenseGrammar::new(
            1,
            1,
            0,
            UnknownParams {
                left_id: 0,
                right_id: 0,
                pos_id: 0,
                cost_per_char: 1,
            },
        );
        let dict = InMemoryDictionary::new(0);
        let analysis = AnalyzedContext::default();
        let ctx = Context::new().with_left_text("文脈");
        let cand = |surface: &str, base: i32| Candidate {
            surface: surface.to_owned(),
            segments: Vec::new(),
            base_cost: base,
            score: base,
            origin: CandidateOrigin::Dictionary,
        };
        let mut cands = vec![cand(BAD, BAD_BASE), cand(GOOD, GOOD_BASE)];
        let input = RankInput {
            reading,
            context: &ctx,
            grammar: &grammar,
            analysis: &analysis,
            dict: &dict,
            deadline: Some(Instant::now() + ikari_testtime::ms(RANK_BUDGET_MS)),
        };
        self.ranker.rank(&input, &mut cands);
        cands.sort_by_key(|c| c.score);
        cands[0].surface.clone()
    }
}

#[test]
fn scores_come_from_the_child_the_cage_started() {
    let rig = Rig::new(true);
    assert_eq!(rig.first("よみ1"), GOOD);
    assert_eq!(rig.first("よみ2"), GOOD);
    assert_eq!(rig.health.spawns(), 1);
    assert_eq!(rig.core.live_children(), 1);
    assert!(
        rig.health.log_note().contains("box=cage"),
        "{}",
        rig.health.log_note()
    );
    let pid = rig.health.child_id().expect("a caged child");
    assert!(pid_alive(pid));
    // エンジンが子を手放すと、檻役が止めて回収する
    let Rig { core, ranker, .. } = rig;
    drop(ranker);
    assert!(wait_until(|| core.live_children() == 0));
    assert!(!pid_alive(pid));
}

#[test]
fn without_a_cage_there_is_no_lm_and_no_child() {
    let rig = Rig::new(false);
    assert_eq!(rig.first("よみ1"), BAD);
    assert_eq!(rig.core.live_children(), 0);
    assert!(rig.health.child_id().is_none());
}

#[test]
fn child_exits_when_the_cage_dies() {
    // 親（ここでは sh。檻役の代わり）が子を残して先に終わる。子の標準入力は、このテストが開いたまま持つので
    // 入力の終わりでは終わらない。親なしになったことに見張りで気づいて終わる
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("script.txt");
    std::fs::write(&script, HEALTHY).unwrap();
    let (reader, _writer) = std::io::pipe().unwrap();
    // 親は子の pid を出したあと、PARENT_ALIVE の間だけ生きる（その間、子は生きている。対照）
    let mut parent = Command::new("/bin/sh")
        .arg("-c")
        .arg("exec 3<&0; \"$0\" --model \"$1\" --gpu cpu <&3 >/dev/null 2>&1 & echo $!; exec sleep \"$2\"")
        .arg(FAKE_HOST)
        .arg(&script)
        .arg(PARENT_ALIVE.as_secs().to_string())
        .stdin(Stdio::from(reader))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(parent.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let pid: u32 = line.trim().parse().unwrap();
    thread::sleep(PARENT_WATCH_INTERVAL + Duration::from_millis(500));
    assert!(pid_alive(pid), "the child must live while its parent does");
    assert!(parent.wait().unwrap().success());
    let limit = PARENT_WATCH_INTERVAL * 3 + Duration::from_secs(2);
    let deadline = Instant::now() + ikari_testtime::scaled(limit);
    while pid_alive(pid) && Instant::now() < deadline {
        thread::sleep(ikari_testtime::ms(20));
    }
    assert!(!pid_alive(pid), "orphaned child {pid} is still running");
}
