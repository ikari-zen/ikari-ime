//! ADR 0060 Q4 の計測。同じ採点要求を、(host) 別プロセス `ikari-lm-host` と (direct) 同じプロセスの中の
//! 直接呼び出しで流して、起動、1 回の時間、RSS を比べる。
//!
//! usage: lm-host-bench --model <gguf> --set <tsv> [--host <exe>] [--n 600] [--warmup 20] [--cands 6]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use ikari_lm::{LlamaScorer, LmScorer, PromptInput, build_prompt, tail_chars};

const LEFT_CONTEXT_CHARS: usize = 20;
const SEED_STRIDE: usize = 7919;

struct Request {
    prompt: String,
    candidates: Vec<String>,
}

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1))
        .cloned()
}

/// 評価セットの行から採点要求を作る。候補は、同じ読みの expected の全部（足りない分は他の行の expected で補う）。
fn load_requests(path: &str, n: usize, cands: usize) -> Vec<Request> {
    let text = std::fs::read_to_string(path).expect("set");
    let mut rows = Vec::new();
    let mut by_reading: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut all_expected: Vec<String> = Vec::new();
    for line in text.lines().skip(1).filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 7 || f[1] != "話題" {
            continue;
        }
        let (context, reading, expected) = (f[4], f[5], f[6]);
        let e = by_reading.entry(reading.to_owned()).or_default();
        if !e.iter().any(|x| x == expected) {
            e.push(expected.to_owned());
        }
        if !all_expected.iter().any(|x| x == expected) {
            all_expected.push(expected.to_owned());
        }
        rows.push((context.to_owned(), reading.to_owned()));
    }
    assert!(!rows.is_empty(), "no usable rows in {path}");
    let mut out = Vec::new();
    for i in 0..n {
        let (context, reading) = &rows[i % rows.len()];
        let mut candidates = by_reading[reading].clone();
        let mut k = (i * SEED_STRIDE) % all_expected.len();
        while candidates.len() < cands {
            let c = &all_expected[k % all_expected.len()];
            if !candidates.contains(c) {
                candidates.push(c.clone());
            }
            k += 1;
        }
        let prompt = build_prompt(&PromptInput {
            left_context: tail_chars(context, LEFT_CONTEXT_CHARS),
            reading,
            ..Default::default()
        });
        out.push(Request { prompt, candidates });
    }
    out
}

fn rss_kb(pid: u32) -> Option<u64> {
    if cfg!(windows) {
        let o = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("(Get-Process -Id {pid}).WorkingSet64"),
            ])
            .output()
            .ok()?;
        let bytes: u64 = String::from_utf8_lossy(&o.stdout).trim().parse().ok()?;
        Some(bytes / 1024)
    } else {
        let o = Command::new("ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        String::from_utf8_lossy(&o.stdout).trim().parse().ok()
    }
}

fn pct(sorted: &[Duration], p: f64) -> f64 {
    let i = ((sorted.len() as f64 * p).ceil() as usize).clamp(1, sorted.len()) - 1;
    sorted[i].as_secs_f64() * 1000.0
}

fn report(label: &str, mut d: Vec<Duration>) {
    d.sort();
    let mean = d.iter().map(Duration::as_secs_f64).sum::<f64>() * 1000.0 / d.len() as f64;
    println!(
        "{label}: n={} mean={mean:.2}ms p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms",
        d.len(),
        pct(&d, 0.50),
        pct(&d, 0.95),
        pct(&d, 0.99),
        pct(&d, 1.0)
    );
}

struct Host {
    child: std::process::Child,
    cin: std::process::ChildStdin,
    cout: BufReader<std::process::ChildStdout>,
}

type Phase = (f64, Vec<Duration>, Vec<Vec<f32>>, u64);

fn gap(ms: u64) {
    if ms > 0 {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// 別プロセス。(起動から READY までの ms, 要求ごとの往復, 結果, 子の RSS MB)
fn host_phase(
    host: &PathBuf,
    model: &PathBuf,
    reqs: &[Request],
    warmup: usize,
    gap_ms: u64,
) -> Phase {
    let t0 = Instant::now();
    let mut child = Command::new(host)
        .arg(model)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn host");
    let cin = child.stdin.take().expect("stdin");
    let cout = BufReader::new(child.stdout.take().expect("stdout"));
    let mut h = Host { child, cin, cout };
    let mut line = String::new();
    h.cout.read_line(&mut line).expect("ready");
    assert_eq!(line.trim(), "READY", "host did not become ready");
    let ready_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let mut times = Vec::new();
    let mut results = Vec::new();
    for (i, r) in reqs.iter().enumerate() {
        gap(gap_ms);
        let t = Instant::now();
        let msg = format!("{}\t{}\n", r.prompt, r.candidates.join("\t"));
        h.cin.write_all(msg.as_bytes()).expect("write");
        h.cin.flush().expect("flush");
        line.clear();
        h.cout.read_line(&mut line).expect("read");
        let dt = t.elapsed();
        let vals: Vec<f32> = line
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        assert_eq!(vals.len(), r.candidates.len(), "bad reply: {line}");
        results.push(vals);
        if i >= warmup {
            times.push(dt);
        }
    }
    let rss = rss_kb(h.child.id()).unwrap_or(0) / 1024;
    drop(h.cin); // 標準入力を閉じると host は終わる
    let _ = h.child.wait();
    (ready_ms, times, results, rss)
}

/// 同じプロセスの中。(読み込みの ms, 要求ごとの時間, 結果, 読み込み後の自分の RSS MB)
fn direct_phase(model: &PathBuf, reqs: &[Request], warmup: usize, gap_ms: u64) -> Phase {
    let t0 = Instant::now();
    let mut scorer = LlamaScorer::load(model).expect("load");
    let load_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let mut times = Vec::new();
    let mut results = Vec::new();
    for (i, r) in reqs.iter().enumerate() {
        gap(gap_ms);
        let t = Instant::now();
        let v = scorer.nll(&r.prompt, &r.candidates).expect("nll");
        let dt = t.elapsed();
        results.push(v);
        if i >= warmup {
            times.push(dt);
        }
    }
    let rss = rss_kb(std::process::id()).unwrap_or(0) / 1024;
    (load_ms, times, results, rss)
}

fn main() {
    let model = PathBuf::from(arg("--model").expect("--model"));
    let set = arg("--set").expect("--set");
    let n: usize = arg("--n").map_or(600, |s| s.parse().expect("--n"));
    let warmup: usize = arg("--warmup").map_or(20, |s| s.parse().expect("--warmup"));
    let cands: usize = arg("--cands").map_or(6, |s| s.parse().expect("--cands"));
    // 要求の間に空ける時間。0 は連続（CPU/GPU が冷めない）。実際の入力は間が空くので 200 なども測る
    let gap_ms: u64 = arg("--gap-ms").map_or(0, |s| s.parse().expect("--gap-ms"));
    let direct_first = std::env::args().any(|a| a == "--direct-first");
    let host = arg("--host").map(PathBuf::from).unwrap_or_else(|| {
        let mut p = std::env::current_exe().expect("exe");
        p.set_file_name(format!("ikari-lm-host{}", std::env::consts::EXE_SUFFIX));
        p
    });
    let reqs = load_requests(&set, n + warmup, cands);
    println!(
        "requests={} (warmup {warmup}) candidates/request>={cands} gap={gap_ms}ms direct_first={direct_first}",
        reqs.len()
    );
    // 初回のディスク読みを両方から外す（ページキャッシュに載せる。読んだ Vec はすぐ捨てる）
    let _ = std::fs::read(&model).map(|b| b.len());
    println!(
        "parent baseline RSS: {} MB",
        rss_kb(std::process::id()).unwrap_or(0) / 1024
    );

    let (h, d);
    if direct_first {
        d = direct_phase(&model, &reqs, warmup, gap_ms);
        h = host_phase(&host, &model, &reqs, warmup, gap_ms);
    } else {
        h = host_phase(&host, &model, &reqs, warmup, gap_ms);
        d = direct_phase(&model, &reqs, warmup, gap_ms);
    }
    println!("(a) host: spawn -> READY = {:.0} ms", h.0);
    println!("(a) direct: in-process load = {:.0} ms", d.0);
    println!("(c) host child RSS: {} MB", h.3);
    println!("(c) parent RSS after direct load: {} MB", d.3);
    let mut max_diff = 0f32;
    for (a, b) in d.2.iter().zip(&h.2) {
        for (x, y) in a.iter().zip(b) {
            max_diff = max_diff.max((x - y).abs());
        }
    }
    println!("max |nll_direct - nll_host| = {max_diff}");
    println!("(b) round trips:");
    report("  host   ", h.1.clone());
    report("  direct ", d.1.clone());
    let diff: Vec<Duration> =
        h.1.iter()
            .zip(&d.1)
            .map(|(a, b)| a.saturating_sub(*b))
            .collect();
    report("  host-direct (per request, saturating)", diff);
}
