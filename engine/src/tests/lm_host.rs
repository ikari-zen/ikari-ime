//! 別プロセスの LM（ADR 0060 Q4）をエンジンごと通す。偽の子（`ikari-lm-host-fake`。モデルなし）を使う。
//! 子を落としても、エンジンは生きていて、辞書から作った候補（LM なしの順位）を返し続け、子を起動し直すと LM が戻る。

use ikari_stack::{StackPaths, build_stack};

use super::*;

const FAKE_EXE_STEM: &str = "ikari-lm-host-fake";
const FAKE_TRACE_DIR_ENV: &str = "IKARI_LM_HOST_FAKE_TRACE_DIR";
const FAKE_TRACE_CHILD_ENV: &str = "IKARI_LM_HOST_FAKE_TRACE_CHILD";
const INFERENCE_STARTED: &str = "inference-started";
const INFERENCE_COMPLETED: &str = "inference-completed";
const INFERENCE_CANCELLED: &str = "inference-cancelled";
const INFERENCE_HOLD: &str = "inference-hold";
/// 偽の子の台本: LM が効くと `BOOSTED`（瀕死）が 1 位になる。辞書だけなら `BASE_TOP`（品詞）
const SCRIPT: &str = "nll.瀕死=0\nnll.品詞=20\n";

/// 偽の子の実行ファイル。`cargo test --workspace` が `ikari-lm` の bin として、テストの exe の 1 つ上に作る
fn fake_host() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let path = exe
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join(format!("{FAKE_EXE_STEM}{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.is_file(),
        "{} is missing: run `cargo test --workspace`, or `cargo build -p ikari-lm --bin {FAKE_EXE_STEM}` first",
        path.display()
    );
    path
}

struct HostEngine {
    dir: PathBuf,
    script: PathBuf,
    engine: Option<Engine>,
    rev: std::cell::Cell<u64>,
}

impl HostEngine {
    fn new() -> HostEngine {
        HostEngine::with_script(SCRIPT)
    }

    /// 偽の子の台本を指定して作る
    fn with_script(script_text: &str) -> HostEngine {
        let dir = temp_dir("lm-host");
        // 子は低い整合性で動く（ADR 0072）。旗のファイルを書けるよう、このフォルダだけ低いラベルにする
        #[cfg(windows)]
        {
            let ok = std::process::Command::new("icacls")
                .arg(&dir)
                .args(["/setintegritylevel", "(OI)(CI)L"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            assert!(ok, "icacls /setintegritylevel failed");
        }
        let script = dir.join("script.txt");
        std::fs::write(&script, script_text).unwrap();
        let user = dir.join("user");
        std::fs::create_dir_all(&user).unwrap();
        let paths = StackPaths {
            dict_dir: fixture_dict_dir().to_path_buf(),
            user_dir: Some(user.clone()),
            vectors: None,
            lm_model: Some(script.clone()),
            lm_host: Some(fake_host()),
            no_packs: true,
        };
        let budget = LmBudget {
            // 偽の子の起動と応答を待てる長さ（製品の値ではない）
            base: ikari_testtime::ms(2000),
            max: ikari_testtime::ms(2000),
            ..LmBudget::default()
        };
        let stack = build_stack(&paths, &stack_options(300, budget)).unwrap();
        let engine = Engine::from_parts(EngineParts {
            stack,
            rebuild: None,
            data_dir: Some(user),
            lm_requested: true,
            lm_budget: budget,
            convert_wait: ikari_testtime::ms(2000),
            lock: None,
        });
        begin(&engine, 1, false);
        HostEngine {
            dir,
            script,
            engine: Some(engine),
            rev: std::cell::Cell::new(0),
        }
    }

    fn e(&self) -> &Engine {
        self.engine.as_ref().unwrap()
    }

    fn flag(&self, suffix: &str) -> PathBuf {
        let mut path = self.script.clone().into_os_string();
        path.push(suffix);
        PathBuf::from(path)
    }

    fn starts(&self) -> usize {
        std::fs::read_to_string(self.flag(".starts"))
            .unwrap_or_default()
            .lines()
            .count()
    }

    fn inference_count(trace_dir: &Path, event: &str) -> usize {
        let path = trace_dir.join(event);
        match std::fs::read_to_string(&path) {
            Ok(text) => text.bytes().filter(|b| *b == b'\n').count(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => panic!("{} を読めません: {error}", path.display()),
        }
    }

    /// 次の文脈の番号と文脈。左文脈は毎回変える（前の結果を使わせない）
    fn next_context(&self) -> (u64, pb::SurroundingText) {
        let rev = self.rev.get() + 1;
        self.rev.set(rev);
        let context = pb::SurroundingText {
            left: format!("文脈その{rev}"),
            right: String::new(),
            available: true,
        };
        (rev, context)
    }

    /// 先行計算を頼む（待たない）
    fn prefetch(&self, rev: u64, context: &pb::SurroundingText) {
        send(
            self.e(),
            envelope::Body::Prefetch(pb::Prefetch {
                session_id: 1,
                reading: READING.into(),
                context: Some(context.clone()),
                context_rev: rev,
            }),
        );
    }

    /// 先行計算のあとに変換する。左文脈は毎回変える（前の結果を使わせない）
    fn convert(&self) -> pb::ConvertReply {
        let (rev, context) = self.next_context();
        self.prefetch(rev, &context);
        self.convert_now(rev, context)
    }

    /// 先行計算を頼まずに変換する（同じ文脈の先行計算が計算中なら、`convert_wait` まで待つ）
    fn convert_now(&self, rev: u64, context: pb::SurroundingText) -> pb::ConvertReply {
        let envelope::Body::ConvertReply(reply) = send(
            self.e(),
            envelope::Body::Convert(pb::Convert {
                session_id: 1,
                reading: READING.into(),
                context: Some(context),
                context_rev: rev,
                page_size: 9,
            }),
        ) else {
            panic!("convert must succeed");
        };
        reply
    }

    fn top(&self) -> String {
        let reply = self.convert();
        surfaces(&reply)[0].to_owned()
    }

    fn log(&self) -> String {
        self.e().inner.log.flush_for_test();
        std::fs::read_to_string(self.dir.join("user").join("logs").join("engine.log"))
            .unwrap_or_default()
    }
}

impl Drop for HostEngine {
    fn drop(&mut self) {
        self.engine = None;
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn the_engine_survives_the_lm_child_and_gets_the_lm_back() {
    let h = HostEngine::new();
    // LM が効く（子が採点した）
    wait_until(|| h.top() == BOOSTED);
    assert_eq!(h.e().inner.lm_state(), LmState::LmReady);
    let before = h.convert();
    let starts = h.starts();
    assert!(starts >= 1);

    // 子を落とす（次の要求で、答えずに終わる）
    std::fs::write(h.flag(".die"), b"").unwrap();
    let after = h.convert();
    // エンジンは返事を返す。候補は辞書から作ったもので、LM なしの順位。数は変わらない（LM は候補を足さない）
    assert_eq!(surfaces(&after)[0], BASE_TOP, "{:?}", surfaces(&after));
    assert!(!after.lm_applied);
    let mut a = surfaces(&after);
    let mut b = surfaces(&before);
    a.sort_unstable();
    b.sort_unstable();
    assert_eq!(a, b);
    // 落ち続ける間も、返し続ける
    for _ in 0..4 {
        assert_eq!(h.top(), BASE_TOP);
    }

    // 子が直ると（エンジンを起動し直せば）、LM が戻る。ここでは諦めたあとなので、エンジンを作り直す
    drop(h);
    let h = HostEngine::new();
    wait_until(|| h.top() == BOOSTED);
}

#[test]
fn a_single_crash_is_followed_by_a_restart_without_restarting_the_engine() {
    let h = HostEngine::new();
    wait_until(|| h.top() == BOOSTED);
    let starts = h.starts();
    std::fs::write(h.flag(".die"), b"").unwrap();
    assert_eq!(h.top(), BASE_TOP);
    std::fs::remove_file(h.flag(".die")).unwrap();
    // 次の要求で子を起動し直し、LM が戻る
    wait_until(|| h.top() == BOOSTED);
    assert!(h.starts() > starts);
    assert_eq!(h.e().inner.lm_state(), LmState::LmReady);
}

/// #429: 最後の読みの先行計算が来たら、走っている古い先行計算の印が立ち（ADR 0074 §5）、子の推論は次の区切りで止まる。
/// 最新の変換が返るまでに、最新の推論だけが完了し、古い推論は未完了（取り消し）であることを確かめる。
/// 時間（`SLOW_INFER_MS`）に頼らず、一時ディレクトリの合図のファイル（`inference-hold`）で偽の子を止めて進める（#296）。
#[test]
fn a_running_stale_inference_does_not_delay_the_newest_reading() {
    if std::env::var_os(FAKE_TRACE_CHILD_ENV).is_some() {
        let trace_dir = PathBuf::from(
            std::env::var_os(FAKE_TRACE_DIR_ENV).expect("fake LM trace directory was not passed"),
        );
        run_stale_inference_check(&trace_dir);
        return;
    }

    let trace_dir = temp_dir("lm-host-trace");
    #[cfg(windows)]
    {
        let ok = std::process::Command::new("icacls")
            .arg(&trace_dir)
            .args(["/setintegritylevel", "(OI)(CI)L"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(ok, "icacls /setintegritylevel failed");
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::lm_host::a_running_stale_inference_does_not_delay_the_newest_reading",
            "--nocapture",
        ])
        .env(FAKE_TRACE_DIR_ENV, &trace_dir)
        .env(FAKE_TRACE_CHILD_ENV, "1")
        .output()
        .unwrap();
    let cleanup = std::fs::remove_dir_all(&trace_dir);
    assert!(cleanup.is_ok(), "trace dir cleanup failed: {cleanup:?}");
    assert!(
        output.status.success(),
        "state-based LM host test failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_stale_inference_check(trace_dir: &Path) {
    let h = HostEngine::new();
    let warmup_started = Instant::now();
    loop {
        let warming = h.e().inner.warming.load(Ordering::Acquire);
        let busy =
            lock_mutex(&h.e().inner.cache).any_computing() || h.e().inner.queue.pending() > 0;
        if !warming && !busy && h.top() == BOOSTED {
            let started = HostEngine::inference_count(trace_dir, INFERENCE_STARTED);
            let completed = HostEngine::inference_count(trace_dir, INFERENCE_COMPLETED);
            let cancelled = HostEngine::inference_count(trace_dir, INFERENCE_CANCELLED);
            // 取り消した推論は completed に数えないので、終わった数は completed + cancelled
            if !h.e().inner.warming.load(Ordering::Acquire) && started == completed + cancelled {
                break;
            }
        }
        let elapsed = warmup_started.elapsed();
        assert!(
            elapsed < Duration::from_secs(60),
            "LM warm-up stalled: started={} completed={} warming={warming} elapsed={elapsed:?}\nlog:\n{}",
            HostEngine::inference_count(trace_dir, INFERENCE_STARTED),
            HostEngine::inference_count(trace_dir, INFERENCE_COMPLETED),
            h.log(),
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let baseline_started = HostEngine::inference_count(trace_dir, INFERENCE_STARTED);
    let baseline_completed = HostEngine::inference_count(trace_dir, INFERENCE_COMPLETED);
    let baseline_cancelled = HostEngine::inference_count(trace_dir, INFERENCE_CANCELLED);
    assert_eq!(
        baseline_started,
        baseline_completed + baseline_cancelled,
        "warm-up left an in-flight inference: started={baseline_started} completed={baseline_completed} cancelled={baseline_cancelled}\nlog:\n{}",
        h.log(),
    );

    // 古い読み（文脈）の先行計算。合図のファイルを置いて偽ホストの推論を途中で止め、推論が始まった状態を待つ
    let hold_path = trace_dir.join(INFERENCE_HOLD);
    std::fs::write(&hold_path, b"1").unwrap();
    let (old_rev, old_context) = h.next_context();
    h.prefetch(old_rev, &old_context);
    let old_started_at = Instant::now();
    loop {
        let started = HostEngine::inference_count(trace_dir, INFERENCE_STARTED);
        let completed = HostEngine::inference_count(trace_dir, INFERENCE_COMPLETED);
        if started > baseline_started {
            break;
        }
        let elapsed = old_started_at.elapsed();
        assert!(
            elapsed < Duration::from_secs(60),
            "stale inference did not start: started={started} completed={completed} baseline_started={baseline_started} baseline_completed={baseline_completed} elapsed={elapsed:?}\nlog:\n{}",
            h.log(),
        );
        std::thread::sleep(Duration::from_millis(2));
    }

    // 最後の読みの先行計算で古い推論に取消の印を立て、偽ホストが古い推論を取り消したのを確かめてから合図を外して Convert する
    let (rev, context) = h.next_context();
    h.prefetch(rev, &context);
    let cancel_started_at = Instant::now();
    loop {
        let cancelled = HostEngine::inference_count(trace_dir, INFERENCE_CANCELLED);
        if cancelled > baseline_cancelled {
            break;
        }
        let elapsed = cancel_started_at.elapsed();
        assert!(
            elapsed < Duration::from_secs(60),
            "stale inference was not cancelled: started={} completed={} cancelled={cancelled} baseline_started={baseline_started} baseline_completed={baseline_completed} elapsed={elapsed:?}\nlog:\n{}",
            HostEngine::inference_count(trace_dir, INFERENCE_STARTED),
            HostEngine::inference_count(trace_dir, INFERENCE_COMPLETED),
            h.log(),
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    std::fs::remove_file(&hold_path).unwrap();

    let reply = h.convert_now(rev, context);
    let elapsed = old_started_at.elapsed();
    let started = HostEngine::inference_count(trace_dir, INFERENCE_STARTED);
    let completed = HostEngine::inference_count(trace_dir, INFERENCE_COMPLETED);
    let cancelled = HostEngine::inference_count(trace_dir, INFERENCE_CANCELLED);
    assert!(
        reply.lm_applied,
        "newest reading did not use LM: started={started} completed={completed} cancelled={cancelled} baseline_started={baseline_started} baseline_completed={baseline_completed} elapsed={elapsed:?} report={:?}\nlog:\n{}",
        reply.lm_report,
        h.log(),
    );
    assert_eq!(
        completed,
        baseline_completed + 1,
        "stale inference completed before the newest reply: started={started} completed={completed} cancelled={cancelled} baseline_started={baseline_started} baseline_completed={baseline_completed} elapsed={elapsed:?}\nlog:\n{}",
        h.log(),
    );
}
