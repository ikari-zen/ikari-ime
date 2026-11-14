//! Engine::handle のテスト。fixture の辞書（メモリ上）とモックの LM で通す。
//! 実辞書のテストは `#[ignore]` で、`IKARI_DICT_DIR` を指定して明示的に流す（未設定なら失敗する）。

use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;

use ikari_core::user::UserDictionary;
use ikari_core::{
    Candidate, Converter, DenseGrammar, Dictionary, Grammar, InMemoryDictionary, LearningStore,
    RankInput, Ranker, UnknownParams,
};

use ikari_stack::{LmDevice, LmHandle, StackMeta, lm_trace};

use super::*;

use crate::admin::{backup_key, utc_stamp};

/// 既定の接続（macOS の経路）のセッションのキー。表を直接見るテスト用
fn sk(id: u64) -> SessionKey {
    SessionKey::new(ConnectionId::DEFAULT, id)
}

const READING: &str = "ひんし";

const BOOSTED: &str = "瀕死";

const BASE_TOP: &str = "品詞";

/// 「瀕死」を最上位にするモックの LM。ZenzRanker と同じく、締め切りまでしか待たない。
struct MockLm {
    delay: Duration,
    boost: AtomicBool,
    loaded: AtomicBool,
    calls: AtomicU64,
    timeouts: AtomicU64,
    warm_ups: AtomicU64,
    warm_thread: Mutex<Option<std::thread::ThreadId>>,
    /// 最後の `rank` が受けた文脈（左文脈、借りたか）
    last_context: Mutex<Option<String>>,
    /// 最後の `rank` が受けた、左文脈のうち履歴から足した字数（ADR 0049）
    last_carried: Mutex<Option<usize>>,
}

impl MockLm {
    fn new(delay: Duration) -> Arc<MockLm> {
        Arc::new(MockLm {
            delay,
            boost: AtomicBool::new(true),
            loaded: AtomicBool::new(false),
            calls: AtomicU64::new(0),
            timeouts: AtomicU64::new(0),
            warm_ups: AtomicU64::new(0),
            warm_thread: Mutex::new(None),
            last_context: Mutex::new(None),
            last_carried: Mutex::new(None),
        })
    }
}

impl Ranker for MockLm {
    fn rank(&self, input: &RankInput<'_>, cands: &mut [Candidate]) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        *self.last_context.lock().unwrap() = Some(input.context.left_text.clone());
        *self.last_carried.lock().unwrap() = Some(input.context.left_carried_chars);
        let now = Instant::now();
        match input.deadline {
            Some(deadline) if now + self.delay <= deadline => {
                lm_trace::record_call(LmDevice::Cpu);
                std::thread::sleep(self.delay);
                lm_trace::record_reply(self.delay, self.delay);
                self.loaded.store(true, Ordering::Release);
                if self.boost.load(Ordering::Relaxed) {
                    for c in cands.iter_mut().filter(|c| c.surface == BOOSTED) {
                        c.score -= 1000;
                    }
                }
            }
            _ => {
                if input.deadline.is_some() {
                    lm_trace::record_call(LmDevice::Cpu);
                    lm_trace::record_timeout(Duration::ZERO);
                }
                self.timeouts.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn name(&self) -> &str {
        "mock-lm"
    }
}

struct MockHandle(Arc<MockLm>);

impl LmHandle for MockHandle {
    fn warm_up(&self) {
        self.0.warm_ups.fetch_add(1, Ordering::Relaxed);
        *self.0.warm_thread.lock().unwrap() = Some(std::thread::current().id());
        self.0.loaded.store(true, Ordering::Release);
    }

    fn stats(&self) -> LmStats {
        LmStats {
            calls: self.0.calls.load(Ordering::Relaxed),
            timeouts: self.0.timeouts.load(Ordering::Relaxed),
            ..LmStats::default()
        }
    }

    fn is_loaded(&self) -> bool {
        self.0.loaded.load(Ordering::Acquire)
    }

    fn device(&self) -> LmDevice {
        if self.is_loaded() {
            LmDevice::Cpu
        } else {
            LmDevice::Unknown
        }
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ikari-engine-test-{tag}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn mock_stack(mock: &Arc<MockLm>, learning_path: Option<&Path>) -> Stack {
    let mut g = DenseGrammar::new(
        2,
        2,
        0,
        UnknownParams {
            left_id: 1,
            right_id: 1,
            pos_id: 0,
            cost_per_char: 5000,
        },
    );
    g.add_pos("名詞,普通名詞,一般,*,*,*");
    let mut d = InMemoryDictionary::new(g.grammar_id());
    d.insert(READING, BASE_TOP, 1, 1, 100, 0);
    d.insert(READING, BOOSTED, 1, 1, 200, 0);
    for i in 0..14 {
        d.insert(READING, &format!("品詞{i}"), 1, 1, 300 + i, 0);
    }
    let (dict, grammar): (Arc<dyn Dictionary>, Arc<dyn Grammar>) = (Arc::new(d), Arc::new(g));
    let converter = Converter::builder(dict.clone(), grammar.clone())
        .ranker(mock.clone())
        .options(ConvertOptions {
            page_size: 9,
            chunk_size: 9,
            ..ConvertOptions::default()
        })
        .build()
        .unwrap();
    let learning = learning_path.map(|p| {
        let store = LearningStore::open(p).unwrap();
        store.attach_grammar(grammar.clone());
        Arc::new(store)
    });
    let lm_handle: Arc<dyn LmHandle> = Arc::new(MockHandle(mock.clone()));
    Stack {
        converter: Arc::new(converter),
        learning,
        lm: Some(lm_handle),
        meta: StackMeta {
            dict_label: "mock".into(),
            ..StackMeta::default()
        },
        vectors: None,
        profile: None,
        dict,
        grammar,
        lm_ranker: None,
        postal: None,
    }
}

/// 時間に依存するテスト用の、倍率（`IKARI_TEST_TIME_SCALE`）を掛けた LM の締め切り。
/// 倍率 1 では `LmBudget::default()` と同じ
fn test_budget() -> LmBudget {
    let b = LmBudget::default();
    LmBudget {
        base: ikari_testtime::scaled(b.base),
        per_char: ikari_testtime::scaled(b.per_char),
        max: ikari_testtime::scaled(b.max),
    }
}

/// 倍率を掛けた、Space からの待ち時間。倍率 1 では `DEFAULT_CONVERT_WAIT`
fn test_wait() -> Duration {
    ikari_testtime::scaled(DEFAULT_CONVERT_WAIT)
}

fn engine_with(mock: &Arc<MockLm>, learning_path: Option<&Path>) -> Engine {
    Engine::from_parts(EngineParts {
        stack: mock_stack(mock, learning_path),
        rebuild: None,
        data_dir: None,
        lm_requested: true,
        lm_budget: test_budget(),
        convert_wait: test_wait(),
        lock: None,
    })
}

/// `delay_ms` は倍率を掛けて使う
fn engine(delay_ms: u64) -> (Engine, Arc<MockLm>) {
    let mock = MockLm::new(ikari_testtime::ms(delay_ms));
    (engine_with(&mock, None), mock)
}

fn send_raw(e: &Engine, schema: u32, id: u64, body: envelope::Body) -> Envelope {
    let req = Envelope {
        schema_version: schema,
        request_id: id,
        body: Some(body),
    }
    .encode_to_vec();
    let reply = Envelope::decode(e.handle(&req).as_slice()).expect("reply must decode");
    assert_eq!(reply.request_id, id, "request_id is echoed");
    assert_eq!(reply.schema_version, SCHEMA_VERSION);
    reply
}

/// 文節の返事の種類を要求として受けたら `BAD_REQUEST`（ADR 0006）
#[test]
fn segment_replies_sent_as_requests_are_bad_requests() {
    let (e, _mock) = engine(0);
    for body in [
        envelope::Body::SegmentLayout(pb::SegmentLayout::default()),
        envelope::Body::SegmentPageReply(pb::SegmentPageReply::default()),
    ] {
        assert_eq!(error_code(&send(&e, body)), ErrorCode::BadRequest);
    }
}

fn send(e: &Engine, body: envelope::Body) -> envelope::Body {
    send_raw(e, SCHEMA_VERSION, 1, body)
        .body
        .expect("reply body")
}

fn error_code(body: &envelope::Body) -> ErrorCode {
    match body {
        envelope::Body::Error(e) => ErrorCode::try_from(e.code).unwrap(),
        other => panic!("expected an error, got {other:?}"),
    }
}

fn begin(e: &Engine, session: u64, secure: bool) {
    let r = send(
        e,
        envelope::Body::BeginSession(pb::BeginSession {
            session_id: session,
            client_bundle_id: "test".into(),
            secure,
        }),
    );
    assert!(matches!(r, envelope::Body::Ack(_)), "{r:?}");
}

fn prefetch(e: &Engine, session: u64, reading: &str, rev: u64) {
    let r = send(
        e,
        envelope::Body::Prefetch(pb::Prefetch {
            session_id: session,
            reading: reading.into(),
            context: None,
            context_rev: rev,
        }),
    );
    assert!(matches!(r, envelope::Body::Ack(_)), "{r:?}");
}

fn convert_req(session: u64, reading: &str, rev: u64, page_size: u32) -> envelope::Body {
    envelope::Body::Convert(pb::Convert {
        session_id: session,
        reading: reading.into(),
        context: None,
        context_rev: rev,
        page_size,
    })
}

fn convert(e: &Engine, session: u64, reading: &str, rev: u64) -> pb::ConvertReply {
    match send(e, convert_req(session, reading, rev, 0)) {
        envelope::Body::ConvertReply(r) => r,
        other => panic!("unexpected: {other:?}"),
    }
}

fn surfaces(r: &pb::ConvertReply) -> Vec<&str> {
    r.candidates.iter().map(|c| c.surface.as_str()).collect()
}

fn page(e: &Engine, conversion_id: u64, index: u32) -> envelope::Body {
    send(
        e,
        envelope::Body::Page(pb::PageRequest {
            conversion_id,
            index,
        }),
    )
}

fn wait_until(mut cond: impl FnMut() -> bool) {
    let give_up = Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(Instant::now() < give_up, "condition not met in time");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn hello_reports_version_label_and_lm_state() {
    let (e, _mock) = engine(0);
    let r = send(&e, envelope::Body::Hello(pb::Hello::default()));
    let envelope::Body::HelloReply(h) = r else {
        panic!("{r:?}");
    };
    assert_eq!(h.engine_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(h.dict_label, "mock");
    assert!(
        [LmState::LmLoading as i32, LmState::LmReady as i32].contains(&h.lm),
        "{}",
        h.lm
    );
    // warm_up のあとは READY
    wait_until(|| {
        matches!(
            send(&e, envelope::Body::Hello(pb::Hello::default())),
            envelope::Body::HelloReply(pb::HelloReply { lm, .. }) if lm == LmState::LmReady as i32
        )
    });
}

// ---- 実辞書（`#[ignore]`。IKARI_DICT_DIR が要る。未設定なら失敗する）----

const DICT_DIR_ENV: &str = "IKARI_DICT_DIR";
const MODEL_ENV: &str = "IKARI_ZENZ_MODEL";

/// `src` を指す symlink を `dst` に作る（#[ignore] の実データのテスト用。Windows ではフォルダと
/// ファイルで作り分ける。作るには特権か開発者モードが要る）。
fn symlink_path(src: &Path, dst: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(src, dst).unwrap();
    #[cfg(windows)]
    if src.is_dir() {
        std::os::windows::fs::symlink_dir(src, dst).unwrap();
    } else {
        std::os::windows::fs::symlink_file(src, dst).unwrap();
    }
}

/// `IKARI_DICT_DIR` の辞書を `dict/` に見せるデータのディレクトリを作る。
fn real_data_dir(tag: &str) -> PathBuf {
    let dict = std::env::var_os(DICT_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{DICT_DIR_ENV} is not set (required by #[ignore] tests)"));
    let dir = temp_dir(tag);
    symlink_path(&dict, &UserDir::new(&dir).dict_dir());
    dir
}

// ---- 性能（実辞書と実モデル）----
// cargo test -p ikari-engine --release --features lm-metal -- --ignored --nocapture perf
// IKARI_DICT_DIR（必須）と IKARI_ZENZ_MODEL（LM を使うとき）を指定する。

const PERF_READINGS: [&str; 12] = [
    "ひんし",
    "きょう",
    "ありがとう",
    "よろしくおねがい",
    "かいぎしつ",
    "へんかん",
    "あした",
    "とうきょうと",
    "にほんご",
    "しごと",
    "かんじ",
    "おつかれさま",
];

const KEY_INTERVAL: Duration = Duration::from_millis(120);

const PERF_ROUNDS: usize = 8;

const PERF_P99_ENGINE_MICROS: u64 = 1_000;

const PERF_COLD_CONVERT_MAX: Duration = Duration::from_millis(30);

const PERF_LM_READY_LIMIT: Duration = Duration::from_secs(120);

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let idx = ((sorted.len() as f64 * p).ceil() as usize).clamp(1, sorted.len()) - 1;
    sorted[idx]
}

/// 実辞書（`IKARI_DICT_DIR`）と、あればモデル（`IKARI_ZENZ_MODEL`）でエンジンを開く。
fn open_real(tag: &str) -> (Engine, PathBuf, bool) {
    let dir = real_data_dir(tag);
    let model = std::env::var_os(MODEL_ENV).filter(|v| !v.is_empty());
    let use_lm = cfg!(feature = "lm") && model.is_some();
    if let Some(model) = model.filter(|_| use_lm) {
        std::fs::create_dir_all(dir.join(MODELS_DIR_NAME)).unwrap();
        symlink_path(
            Path::new(&model),
            &dir.join(MODELS_DIR_NAME).join(MODEL_FILE_NAME),
        );
    }
    let e = Engine::open(&EngineConfig {
        data_dir: dir.to_string_lossy().into_owned(),
        enable_lm: use_lm,
        idle_unload_secs: 300,
    })
    .unwrap();
    (e, dir, use_lm)
}

// ---- 管理（ADR 0005 §8）。fixture の辞書をファイルに作り、`Engine::open` で開いて通す ----

/// fixture の辞書（testdata/sudachi-mini）をビルドした `dict/`。テスト全体で 1 回だけ作る。
fn fixture_dict_dir() -> &'static Path {
    static DICT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DICT.get_or_init(|| {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../testdata/sudachi-mini");
        let out = temp_dir("fixture-dict");
        ikari_dict_builder::build(&ikari_dict_builder::BuildConfig {
            lex: vec![src.join("lex.csv")],
            matrix: src.join("matrix.def"),
            out_dir: out.clone(),
            unk_pos: "名詞,普通名詞,一般,*,*,*".to_owned(),
            unk_cost: 10_000,
            label: "engine-admin-test".to_owned(),
        })
        .expect("build the fixture dictionary");
        out
    })
}

/// 本物のファイルを使うエンジン（LM なし）とそのデータのディレクトリ。
struct Real {
    dir: PathBuf,
    engine: Option<Engine>,
}

impl Real {
    fn new() -> Real {
        Real::with_learning_file(None)
    }

    /// 開く前に `learning.tsv` を置く（不正な中身の確認用）
    fn with_learning_file(learning: Option<&[u8]>) -> Real {
        let dir = temp_dir("admin");
        let dict = UserDir::new(&dir).dict_dir();
        std::fs::create_dir_all(&dict).unwrap();
        for entry in std::fs::read_dir(fixture_dict_dir()).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), dict.join(entry.file_name())).unwrap();
        }
        if let Some(bytes) = learning {
            std::fs::write(UserDir::new(&dir).learning_tsv(), bytes).unwrap();
        }
        let mut real = Real { dir, engine: None };
        real.open();
        real
    }

    fn open(&mut self) {
        self.engine = Some(
            crate::lock::retry_while_held(|| {
                Engine::open(&EngineConfig {
                    data_dir: self.dir.to_string_lossy().into_owned(),
                    enable_lm: false,
                    idle_unload_secs: 300,
                })
            })
            .expect("open the engine"),
        );
    }

    fn e(&self) -> &Engine {
        self.engine.as_ref().unwrap()
    }

    fn user(&self) -> UserDir {
        UserDir::new(&self.dir)
    }

    /// 書き込みのスレッドが書き終えるのを待ってから読む（ログは非同期。ADR 0034 §3.1）
    fn log(&self) -> String {
        if let Some(e) = &self.engine {
            e.inner.log.flush_for_test();
        }
        std::fs::read_to_string(self.dir.join("logs").join("engine.log")).unwrap_or_default()
    }
}

impl Drop for Real {
    fn drop(&mut self) {
        self.engine = None;
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 不正な `learning.tsv` は `.bad` に退避して空で起動し、Hello の返事で 1 回だけ伝える（#271）
#[test]
fn broken_learning_file_is_quarantined_and_reported_once() {
    let broken = b"not a learning file\n";
    let real = Real::with_learning_file(Some(broken));
    let tsv = real.user().learning_tsv();
    let bad = tsv.with_file_name("learning.tsv.bad");
    assert_eq!(std::fs::read(&bad).unwrap(), broken);
    assert!(!tsv.exists());
    let flags: Vec<bool> = (0..3)
        .map(
            |_| match send(real.e(), envelope::Body::Hello(pb::Hello::default())) {
                envelope::Body::HelloReply(h) => h.learning_quarantined,
                other => panic!("expected HelloReply, got {other:?}"),
            },
        )
        .collect();
    assert_eq!(flags, [true, false, false]);
}

/// 正常な（またはない）学習ファイルでは、退避も通知もしない
#[test]
fn healthy_learning_file_is_not_reported() {
    let real = Real::new();
    match send(real.e(), envelope::Body::Hello(pb::Hello::default())) {
        envelope::Body::HelloReply(h) => assert!(!h.learning_quarantined),
        other => panic!("expected HelloReply, got {other:?}"),
    }
    assert!(
        !real
            .user()
            .learning_tsv()
            .with_file_name("learning.tsv.bad")
            .exists()
    );
}

fn user_dict_reply(body: envelope::Body) -> pb::UserDictListReply {
    match body {
        envelope::Body::UserDictListReply(r) => r,
        other => panic!("expected UserDictListReply, got {other:?}"),
    }
}

fn dict_list(e: &Engine) -> pb::UserDictListReply {
    user_dict_reply(send(e, envelope::Body::ListUserDict(pb::UserDictList {})))
}

fn dict_add(e: &Engine, reading: &str, surface: &str, pos: &str) -> pb::UserDictListReply {
    user_dict_reply(send(
        e,
        envelope::Body::AddUserDict(pb::UserDictAdd {
            entry: Some(pb::UserDictEntry {
                reading: reading.into(),
                surface: surface.into(),
                pos: pos.into(),
                comment: String::new(),
            }),
        }),
    ))
}

fn dict_remove(e: &Engine, reading: &str, surface: &str) -> pb::UserDictListReply {
    user_dict_reply(send(
        e,
        envelope::Body::RemoveUserDict(pb::UserDictRemove {
            reading: reading.into(),
            surface: surface.into(),
        }),
    ))
}

fn issue_kind_of(issue: &Option<pb::DataIssue>) -> pb::DataIssueKind {
    pb::DataIssueKind::try_from(issue.as_ref().map_or(0, |i| i.kind)).unwrap()
}

/// 先頭から数ページ分の候補の表記（ページ送りをして集める）。
fn all_surfaces(e: &Engine, session: u64, reading: &str) -> Vec<String> {
    let first = convert(e, session, reading, 0);
    let mut out: Vec<String> = first.candidates.iter().map(|c| c.surface.clone()).collect();
    let mut has_next = first.has_next;
    let mut index = 1;
    while has_next && index < 20 {
        let envelope::Body::PageReply(p) = page(e, first.conversion_id, index) else {
            panic!("page reply expected");
        };
        out.extend(p.candidates.iter().map(|c| c.surface.clone()));
        has_next = p.has_next;
        index += 1;
    }
    out
}

fn count_lines(text: &str, needle: &str) -> usize {
    text.lines().filter(|l| l.contains(needle)).count()
}

// ---- 学習の管理 ----

fn commit_first(e: &Engine, session: u64, reading: &str) {
    let conv = convert(e, session, reading, 0);
    let r = send(
        e,
        envelope::Body::Commit(pb::Commit {
            conversion_id: conv.conversion_id,
            candidate_index: 0,
        }),
    );
    assert!(matches!(r, envelope::Body::Ack(_)), "{r:?}");
}

/// 取り込みの学習（initial の層）を 1 行足す。
fn add_initial_row(e: &Engine, source: u32) {
    use ikari_core::user::{ContextFeatures, Feature, FeatureKind, SegmentKey};
    let stack = e.inner.stack();
    let feats = ContextFeatures {
        features: vec![Feature {
            kind: FeatureKind::Unconditional,
            hash: 0,
            weight: 0.2,
        }],
    };
    let key = SegmentKey {
        surface: "錨".into(),
        reading: "いかり".into(),
        pos_name: "名詞,普通名詞,一般,*,*,*".into(),
    };
    stack
        .learning
        .as_ref()
        .unwrap()
        .add_initial(source, "いかり", &[key], &feats, 1);
}

fn learning_status(e: &Engine) -> pb::LearningStatusReply {
    match send(e, envelope::Body::LearningStatus(pb::LearningStatus {})) {
        envelope::Body::LearningStatusReply(r) => r,
        other => panic!("expected LearningStatusReply, got {other:?}"),
    }
}

fn learning_reset(e: &Engine, layer: pb::LearningLayer) -> envelope::Body {
    send(
        e,
        envelope::Body::ResetLearning(pb::LearningReset {
            layer: layer as i32,
        }),
    )
}

fn reset_ok(e: &Engine, layer: pb::LearningLayer) -> pb::LearningStatusReply {
    match learning_reset(e, layer) {
        envelope::Body::LearningStatusReply(r) => r,
        other => panic!("expected LearningStatusReply, got {other:?}"),
    }
}

/// 学習ファイルの (usage の行数, initial の行数)
fn rows_on_disk(t: &Real) -> (usize, usize) {
    LearningStore::open(&t.user().learning_tsv())
        .unwrap()
        .layer_counts()
}

/// 使用の層と初期学習の層の両方に行があるエンジン
fn engine_with_both_layers() -> Real {
    let t = Real::new();
    begin(t.e(), 1, false);
    commit_first(t.e(), 1, "いかり");
    add_initial_row(t.e(), 3);
    t
}

fn backup_files(t: &Real) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(t.dir.join("backups"))
        .map(|d| {
            d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

// ---- エクスポートとインポート ----

fn export(e: &Engine) -> pb::ExportReply {
    match send(e, envelope::Body::ExportData(pb::ExportData {})) {
        envelope::Body::ExportReply(r) => r,
        other => panic!("expected ExportReply, got {other:?}"),
    }
}

fn import(e: &Engine, content: &[u8], dry_run: bool) -> pb::ImportReply {
    match send(
        e,
        envelope::Body::ImportData(pb::ImportData {
            content: content.to_vec(),
            dry_run,
        }),
    ) {
        envelope::Body::ImportReply(r) => r,
        other => panic!("expected ImportReply, got {other:?}"),
    }
}

/// 学習ファイルを今のメモリで書く（`add_initial` はタイマーの保存の対象にならないため）
fn save_learning(e: &Engine) {
    e.inner.stack().learning.as_ref().unwrap().save().unwrap();
}

/// ユーザーデータのファイルの中身（比べるための写し）
fn user_files(t: &Real) -> Vec<Option<String>> {
    let user = t.user();
    [
        user.profile_tsv(),
        user.user_dict_tsv(),
        user.learning_tsv(),
        user.ingest_sources_tsv(),
    ]
    .iter()
    .map(|p| std::fs::read_to_string(p).ok())
    .collect()
}

fn set_profile_file(t: &Real, settings: &[(&str, ikari_core::user::ProfileWeight)]) {
    let settings: Vec<(String, _)> = settings
        .iter()
        .map(|(i, w)| ((*i).to_owned(), *w))
        .collect();
    ikari_core::user::Profile::from_settings(&settings)
        .unwrap()
        .save(&t.user())
        .unwrap();
}

/// 読みを数から作る（ひらがな 4 文字で 46^4 通り）
fn kana_reading(mut n: usize) -> String {
    const KANA: &str = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをん";
    let chars: Vec<char> = KANA.chars().collect();
    let mut out = String::new();
    for _ in 0..4 {
        out.push(chars[n % chars.len()]);
        n /= chars.len();
    }
    out
}

// ---- 属性プロファイル ----

fn profile_state(id: &str, weight: pb::ProfileWeightKind) -> pb::ProfileDomainState {
    pb::ProfileDomainState {
        id: id.into(),
        name: String::new(),
        weight: weight as i32,
    }
}

fn profile_get(e: &Engine) -> Vec<pb::ProfileDomainState> {
    match send(e, envelope::Body::GetProfile(pb::ProfileGet {})) {
        envelope::Body::ProfileReply(r) => r.domains,
        other => panic!("expected ProfileReply, got {other:?}"),
    }
}

fn profile_set(e: &Engine, domains: Vec<pb::ProfileDomainState>) -> envelope::Body {
    send(e, envelope::Body::SetProfile(pb::ProfileSet { domains }))
}

fn weights(domains: &[pb::ProfileDomainState]) -> Vec<(&str, i32)> {
    domains.iter().map(|d| (d.id.as_str(), d.weight)).collect()
}

mod admin_dict;
mod admin_import;
mod admin_learning;
mod connection;
mod context;
mod convert;
mod crash_record;
mod diag;
mod emoji;
mod lifecycle;
mod model_check;
mod postal;
mod protocol;
mod real;
mod segments;
mod session;
