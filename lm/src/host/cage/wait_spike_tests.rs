//! スパイク（#536。捨てるコード。マージしない）: 檻役が応答しないとき、エンジンの側の口（`XpcCageClient`）が
//! どれだけ待つかを、プロセスの中の匿名の口で測る。`cargo test -p ikari-lm --lib wait_spike -- --ignored --nocapture`
//!
//! 檻役の役の形:
//! - `Hung`: 接続を受けて伝言を受け取るが、返事をしない（応答しない状態）。伝言を放すと、libxpc が相手に
//!   すぐ誤りを返す（測った: 約 0.1ms）ので、持ったままにする
//! - `Stopped`: 接続を受けるが始めない（伝言は列に残る。SIGSTOP で止めた相手に近い。推測）
//! - `Dead`: 受け口を取り消したあと（落ちた相手）
//! - `StopHung`: `hello` と `launch` には答え、`stop` にだけ答えない

use std::fs::File;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::xpc::{self, Connection, Kind, Obj};
use super::*;

const ANY_PEER: &str = "always";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Hung,
    Stopped,
    Dead,
    StopHung,
}

/// 始めない接続は放すと libxpc が止めるので、持ったままにする
static KEEP: Mutex<Vec<Obj>> = Mutex::new(Vec::new());

fn listener(mode: Mode) -> Connection {
    let l = Connection::anonymous_listener().unwrap();
    l.on_event(move |event| {
        if xpc::kind_of(event) != Kind::Connection {
            return;
        }
        let Some(peer) = Obj::retain(event) else {
            return;
        };
        match mode {
            Mode::Stopped => {
                KEEP.lock().unwrap().push(peer);
            }
            Mode::Hung | Mode::Dead => {
                // 伝言を持ったままにする（放すと libxpc が「返事なし」を相手に知らせる）
                peer.on_event(|ev| {
                    if let Some(m) = Obj::retain(ev) {
                        KEEP.lock().unwrap().push(m);
                    }
                });
                peer.resume();
                KEEP.lock().unwrap().push(peer);
            }
            Mode::StopHung => {
                let holder = Arc::new(Mutex::new(Some(peer.clone())));
                let h = Arc::clone(&holder);
                peer.on_event(move |ev| {
                    if xpc::kind_of(ev) != Kind::Dictionary {
                        return;
                    }
                    let Some(m) = Obj::retain(ev) else { return };
                    let op = m.get_str(wire::KEY_OP).unwrap_or_default();
                    if op == wire::OP_STOP {
                        KEEP.lock().unwrap().push(m); // 答えない（持ったまま）
                        return;
                    }
                    let Some(reply) = m.reply_to() else { return };
                    reply.set_str(wire::KEY_REPLY, CageReply::Ok.word());
                    reply.set_u64(wire::KEY_VERSION, u64::from(CAGE_PROTOCOL_VERSION));
                    if op == wire::OP_LAUNCH {
                        reply.set_u64(wire::KEY_PID, 999_999);
                    }
                    if let Some(c) = h.lock().unwrap().as_ref() {
                        c.send(&reply);
                    }
                });
                peer.resume();
                KEEP.lock().unwrap().push(peer);
            }
        }
    });
    l.resume();
    l
}

fn stats(name: &str, xs: &mut [f64]) {
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = xs.len();
    let mean = xs.iter().sum::<f64>() / n as f64;
    let sd = (xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64).sqrt();
    println!(
        "SPIKE536 {name}: n={n} min={:.1}ms median={:.1}ms max={:.1}ms mean={mean:.1}ms sd={sd:.1}ms",
        xs[0],
        xs[n / 2],
        xs[n - 1]
    );
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn reps(default: usize) -> usize {
    std::env::var("SPIKE536_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn launch_args() -> CageLaunch {
    let (stdin, _w) = io::pipe().unwrap();
    let (_r, stdout) = io::pipe().unwrap();
    CageLaunch {
        stdin,
        stdout,
        model: File::open("/dev/null").unwrap(),
        gpu: CageGpu::Cpu,
    }
}

#[test]
#[ignore = "spike #536: measures multi-second waits"]
fn wait_spike_measure() {
    let n = reps(10);
    for mode in [Mode::Hung, Mode::Stopped, Mode::Dead] {
        let l = listener(mode);
        let endpoint = l.endpoint().unwrap();
        if mode == Mode::Dead {
            l.cancel();
            std::thread::sleep(Duration::from_millis(100));
        }
        // 1) hello（殻が口を渡したときのエンジンの確かめ。CageSlot::attach_unverified）
        let mut xs = Vec::new();
        for _ in 0..n {
            let client = XpcCageClient::from_obj(endpoint.clone(), ANY_PEER).unwrap();
            let slot = CageSlot::default();
            let t = Instant::now();
            let r = slot.attach_unverified(Arc::new(client));
            xs.push(ms(t));
            assert!(r.is_err(), "{mode:?}");
        }
        stats(&format!("{mode:?} attach_unverified(hello)"), &mut xs);
        // 2) launch（読み込みのスレッド。接続ごとに hello → launch）
        let mut xs = Vec::new();
        for _ in 0..n.min(5) {
            let client = XpcCageClient::from_obj(endpoint.clone(), ANY_PEER).unwrap();
            let t = Instant::now();
            let r = client.launch(launch_args());
            xs.push(ms(t));
            assert!(r.is_err(), "{mode:?}");
        }
        stats(&format!("{mode:?} launch"), &mut xs);
    }
    // 3) stop（子が固まったときの「止める」。採点のスレッドの drop の中）
    let l = listener(Mode::StopHung);
    let endpoint = l.endpoint().unwrap();
    let mut xs = Vec::new();
    for _ in 0..n {
        let client = XpcCageClient::from_obj(endpoint.clone(), ANY_PEER).unwrap();
        let mut child = client.launch(launch_args()).unwrap();
        let t = Instant::now();
        let r = child.stop();
        xs.push(ms(t));
        assert!(r.is_err());
    }
    stats("StopHung stop", &mut xs);
}
