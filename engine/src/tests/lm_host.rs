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
        let dir = temp_dir("lm-host");
        let script = dir.join("script.txt");
        std::fs::write(&script, SCRIPT).unwrap();
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

    /// 先行計算のあとに変換する。左文脈は毎回変える（前の結果を使わせない）
    fn convert(&self) -> pb::ConvertReply {
        let rev = self.rev.get() + 1;
        self.rev.set(rev);
        let context = pb::SurroundingText {
            left: format!("文脈その{rev}"),
            right: String::new(),
            available: true,
        };
        send(
            self.e(),
            envelope::Body::Prefetch(pb::Prefetch {
                session_id: 1,
                reading: READING.into(),
                context: Some(context.clone()),
                context_rev: rev,
            }),
        );
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
