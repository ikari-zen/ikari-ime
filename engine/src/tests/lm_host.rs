//! 別プロセスの LM（ADR 0060 Q4）をエンジンごと通す。偽の子（`ikari-lm-host-fake`。モデルなし）を使う。
//! 子を落としても、エンジンは生きていて、辞書から作った候補（LM なしの順位）を返し続け、子を起動し直すと LM が戻る。

use ikari_stack::{StackPaths, build_stack};

use super::*;

const FAKE_EXE_STEM: &str = "ikari-lm-host-fake";
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
            user_dir: Some(user),
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
            data_dir: None,
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

/// 偽の子の採点 1 回の時間（ms）。#429 の再現用。打鍵の間隔より長い推論（遅い機械）に見立てる
const SLOW_INFER_MS: u64 = 300;
/// 古い先行計算を頼んでから、最後の読みの先行計算を頼むまで（ms）。古い推論が走り出すのに足りる長さ
const SUPERSEDE_AFTER_MS: u64 = 30;
/// 偽の子の採点 1 回を分ける区切り（decode に見立てる。ADR 0074 §4 の段階 1 は区切りの間で止める）
const SLOW_INFER_STEPS: u64 = 4;

/// #429: 最後の読みの先行計算が来たら、走っている古い先行計算の印が立ち（ADR 0074 §5）、子の推論は次の区切りで止まる。
/// 最後の読みは「古い推論の区切り 1 つ分 + 自分の推論」で LM が付く
#[test]
fn a_running_stale_inference_does_not_delay_the_newest_reading() {
    let h = HostEngine::with_script(&format!(
        "{SCRIPT}delay_ms={SLOW_INFER_MS}\ndelay_steps={SLOW_INFER_STEPS}\n"
    ));
    // 子を起動して暖める（LM が効く）
    wait_until(|| h.top() == BOOSTED);

    // 古い読み（文脈）の先行計算。子が採点を始めるまで少し待つ
    let (old_rev, old_context) = h.next_context();
    h.prefetch(old_rev, &old_context);
    std::thread::sleep(ikari_testtime::ms(SUPERSEDE_AFTER_MS));

    // 最後の読みの先行計算と、すぐの Space（計算中の先行計算を待つ）
    let (rev, context) = h.next_context();
    let started = Instant::now();
    h.prefetch(rev, &context);
    let reply = h.convert_now(rev, context);
    let elapsed = started.elapsed();
    eprintln!(
        "#429 newest reading: {elapsed:?} lm_applied={}",
        reply.lm_applied
    );
    assert!(reply.lm_applied);
    // 段階 1 の見込み: 自分の推論 1 回分 + 古い推論の区切り 1 つ分。余裕は推論の 1/4（プロセス間の往復を含む）。
    // 止まらなければ、古い推論の残りを待って、ほぼ 2 回分になる
    let limit =
        ikari_testtime::ms(SLOW_INFER_MS + SLOW_INFER_MS / SLOW_INFER_STEPS + SLOW_INFER_MS / 4);
    assert!(
        elapsed < limit,
        "{}",
        ikari_testtime::report(elapsed, format_args!("< {limit:?}"))
    );
}
