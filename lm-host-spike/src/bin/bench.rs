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

fn main() {
    let model = PathBuf::from(arg("--model").expect("--model"));
    let set = arg("--set").expect("--set");
    let n: usize = arg("--n").map_or(600, |s| s.parse().expect("--n"));
    let warmup: usize = arg("--warmup").map_or(20, |s| s.parse().expect("--warmup"));
    let cands: usize = arg("--cands").map_or(6, |s| s.parse().expect("--cands"));
    let host = arg("--host").map(PathBuf::from).unwrap_or_else(|| {
        let mut p = std::env::current_exe().expect("exe");
        p.set_file_name(format!("ikari-lm-host{}", std::env::consts::EXE_SUFFIX));
        p
    });
    let reqs = load_requests(&set, n + warmup, cands);
    println!(
        "requests={} (warmup {warmup}) candidates/request>={cands}",
        reqs.len()
    );

    // 初回のディスク読みを両方から外す（ページキャッシュに載せる）
    let _ = std::fs::read(&model).map(|b| b.len());
    let me = std::process::id();
    println!("parent baseline RSS: {} MB", rss_kb(me).unwrap_or(0) / 1024);

    // ---- host（別プロセス）
    let t0 = Instant::now();
    let mut child = Command::new(&host)
        .arg(&model)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn host");
    let mut cin = child.stdin.take().expect("stdin");
    let mut cout = BufReader::new(child.stdout.take().expect("stdout"));
    let mut line = String::new();
    cout.read_line(&mut line).expect("ready");
    assert_eq!(line.trim(), "READY", "host did not become ready");
    println!(
        "(a) host: spawn -> READY = {:.0} ms",
        t0.elapsed().as_secs_f64() * 1000.0
    );

    let mut host_times = Vec::new();
    let mut host_results: Vec<Vec<f32>> = Vec::new();
    for (i, r) in reqs.iter().enumerate() {
        let t = Instant::now();
        let msg = format!("{}\t{}\n", r.prompt, r.candidates.join("\t"));
        cin.write_all(msg.as_bytes()).expect("write");
        cin.flush().expect("flush");
        line.clear();
        cout.read_line(&mut line).expect("read");
        let dt = t.elapsed();
        let vals: Vec<f32> = line
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        assert_eq!(vals.len(), r.candidates.len(), "bad reply: {line}");
        host_results.push(vals);
        if i >= warmup {
            host_times.push(dt);
        }
    }
    println!(
        "host child RSS: {} MB",
        rss_kb(child.id()).unwrap_or(0) / 1024
    );
    println!(
        "parent RSS (host mode): {} MB",
        rss_kb(me).unwrap_or(0) / 1024
    );
    drop(cin); // 標準入力を閉じると host は終わる
    let _ = child.wait();

    // ---- direct（同じプロセスの中）
    let t0 = Instant::now();
    let mut scorer = LlamaScorer::load(&model).expect("load");
    println!(
        "(a) direct: in-process load = {:.0} ms",
        t0.elapsed().as_secs_f64() * 1000.0
    );
    let mut direct_times = Vec::new();
    let mut max_diff = 0f32;
    for (i, r) in reqs.iter().enumerate() {
        let t = Instant::now();
        let v = scorer.nll(&r.prompt, &r.candidates).expect("nll");
        let dt = t.elapsed();
        for (a, b) in v.iter().zip(&host_results[i]) {
            max_diff = max_diff.max((a - b).abs());
        }
        if i >= warmup {
            direct_times.push(dt);
        }
    }
    println!(
        "parent RSS (direct mode, model loaded): {} MB",
        rss_kb(me).unwrap_or(0) / 1024
    );
    println!("max |nll_direct - nll_host| = {max_diff}");
    println!("(b) round trips:");
    report("  host   ", host_times.clone());
    report("  direct ", direct_times.clone());
    let diff: Vec<Duration> = host_times
        .iter()
        .zip(&direct_times)
        .map(|(h, d)| h.saturating_sub(*d))
        .collect();
    report("  host-direct (per request, saturating)", diff);
}
