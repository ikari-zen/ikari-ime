//! Rust 版の殻の性能の測り手（ADR 0041 §6.3。P1・P3・P4）。移行の前の Swift 版の測り手（M5 で消した）と同じ手順・同じ
//! 出力の形で、`cargo xtask perf shell` が呼ぶ。基準値（`eval/reports/shell-perf/baseline.json` の `swift.*`）は固定。
//!
//! - P1: キー 1 回の `Shell::dispatch` の時間（ナノ秒）。同期のエンジン要求（convert、page、segment_page、resize_segment、
//!   reconvert、postal_lookup）を送ったキーは除く（「Convert を含まない」）。共通のシナリオ全部を `--repeat` 回くり返す
//! - P3: 殻の初期化（`Shell::new` と、入力先の確定 `Activate` まで。ショートコードの表と hello は替え玉のエンジンが返す）の時間。`--repeat` 回
//! - P4: そのプロセスの最大 RSS（`eval/src/rss.rs` をそのまま取り込む）
//!
//! `Shell` が stub のうち（M1-a）は、動作が 1 つも返らないので、出力に `"stub": true` を付ける（数値は比べられない）。
//!
//! - alloc（`--features dhat-heap` のときだけ。`cargo xtask perf alloc` が呼ぶ）: P1 と同じキーの列で、`dispatch` 1 回あたりの
//!   ヒープ確保の回数とバイト数（dhat。同期のエンジン要求を送ったキーは除く）。確保の場所は `--dhat-file` に書く（dhat-heap.json）
//!
//! 使い方: `cargo bench -p ikari-shell-core --bench shell_perf -- --metric p1|p3 [--repeat 100] [--shortcode-entries 2000]`
//! 出力: 標準出力に JSON を 1 つ。`cargo bench` が付ける `--bench` は読み捨てる。

// 測り手なので、誤りは標準エラーに出して終了コード 1 にする（lib.rs の deny は `not(test)` のときだけ）
#![allow(clippy::unwrap_used, clippy::expect_used)]

// eval/src/rss.rs をそのまま取り込む（依存を増やさない）。その中のテスト用の import は、ここでは使わない
#[allow(unused_imports)]
#[path = "../../eval/src/rss.rs"]
mod rss;

use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ikari_proto::v1;
use ikari_proto::v1::envelope::Body;
use ikari_shell_core::scenario::{self, EngineScript, ScenarioSurrounding, ScriptPage};
use ikari_shell_core::{
    CandidateKind, Clock, EngineError, EngineErrorCode, EngineTransport, SelectedText,
    SettingsSource, Shell, ShellConfig, ShellEvent, SurroundingText, TextAccess, TransportError,
    TransportHandle,
};
use prost::Message;

/// 確保の計測（`dhat-heap` のときだけ。ADR 0043）。付けないときは何も数えない
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

/// いままでの確保の累計（回数、バイト数）。`dhat-heap` がなければ常に 0
fn heap_totals() -> (u64, u64) {
    #[cfg(feature = "dhat-heap")]
    {
        let stats = dhat::HeapStats::get();
        (stats.total_blocks, stats.total_bytes)
    }
    #[cfg(not(feature = "dhat-heap"))]
    {
        (0, 0)
    }
}

/// P3 の既定のショートコードの数（実データの emoji.tsv が約 1,970 行）
const DEFAULT_SHORTCODE_ENTRIES: usize = 2000;
const DEFAULT_REPEAT: usize = 100;
/// P3 の設定（Swift 版と同じ数行）
const STARTUP_SETTINGS: &str =
    "paging.size\t3\nkeymap.preset\tmacos\nengine.lm\ttrue\nemoji.shortcode\ttrue\n";
const SHELL_VERSION: &str = "bench";
/// 返事を待つ（同期の）要求。P1 では、これを送ったキーを除く
const SYNC_REQUESTS: &[&str] = &[
    "convert",
    "page",
    "segment_page",
    "resize_segment",
    "reconvert",
    "postal_lookup",
];

/// 入力中（ローマ字の 1 打）の台本。共通シナリオ（変換中の操作に偏る）が測れない、日常でいちばん多い打鍵を測る。
/// 空白区切りのキーの指定（`parse_press` の書き方）。母音、子音+母音、促音、撥音、拗音、長い読み、Backspace、Esc を含む
const TYPING_SCRIPTS: &[&str] = &[
    "a r i g a t o u g o z a i m a s u",
    "k y o u k a k k o i i n n j a n a i",
    "w a t a s i h a k y o u z e n z e n k a i m o n o i k i m a s u",
    "s i n b u n n backspace backspace backspace k a",
    "t o k y o u t o r i t u escape",
    "j y u t t y o u n n a k a n a k a n j i",
];
/// 入力中の台本をくり返す回数は、シナリオ全部のくり返し（`--repeat`）のこの倍にする（台本が短いので）
const TYPING_REPEAT_FACTOR: usize = 20;

struct Options {
    metric: String,
    repeat: usize,
    shortcode_entries: usize,
    /// P1 のキーごとの内訳（遅い順の上位）を標準エラーに出す
    detail: bool,
    /// alloc のときの dhat-heap.json の書き出し先
    dhat_file: String,
}

fn parse_options() -> Result<Options, String> {
    let mut options = Options {
        metric: "p1".into(),
        repeat: DEFAULT_REPEAT,
        shortcode_entries: DEFAULT_SHORTCODE_ENTRIES,
        detail: false,
        dhat_file: "dhat-heap.json".into(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--bench" => {}
            "--detail" => options.detail = true,
            "--dhat-file" => options.dhat_file = value("--dhat-file")?,
            "--metric" => options.metric = value("--metric")?,
            "--repeat" => {
                options.repeat = value("--repeat")?
                    .parse()
                    .ok()
                    .filter(|count| *count > 0)
                    .ok_or("bad --repeat")?;
            }
            "--shortcode-entries" => {
                options.shortcode_entries = value("--shortcode-entries")?
                    .parse()
                    .map_err(|_| "bad --shortcode-entries")?;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(options)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let options = parse_options()?;
    // alloc: dhat の計測を始める。この変数が落ちるとき（run の終わり）に dhat-heap.json を書く
    #[cfg(feature = "dhat-heap")]
    let _profiler = (options.metric == "alloc").then(|| {
        dhat::Profiler::builder()
            .file_name(&options.dhat_file)
            .build()
    });
    let measured = match options.metric.as_str() {
        "p1" => measure_keys(&options)?,
        "alloc" => {
            if !cfg!(feature = "dhat-heap") {
                return Err("alloc needs --features dhat-heap".into());
            }
            measure_keys(&options)?
        }
        "p3" => measure_startup(&options)?,
        other => return Err(format!("unknown metric {other}")),
    };
    let typing = if matches!(options.metric.as_str(), "p1" | "alloc") {
        Some(measure_typing(&options)?)
    } else {
        None
    };
    report(&options, measured, typing)
}

// ---------------------------------------------------------------- 統計と出力

/// 順位の最も近い値（最近傍順位法）。`sorted` は昇順で空でない。
fn percentile(sorted: &[u64], fraction: f64) -> u64 {
    let rank = (fraction * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn report(options: &Options, measured: Measured, typing: Option<Typing>) -> Result<(), String> {
    let Measured {
        mut samples,
        without_transport,
        excluded,
        stub,
        allocs,
        startup,
        first,
        steady,
    } = measured;
    if samples.is_empty() {
        return Err("no samples".into());
    }
    samples.sort_unstable();
    let mut object = serde_json::json!({
        "implementation": "rust",
        "metric": options.metric,
        "samples": samples.len(),
        "excluded_samples": excluded,
        "repeat": options.repeat,
        "p50_ns": percentile(&samples, 0.5),
        "p99_ns": percentile(&samples, 0.99),
        "max_ns": samples[samples.len() - 1],
        "stub": stub,
    });
    // 替え玉のエンジン（`ScriptedTransport`: 要求の復号、台本の照合、返事の組み立てと符号化）で使った時間を引いた値。
    // Swift 版の測り手はエンジンへの口を持たない（FFI の符号化を含まない）ので、これが近い条件の値
    if !without_transport.is_empty() {
        let mut sorted = without_transport;
        sorted.sort_unstable();
        object["p50_ns_without_transport"] = percentile(&sorted, 0.5).into();
        object["p99_ns_without_transport"] = percentile(&sorted, 0.99).into();
        object["max_ns_without_transport"] = sorted[sorted.len() - 1].into();
    }
    if options.metric == "p3" {
        object["shortcode_entries"] = options.shortcode_entries.into();
    }
    if let Some(bytes) = rss::max_rss_bytes() {
        object["max_rss_bytes"] = bytes.into();
    }
    // 起動と定常の内訳（足した指標。上の `p50_ns` などは従来の意味のまま）
    if options.metric == "p1" || options.metric == "alloc" {
        object["startup"] = group_summary(&startup, options.metric == "alloc");
        object["first"] = group_summary(&first, options.metric == "alloc");
        object["steady"] = group_summary(&steady, options.metric == "alloc");
    }
    if let Some(typing) = typing {
        object["typing"] = serde_json::json!({
            "pending": group_summary(&typing.pending, options.metric == "alloc"),
            "reading": group_summary(&typing.reading, options.metric == "alloc"),
            "edit": group_summary(&typing.edit, options.metric == "alloc"),
        });
    }
    if options.metric == "alloc" {
        object["alloc"] = alloc_summary(&allocs);
        object["dhat_file"] = options.dhat_file.clone().into();
    }
    println!("{object}");
    Ok(())
}

/// 起動か定常の 1 組の要約（時間の p50・p99・max と、alloc のときは確保の分布）
fn group_summary(group: &Group, with_alloc: bool) -> serde_json::Value {
    let mut object = serde_json::json!({
        "dispatches": group.dispatches,
        "samples": group.samples.len(),
    });
    if group.samples.is_empty() {
        return object;
    }
    let mut sorted = group.samples.clone();
    sorted.sort_unstable();
    object["p50_ns"] = percentile(&sorted, 0.5).into();
    object["p99_ns"] = percentile(&sorted, 0.99).into();
    object["max_ns"] = sorted[sorted.len() - 1].into();
    if with_alloc {
        object["alloc"] = alloc_summary(&group.allocs);
    }
    object
}

/// 確保の回数とバイト数の分布（p50・p99・max と、全部の合計）
fn alloc_summary(allocs: &[(u64, u64)]) -> serde_json::Value {
    let mut blocks: Vec<u64> = allocs.iter().map(|(blocks, _)| *blocks).collect();
    let mut bytes: Vec<u64> = allocs.iter().map(|(_, bytes)| *bytes).collect();
    blocks.sort_unstable();
    bytes.sort_unstable();
    let sum = |values: &[u64]| values.iter().sum::<u64>();
    serde_json::json!({
        "blocks_p50": percentile(&blocks, 0.5),
        "blocks_p99": percentile(&blocks, 0.99),
        "blocks_max": blocks[blocks.len() - 1],
        "blocks_total": sum(&blocks),
        "bytes_p50": percentile(&bytes, 0.5),
        "bytes_p99": percentile(&bytes, 0.99),
        "bytes_max": bytes[bytes.len() - 1],
        "bytes_total": sum(&bytes),
    })
}

// ---------------------------------------------------------------- P1

/// 起動（`Activate`。入力先の切り替えと、最初の `finish`）と、定常（それ以外のイベント。打鍵ごと）の分け前。
/// 測り手は dispatch ごとに新しい `Shell` を作るので、起動の費用が全体に混ざる。それを分けて見るための集計
#[derive(Default)]
struct Group {
    /// 同期のエンジン要求を送ったものも含めた dispatch の数
    dispatches: usize,
    /// 同期のエンジン要求を送ったものを除いた時間（ナノ秒）
    samples: Vec<u64>,
    /// 同じものを除いた確保（回数、バイト数）
    allocs: Vec<(u64, u64)>,
}

impl Group {
    fn push(&mut self, is_sync: bool, elapsed: u64, alloc: (u64, u64)) {
        self.dispatches += 1;
        if !is_sync {
            self.samples.push(elapsed);
            self.allocs.push(alloc);
        }
    }
}

struct Measured {
    /// 起動（`Activate`）の分。P1 の指標（`samples` ほか）は従来どおり全部を含み、ここは別に足した指標
    startup: Group,
    /// 起動の直後の最初の 1 打（`Activate` 以外の最初のイベント。エンジンの状態や設定の取り直しの結果が、ここの
    /// `finish` で取り込まれてメニューなどが作り直される）の分
    first: Group,
    /// 定常（最初の 1 打のあとの、`Activate` 以外のイベント）の分
    steady: Group,
    samples: Vec<u64>,
    /// `samples` から替え玉のエンジンで使った時間を引いたもの（P1 のみ）
    without_transport: Vec<u64>,
    excluded: usize,
    stub: bool,
    /// alloc のとき: `dispatch` 1 回ごとの確保（回数、バイト数）
    allocs: Vec<(u64, u64)>,
}

/// `--detail` 用: キー 1 つぶんの記録
struct KeyRecord {
    label: String,
    total: Vec<u64>,
    transport: Vec<u64>,
    /// alloc のとき: `dispatch` ごとの確保の回数
    blocks: Vec<u64>,
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn median(values: &mut [u64]) -> u64 {
    values.sort_unstable();
    values[values.len() / 2]
}

fn print_detail(records: &mut [KeyRecord], alloc: bool) {
    if alloc {
        records.sort_by_key(|record| std::cmp::Reverse(median(&mut record.blocks.clone())));
        eprintln!("# 確保の多いキー(確保の回数の中央値の降順の上位 25)  キー");
        for record in records.iter_mut().take(25) {
            eprintln!("{:>8}  {}", median(&mut record.blocks), record.label);
        }
        return;
    }
    for record in records.iter_mut() {
        record.total.sort_unstable();
    }
    records.sort_by_key(|record| std::cmp::Reverse(record.total[record.total.len() / 2]));
    eprintln!("# 遅いキー(中央値の降順の上位 25): 合計 / エンジン側 / 殻の分 [ns]  キー");
    for record in records.iter_mut().take(25) {
        let total = median(&mut record.total);
        let transport = median(&mut record.transport);
        eprintln!(
            "{total:>8} {transport:>8} {:>8}  {}",
            total.saturating_sub(transport),
            record.label
        );
    }
    let mut medians: Vec<u64> = records.iter_mut().map(|r| median(&mut r.total)).collect();
    medians.sort_unstable();
    eprintln!(
        "# キーごとの中央値の分布: keys={} p10={} p50={} p90={} p99={}",
        medians.len(),
        percentile(&medians, 0.1),
        percentile(&medians, 0.5),
        percentile(&medians, 0.9),
        percentile(&medians, 0.99)
    );
}

/// 入力中の 1 打の区分（台本を一度流して決める）。
#[derive(Clone, Copy, PartialEq)]
enum TypingClass {
    /// 読みが変わらない（ローマ字の途中。先読みは送らない）
    Pending,
    /// 読み（かな）が変わって先読み（prefetch）を送った
    Reading,
    /// Backspace、Esc
    Edit,
}

#[derive(Default)]
struct Typing {
    pending: Group,
    reading: Group,
    edit: Group,
}

fn press(spec: &str) -> Result<ShellEvent, String> {
    scenario::parse_press(spec)
        .map(ShellEvent::Key)
        .map_err(|error| error.to_string())
}

fn activate() -> ShellEvent {
    ShellEvent::Activate {
        bundle_id: "com.apple.TextEdit".into(),
        secure: false,
    }
}

fn empty_text() -> ScenarioText {
    ScenarioText {
        surrounding: ScenarioSurrounding::unavailable(),
        selection: None,
    }
}

/// 入力中の台本を流す。1 打ごとの区分は、最初に一度流して（時間と確保は数えない）決める。
fn measure_typing(options: &Options) -> Result<Typing, String> {
    let mut typing = Typing::default();
    let classes: Vec<Vec<TypingClass>> = TYPING_SCRIPTS
        .iter()
        .map(|script| classify_typing(script))
        .collect::<Result<_, _>>()?;
    for _ in 0..options.repeat * TYPING_REPEAT_FACTOR {
        for (script, classes) in TYPING_SCRIPTS.iter().zip(&classes) {
            let log = Arc::new(Mutex::new(EngineLog::new(EngineScript::default())));
            let mut shell = new_shell("", Arc::clone(&log))?;
            let mut text = empty_text();
            std::hint::black_box(setup_activate_dispatch(&mut shell, activate(), &mut text));
            for (spec, class) in script.split_whitespace().zip(classes) {
                let event = press(spec)?;
                let heap_before = heap_totals();
                let started = Instant::now();
                let actions = match class {
                    TypingClass::Pending => typing_pending_dispatch(&mut shell, event, &mut text),
                    TypingClass::Reading => typing_reading_dispatch(&mut shell, event, &mut text),
                    TypingClass::Edit => typing_edit_dispatch(&mut shell, event, &mut text),
                };
                let elapsed = nanos(started.elapsed());
                let heap_after = heap_totals();
                std::hint::black_box(actions);
                let alloc = (heap_after.0 - heap_before.0, heap_after.1 - heap_before.1);
                match class {
                    TypingClass::Pending => typing.pending.push(false, elapsed, alloc),
                    TypingClass::Reading => typing.reading.push(false, elapsed, alloc),
                    TypingClass::Edit => typing.edit.push(false, elapsed, alloc),
                }
            }
        }
    }
    Ok(typing)
}

fn classify_typing(script: &str) -> Result<Vec<TypingClass>, String> {
    let log = Arc::new(Mutex::new(EngineLog::new(EngineScript::default())));
    let mut shell = new_shell("", Arc::clone(&log))?;
    let mut text = empty_text();
    shell.dispatch(activate(), &mut text);
    let mut classes = Vec::new();
    for spec in script.split_whitespace() {
        let prefetches = |log: &Mutex<EngineLog>| lock(log).counts.get("prefetch").copied();
        let before = prefetches(&log);
        shell.dispatch(press(spec)?, &mut text);
        classes.push(if matches!(spec, "backspace" | "escape") {
            TypingClass::Edit
        } else if prefetches(&log) != before {
            TypingClass::Reading
        } else {
            TypingClass::Pending
        });
    }
    Ok(classes)
}

macro_rules! typing_dispatch {
    ($name:ident, $tag:literal) => {
        /// 入力中の `dispatch`。区分ごとに別の関数にして、確保と CPU の記録で呼び出しの列に名前が残るようにする
        #[inline(never)]
        fn $name(
            shell: &mut Shell,
            event: ShellEvent,
            text: &mut ScenarioText,
        ) -> Vec<ikari_shell_core::ShellAction> {
            // 中身が同じ関数はリンカがまとめるので、違う値を通す
            std::hint::black_box($tag);
            std::hint::black_box(shell.dispatch(event, text))
        }
    };
}
typing_dispatch!(setup_activate_dispatch, "setup_activate");
typing_dispatch!(typing_pending_dispatch, "typing_pending");
typing_dispatch!(typing_reading_dispatch, "typing_reading");
typing_dispatch!(typing_edit_dispatch, "typing_edit");

/// 起動の `dispatch`（`Activate`）。確保と CPU の記録で、呼び出しの列に名前が残るよう、インライン化しない
/// （`cargo xtask perf alloc` と `perf profile` が、この名前で起動と定常を分ける）
#[inline(never)]
fn startup_dispatch(
    shell: &mut Shell,
    event: ShellEvent,
    text: &mut ScenarioText,
) -> Vec<ikari_shell_core::ShellAction> {
    // 末尾呼び出しにすると、呼び出しの列からこの関数の枠が消える。結果を `black_box` に通して避ける
    // 三つの関数の中身が同じだとリンカが一つにまとめ、名前が区別できなくなる。違う値を通して避ける
    std::hint::black_box("startup");
    std::hint::black_box(shell.dispatch(event, text))
}

/// 起動の直後の最初の 1 打の `dispatch`。上と同じ理由でインライン化しない
#[inline(never)]
fn first_dispatch(
    shell: &mut Shell,
    event: ShellEvent,
    text: &mut ScenarioText,
) -> Vec<ikari_shell_core::ShellAction> {
    // 三つの関数の中身が同じだとリンカが一つにまとめ、名前が区別できなくなる。違う値を通して避ける
    std::hint::black_box("first");
    std::hint::black_box(shell.dispatch(event, text))
}

/// 定常の `dispatch`（最初の 1 打のあとの、`Activate` 以外のイベント。打鍵ごと）。上と同じ理由でインライン化しない
#[inline(never)]
fn steady_dispatch(
    shell: &mut Shell,
    event: ShellEvent,
    text: &mut ScenarioText,
) -> Vec<ikari_shell_core::ShellAction> {
    // 末尾呼び出しにすると、呼び出しの列からこの関数の枠が消える。結果を `black_box` に通して避ける
    // 三つの関数の中身が同じだとリンカが一つにまとめ、名前が区別できなくなる。違う値を通して避ける
    std::hint::black_box("steady");
    std::hint::black_box(shell.dispatch(event, text))
}

/// 替え玉のエンジンの、同期の要求の数と、使った時間の合計
fn sync_state(log: &Mutex<EngineLog>) -> (usize, u64) {
    let guard = lock(log);
    (guard.sync_requests, guard.transport_ns)
}

fn measure_keys(options: &Options) -> Result<Measured, String> {
    let files =
        scenario::load_dir(&scenario::scenarios_dir()).map_err(|error| error.to_string())?;
    let mut samples = Vec::new();
    let mut without_transport = Vec::new();
    let mut allocs = Vec::new();
    let mut excluded = 0;
    let mut startup = Group::default();
    let mut first = Group::default();
    let mut steady = Group::default();
    let mut any_action = false;
    let mut records: Vec<KeyRecord> = Vec::new();
    for round in 0..options.repeat {
        let mut slot = 0;
        for (_, file) in &files {
            for scenario in &file.scenarios {
                let setup = file.defaults.resolve(&scenario.setup);
                let log = Arc::new(Mutex::new(EngineLog::new(setup.engine.clone())));
                let mut shell = new_shell(&setup.settings_tsv(), Arc::clone(&log))?;
                let mut text = ScenarioText {
                    surrounding: setup.surrounding.clone(),
                    selection: None,
                };
                // 最初の `Activate`（`Shell::new` の直後。エンジンへの hello、最初の `finish` のメニューなどを含む起動）。
                // P1 の従来の指標には入れず、`startup` にだけ数える
                let first_activate = {
                    let (before, _) = sync_state(&log);
                    let heap_before = heap_totals();
                    let started = Instant::now();
                    let actions = startup_dispatch(
                        &mut shell,
                        ShellEvent::Activate {
                            bundle_id: setup.client.bundle_id.clone(),
                            secure: false,
                        },
                        &mut text,
                    );
                    let elapsed = nanos(started.elapsed());
                    let heap_after = heap_totals();
                    std::hint::black_box(actions);
                    (
                        sync_state(&log).0 > before,
                        elapsed,
                        (heap_after.0 - heap_before.0, heap_after.1 - heap_before.1),
                    )
                };
                startup.push(first_activate.0, first_activate.1, first_activate.2);
                let mut keys_seen = 0_usize;
                for (step_index, step) in scenario.steps.iter().enumerate() {
                    if let Some(surrounding) = &step.surrounding {
                        text.surrounding = surrounding.clone();
                    }
                    text.selection = step.selection.clone();
                    let events = step
                        .events(setup.client.secure)
                        .map_err(|error| error.to_string())?;
                    for (event_index, event) in events.into_iter().enumerate() {
                        let (before, transport_before) = sync_state(&log);
                        let is_startup = matches!(event, ShellEvent::Activate { .. });
                        let label = if options.detail && round == 0 {
                            format!("{} #{step_index}.{event_index} {event:?}", scenario.name)
                        } else {
                            String::new()
                        };
                        let heap_before = heap_totals();
                        let started = Instant::now();
                        let is_first = !is_startup && keys_seen == 0;
                        keys_seen += usize::from(!is_startup);
                        let actions = if is_startup {
                            startup_dispatch(&mut shell, event, &mut text)
                        } else if is_first {
                            first_dispatch(&mut shell, event, &mut text)
                        } else {
                            steady_dispatch(&mut shell, event, &mut text)
                        };
                        let elapsed = nanos(started.elapsed());
                        let heap_after = heap_totals();
                        any_action |= !std::hint::black_box(actions).is_empty();
                        let guard = lock(&log);
                        let transport = guard.transport_ns - transport_before;
                        let is_sync = guard.sync_requests > before;
                        drop(guard);
                        if options.detail {
                            if round == 0 {
                                records.push(KeyRecord {
                                    label,
                                    total: Vec::new(),
                                    transport: Vec::new(),
                                    blocks: Vec::new(),
                                });
                            }
                            if !is_sync && let Some(record) = records.get_mut(slot) {
                                record.total.push(elapsed);
                                record.transport.push(transport);
                                record.blocks.push(heap_after.0 - heap_before.0);
                            }
                            slot += 1;
                        }
                        let alloc = (heap_after.0 - heap_before.0, heap_after.1 - heap_before.1);
                        if is_startup {
                            startup.push(is_sync, elapsed, alloc);
                        } else if is_first {
                            first.push(is_sync, elapsed, alloc);
                        } else {
                            steady.push(is_sync, elapsed, alloc);
                        }
                        if is_sync {
                            excluded += 1;
                        } else {
                            samples.push(elapsed);
                            allocs
                                .push((heap_after.0 - heap_before.0, heap_after.1 - heap_before.1));
                            without_transport.push(elapsed.saturating_sub(transport));
                        }
                    }
                }
            }
        }
    }
    if options.detail {
        records.retain(|record| !record.total.is_empty());
        print_detail(&mut records, options.metric == "alloc");
    }
    Ok(Measured {
        samples,
        without_transport,
        excluded,
        stub: !any_action,
        allocs,
        startup,
        first,
        steady,
    })
}

// ---------------------------------------------------------------- P3

fn measure_startup(options: &Options) -> Result<Measured, String> {
    let mut samples = Vec::with_capacity(options.repeat);
    let mut stub = true;
    for _ in 0..options.repeat {
        let log = Arc::new(Mutex::new(EngineLog::new(EngineScript::default())));
        lock(&log).emoji_entries = options.shortcode_entries;
        let mut text = ScenarioText {
            surrounding: ScenarioSurrounding::unavailable(),
            selection: None,
        };
        let started = Instant::now();
        let mut shell = new_shell(STARTUP_SETTINGS, Arc::clone(&log))?;
        let actions = shell.dispatch(
            ShellEvent::Activate {
                bundle_id: "com.apple.TextEdit".into(),
                secure: false,
            },
            &mut text,
        );
        let elapsed = started.elapsed();
        std::hint::black_box(actions);
        // 初期化がエンジンへ何か送った（hello など）なら、stub ではない
        stub &= lock(&log).total_requests == 0;
        samples.push(nanos(elapsed));
    }
    Ok(Measured {
        samples,
        without_transport: Vec::new(),
        excluded: 0,
        stub,
        allocs: Vec::new(),
        startup: Group::default(),
        first: Group::default(),
        steady: Group::default(),
    })
}

fn new_shell(settings: &str, log: Arc<Mutex<EngineLog>>) -> Result<Shell, String> {
    let config = ShellConfig {
        shell_version: SHELL_VERSION.into(),
        settings: SettingsSource::Inline(settings.to_owned()),
        resources_dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("../macos/Resources"),
    };
    Shell::new(
        config,
        Box::new(ScriptedTransport { log }),
        Arc::new(FixedClock),
    )
    .map_err(|error| format!("Shell::new: {error}"))
}

// ---------------------------------------------------------------- ホストの替え玉（tests/scenarios.rs と同じ振る舞い）

struct ScenarioText {
    surrounding: ScenarioSurrounding,
    selection: Option<SelectedText>,
}

impl TextAccess for ScenarioText {
    fn surrounding(&mut self) -> Option<SurroundingText> {
        self.surrounding.to_host()
    }

    fn selection(&mut self) -> Option<SelectedText> {
        self.selection.clone()
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Duration {
        Duration::ZERO
    }
}

/// 替え玉のエンジンの状態: 台本、種類ごとの回数、同期の要求の数。
struct EngineLog {
    script: EngineScript,
    counts: HashMap<&'static str, u32>,
    sync_requests: usize,
    total_requests: usize,
    emoji_entries: usize,
    /// 替え玉のエンジンの中で使った時間の合計（ナノ秒）
    transport_ns: u64,
}

impl EngineLog {
    fn new(script: EngineScript) -> Self {
        Self {
            script,
            counts: HashMap::new(),
            sync_requests: 0,
            total_requests: 0,
            emoji_entries: 0,
            transport_ns: 0,
        }
    }

    /// 要求を数え、台本に失敗があればその誤りを返す。
    fn record(&mut self, kind: &'static str) -> Option<EngineError> {
        self.total_requests += 1;
        if SYNC_REQUESTS.contains(&kind) {
            self.sync_requests += 1;
        }
        let nth = self.counts.entry(kind).or_insert(0);
        *nth += 1;
        self.script.failure_for(kind, *nth)
    }
}

fn lock(log: &Mutex<EngineLog>) -> MutexGuard<'_, EngineLog> {
    log.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 要求の種類（`Envelope.body` の欄の名前）。台本の失敗の照合と、同期の要求の判定に使う。
fn kind_of(body: &Body) -> &'static str {
    match body {
        Body::Hello(_) => "hello",
        Body::BeginSession(_) => "begin_session",
        Body::EndSession(_) => "end_session",
        Body::Prefetch(_) => "prefetch",
        Body::Convert(_) => "convert",
        Body::Page(_) => "page",
        Body::Commit(_) => "commit",
        Body::CommitText(_) => "commit_text",
        Body::Cancel(_) => "cancel",
        Body::Shutdown(_) => "shutdown",
        Body::SegmentPage(_) => "segment_page",
        Body::ResizeSegment(_) => "resize_segment",
        Body::CommitSegments(_) => "commit_segments",
        Body::Reconvert(_) => "reconvert",
        Body::PostalLookup(_) => "postal_lookup",
        Body::Diag(_) => "diag",
        Body::EmojiTable(_) => "emoji_table",
        _ => "relay",
    }
}

/// `Envelope` を読み、台本どおりに返すトランスポート。
struct ScriptedTransport {
    log: Arc<Mutex<EngineLog>>,
}

impl EngineTransport for ScriptedTransport {
    fn connect(&self) -> Result<TransportHandle, TransportError> {
        Ok(TransportHandle(1))
    }

    fn request(
        &self,
        _handle: TransportHandle,
        bytes: &[u8],
        _timeout: Duration,
    ) -> Result<Vec<u8>, TransportError> {
        let started = Instant::now();
        let result = self.serve(bytes);
        lock(&self.log).transport_ns += nanos(started.elapsed());
        result
    }

    fn post(&self, _handle: TransportHandle, bytes: &[u8]) {
        let started = Instant::now();
        if let Ok(v1::Envelope {
            body: Some(body), ..
        }) = v1::Envelope::decode(bytes)
        {
            lock(&self.log).record(kind_of(&body));
        }
        lock(&self.log).transport_ns += nanos(started.elapsed());
    }

    fn close(&self, _handle: TransportHandle) {}

    fn pid(&self, _handle: TransportHandle) -> Option<i32> {
        Some(1)
    }
}

impl ScriptedTransport {
    fn serve(&self, bytes: &[u8]) -> Result<Vec<u8>, TransportError> {
        let envelope = v1::Envelope::decode(bytes).map_err(|_| TransportError::Unavailable)?;
        let body = envelope.body.ok_or(TransportError::Unavailable)?;
        let mut log = lock(&self.log);
        let failure = log.record(kind_of(&body));
        let reply = match failure {
            Some(EngineError::Unavailable) => return Err(TransportError::Unavailable),
            Some(EngineError::Timeout) => return Err(TransportError::Timeout),
            Some(EngineError::InvalidReply) => return Ok(vec![0xff, 0xff, 0xff]),
            Some(EngineError::Engine(code)) => error_reply(code),
            None => answer(&log, &body),
        };
        Ok(v1::Envelope {
            schema_version: ikari_proto::SCHEMA_VERSION,
            request_id: envelope.request_id,
            body: Some(reply),
        }
        .encode_to_vec())
    }
}

/// 台本どおりの返事。
fn answer(log: &EngineLog, body: &Body) -> Body {
    match body {
        Body::Hello(_) => Body::HelloReply(v1::HelloReply {
            engine_version: "bench".into(),
            dict_label: "bench".into(),
            lm: v1::LmState::LmReady as i32,
        }),
        Body::Convert(_) => match log.script.pages.first() {
            Some(page) => Body::ConvertReply(convert_reply(page)),
            None => error_reply(EngineErrorCode::BadRequest),
        },
        Body::Page(request) => match log.script.pages.get(request.index as usize) {
            Some(page) => Body::PageReply(convert_reply(page)),
            None => error_reply(EngineErrorCode::BadRequest),
        },
        Body::SegmentPage(_) | Body::ResizeSegment(_) | Body::Reconvert(_) => {
            error_reply(EngineErrorCode::BadRequest)
        }
        Body::PostalLookup(_) => Body::PostalLookupReply(v1::PostalLookupReply::default()),
        Body::EmojiTable(_) => Body::EmojiTableReply(v1::EmojiTableReply {
            entries: (0..log.emoji_entries)
                .map(|index| v1::EmojiEntry {
                    name: format!("name{index:06}"),
                    emoji: "😀".into(),
                })
                .collect(),
            data_label: "bench".into(),
        }),
        _ => Body::Ack(v1::Ack {}),
    }
}

fn convert_reply(page: &ScriptPage) -> v1::ConvertReply {
    v1::ConvertReply {
        conversion_id: page.conversion_id,
        page_index: page.page_index,
        candidates: page
            .candidates
            .iter()
            .map(|candidate| v1::Candidate {
                surface: candidate.surface.clone(),
                annotation: candidate.annotation.clone(),
                kind: candidate_kind(candidate.kind) as i32,
            })
            .collect(),
        has_next: page.has_next,
        lm_applied: page.lm_applied,
        confident: page.confident,
        ..Default::default()
    }
}

fn candidate_kind(kind: CandidateKind) -> v1::CandidateKind {
    match kind {
        CandidateKind::Unspecified => v1::CandidateKind::Unspecified,
        CandidateKind::Dictionary => v1::CandidateKind::Dictionary,
        CandidateKind::Hiragana => v1::CandidateKind::Hiragana,
        CandidateKind::Katakana => v1::CandidateKind::Katakana,
        CandidateKind::Rewriter => v1::CandidateKind::Rewriter,
        CandidateKind::Literal => v1::CandidateKind::Literal,
    }
}

fn error_reply(code: EngineErrorCode) -> Body {
    let code = match code {
        EngineErrorCode::Unspecified => v1::ErrorCode::Unspecified,
        EngineErrorCode::SchemaMismatch => v1::ErrorCode::SchemaMismatch,
        EngineErrorCode::BadRequest => v1::ErrorCode::BadRequest,
        EngineErrorCode::UnknownSession => v1::ErrorCode::UnknownSession,
        EngineErrorCode::UnknownConversion => v1::ErrorCode::UnknownConversion,
        EngineErrorCode::EngineNotReady => v1::ErrorCode::EngineNotReady,
        EngineErrorCode::Internal => v1::ErrorCode::Internal,
        EngineErrorCode::StaleLayout => v1::ErrorCode::StaleLayout,
    };
    Body::Error(v1::ErrorReply {
        code: code as i32,
        message: String::new(),
    })
}
