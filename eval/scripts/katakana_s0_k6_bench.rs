// ADR 0037 §6.1 S0: K6 の判定 1 回の時間を測る捨てコード（本体には入れない）。
//   rustc -O katakana_s0_k6_bench.rs -o k6bench && ./k6bench <table.tsv> <table.tsv.readings>
// table.tsv: 1 行目 `#unseen<TAB>値`、以降 `並び<TAB>値`（並びは \u0001 区切りのモーラ。^ と $ は語の頭と末尾）。
// 読みごとに、モーラに分け、2 つ組と 3 つ組を作り、表を引いて平均する（ADR §3.1 の実行時の手順）。
use std::collections::HashMap;
use std::time::Instant;

const SMALL_JOIN: &str = "ゃゅょぁぃぅぇぉゎ";

fn moras(r: &str) -> Vec<String> {
    let mut u: Vec<String> = Vec::new();
    for c in r.chars() {
        if SMALL_JOIN.contains(c) && !u.is_empty() {
            u.last_mut().unwrap().push(c);
        } else {
            u.push(c.to_string());
        }
    }
    u
}

fn score(table: &HashMap<String, f32>, unseen: f32, r: &str) -> f32 {
    let mut p = vec!["^".to_string()];
    p.extend(moras(r));
    p.push("$".to_string());
    let mut sum = 0.0f32;
    let mut cnt = 0u32;
    let mut key = String::new();
    for n in [2usize, 3] {
        if p.len() < n {
            continue;
        }
        for i in 0..=(p.len() - n) {
            key.clear();
            for (j, u) in p[i..i + n].iter().enumerate() {
                if j > 0 {
                    key.push('\u{1}');
                }
                key.push_str(u);
            }
            sum += table.get(key.as_str()).copied().unwrap_or(unseen);
            cnt += 1;
        }
    }
    sum / cnt.max(1) as f32
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let t = std::fs::read_to_string(&args[1]).unwrap();
    let mut table = HashMap::new();
    let mut unseen = 0.0f32;
    let load = Instant::now();
    for l in t.lines() {
        let (k, v) = l.split_once('\t').unwrap();
        let v: f32 = v.parse().unwrap();
        if k == "#unseen" {
            unseen = v;
        } else {
            table.insert(k.to_string(), v);
        }
    }
    let load_ms = load.elapsed().as_secs_f64() * 1e3;
    let readings: Vec<String> = std::fs::read_to_string(&args[2]).unwrap().lines().map(|s| s.to_string()).collect();
    let mut total = 0.0f32;
    let reps = 50;
    let t0 = Instant::now();
    for _ in 0..reps {
        for r in &readings {
            total += score(&table, unseen, r);
        }
    }
    let dt = t0.elapsed().as_secs_f64() / (reps * readings.len()) as f64;
    println!(
        "table entries={} load={:.1}ms readings={} per-judgement={:.2}us (checksum {:.3})",
        table.len(),
        load_ms,
        readings.len(),
        dt * 1e6,
        total
    );
}
