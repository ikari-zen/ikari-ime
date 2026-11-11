//! 往復時間の測定（S1。ADR 0040 §6、NFR-02。issue #334）。純粋（OS に依存しない。Mac でテストする）。
//!
//! 測るのは 3 つ（`Kind`）。
//! - `Request`: 要求を送ってから返事を受けるまで（パイプの往復 + エンジンの処理）。`transport::request` の 1 回
//! - `Key`: キーイベントを受けてから、返った `TsfOp` を実行し終わるまで（TIP の内側。`Request` を含む）
//! - `FirstConnect`: 最初の接続の試行から、最初の接続成功まで（失敗の試行と、試行の間隔を含む）
//!
//! **入力された文字列や読みは記録しない**。受けるのは時間の数値、種類、成否だけで、文字列を渡せる口がない。
//!
//! - 既定はオフ。オフのときのコストは、`start` と `finish` の分岐 1 つずつ（時計も錠も触らない）。
//! - 種類ごとに、最新の `WINDOW` 件（成功したものだけ）を固定長のリングに持つ。失敗は数だけ持つ。
//! - 書き出しは、一定の件数ごとに `flush_if_due` が `LineSink` へ 1 行ずつ渡す。**呼ぶ側が、キー処理が終わった
//!   あとで呼ぶ**（`finish` は書かない。測定が測る対象を遅くしない）。
//!
//! 判断は持たない（測って、まとめて、渡すだけ）。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// 種類ごとに持つ最新の件数（4 バイト × 3 種類 × この数 = 約 12KB）
pub const WINDOW: usize = 1000;
/// 要求・キーの種類で、これだけ溜まったら書き出す対象にする
pub const FLUSH_EVERY: u32 = 200;
/// 測定をオンにする環境変数の名前（値が `1` のとき）
pub const ENABLE_ENV: &str = "IKARI_TIP_LATENCY";
/// 測定をオンにするファイルの名前（ユーザーのデータの場所の直下。存在すればオン）
pub const ENABLE_MARKER_FILE: &str = "latency.on";
/// ログのファイルの名前（ユーザーのデータの場所の `logs` の下）
pub const LOG_FILE_NAME: &str = "tip-latency.log";

/// ログのファイルがこれを超えたら、作り直す（測定のログがディスクを埋めない）
pub const MAX_LOG_BYTES: u64 = 1024 * 1024;
/// ログを置くディレクトリの名前（ユーザーのデータの場所の下。`engine.log` と同じ）
pub const LOG_DIR_NAME: &str = "logs";

/// 環境変数の値と印のファイルの有無から、測定をオンにするか決める。
pub fn enabled(env_value: Option<&str>, marker_exists: bool) -> bool {
    env_value == Some("1") || marker_exists
}

/// 測る区間の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Request,
    Key,
    FirstConnect,
}

const KINDS: [Kind; 3] = [Kind::Request, Kind::Key, Kind::FirstConnect];

impl Kind {
    /// ログに書く名前
    pub fn name(self) -> &'static str {
        match self {
            Kind::Request => "request",
            Kind::Key => "key",
            Kind::FirstConnect => "first_connect",
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    /// 書き出す対象にする件数
    fn flush_threshold(self) -> u32 {
        match self {
            // 1 回しか起きないので、起きたらすぐ
            Kind::FirstConnect => 1,
            Kind::Request | Kind::Key => FLUSH_EVERY,
        }
    }
}

/// 時計（テストは差し込む）。
pub trait Ticks: Send + Sync {
    /// 単調に増えるマイクロ秒
    fn now_us(&self) -> u64;
    /// ログの行に付ける壁時計（UNIX 秒）
    fn unix_secs(&self) -> u64;
}

/// 書き出し先（本番はファイルへの追記。テストは集める）。失敗は握りつぶしてよい（測定のために本体を止めない）。
pub trait LineSink: Send + Sync {
    fn write_lines(&self, lines: &[String]);
}

/// 区間の開始の印。オフのときは中身がない。
#[derive(Debug, Clone, Copy)]
pub struct Stamp(Option<u64>);

#[derive(Default)]
struct Ring {
    samples: Vec<u32>,
    next: usize,
    /// 成功の合計（リングから落ちた分も含む）
    ok_total: u64,
    errors: u64,
    /// 前の書き出しからの件数
    pending: u32,
}

impl Ring {
    fn push(&mut self, micros: u64) {
        let value = u32::try_from(micros).unwrap_or(u32::MAX);
        if self.samples.len() < WINDOW {
            self.samples.push(value);
        } else {
            self.samples[self.next] = value;
        }
        self.next = (self.next + 1) % WINDOW;
        self.ok_total += 1;
        self.pending += 1;
    }
}

/// 最初の接続の区間の状態
#[derive(Clone, Copy)]
enum Connect {
    NotStarted,
    Started(u64),
    Done,
}

struct State {
    rings: [Ring; KINDS.len()],
    connect: Connect,
}

/// 測定の本体。`Arc` で共有する。
pub struct Meter {
    /// `None` はオフ
    inner: Option<Inner>,
}

struct Inner {
    ticks: Box<dyn Ticks>,
    sink: Box<dyn LineSink>,
    pid: u32,
    state: Mutex<State>,
}

impl Meter {
    /// オフの測定（何も測らない）。
    pub fn disabled() -> Self {
        Self { inner: None }
    }

    pub fn new(ticks: Box<dyn Ticks>, sink: Box<dyn LineSink>, pid: u32) -> Self {
        Self {
            inner: Some(Inner {
                ticks,
                sink,
                pid,
                state: Mutex::new(State {
                    rings: Default::default(),
                    connect: Connect::NotStarted,
                }),
            }),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// 区間を始める。
    pub fn start(&self) -> Stamp {
        match &self.inner {
            Some(inner) => Stamp(Some(inner.ticks.now_us())),
            None => Stamp(None),
        }
    }

    /// 区間を終える。`ok` は成否（失敗は時間を入れず、数だけ数える）。
    pub fn finish(&self, kind: Kind, stamp: Stamp, ok: bool) {
        let (Some(inner), Stamp(Some(began))) = (&self.inner, stamp) else {
            return;
        };
        let elapsed = inner.ticks.now_us().saturating_sub(began);
        let mut state = inner.lock();
        let ring = &mut state.rings[kind.index()];
        if ok {
            ring.push(elapsed);
        } else {
            ring.errors += 1;
            ring.pending += 1;
        }
    }

    /// 接続の試行を始める。最初の 1 回だけ時刻を覚える（以後の試行は何もしない）。
    pub fn connect_attempt(&self) {
        let Some(inner) = &self.inner else { return };
        let now = inner.ticks.now_us();
        let mut state = inner.lock();
        if matches!(state.connect, Connect::NotStarted) {
            state.connect = Connect::Started(now);
        }
    }

    /// 接続が成功した。最初の成功だけ、`FirstConnect` に入れる。
    pub fn connect_succeeded(&self) {
        let Some(inner) = &self.inner else { return };
        let now = inner.ticks.now_us();
        let mut state = inner.lock();
        if let Connect::Started(began) = state.connect {
            state.connect = Connect::Done;
            state.rings[Kind::FirstConnect.index()].push(now.saturating_sub(began));
        }
    }

    /// 溜まった件数が書き出す件数に達した種類があれば、行にして渡す。**キー処理が終わったあとで呼ぶ**。
    pub fn flush_if_due(&self) {
        self.flush(false);
    }

    /// 前の書き出しから増えた分を、件数にかかわらず渡す（プロセスの終わり、入力の終わりのとき）。
    pub fn flush_all(&self) {
        self.flush(true);
    }

    fn flush(&self, force: bool) {
        let Some(inner) = &self.inner else { return };
        let now = inner.ticks.unix_secs();
        let lines: Vec<String> = {
            let mut state = inner.lock();
            KINDS
                .iter()
                .filter_map(|&kind| {
                    let ring = &mut state.rings[kind.index()];
                    let due = if force {
                        ring.pending > 0
                    } else {
                        ring.pending >= kind.flush_threshold()
                    };
                    if !due {
                        return None;
                    }
                    ring.pending = 0;
                    Some(format_line(now, inner.pid, kind, ring))
                })
                .collect()
        };
        // 錠の外で書く（書く間、測定の点を塞がない）
        if !lines.is_empty() {
            inner.sink.write_lines(&lines);
        }
    }
}

/// 本番の時計
pub struct SystemTicks {
    base: Instant,
}

impl SystemTicks {
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
        }
    }
}

impl Default for SystemTicks {
    fn default() -> Self {
        Self::new()
    }
}

impl Ticks for SystemTicks {
    fn now_us(&self) -> u64 {
        u64::try_from(self.base.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    fn unix_secs(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }
}

/// ファイルへの追記。書けなければ捨てる（測定のために本体の処理を止めない）。
pub struct FileSink {
    path: PathBuf,
}

impl FileSink {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl LineSink for FileSink {
    fn write_lines(&self, lines: &[String]) {
        let _ = self.append(lines);
    }
}

impl FileSink {
    fn append(&self, lines: &[String]) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let too_big = fs::metadata(&self.path).is_ok_and(|meta| meta.len() > MAX_LOG_BYTES);
        let mut options = OpenOptions::new();
        options.create(true);
        if too_big {
            options.write(true).truncate(true);
        } else {
            options.append(true);
        }
        let mut file = options.open(&self.path)?;
        let mut text = lines.join("\n");
        text.push('\n');
        file.write_all(text.as_bytes())
    }
}

impl Meter {
    /// 環境変数と印のファイル（`<dir>/latency.on`）で、オンかを決めて作る。オフなら何も測らない。
    ///
    /// 印のファイルがあるのは、TIP の DLL が各アプリのプロセスに入るので、環境変数がアプリに見えないことがあるため
    /// （エクスプローラーから起動したアプリは、あとから設定した環境変数を引き継がない）。
    pub fn from_settings(user_dir: Option<&Path>, env_value: Option<&str>, pid: u32) -> Self {
        let marker = user_dir.is_some_and(|dir| dir.join(ENABLE_MARKER_FILE).exists());
        if !enabled(env_value, marker) {
            return Self::disabled();
        }
        let Some(dir) = user_dir else {
            // 書き出し先が決められない
            return Self::disabled();
        };
        let path = dir.join(LOG_DIR_NAME).join(LOG_FILE_NAME);
        Self::new(
            Box::new(SystemTicks::new()),
            Box::new(FileSink::new(path)),
            pid,
        )
    }
}

/// このプロセスの測定（初めて呼んだときに、オンかを決めて固定する）。
pub fn global(user_dir: impl FnOnce() -> Option<PathBuf>) -> &'static Arc<Meter> {
    static METER: OnceLock<Arc<Meter>> = OnceLock::new();
    METER.get_or_init(|| {
        let env_value = std::env::var(ENABLE_ENV).ok();
        Arc::new(Meter::from_settings(
            user_dir().as_deref(),
            env_value.as_deref(),
            std::process::id(),
        ))
    })
}

impl Inner {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// 並べ替えた値の百分位（最も近い順位。`sorted` は空でない）
fn percentile(sorted: &[u32], percent: usize) -> u32 {
    let rank = (sorted.len() * percent).div_ceil(100).max(1);
    sorted[rank - 1]
}

/// 1 行。`t`（UNIX 秒）、`pid`、`kind`、`n`（成功の合計）、`window`（統計に使った件数）、`err`（失敗の合計）、
/// `min_us`、`p50_us`、`p95_us`、`max_us`。件数がなければ値は 0。**入力の内容は入らない**。
fn format_line(unix_secs: u64, pid: u32, kind: Kind, ring: &Ring) -> String {
    let mut sorted = ring.samples.clone();
    sorted.sort_unstable();
    let (min, p50, p95, max) = match sorted.as_slice() {
        [] => (0, 0, 0, 0),
        all => (
            all[0],
            percentile(all, 50),
            percentile(all, 95),
            all[all.len() - 1],
        ),
    };
    format!(
        "t={unix_secs} pid={pid} kind={} n={} window={} err={} min_us={min} p50_us={p50} p95_us={p95} max_us={max}",
        kind.name(),
        ring.ok_total,
        sorted.len(),
        ring.errors,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    #[derive(Default)]
    struct FakeTicks(AtomicU64);

    impl FakeTicks {
        fn advance(&self, micros: u64) {
            self.0.fetch_add(micros, Ordering::SeqCst);
        }
    }

    struct SharedTicks(Arc<FakeTicks>);

    impl Ticks for SharedTicks {
        fn now_us(&self) -> u64 {
            self.0.0.load(Ordering::SeqCst)
        }
        fn unix_secs(&self) -> u64 {
            1_000
        }
    }

    #[derive(Default)]
    struct Collect(Mutex<Vec<String>>);

    struct SharedSink(Arc<Collect>);

    impl LineSink for SharedSink {
        fn write_lines(&self, lines: &[String]) {
            self.0.0.lock().unwrap().extend_from_slice(lines);
        }
    }

    fn meter() -> (Meter, Arc<FakeTicks>, Arc<Collect>) {
        let ticks = Arc::new(FakeTicks::default());
        let sink = Arc::new(Collect::default());
        let meter = Meter::new(
            Box::new(SharedTicks(Arc::clone(&ticks))),
            Box::new(SharedSink(Arc::clone(&sink))),
            42,
        );
        (meter, ticks, sink)
    }

    fn measure(meter: &Meter, ticks: &FakeTicks, kind: Kind, micros: u64, ok: bool) {
        let stamp = meter.start();
        ticks.advance(micros);
        meter.finish(kind, stamp, ok);
    }

    fn lines(sink: &Collect) -> Vec<String> {
        sink.0.lock().unwrap().clone()
    }

    #[test]
    fn enabled_by_env_one_or_marker_only() {
        assert!(!enabled(None, false));
        assert!(!enabled(Some("0"), false));
        assert!(!enabled(Some(""), false));
        assert!(enabled(Some("1"), false));
        assert!(enabled(None, true));
    }

    #[test]
    fn disabled_meter_records_and_writes_nothing() {
        let meter = Meter::disabled();
        assert!(!meter.is_enabled());
        let stamp = meter.start();
        meter.finish(Kind::Key, stamp, true);
        meter.connect_attempt();
        meter.connect_succeeded();
        meter.flush_all();
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        let (meter, ticks, sink) = meter();
        for micros in 1..=100 {
            measure(&meter, &ticks, Kind::Request, micros, true);
        }
        meter.flush_all();
        assert_eq!(
            lines(&sink),
            vec![
                "t=1000 pid=42 kind=request n=100 window=100 err=0 min_us=1 p50_us=50 p95_us=95 max_us=100"
            ]
        );
    }

    #[test]
    fn single_sample_is_every_statistic() {
        let (meter, ticks, sink) = meter();
        measure(&meter, &ticks, Kind::Key, 7, true);
        meter.flush_all();
        let line = &lines(&sink)[0];
        assert!(
            line.contains("min_us=7 p50_us=7 p95_us=7 max_us=7"),
            "{line}"
        );
    }

    #[test]
    fn window_keeps_only_the_latest_samples() {
        let (meter, ticks, sink) = meter();
        // 古い 1000 件は大きく、そのあとの 1000 件は小さい
        for _ in 0..WINDOW {
            measure(&meter, &ticks, Kind::Request, 9_000, true);
        }
        for _ in 0..WINDOW {
            measure(&meter, &ticks, Kind::Request, 100, true);
        }
        meter.flush_all();
        let line = &lines(&sink)[0];
        assert!(line.contains("n=2000 window=1000"), "{line}");
        assert!(
            line.contains("min_us=100 p50_us=100 p95_us=100 max_us=100"),
            "{line}"
        );
    }

    #[test]
    fn failures_are_counted_without_time() {
        let (meter, ticks, sink) = meter();
        measure(&meter, &ticks, Kind::Request, 500, true);
        measure(&meter, &ticks, Kind::Request, 99_999, false);
        meter.flush_all();
        let line = &lines(&sink)[0];
        assert!(line.contains("n=1 window=1 err=1"), "{line}");
        assert!(line.contains("max_us=500"), "{line}");
    }

    #[test]
    fn only_failures_still_report_with_zero_values() {
        let (meter, ticks, sink) = meter();
        measure(&meter, &ticks, Kind::Request, 1, false);
        meter.flush_all();
        let line = &lines(&sink)[0];
        assert!(
            line.contains("window=0 err=1 min_us=0 p50_us=0 p95_us=0 max_us=0"),
            "{line}"
        );
    }

    #[test]
    fn flush_if_due_waits_for_the_threshold_and_writes_per_kind() {
        let (meter, ticks, sink) = meter();
        for _ in 0..FLUSH_EVERY - 1 {
            measure(&meter, &ticks, Kind::Key, 10, true);
        }
        meter.flush_if_due();
        assert!(lines(&sink).is_empty());
        measure(&meter, &ticks, Kind::Key, 10, true);
        measure(&meter, &ticks, Kind::Request, 10, true);
        meter.flush_if_due();
        let written = lines(&sink);
        assert_eq!(written.len(), 1, "{written:?}");
        assert!(written[0].contains("kind=key"), "{written:?}");
        // 書いたあとは、また溜まるまで書かない
        meter.flush_if_due();
        assert_eq!(lines(&sink).len(), 1);
    }

    #[test]
    fn first_connect_is_measured_once_from_the_first_attempt() {
        let (meter, ticks, sink) = meter();
        meter.connect_succeeded();
        meter.flush_all();
        assert!(
            lines(&sink).is_empty(),
            "a success without an attempt is ignored"
        );

        meter.connect_attempt();
        ticks.advance(300);
        meter.connect_attempt();
        ticks.advance(200);
        meter.connect_succeeded();
        meter.flush_if_due();
        let written = lines(&sink);
        assert_eq!(written.len(), 1);
        assert!(written[0].contains("kind=first_connect n=1"), "{written:?}");
        assert!(written[0].contains("max_us=500"), "{written:?}");

        // 2 回目以降の接続は測らない
        meter.connect_attempt();
        ticks.advance(10);
        meter.connect_succeeded();
        meter.flush_all();
        assert_eq!(lines(&sink).len(), 1);
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ikari-tip-latency-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn from_settings_is_off_without_env_or_marker() {
        let dir = temp_dir("off");
        assert!(!Meter::from_settings(Some(&dir), None, 1).is_enabled());
        assert!(!Meter::from_settings(None, Some("1"), 1).is_enabled());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn from_settings_marker_turns_it_on_and_lines_are_appended() {
        let dir = temp_dir("on");
        fs::write(dir.join(ENABLE_MARKER_FILE), "").unwrap();
        let meter = Meter::from_settings(Some(&dir), None, 7);
        assert!(meter.is_enabled());
        let stamp = meter.start();
        meter.finish(Kind::Key, stamp, true);
        meter.flush_all();
        meter.flush_all();
        let stamp = meter.start();
        meter.finish(Kind::Key, stamp, true);
        meter.flush_all();
        let text = fs::read_to_string(dir.join(LOG_DIR_NAME).join(LOG_FILE_NAME)).unwrap();
        assert_eq!(text.lines().count(), 2, "{text}");
        assert!(text.lines().all(|l| l.contains("pid=7 kind=key")), "{text}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn file_sink_starts_over_when_the_file_is_too_big() {
        let dir = temp_dir("big");
        let path = dir.join("x.log");
        fs::write(
            &path,
            vec![b'a'; usize::try_from(MAX_LOG_BYTES).unwrap() + 1],
        )
        .unwrap();
        FileSink::new(path.clone()).write_lines(&["new".to_owned()]);
        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn log_line_has_only_numbers_and_fixed_words() {
        let (meter, ticks, sink) = meter();
        measure(&meter, &ticks, Kind::Key, 123, true);
        meter.flush_all();
        let line = &lines(&sink)[0];
        assert!(line.split(' ').all(|field| {
            let (key, value) = field.split_once('=').unwrap();
            !key.is_empty() && (value.chars().all(|c| c.is_ascii_digit()) || key == "kind")
        }));
    }
}
