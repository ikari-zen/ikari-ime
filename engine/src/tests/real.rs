//! 実辞書と性能（`#[ignore]`）

use super::*;

#[test]
#[ignore = "needs IKARI_DICT_DIR (a real dictionary)"]
fn real_dictionary_converts_with_context() {
    let dir = real_data_dir("real");
    let e = Engine::open(&EngineConfig {
        data_dir: dir.to_string_lossy().into_owned(),
        enable_lm: false,
        idle_unload_secs: 300,
    })
    .unwrap();
    let envelope::Body::HelloReply(h) = send(&e, envelope::Body::Hello(pb::Hello::default()))
    else {
        panic!();
    };
    assert_eq!(h.lm, LmState::LmDisabled as i32);
    begin(&e, 1, false);
    let context = pb::SurroundingText {
        left: "ユーザー辞書に単語を登録するときは".into(),
        right: String::new(),
        available: true,
    };
    send(
        &e,
        envelope::Body::Prefetch(pb::Prefetch {
            session_id: 1,
            reading: READING.into(),
            context: Some(context.clone()),
            context_rev: 1,
        }),
    );
    let envelope::Body::ConvertReply(r) = send(
        &e,
        envelope::Body::Convert(pb::Convert {
            session_id: 1,
            reading: READING.into(),
            context: Some(context),
            context_rev: 1,
            page_size: 9,
        }),
    ) else {
        panic!();
    };
    assert_eq!(surfaces(&r)[0], BASE_TOP, "{:?}", surfaces(&r));
    drop(e);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[ignore = "needs IKARI_DICT_DIR (and IKARI_ZENZ_MODEL with --features lm-metal); run with --release"]
fn perf_typing_simulation_hits_the_cache() {
    let (e, dir, use_lm) = open_real("perf");
    if use_lm {
        let give_up = Instant::now() + PERF_LM_READY_LIMIT;
        while e.inner.lm_state() != LmState::LmReady {
            assert!(Instant::now() < give_up, "LM did not become ready");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    begin(&e, 1, false);
    let mut engine_micros = Vec::new();
    let mut wall_micros = Vec::new();
    let mut lm_hits = 0;
    let mut rev = 0;
    for _ in 0..PERF_ROUNDS {
        for reading in PERF_READINGS {
            let chars: Vec<char> = reading.chars().collect();
            for end in 1..=chars.len() {
                rev += 1;
                prefetch(&e, 1, &chars[..end].iter().collect::<String>(), rev);
                std::thread::sleep(KEY_INTERVAL);
            }
            let started = Instant::now();
            let r = convert(&e, 1, reading, rev);
            wall_micros.push(started.elapsed().as_micros() as u64);
            engine_micros.push(u64::from(r.engine_micros));
            lm_hits += usize::from(r.lm_applied);
            send(
                &e,
                envelope::Body::Cancel(pb::Cancel {
                    conversion_id: r.conversion_id,
                }),
            );
        }
    }
    engine_micros.sort_unstable();
    wall_micros.sort_unstable();
    eprintln!(
        "convert (cache hit, lm={use_lm}): engine p50={}us p99={}us max={}us / wall p99={}us, lm_applied {}/{}",
        percentile(&engine_micros, 0.5),
        percentile(&engine_micros, 0.99),
        engine_micros.last().unwrap(),
        percentile(&wall_micros, 0.99),
        lm_hits,
        engine_micros.len()
    );
    assert!(percentile(&engine_micros, 0.99) <= PERF_P99_ENGINE_MICROS);
    drop(e);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[ignore = "needs IKARI_DICT_DIR (and IKARI_ZENZ_MODEL with --features lm-metal); run with --release"]
fn perf_convert_right_after_open_respects_the_deadline() {
    let started = Instant::now();
    let (e, dir, _) = open_real("perf-cold");
    eprintln!("open: {:?}", started.elapsed());
    begin(&e, 1, false);
    // モデルを読み込み中でも、変換 1 回の締め切りを超えない
    for (i, reading) in PERF_READINGS.iter().enumerate() {
        let t = Instant::now();
        let r = convert(&e, 1, reading, i as u64);
        let wall = t.elapsed();
        eprintln!(
            "cold convert {reading}: {wall:?} (lm_applied {})",
            r.lm_applied
        );
        assert!(wall <= PERF_COLD_CONVERT_MAX, "{reading}: {wall:?}");
    }
    drop(e);
    let _ = std::fs::remove_dir_all(dir);
}
