//! `ikari-npu-probe`: Intel NPU で zenz を動かせるかを測る道具（ADR 0075 §4）。製品には入らない。
//!
//!   ikari-npu-probe                 全部を測って、画面に要約、フォルダに report.txt（ダブルクリック可）
//!   ikari-npu-probe --list          装置の一覧（OpenVINO と llama.cpp。モデルは読まない）
//!   ikari-npu-probe --cleanup       AppContainer のプロファイルと cache\ を消す
//!   ikari-npu-probe --no-appcontainer  AppContainer の確認を飛ばす
//!
//! 装置ごとに自分を起動し直して測る（`--worker`。失敗しても次の行に進む）。通信しない。書くのはツールの
//! フォルダの中（report.txt、cache\）だけ。

mod device;
mod input;
mod llama_dev;
mod mem;
mod ov;
mod report;
mod sandbox;
mod score;
mod stats;
mod sysinfo;
mod worker;

use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::Parser;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use device::{Device, OvTarget, Precision};
use report::{DeviceRun, Header, Run, SandboxRow};
use sandbox::Confine;
use worker::{Job, Mode, WorkerResult};

/// モデルを置くフォルダ（ツールのフォルダの下）
pub const MODELS_DIR: &str = "models";
/// GGUF の名前（今の製品と同じ）
pub const GGUF_NAME: &str = "zenz-v3.2-small-Q5_K_M.gguf";
/// OpenVINO のキャッシュとトークン列を置くフォルダ（実行のはじめに空にする）
const CACHE_DIR: &str = "cache";
const REPORT_FILE: &str = "report.txt";
/// 子が結果を書く行の印
const RESULT_PREFIX: &str = "IKARI_NPU_PROBE_RESULT ";
/// AppContainer の許可を足してよいフォルダの名前の先頭（デスクトップなどに直に展開したときは足さない）
const FOLDER_PREFIX: &str = "ikari-npu-probe";
const DEFAULT_REPS: usize = 20;
const DEFAULT_BUDGET_SECS: u64 = 120;
/// 全件を測る子の待ちの上限
const FULL_TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// 初回だけ・1 件だけ・一覧の子の待ちの上限（NPU のコンパイルが長い見込み）
const SHORT_TIMEOUT: Duration = Duration::from_secs(8 * 60);
/// 伏せる語の最短の長さ（短い利用者名で文を壊さない）
const MIN_REDACT_LEN: usize = 3;
/// ハッシュの先頭の桁数（ADR 0075 §4.4）
const SHA_PREFIX: usize = 12;
/// モデルの sha256 の一覧（lane/npu-adr の変換が出す。models\ に置く）
const SUMS_FILE: &str = "SHA256SUMS";

/// 自分の exe のフォルダ
pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
}

#[derive(Parser, Debug)]
#[command(
    name = "ikari-npu-probe",
    version,
    about = "Intel NPU で zenz（LM）を動かせるかを測る（ADR 0075）。通信しない"
)]
struct Cli {
    /// 装置の一覧を出して終わる（モデルは読まない）
    #[arg(long)]
    list: bool,
    /// AppContainer のプロファイルと cache\ を消して終わる
    #[arg(long)]
    cleanup: bool,
    /// AppContainer の確認を飛ばす（プロファイルを作らない）
    #[arg(long)]
    no_appcontainer: bool,
    /// 測る装置を絞る（カンマ区切り。例: llama-cpu,ov-npu-int8）
    #[arg(long)]
    only: Option<String>,
    /// 暖機のあと全件を流す回数
    #[arg(long, default_value_t = DEFAULT_REPS)]
    reps: usize,
    /// 1 つの装置で回数を流す時間の上限（秒）
    #[arg(long, default_value_t = DEFAULT_BUDGET_SECS)]
    budget_secs: u64,
    /// ツールのフォルダ（省略時は exe のフォルダ）
    #[arg(long)]
    dir: Option<PathBuf>,
    /// 入力（省略時は埋めたもの）
    #[arg(long)]
    inputs: Option<PathBuf>,
    /// OpenVINO の C API の DLL（省略時は exe の隣）
    #[arg(long)]
    ov_lib: Option<PathBuf>,
    /// 終わったあと Enter を待たない
    #[arg(long)]
    no_wait: bool,
    #[arg(long, hide = true)]
    worker: Option<String>,
    #[arg(long, hide = true)]
    batch: Option<usize>,
    #[arg(long, hide = true)]
    mode: Option<String>,
    #[arg(long, hide = true)]
    cache: Option<PathBuf>,
}

/// 一覧の子の結果
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct InfoResult {
    ov: Option<ov::OvInfo>,
    ov_error: Option<String>,
    llama_devices: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Some(kind) = cli.worker.clone() {
        return worker_main(&cli, &kind);
    }
    let Some(dir) = cli.dir.clone().or_else(exe_dir) else {
        eprintln!("exe の場所が分からない");
        return ExitCode::FAILURE;
    };
    let code = if cli.cleanup {
        cleanup(&dir)
    } else if cli.list {
        list(&cli, &dir)
    } else {
        match run_all(&cli, &dir) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("止まりました: {e}");
                ExitCode::FAILURE
            }
        }
    };
    if !cli.no_wait && !cli.list && !cli.cleanup {
        eprint!("\nEnter で閉じる");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
    }
    code
}

fn load_inputs(path: Option<&Path>) -> Result<Vec<input::Item>, String> {
    match path {
        Some(p) => {
            let text = std::fs::read_to_string(p).map_err(|e| format!("入力を読めない: {e}"))?;
            input::parse(&text)
        }
        None => input::parse(input::EMBEDDED),
    }
}

fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".into())
}

fn print_result<T: Serialize>(value: &T) -> ExitCode {
    match serde_json::to_string(value) {
        Ok(json) => {
            println!("{RESULT_PREFIX}{json}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("結果を書けない: {e}");
            ExitCode::FAILURE
        }
    }
}

fn worker_main(cli: &Cli, kind: &str) -> ExitCode {
    let dir = cli.dir.clone().or_else(exe_dir).unwrap_or_default();
    if kind == "info" {
        let info = std::panic::catch_unwind(|| {
            let (ov, ov_error) = match ov::info(cli.ov_lib.as_deref()) {
                Ok(i) => (Some(i), None),
                Err(e) => (None, Some(e)),
            };
            InfoResult {
                ov,
                ov_error,
                llama_devices: llama_dev::devices(),
            }
        })
        .unwrap_or_else(|p| InfoResult {
            ov_error: Some(format!("panic: {}", panic_message(p.as_ref()))),
            ..InfoResult::default()
        });
        return print_result(&info);
    }
    let device: Device = match kind.parse() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let mode = cli
        .mode
        .as_deref()
        .and_then(Mode::parse)
        .unwrap_or(Mode::Full);
    let job = Job {
        device,
        batch: cli.batch,
        mode,
        reps: cli.reps,
        budget: Duration::from_secs(cli.budget_secs),
        dir,
        inputs: cli.inputs.clone(),
        cache: cli.cache.clone(),
        ov_lib: cli.ov_lib.clone(),
    };
    let result = match load_inputs(job.inputs.as_deref()) {
        Ok(items) => std::panic::catch_unwind(|| worker::run(&job, &items)).unwrap_or_else(|p| {
            WorkerResult::failed(
                device,
                job.batch,
                mode,
                format!("panic: {}", panic_message(p.as_ref())),
            )
        }),
        Err(e) => WorkerResult::failed(device, job.batch, mode, e),
    };
    print_result(&result)
}

fn job_args(job: &Job) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec![
        "--worker".into(),
        job.device.to_string().into(),
        "--mode".into(),
        job.mode.name().into(),
        "--reps".into(),
        job.reps.to_string().into(),
        "--budget-secs".into(),
        job.budget.as_secs().to_string().into(),
        "--dir".into(),
        job.dir.clone().into(),
    ];
    let mut push = |flag: &str, v: Option<OsString>| {
        if let Some(v) = v {
            a.push(flag.into());
            a.push(v);
        }
    };
    push("--batch", job.batch.map(|b| b.to_string().into()));
    push("--inputs", job.inputs.clone().map(Into::into));
    push("--cache", job.cache.clone().map(Into::into));
    push("--ov-lib", job.ov_lib.clone().map(Into::into));
    a
}

/// 子の標準出力の、印のついた最後の行
fn parse_result<T: for<'de> Deserialize<'de>>(stdout: &[u8]) -> Option<T> {
    String::from_utf8_lossy(stdout)
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix(RESULT_PREFIX))
        .and_then(|json| serde_json::from_str(json).ok())
}

fn spawn_raw(
    exe: &Path,
    args: &[OsString],
    confine: &Confine<'_>,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let c = sandbox::run(exe, args, confine, timeout)
        .map_err(|e| format!("起動できない（{}）: {e}", confine.name()))?;
    if c.timed_out {
        return Err(format!("時間切れ（{} 分）", timeout.as_secs() / 60));
    }
    if parse_result::<serde_json::Value>(&c.stdout).is_none() {
        return Err(format!(
            "子が結果を返さなかった（終了コード {}）",
            c.code
                .map_or("?".into(), |c| format!("{c} / 0x{:08x}", c as u32))
        ));
    }
    Ok(c.stdout)
}

/// 子で 1 つ測る。どう失敗しても結果を返す
fn spawn_worker(exe: &Path, job: &Job, confine: &Confine<'_>, redact: &Redactor) -> WorkerResult {
    let timeout = if job.mode == Mode::Full {
        FULL_TIMEOUT
    } else {
        SHORT_TIMEOUT
    };
    let mut r = match spawn_raw(exe, &job_args(job), confine, timeout) {
        Ok(out) => parse_result(&out).unwrap_or_else(|| {
            WorkerResult::failed(job.device, job.batch, job.mode, "結果が壊れている".into())
        }),
        Err(e) => WorkerResult::failed(job.device, job.batch, job.mode, e),
    };
    for s in [&mut r.error, &mut r.note, &mut r.ran_on]
        .into_iter()
        .flatten()
    {
        *s = redact.apply(s);
    }
    r
}

/// 報告からパス、利用者名、機械名を伏せる（ADR 0075 §4.4）
struct Redactor(Vec<(String, &'static str)>);

impl Redactor {
    fn new(dir: &Path) -> Self {
        let mut pairs = vec![(dir.display().to_string(), "<dir>")];
        for (var, mark) in [
            ("USERPROFILE", "<home>"),
            ("HOME", "<home>"),
            ("USERNAME", "<user>"),
            ("USER", "<user>"),
            ("COMPUTERNAME", "<pc>"),
        ] {
            if let Ok(v) = std::env::var(var) {
                pairs.push((v, mark));
            }
        }
        pairs.retain(|(v, _)| v.chars().count() >= MIN_REDACT_LEN);
        // 長いものから（パスの中の利用者名より先にパスを伏せる）
        pairs.sort_by_key(|(v, _)| std::cmp::Reverse(v.len()));
        Redactor(pairs)
    }

    fn apply(&self, s: &str) -> String {
        let mut out = s.to_owned();
        for (v, mark) in &self.0 {
            out = out.replace(v.as_str(), mark);
            let slashed = v.replace('\\', "/");
            if slashed != *v {
                out = out.replace(slashed.as_str(), mark);
            }
        }
        out
    }
}

fn progress(msg: &str) {
    eprintln!("… {msg}");
}

fn sha256_hex(path: &Path) -> Option<String> {
    const CHUNK: usize = 1 << 20;
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// `SHA256SUMS`（`<hex>  <名前>` の行）を読む。名前はパスの最後だけ
fn read_sums(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let (hex, name) = l.trim().split_once(char::is_whitespace)?;
            let name = name.trim().trim_start_matches('*');
            let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
            Some((name.to_owned(), hex.to_ascii_lowercase()))
        })
        .collect()
}

/// models\ の GGUF と IR の sha256 の先頭 12 桁。`SHA256SUMS` があれば、全桁で照らした結果を名前の後ろに付ける
/// （契約: IR は作り直すたびに sha256 が変わるので、zip の中の `SHA256SUMS` を正とする）
fn model_hashes(dir: &Path) -> Vec<(String, String)> {
    let models = dir.join(MODELS_DIR);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&models)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|f| {
                    f.extension()
                        .is_some_and(|x| x == "xml" || x == "bin" || x == "gguf")
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let sums = std::fs::read_to_string(models.join(SUMS_FILE))
        .map(|t| read_sums(&t))
        .ok();
    files
        .into_iter()
        .filter_map(|f| {
            let name = f.file_name()?.to_string_lossy().into_owned();
            let hex = sha256_hex(&f)?;
            let check = match sums
                .as_ref()
                .map(|s| s.iter().find(|(n, _)| *n == name).map(|(_, h)| *h == hex))
            {
                None => "",
                Some(None) => "（SHA256SUMS に無い）",
                Some(Some(true)) => "（SHA256SUMS と一致）",
                Some(Some(false)) => "（SHA256SUMS と違う）",
            };
            Some((format!("{name}{check}"), hex[..SHA_PREFIX].to_owned()))
        })
        .collect()
}

fn selected_devices(only: Option<&str>) -> Result<Vec<Device>, String> {
    let Some(only) = only else {
        return Ok(Device::all());
    };
    let wanted: Vec<Device> = only
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::parse)
        .collect::<Result<_, _>>()?;
    // 基準（llama-cpu）はトークン列を出すので、いつも先に
    Ok(Device::all()
        .into_iter()
        .filter(|d| wanted.contains(d))
        .collect())
}

fn info(exe: &Path, dir: &Path, ov_lib: Option<&Path>) -> InfoResult {
    let mut args: Vec<OsString> =
        vec!["--worker".into(), "info".into(), "--dir".into(), dir.into()];
    if let Some(l) = ov_lib {
        args.push("--ov-lib".into());
        args.push(l.into());
    }
    match spawn_raw(exe, &args, &Confine::Normal, SHORT_TIMEOUT) {
        Ok(out) => parse_result(&out).unwrap_or_default(),
        Err(e) => InfoResult {
            ov_error: Some(e),
            ..InfoResult::default()
        },
    }
}

fn list(cli: &Cli, dir: &Path) -> ExitCode {
    let Ok(exe) = std::env::current_exe() else {
        return ExitCode::FAILURE;
    };
    let redact = Redactor::new(dir);
    let i = info(&exe, dir, cli.ov_lib.as_deref());
    match (&i.ov, &i.ov_error) {
        (Some(o), _) => {
            println!("OpenVINO {}", o.version);
            for (name, full, drv) in &o.devices {
                let drv = drv
                    .as_ref()
                    .map_or(String::new(), |d| format!("（ドライバ {d}）"));
                println!("  {name}: {full}{drv}");
            }
        }
        (None, e) => println!(
            "OpenVINO: 読めない（{}）",
            redact.apply(e.as_deref().unwrap_or("?"))
        ),
    }
    println!(
        "llama.cpp: {}",
        i.llama_devices.as_deref().unwrap_or("このビルドに無い")
    );
    println!(
        "GGUF: {}",
        if worker::gguf_path(dir).is_some() {
            "あり"
        } else {
            "無い"
        }
    );
    for p in Precision::ALL {
        for b in score::BATCH_SIZES {
            let (xml, _) = ov::ir_files(dir, p, b);
            println!(
                "IR {}: {}",
                ov::ir_name(p, b),
                if xml.is_file() { "あり" } else { "無い" }
            );
        }
    }
    ExitCode::SUCCESS
}

fn cleanup(dir: &Path) -> ExitCode {
    let mut code = ExitCode::SUCCESS;
    match sandbox::cleanup(dir) {
        Ok(true) => println!("AppContainer のプロファイルを消しました（無ければ何もしていません）"),
        Ok(false) => println!("AppContainer のプロファイルは残っていません"),
        Err(e) => {
            println!("AppContainer のプロファイルを消せません: {e}");
            code = ExitCode::FAILURE;
        }
    }
    let cache = dir.join(CACHE_DIR);
    if cache.exists() {
        match std::fs::remove_dir_all(&cache) {
            Ok(()) => println!("{CACHE_DIR}\\ を消しました"),
            Err(e) => {
                println!("{CACHE_DIR}\\ を消せません: {e}");
                code = ExitCode::FAILURE;
            }
        }
    }
    code
}

/// 全部を測る
fn run_all(cli: &Cli, dir: &Path) -> Result<(), String> {
    let started = Instant::now();
    let exe = std::env::current_exe().map_err(|e| format!("exe の場所: {e}"))?;
    let items = load_inputs(cli.inputs.as_deref())?;
    let devices = selected_devices(cli.only.as_deref())?;
    let redact = Redactor::new(dir);
    let cache = dir.join(CACHE_DIR);
    if cache.exists() {
        std::fs::remove_dir_all(&cache)
            .map_err(|e| format!("{CACHE_DIR}\\ を空にできない: {e}"))?;
    }
    std::fs::create_dir_all(&cache).map_err(|e| format!("{CACHE_DIR}\\ を作れない: {e}"))?;

    progress("装置を調べています");
    let i = info(&exe, dir, cli.ov_lib.as_deref());
    let mut run = Run {
        header: Header {
            tool: format!(
                "ikari-npu-probe {} ({})",
                env!("CARGO_PKG_VERSION"),
                option_env!("IKARI_NPU_PROBE_COMMIT").unwrap_or("dev")
            ),
            os: sysinfo::os_version(),
            cpu: sysinfo::cpu_name(),
            ov_error: i.ov_error.as_deref().map(|e| redact.apply(e)),
            ov: i.ov.clone(),
            llama_devices: i.llama_devices.clone(),
            models: model_hashes(dir),
            inputs: items.len(),
            candidates: items.iter().map(|i| i.candidates.len()).sum(),
            token_match: None,
        },
        reference: items.iter().map(|i| i.nll_llama_cpu.clone()).collect(),
        ..Run::default()
    };
    let ov_devices: Option<Vec<String>> =
        i.ov.as_ref()
            .map(|o| o.devices.iter().map(|(n, _, _)| n.clone()).collect());

    let base_job = Job {
        device: Device::LlamaCpu,
        batch: None,
        mode: Mode::Full,
        reps: cli.reps,
        budget: Duration::from_secs(cli.budget_secs),
        dir: dir.to_path_buf(),
        inputs: cli.inputs.clone(),
        cache: None,
        ov_lib: cli.ov_lib.clone(),
    };
    for device in devices {
        let mut d = DeviceRun {
            device,
            full: Vec::new(),
            first: Vec::new(),
            skipped: None,
        };
        match device {
            Device::LlamaOvNpu => {
                d.skipped = Some("作っていない（後回し。ADR 0075 §4.3 の A）".into());
            }
            Device::LlamaCpu | Device::LlamaVulkan => {
                if worker::gguf_path(dir).is_none() {
                    d.skipped = Some(format!("GGUF が無い（{MODELS_DIR}\\{GGUF_NAME}）"));
                } else {
                    progress(&format!("{device} を測っています"));
                    let job = Job {
                        device,
                        ..base_job.clone()
                    };
                    let r = spawn_worker(&exe, &job, &Confine::Normal, &redact);
                    if let Some(t) = r.tokens.as_ref().filter(|_| device == Device::LlamaCpu) {
                        run.header.token_match = Some(token_match(t, &items));
                    }
                    d.full.push(r);
                }
            }
            Device::Ov(target, precision) => {
                let batches: Vec<usize> = score::BATCH_SIZES
                    .into_iter()
                    .filter(|&b| ov::ir_files(dir, precision, b).0.is_file())
                    .collect();
                if batches.is_empty() {
                    d.skipped = Some(format!(
                        "IR が無い（{MODELS_DIR}\\{}）",
                        ov::ir_name(precision, 1)
                    ));
                } else if let Some(list) = ov_devices
                    .as_ref()
                    .filter(|l| !l.iter().any(|n| n.starts_with(target.ov_name())))
                {
                    d.skipped = Some(format!("装置が見えない（OpenVINO: {}）", list.join(", ")));
                } else {
                    for &b in &batches {
                        let job = Job {
                            device,
                            batch: Some(b),
                            cache: Some(cache.join(format!("{device}-b{b}"))),
                            ..base_job.clone()
                        };
                        progress(&format!(
                            "{device} B{b} を測っています（初回はコンパイル込み）"
                        ));
                        let r = spawn_worker(&exe, &job, &Confine::Normal, &redact);
                        let ok = r.ok;
                        d.full.push(r);
                        if ok {
                            progress(&format!("{device} B{b} のキャッシュありの初回"));
                            let first = Job {
                                mode: Mode::First,
                                ..job
                            };
                            d.first
                                .push(spawn_worker(&exe, &first, &Confine::Normal, &redact));
                        }
                    }
                }
            }
        }
        run.devices.push(d);
    }

    sandbox_checks(&mut run, &exe, dir, cli, &base_job, &redact);
    run.elapsed = started.elapsed();

    let summary = report::summary(&run);
    let mut text = report::report(&run);
    text.push_str("\n== 要約（画面と同じ）==\n");
    for l in &summary {
        text.push_str(l);
        text.push('\n');
    }
    let report_path = dir.join(REPORT_FILE);
    let written = std::fs::write(&report_path, &text);
    println!();
    println!("==== 要約（この画面を写す）====");
    for l in &summary {
        println!("{l}");
    }
    match written {
        Ok(()) => println!("（詳細は {REPORT_FILE}。消すときは --cleanup のあとフォルダごと削除）"),
        Err(e) => println!("（{REPORT_FILE} を書けない: {e}）"),
    }
    Ok(())
}

/// llama.cpp のトークン列と、入力（fixture）のトークン列の一致（一致した件、比べた件）
fn token_match(t: &score::TokenSet, items: &[input::Item]) -> (usize, usize) {
    let same = t
        .items
        .iter()
        .zip(items)
        .filter(|(tok, item)| {
            tok.prompt == item.prompt_tokens
                && tok.candidates == item.candidate_tokens
                && t.eos == score::END_TOKEN
        })
        .count();
    (same, items.len())
}

/// 檻の中で装置に触れるか（ADR 0075 §6）。ふつうに動いた装置だけを、1 件ずつ
fn sandbox_checks(run: &mut Run, exe: &Path, dir: &Path, cli: &Cli, base: &Job, redact: &Redactor) {
    if !cfg!(windows) {
        run.sandbox_notes.push("檻の確認は Windows だけ".into());
        return;
    }
    let mut targets = Vec::new();
    for t in [Device::LlamaCpu, Device::LlamaVulkan] {
        if run.get(t).and_then(DeviceRun::best).is_some() {
            targets.push((t, None));
        }
    }
    for target in OvTarget::ALL {
        let ran = Precision::ALL.into_iter().find_map(|p| {
            let d = Device::Ov(target, p);
            run.get(d).and_then(DeviceRun::best).map(|r| (d, r.batch))
        });
        targets.extend(ran);
    }
    if targets.is_empty() {
        run.sandbox_notes
            .push("ふつうに動いた装置が無いので、檻の確認を飛ばした".into());
        return;
    }
    let jobs: Vec<Job> = targets
        .iter()
        .map(|&(device, batch)| Job {
            device,
            batch,
            mode: Mode::Quick,
            cache: None,
            ..base.clone()
        })
        .collect();
    let check = |confine: &Confine<'_>, rows: &mut Vec<SandboxRow>| {
        for job in &jobs {
            progress(&format!("{} で {} を 1 件", confine.name(), job.device));
            let r = spawn_worker(exe, job, confine, redact);
            rows.push(SandboxRow {
                confine: confine.name(),
                device: job.device,
                ok: r.ok,
                reason: r.error,
            });
        }
    };
    let mut rows = Vec::new();
    check(&Confine::LowIntegrity, &mut rows);
    let folder_ok = dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with(FOLDER_PREFIX));
    if cli.no_appcontainer {
        run.sandbox_notes
            .push("AppContainer は --no-appcontainer で飛ばした".into());
    } else if !folder_ok {
        run.sandbox_notes.push(format!(
            "AppContainer を飛ばした（フォルダ名が {FOLDER_PREFIX} で始まらない。許可を足すフォルダを限るため）"
        ));
    } else {
        match sandbox::Profile::create(dir) {
            Ok(profile) => {
                check(&Confine::AppContainer(&profile), &mut rows);
                if let Err(e) = profile.remove() {
                    run.sandbox_notes.push(format!(
                        "AppContainer のプロファイルを消せなかった（--cleanup を流す）: {}",
                        redact.apply(&e.to_string())
                    ));
                }
            }
            Err(e) => run.sandbox_notes.push(format!(
                "AppContainer のプロファイルを作れない: {}",
                redact.apply(&e.to_string())
            )),
        }
    }
    run.sandbox = rows;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redactor_hides_dir_and_names() {
        let r = Redactor(vec![
            (r"C:\Users\someone\Desktop\ikari-npu-probe".into(), "<dir>"),
            ("someone".into(), "<user>"),
        ]);
        assert_eq!(
            r.apply(r"cannot open C:\Users\someone\Desktop\ikari-npu-probe\models\x.xml"),
            r"cannot open <dir>\models\x.xml"
        );
        assert_eq!(
            r.apply("C:/Users/someone/Desktop/ikari-npu-probe/a"),
            "<dir>/a"
        );
        assert_eq!(r.apply("owner someone"), "owner <user>");
    }

    #[test]
    fn result_line_is_found_after_noise() {
        let out = format!(
            "noise\n{RESULT_PREFIX}{{\"ov\":null,\"ov_error\":\"x\",\"llama_devices\":null}}\nmore\n"
        );
        let r: InfoResult = parse_result(out.as_bytes()).unwrap();
        assert_eq!(r.ov_error.as_deref(), Some("x"));
        assert!(parse_result::<InfoResult>(b"nothing").is_none());
    }

    #[test]
    fn only_keeps_canonical_order() {
        let d = selected_devices(Some("ov-npu-int8, llama-cpu")).unwrap();
        assert_eq!(
            d,
            [Device::LlamaCpu, Device::Ov(OvTarget::Npu, Precision::Int8)]
        );
        assert!(selected_devices(Some("gpu")).is_err());
    }

    #[test]
    fn job_args_round_trip_through_clap() {
        let job = Job {
            device: Device::Ov(OvTarget::Gpu, Precision::Fp16),
            batch: Some(8),
            mode: Mode::First,
            reps: 3,
            budget: Duration::from_secs(9),
            dir: PathBuf::from("/tmp/ikari-npu-probe"),
            inputs: None,
            cache: Some(PathBuf::from("/tmp/c")),
            ov_lib: None,
        };
        let mut argv: Vec<OsString> = vec!["ikari-npu-probe".into()];
        argv.extend(job_args(&job));
        let cli = Cli::try_parse_from(argv).unwrap();
        assert_eq!(cli.worker.as_deref(), Some("ov-gpu-fp16"));
        assert_eq!(cli.batch, Some(8));
        assert_eq!(cli.mode.as_deref(), Some("first"));
        assert_eq!(cli.reps, 3);
        assert_eq!(cli.budget_secs, 9);
        assert_eq!(cli.cache, job.cache);
    }

    #[test]
    fn token_match_compares_with_fixture() {
        let items = input::parse(input::EMBEDDED).unwrap();
        let mut t = worker::token_set(&items);
        assert_eq!(token_match(&t, &items), (items.len(), items.len()));
        t.items[0].prompt.push(9);
        assert_eq!(token_match(&t, &items), (items.len() - 1, items.len()));
        t.eos = 2;
        assert_eq!(token_match(&t, &items).0, 0);
    }

    #[test]
    fn sums_are_read_by_file_name() {
        let sums = read_sums("ABC  ir/zenz-v3.2-small-L128-fp16.xml\ndef *x.bin\n\n");
        assert_eq!(
            sums,
            [
                ("zenz-v3.2-small-L128-fp16.xml".to_owned(), "abc".to_owned()),
                ("x.bin".to_owned(), "def".to_owned())
            ]
        );
    }
}
