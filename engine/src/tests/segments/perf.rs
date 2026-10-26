//! 実辞書での文節の速さ（LM なし、エンジン内。ADR 0006 §10.1。目標 p99 ≤ 2ms）。
//! `IKARI_DICT_DIR=... cargo test --release -p ikari-engine -- --ignored --nocapture segment_perf`

use super::resize::resize_body;
use super::segment_page::{choice, seg_page_body};
use super::*;

const READINGS: [&str; 5] = [
    "きょうはいいてんきですね",
    "ゆーざーじしょにたんごをとうろくする",
    "りぽじとりめい",
    "はじめてしった",
    "ひんしのしよう",
];
const ROUNDS: usize = 40;
const BUDGET_P99_MICROS: u64 = 2_000;
const WARM_UP_WAIT: Duration = Duration::from_millis(1_000);

fn report(name: &str, mut micros: Vec<u64>) -> u64 {
    micros.sort_unstable();
    let p99 = percentile(&micros, 0.99);
    println!(
        "segment_perf {name}: n={} p50={}us p99={}us max={}us",
        micros.len(),
        percentile(&micros, 0.5),
        p99,
        micros.last().unwrap()
    );
    p99
}

#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary); run with --release"]
fn segment_perf_page_resize_and_reconvert() {
    let (e, dir, _) = open_real("seg-perf");
    // 起動のあとに裏で行う、逆引き表の作成と辞書のページの読み込み（合わせて約 0.2 秒）を待つ。
    // これが済む前の最初の再変換は 1〜8ms かかる（ページフォールト。B6）
    std::thread::sleep(WARM_UP_WAIT);
    begin(&e, 1, true);
    let (mut page, mut resize, mut recon) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..ROUNDS {
        for reading in READINGS {
            let r = convert(&e, 1, reading, 0);
            let id = r.conversion_id;
            let n = r.layout.as_ref().unwrap().segments.len();
            for seg in 1..n {
                let t = Instant::now();
                let _ = send(
                    &e,
                    seg_page_body(id, 0, seg as u32, 0, vec![choice(0); seg]),
                );
                page.push(t.elapsed().as_micros() as u64);
            }
            let t = Instant::now();
            let _ = send(&e, resize_body(id, 0, 0, -1, vec![]));
            resize.push(t.elapsed().as_micros() as u64);
            send(&e, envelope::Body::Cancel(pb::Cancel { conversion_id: id }));
        }
    }
    begin(&e, 2, false);
    for _ in 0..ROUNDS {
        for reading in READINGS {
            let t = Instant::now();
            let reply = send(
                &e,
                envelope::Body::Reconvert(pb::Reconvert {
                    session_id: 2,
                    surface: reading.into(),
                    context: None,
                    page_size: 0,
                }),
            );
            recon.push(t.elapsed().as_micros() as u64);
            if let envelope::Body::SegmentLayout(l) = reply {
                send(
                    &e,
                    envelope::Body::Cancel(pb::Cancel {
                        conversion_id: l.conversion_id,
                    }),
                );
            }
        }
    }
    let worst = [
        report("segment_page(first)", page),
        report("resize", resize),
        report("reconvert", recon),
    ];
    let _ = std::fs::remove_dir_all(dir);
    for p99 in worst {
        assert!(
            p99 <= BUDGET_P99_MICROS,
            "p99 {p99}us is over {BUDGET_P99_MICROS}us"
        );
    }
}
