//! 別プロセスの LM（ADR 0060 Q4）: 子が落ちる・固まる・壊れた返事をしても、`ZenzRanker` は辞書だけの順位で
//! 返し続け、子は回収され、起動し直され、連続失敗で諦める。偽の子（`ikari-lm-host-fake`。モデルなし）を使う。
//! 時間は、テストが揺れない長さ（`ikari_testtime` で伸ばせる）。製品の既定値とは別。

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use ikari_core::{
    AnalyzedContext, Candidate, CandidateOrigin, Context, DenseGrammar, InMemoryDictionary,
    RankInput, Ranker, UnknownParams,
};
use ikari_lm::host::{
    HostCage, HostHealth, HostLauncher, HostLimits, HostSpec, ModelHandle, host_loader,
};
use ikari_lm::{CancelToken, LmDevice, ZenzConfig, ZenzRanker, cancel};

const FAKE_HOST: &str = env!("CARGO_BIN_EXE_ikari-lm-host-fake");
/// 偽の子の場所の差し替え（ビルドした機械と別の機械、たとえば VM で、このテストを動かすとき）
const FAKE_HOST_ENV: &str = "IKARI_TEST_LM_HOST_FAKE";

fn fake_host() -> PathBuf {
    std::env::var_os(FAKE_HOST_ENV).map_or_else(|| PathBuf::from(FAKE_HOST), PathBuf::from)
}

/// 台本（モデルの代わり）を置いて、照合したモデルと同じく開いたハンドルにする（FC-3）。`missing` なら、開いたあとで
/// パスから消す（unix。Windows は持っている間は消せないので使わない）
fn script_model(path: &Path, script: &str, missing: bool) -> ModelHandle {
    std::fs::write(path, script).unwrap();
    let model = ModelHandle::open(path).unwrap();
    if missing {
        std::fs::remove_file(path).unwrap();
    }
    model
}
/// ranker が結果を待つ上限。偽の子の起動と応答より桁で長い
const RANK_BUDGET_MS: u64 = 3000;
const REQUEST_DEADLINE_MS: u64 = 300;
const READY_DEADLINE_MS: u64 = 3000;
const MAX_FAILURES: u32 = 3;
/// 良い候補（NLL 0）と悪い候補（NLL 20）。基礎コストは悪い候補のほうが良い（LM が効くと逆転する）
const GOOD: &str = "良";
const BAD: &str = "悪";
const BAD_BASE: i32 = 5000;
const GOOD_BASE: i32 = 5200;
const HEALTHY: &str = "nll.良=0\nnll.悪=20\n";

/// 初めて実行する exe は、OS の走査で起動が 1 秒以上かかることがある。最初に 1 回だけ空打ちする
fn warm_up_exe() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = Command::new(fake_host()).status();
    });
}

fn pid_alive(pid: u32) -> bool {
    if cfg!(windows) {
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
    } else {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}

fn kill_pid(pid: u32) {
    let status = if cfg!(windows) {
        Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .status()
    } else {
        Command::new("kill").args(["-9", &pid.to_string()]).status()
    };
    assert!(status.is_ok_and(|s| s.success()), "could not kill {pid}");
}

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

struct Harness {
    _dir: tempfile::TempDir,
    script: PathBuf,
    ranker: ZenzRanker,
    health: Arc<HostHealth>,
    readings: Cell<u32>,
}

impl Harness {
    fn new(script: &str) -> Harness {
        Harness::with(script, true, ZenzConfig::default())
    }

    fn with(script: &str, use_gpu: bool, base: ZenzConfig) -> Harness {
        Harness::caged(script, use_gpu, base, HostCage::Standard, false)
    }

    /// `missing`: 台本（モデルの代わり）のファイルを置かない。段 2 は、起動の前にモデルへ読み取りの許可を足すので、
    /// 張れない（権限に関わらず失敗する。管理者で動く CI でも同じ）。段 1 の偽の子は、台本なしの既定で起動する
    fn caged(
        script: &str,
        use_gpu: bool,
        base: ZenzConfig,
        cage: HostCage,
        missing: bool,
    ) -> Harness {
        warm_up_exe();
        let dir = tempfile::tempdir().unwrap();
        // 子は低い整合性で動く（ADR 0072）。旗のファイル（`.starts`）を書けるよう、このフォルダだけ低いラベルにする
        #[cfg(windows)]
        {
            let ok = std::process::Command::new("icacls")
                .arg(dir.path())
                .args(["/setintegritylevel", "(OI)(CI)L"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            assert!(ok, "icacls /setintegritylevel failed");
        }
        let path = dir.path().join("script.txt");
        let model = script_model(&path, script, missing);
        let (loader, health) = host_loader(HostSpec {
            program: fake_host(),
            model,
            use_gpu,
            limits: HostLimits {
                ready_deadline: ikari_testtime::ms(READY_DEADLINE_MS),
                request_deadline: ikari_testtime::ms(REQUEST_DEADLINE_MS),
                max_failures: MAX_FAILURES,
            },
            cage,
            launcher: HostLauncher::Direct,
        });
        let cfg = ZenzConfig {
            budget: ikari_testtime::ms(RANK_BUDGET_MS),
            load_retry: Duration::ZERO,
            ..base
        };
        Harness {
            _dir: dir,
            script: path,
            ranker: ZenzRanker::new(loader, cfg),
            health,
            readings: Cell::new(0),
        }
    }

    /// 次に起動する子の台本（台本そのものはハンドルで持っているので、横の `.next` に書く。偽の子の説明）
    fn set_script(&self, script: &str) {
        let mut path = self.script.clone().into_os_string();
        path.push(".next");
        std::fs::write(path, script).unwrap();
    }

    /// 偽の子が起動のたびに書いた `--gpu` の値
    fn starts(&self) -> Vec<String> {
        let mut path = self.script.clone().into_os_string();
        path.push(".starts");
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// 1 位の表記。辞書だけの順位なら BAD、LM が効けば GOOD。読みは毎回変える（遅れて届いた結果を使わせない）
    fn first(&self) -> String {
        let n = self.readings.get() + 1;
        self.readings.set(n);
        rank_once(&self.ranker, &format!("よみ{n}"))
    }

    fn child(&self) -> u32 {
        self.health.child_id().expect("a child is running")
    }
}

/// `reading` を 1 回並べ替えて、1 位の表記を返す（[`Harness::first`] の中身。別のスレッドからも呼べる）
fn rank_once(ranker: &ZenzRanker, reading: &str) -> String {
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
    ranker.rank(&input, &mut cands);
    // LM は並べ替えるだけ（候補を足さない・消さない）
    assert_eq!(cands.len(), 2);
    cands.sort_by_key(|c| c.score);
    cands[0].surface.clone()
}

#[test]
fn scores_come_from_the_child_and_reorder_the_candidates() {
    let h = Harness::new(HEALTHY);
    assert_eq!(h.first(), GOOD);
    assert_eq!(h.first(), GOOD);
    let stats = h.ranker.stats();
    assert_eq!((stats.loads, stats.load_failures), (1, 0));
    assert!(stats.loaded);
    // 装置は子が知らせたもの（`--gpu on` の偽の子は GPU と言う）
    assert_eq!(h.ranker.device(), LmDevice::Gpu);
    assert_eq!(h.health.spawns(), 1);
    assert_eq!(h.health.consecutive_failures(), 0);
    assert_eq!(h.starts(), ["on"]);
}

#[test]
fn gpu_off_is_passed_to_the_child() {
    let h = Harness::with(HEALTHY, false, ZenzConfig::default());
    assert_eq!(h.first(), GOOD);
    assert_eq!(h.ranker.device(), LmDevice::Cpu);
    assert_eq!(h.starts(), ["off"]);
}

#[test]
fn a_killed_child_leaves_the_dictionary_order_and_is_restarted() {
    let h = Harness::new(HEALTHY);
    assert_eq!(h.first(), GOOD);
    let old = h.child();
    kill_pid(old);
    // 子がいない間の 1 回は、辞書だけの順位（LM なし）
    assert_eq!(h.first(), BAD);
    assert!(wait_until(|| !h.ranker.stats().loaded));
    assert_eq!(h.health.consecutive_failures(), 1);
    // 次の要求で起動し直して、LM が戻る。成功したので数は 0 に戻る
    assert_eq!(h.first(), GOOD);
    assert_ne!(h.child(), old);
    assert_eq!(h.health.spawns(), 2);
    assert_eq!(h.health.consecutive_failures(), 0);
    assert!(!pid_alive(old), "the killed child must be reaped");
}

#[test]
fn a_hung_child_is_killed_at_the_deadline() {
    let h = Harness::new("hang_after=0\n");
    let began = Instant::now();
    assert_eq!(h.first(), BAD);
    // ranker は自分の待ち（変換の締め切り）で戻る。子は、要求の締め切りで kill されて回収される
    assert!(began.elapsed() < ikari_testtime::ms(RANK_BUDGET_MS + 2000));
    assert!(wait_until(|| h.health.consecutive_failures() == 1));
    assert_eq!(h.health.child_id(), None);
    assert_eq!(h.starts().len(), 1);
    // 直った子（台本を替える）なら、次の要求で LM が戻る
    h.set_script(HEALTHY);
    assert!(wait_until(|| h.first() == GOOD));
    assert_eq!(h.health.consecutive_failures(), 0);
}

#[test]
fn consecutive_failures_give_up_and_stop_spawning() {
    // GPU を使わない設定（GPU を使う設定では、諦める前に CPU の子で試し直す。下の別のテスト）
    let h = Harness::with("exit_after=0\n", false, ZenzConfig::default());
    for n in 1..=MAX_FAILURES {
        assert_eq!(h.first(), BAD);
        assert!(wait_until(|| h.health.consecutive_failures() == n));
    }
    assert!(h.health.gave_up());
    assert_eq!(h.health.spawns(), u64::from(MAX_FAILURES));
    // 諦めたあとは、子を起動せず、辞書だけの順位で戻り続ける（直った子に替えても、起動し直すまで試さない）
    h.set_script(HEALTHY);
    for _ in 0..3 {
        assert_eq!(h.first(), BAD);
    }
    assert_eq!(h.health.spawns(), u64::from(MAX_FAILURES));
    assert_eq!(h.starts().len(), MAX_FAILURES as usize);
    let stats = h.ranker.stats();
    assert!(!stats.loaded);
    assert!(stats.load_failures > 0);
}

#[test]
fn a_new_loader_forgets_that_the_old_one_gave_up() {
    // 断念はメモリだけで覚える。エンジンを起動し直す（loader を作り直す）と、もう一度試す
    // GPU を使わない設定（GPU を使う設定では、諦める前に CPU の子で試し直す。下の別のテスト）
    let h = Harness::with("exit_after=0\n", false, ZenzConfig::default());
    for n in 1..=MAX_FAILURES {
        assert_eq!(h.first(), BAD);
        assert!(wait_until(|| h.health.consecutive_failures() == n));
    }
    assert!(h.health.gave_up());
    let again = Harness::new(HEALTHY);
    assert_eq!(again.first(), GOOD);
}

#[test]
fn a_success_between_failures_resets_the_count() {
    // 1 回答えては死ぬ子: 成功、失敗、成功、失敗…と続き、連続 3 回にはならない
    let h = Harness::new(&format!("{HEALTHY}exit_after=1\n"));
    let mut good = 0;
    for _ in 0..(MAX_FAILURES * 2 + 2) {
        if h.first() == GOOD {
            good += 1;
        }
        assert!(h.health.consecutive_failures() <= 1);
    }
    assert!(good >= MAX_FAILURES);
    assert!(!h.health.gave_up());
}

#[test]
fn a_child_that_never_gets_ready_is_killed_and_counted() {
    let h = Harness::new("ready=never\n");
    assert_eq!(h.first(), BAD);
    assert!(wait_until(|| h.health.consecutive_failures() == 1));
    assert_eq!(h.health.child_id(), None);
    // ワーカーが数えるのは、loader が戻ったあと（子の失敗の数より少し遅れる）
    assert!(wait_until(|| h.ranker.stats().load_failures == 1));
    assert_eq!(h.ranker.stats().loads, 0);
}

#[test]
fn a_version_mismatch_or_a_load_failure_is_counted_as_a_failure() {
    // 版 1（取り消しを知らない子。ADR 0074）も使わない
    for script in ["version=999\n", "version=1\n", "load=fail\n"] {
        let h = Harness::new(script);
        assert_eq!(h.first(), BAD, "{script}");
        assert!(
            wait_until(|| h.health.consecutive_failures() == 1),
            "{script}"
        );
        assert!(!h.ranker.stats().loaded);
        assert_eq!(h.health.child_id(), None);
    }
}

#[test]
fn a_missing_program_is_a_failure_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let model = script_model(&dir.path().join("model"), "", false);
    let (loader, health) = host_loader(HostSpec {
        program: dir.path().join("no-such-exe"),
        model,
        use_gpu: true,
        limits: HostLimits::default(),
        cage: HostCage::Standard,
        launcher: HostLauncher::Direct,
    });
    assert!(loader().is_err());
    assert_eq!(health.consecutive_failures(), 1);
    assert_eq!(health.spawns(), 1);
}

#[test]
fn an_inference_error_from_a_live_child_is_not_counted() {
    let h = Harness::new("fail_infer=1\n");
    for _ in 0..(MAX_FAILURES + 2) {
        assert_eq!(h.first(), BAD);
    }
    // 子は生きたまま（起動は 1 回）。諦めない
    assert_eq!(h.health.spawns(), 1);
    assert_eq!(h.health.consecutive_failures(), 0);
    assert!(pid_alive(h.child()));
    assert!(wait_until(
        || h.ranker.stats().inference_errors >= u64::from(MAX_FAILURES)
    ));
}

#[test]
fn a_garbage_reply_kills_the_child() {
    let h = Harness::new("garbage_after=0\n");
    assert_eq!(h.first(), BAD);
    assert!(wait_until(|| h.health.consecutive_failures() == 1));
    assert_eq!(h.health.child_id(), None);
}

#[test]
fn after_a_gpu_load_fallback_the_child_is_started_without_gpu() {
    let h = Harness::new(&format!("{HEALTHY}gpu=fallback\n"));
    assert_eq!(h.first(), GOOD);
    assert_eq!(h.ranker.device(), LmDevice::Cpu);
    kill_pid(h.child());
    assert_eq!(h.first(), BAD);
    assert_eq!(h.first(), GOOD);
    // 2 回目の起動は GPU を試さない（ADR 0059 §2.4 の「この起動の間は CPU」と同じ）
    assert_eq!(h.starts(), ["on", "cpu"]);
    assert!(h.health.gpu_disabled());
}

#[test]
fn dropping_the_ranker_kills_the_child() {
    let h = Harness::new(HEALTHY);
    assert_eq!(h.first(), GOOD);
    let pid = h.child();
    assert!(pid_alive(pid));
    drop(h);
    assert!(wait_until(|| !pid_alive(pid)));
}

#[test]
fn idle_unload_ends_the_child_and_the_next_request_starts_a_new_one() {
    let cfg = ZenzConfig {
        idle_unload: ikari_testtime::ms(200),
        ..ZenzConfig::default()
    };
    let h = Harness::with(HEALTHY, true, cfg);
    assert_eq!(h.first(), GOOD);
    let pid = h.child();
    assert!(wait_until(|| h.ranker.stats().unloads == 1));
    assert!(wait_until(|| !pid_alive(pid)));
    assert_eq!(h.health.child_id(), None);
    assert_eq!(h.first(), GOOD);
    assert_eq!(h.health.spawns(), 2);
    // アイドルの解放は失敗ではない
    assert_eq!(h.health.consecutive_failures(), 0);
}

#[test]
fn a_gpu_child_that_keeps_crashing_is_replaced_by_a_cpu_child() {
    // GPU の子は最初の要求で落ちる。CPU の子は健康
    let h = Harness::new(&format!("{HEALTHY}die_on_gpu=1\n"));
    for n in 1..=MAX_FAILURES {
        assert_eq!(h.first(), BAD);
        // 上限に達した回で、諦めずに CPU だけに切り替わり、数は 0 に戻る
        let expected = if n == MAX_FAILURES { 0 } else { n };
        assert!(wait_until(
            || h.health.consecutive_failures() == expected && h.health.child_id().is_none()
        ));
    }
    assert!(h.health.gpu_disabled());
    assert!(!h.health.gave_up());
    // 次の要求から CPU の子で LM が戻る
    assert!(wait_until(|| h.first() == GOOD));
    assert_eq!(h.ranker.device(), LmDevice::Cpu);
    let mut expected = vec!["on"; MAX_FAILURES as usize];
    expected.push("cpu");
    assert_eq!(h.starts(), expected);
    assert!(!h.health.gave_up());
}

#[test]
fn when_the_cpu_child_also_keeps_crashing_the_host_gives_up() {
    // GPU でも CPU でも、最初の要求で落ちる
    let h = Harness::new("exit_after=0\n");
    for _ in 0..(MAX_FAILURES * 2) {
        assert!(!h.health.gave_up());
        let spawns = h.health.spawns();
        assert_eq!(h.first(), BAD);
        assert!(wait_until(
            || h.health.spawns() == spawns + 1 && h.health.child_id().is_none()
        ));
    }
    assert!(wait_until(|| h.health.gave_up()));
    assert!(h.health.gpu_disabled());
    // GPU で上限まで、そのあと CPU で上限まで。以後は起動しない
    let mut expected = vec!["on"; MAX_FAILURES as usize];
    expected.extend(vec!["cpu"; MAX_FAILURES as usize]);
    assert_eq!(h.starts(), expected);
    assert_eq!(h.first(), BAD);
    assert_eq!(h.health.spawns(), u64::from(MAX_FAILURES * 2));
}

#[test]
fn no_cpu_retry_when_gpu_is_off_or_was_never_used() {
    // 設定で GPU を使わない: 上限で諦める（CPU で試し直しても同じ）
    let off = Harness::with("exit_after=0\n", false, ZenzConfig::default());
    // GPU を使う設定だが、子が CPU に載せたと知らせた（GPU のない機械）
    let unused = Harness::new("gpu=unused\nexit_after=0\n");
    for h in [&off, &unused] {
        for n in 1..=MAX_FAILURES {
            assert_eq!(h.first(), BAD);
            assert!(wait_until(|| h.health.consecutive_failures() == n));
        }
        assert!(h.health.gave_up());
        assert!(!h.health.gpu_disabled());
        assert_eq!(h.health.spawns(), u64::from(MAX_FAILURES));
    }
}

/// 取り消し（ADR 0074 §3.1）は失敗に数えず、子も替えない。取り消した要求の返事（子が最後まで採点した `Scores`、
/// または子が気づいた `Failed(Cancelled)`）は読み捨て、次の要求は同じ子で LM が効く
#[test]
fn a_cancelled_request_is_not_a_failure_and_its_late_reply_is_dropped() {
    // 子の推論（ms）。古い要求の残り + 次の要求が `REQUEST_DEADLINE_MS` に収まる長さ
    const DELAY_MS: u64 = 100;
    // 古い要求を送ってから取り消すまで（ms）。子が採点を始めるのに足りる長さ
    const CANCEL_AFTER_MS: u64 = 30;
    // 1 = 子は止まらず最後まで採点する。4 = 子は区切りで気づいて `Failed(Cancelled)` を返す
    // 子の推論の長さも、待ちと同じ倍率で伸ばす（伸ばさないと、倍率 5 で取り消す前に子が答え終わる）
    let delay_ms = ikari_testtime::ms(DELAY_MS).as_millis();
    for steps in [1, 4] {
        let h = Harness::new(&format!(
            "{HEALTHY}delay_ms={delay_ms}\ndelay_steps={steps}\n"
        ));
        assert_eq!(h.first(), GOOD);
        let child = h.child();
        let token = CancelToken::new();
        thread::scope(|s| {
            let ranker = &h.ranker;
            let t = token.clone();
            let old = s.spawn(move || {
                let _scope = cancel::scope(t);
                rank_once(ranker, "古い")
            });
            thread::sleep(ikari_testtime::ms(CANCEL_AFTER_MS));
            token.cancel();
            // 取り消された要求は LM なし（辞書の順位）で戻る
            assert_eq!(old.join().unwrap(), BAD, "steps={steps}");
        });
        assert_eq!(h.first(), GOOD, "steps={steps}");
        assert_eq!(h.child(), child, "steps={steps}");
        assert_eq!(h.health.consecutive_failures(), 0, "steps={steps}");
        let st = h.ranker.stats();
        assert_eq!(
            (st.cancelled, st.inference_errors, st.timeouts),
            (1, 0, 0),
            "steps={steps} {st:?}"
        );
    }
}

/// 段 2 だけを求めたとき: 張れる OS（Windows）では段 2 の子が採点し、暖機も同じ子（同じ檻）で済む。張れない OS では
/// 子を起動せず、辞書だけの順位で返す
#[test]
fn a_required_appcontainer_runs_the_child_in_it_or_starts_none() {
    let h = Harness::caged(
        HEALTHY,
        true,
        ZenzConfig::default(),
        HostCage::RequireAppContainer,
        false,
    );
    h.ranker.warm_up();
    if cfg!(windows) {
        assert!(
            h.health.log_note().contains("box=appcontainer"),
            "{}",
            h.health.log_note()
        );
        assert_eq!(h.health.spawns(), 1);
        assert_eq!(h.first(), GOOD);
        // 暖機の子のまま（起動し直していない）
        assert_eq!(h.health.spawns(), 1);
    } else {
        assert_eq!(h.first(), BAD);
        assert!(h.health.consecutive_failures() >= 1);
        assert_eq!(h.health.child_id(), None);
        assert!(h.starts().is_empty());
        assert!(
            h.health.log_note().contains("box=unknown"),
            "{}",
            h.health.log_note()
        );
    }
}

/// 段 2 だけを求めて、段 2 を張れない（モデルに許可を足せない。Windows でない）とき: 段 1 に落とさず、子を起動しない
/// （LM なし）
#[test]
#[cfg_attr(
    windows,
    ignore = "FC-3: 持っている間はモデルを消せないので、段 2 を張れない状態をほかの形で作る（D-3。VM で確かめる）"
)]
fn a_required_appcontainer_that_cannot_be_set_up_starts_no_child() {
    let h = Harness::caged(
        HEALTHY,
        true,
        ZenzConfig::default(),
        HostCage::RequireAppContainer,
        true,
    );
    assert_eq!(h.first(), BAD);
    assert!(h.health.consecutive_failures() >= 1);
    assert_eq!(h.health.child_id(), None);
    // 段 1 の子も起動していない（起動すれば、偽の子が `.starts` に書く）
    assert!(h.starts().is_empty(), "{:?}", h.starts());
    assert!(!h.health.log_note().contains("box=low"));
}

/// 段 2 を優先して、段 2 を張れないとき: 段 1 の子が起動して採点する（台本なしの偽の子。並びは変わらない）
#[test]
#[cfg_attr(
    windows,
    ignore = "FC-3: 持っている間はモデルを消せないので、段 2 を張れない状態をほかの形で作る（D-3。VM で確かめる）"
)]
fn a_preferred_appcontainer_falls_back_to_the_standard_cage() {
    let h = Harness::caged(
        HEALTHY,
        true,
        ZenzConfig::default(),
        HostCage::PreferAppContainer,
        true,
    );
    h.first();
    assert!(h.ranker.stats().loaded);
    assert_eq!(h.health.consecutive_failures(), 0);
    assert!(
        !h.health.log_note().contains("box=appcontainer"),
        "{}",
        h.health.log_note()
    );
    assert_eq!(h.starts(), ["on"]);
}

/// 段 2 を優先して、張れるとき（Windows）は段 2。張れない OS では段 1
#[test]
fn a_preferred_appcontainer_is_used_when_it_can_be_set_up() {
    let h = Harness::caged(
        HEALTHY,
        true,
        ZenzConfig::default(),
        HostCage::PreferAppContainer,
        false,
    );
    assert_eq!(h.first(), GOOD);
    let note = h.health.log_note();
    if cfg!(windows) {
        assert!(note.contains("box=appcontainer"), "{note}");
    } else {
        assert!(!note.contains("box=appcontainer"), "{note}");
    }
}
