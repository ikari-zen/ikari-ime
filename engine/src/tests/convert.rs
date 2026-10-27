//! 先行計算、変換、ページ

use super::*;
use crate::convert::CONVERT_WAIT_MAX;

#[test]
fn prefetch_then_convert_hits_the_cache() {
    let (e, _mock) = engine(0);
    begin(&e, 1, false);
    prefetch(&e, 1, READING, 7);
    wait_until(|| !lock_mutex(&e.inner.cache).any_computing());
    let r = convert(&e, 1, READING, 7);
    assert_eq!(
        surfaces(&r)[0],
        BOOSTED,
        "the LM result computed ahead is used"
    );
    assert!(r.lm_applied);
    assert!(r.engine_micros < 5_000, "{}us", r.engine_micros);
    assert!(matches!(
        r.candidates[0].kind,
        k if k == pb::CandidateKind::Dictionary as i32
    ));
    assert!(
        r.candidates
            .iter()
            .any(|c| c.kind == pb::CandidateKind::Hiragana as i32 || r.has_next)
    );
}

#[test]
fn different_context_rev_misses_the_cache() {
    let (e, _mock) = engine(0);
    begin(&e, 1, false);
    prefetch(&e, 1, READING, 1);
    wait_until(|| !lock_mutex(&e.inner.cache).any_computing());
    let r = convert(&e, 1, READING, 2);
    assert_eq!(surfaces(&r)[0], BASE_TOP);
    assert!(!r.lm_applied);
}

#[test]
fn convert_waits_for_a_running_prefetch_until_the_budget() {
    // 15ms で終わる計算は、30ms の待ち時間の内に間に合う
    let (e, _mock) = engine(15);
    begin(&e, 1, false);
    prefetch(&e, 1, READING, 0);
    let started = Instant::now();
    let r = convert(&e, 1, READING, 0);
    assert_eq!(surfaces(&r)[0], BOOSTED);
    assert!(r.lm_applied);
    assert!(started.elapsed() < Duration::from_millis(200));
}

#[test]
fn convert_falls_back_without_the_lm_when_the_prefetch_is_too_slow() {
    // 200ms かかる計算は、30ms の待ち時間に間に合わない
    let (e, _mock) = engine(200);
    begin(&e, 1, false);
    prefetch(&e, 1, READING, 0);
    let started = Instant::now();
    let r = convert(&e, 1, READING, 0);
    let waited = started.elapsed();
    assert_eq!(surfaces(&r)[0], BASE_TOP);
    assert!(!r.lm_applied);
    assert!(waited < Duration::from_millis(150), "{waited:?}");
    assert!(
        waited >= Duration::from_millis(20),
        "waits for the budget: {waited:?}"
    );
}

#[test]
fn only_the_latest_prefetch_per_session_is_computed() {
    let (e, mock) = engine(30);
    begin(&e, 1, false);
    for (i, reading) in ["ひ", "ひん", "ひんし"].iter().enumerate() {
        prefetch(&e, 1, reading, i as u64);
    }
    wait_until(|| !lock_mutex(&e.inner.cache).any_computing());
    // 先頭の依頼はワーカーが取る前に置き換えられるか、取られても 1 件だけ
    assert!(
        mock.calls.load(Ordering::Relaxed) <= 2,
        "{}",
        mock.calls.load(Ordering::Relaxed)
    );
    let r = convert(&e, 1, "ひんし", 2);
    assert_eq!(surfaces(&r)[0], BOOSTED);
}

#[test]
fn displayed_pages_are_never_replaced() {
    let (e, mock) = engine(0);
    begin(&e, 1, false);
    prefetch(&e, 1, READING, 0);
    wait_until(|| !lock_mutex(&e.inner.cache).any_computing());
    let first = convert(&e, 1, READING, 0);
    assert_eq!(surfaces(&first)[0], BOOSTED);
    // LM の結果が変わっても、表示済みのページは同じ
    mock.boost.store(false, Ordering::Relaxed);
    let again = match page(&e, first.conversion_id, 0) {
        envelope::Body::PageReply(r) => r,
        other => panic!("{other:?}"),
    };
    assert_eq!(surfaces(&again), surfaces(&first));
    assert_eq!(again.page_index, 0);
    assert_eq!(again.conversion_id, first.conversion_id);
    let second = match page(&e, first.conversion_id, 1) {
        envelope::Body::PageReply(r) => r,
        other => panic!("{other:?}"),
    };
    assert_eq!(second.page_index, 1);
    assert!(!second.candidates.is_empty());
    assert!(
        !second.has_next,
        "16 candidates + kana fit in two pages: {}",
        second.candidates.len()
    );
    // 範囲外のページ
    assert_eq!(
        error_code(&page(&e, first.conversion_id, 50)),
        ErrorCode::BadRequest
    );
}

#[test]
fn page_size_slices_the_generated_list() {
    let (e, _mock) = engine(0);
    begin(&e, 1, false);
    let all: Vec<String> = {
        let r = convert(&e, 1, READING, 0);
        let p1 = match page(&e, r.conversion_id, 1) {
            envelope::Body::PageReply(p) => p,
            other => panic!("{other:?}"),
        };
        surfaces(&r)
            .into_iter()
            .chain(surfaces(&p1))
            .map(str::to_owned)
            .collect()
    };
    let envelope::Body::ConvertReply(small) = send(&e, convert_req(1, READING, 0, 4)) else {
        panic!();
    };
    assert_eq!(small.candidates.len(), 4);
    assert!(small.has_next);
    let mut joined: Vec<String> = surfaces(&small).into_iter().map(str::to_owned).collect();
    let mut index = 1;
    loop {
        let envelope::Body::PageReply(p) = page(&e, small.conversion_id, index) else {
            panic!();
        };
        joined.extend(surfaces(&p).into_iter().map(str::to_owned));
        if !p.has_next {
            break;
        }
        index += 1;
    }
    assert_eq!(joined, all);
}

#[test]
fn secure_sessions_are_not_prefetched_learned_or_remembered() {
    let dir = temp_dir("secure");
    let mock = MockLm::new(Duration::ZERO);
    let path = dir.join("learning.tsv");
    let e = engine_with(&mock, Some(&path));
    begin(&e, 1, true);
    prefetch(&e, 1, READING, 0);
    assert_eq!(lock_mutex(&e.inner.cache).len(), 0, "nothing is cached");
    let r = convert(&e, 1, READING, 0);
    assert!(!r.candidates.is_empty());
    assert!(matches!(
        send(
            &e,
            envelope::Body::Commit(pb::Commit {
                conversion_id: r.conversion_id,
                candidate_index: 1
            })
        ),
        envelope::Body::Ack(_)
    ));
    send(
        &e,
        envelope::Body::CommitText(pb::CommitText {
            session_id: 1,
            reading: "あ".into(),
            surface: "secret".into(),
        }),
    );
    let stack = e.inner.stack();
    assert!(stack.learning.as_ref().unwrap().is_empty());
    assert!(lock_mutex(&e.inner.sessions)[&1].history.is_empty());
    assert!(!lock_mutex(&e.inner.save).dirty);
    drop(e);
    assert!(!path.exists(), "nothing was saved");
    let _ = std::fs::remove_dir_all(dir);
}

// ---- L0: ログの注記（ADR 0021）、L1: Space から最大 30ms 待つ ----

/// ログを `data_dir/logs/engine.log` に書くエンジン
fn logged_engine(delay_ms: u64, budget: LmBudget, tag: &str) -> (Engine, PathBuf) {
    let dir = temp_dir(tag);
    let mock = MockLm::new(Duration::from_millis(delay_ms));
    let e = Engine::from_parts(EngineParts {
        stack: mock_stack(&mock, None),
        rebuild: None,
        data_dir: Some(dir.clone()),
        lm_requested: true,
        lm_budget: budget,
        lock: None,
    });
    (e, dir)
}

/// 書き込みのスレッドが書き終えるのを待ってから読む（ログは非同期。ADR 0034 §3.1）
fn log_text(e: &Engine, dir: &Path) -> String {
    e.inner.log.flush_for_test();
    std::fs::read_to_string(dir.join("logs").join("engine.log")).unwrap_or_default()
}

fn log_lines(e: &Engine, dir: &Path, kind: &str) -> Vec<String> {
    log_text(e, dir)
        .lines()
        .filter(|l| l.split_whitespace().nth(1) == Some(kind))
        .map(str::to_owned)
        .collect()
}

fn last_convert_line(e: &Engine, dir: &Path) -> String {
    log_lines(e, dir, "convert").pop().expect("a convert line")
}

fn prefetch_and_settle(e: &Engine, dir: &Path, reading: &str, rev: u64) {
    prefetch(e, 1, reading, rev);
    wait_until(|| !lock_mutex(&e.inner.cache).any_computing());
    wait_until(|| !log_lines(e, dir, "prefetch_done").is_empty());
}

#[test]
fn convert_log_says_hit_with_the_lm() {
    let (e, dir) = logged_engine(0, LmBudget::default(), "log-hit");
    begin(&e, 1, false);
    prefetch_and_settle(&e, &dir, READING, 7);
    let r = convert(&e, 1, READING, 7);
    assert!(r.lm_applied);
    let line = last_convert_line(&e, &dir);
    assert!(line.contains("segments=1 cache=hit lm_applied=1"), "{line}");
    let done = &log_lines(&e, &dir, "prefetch_done")[0];
    assert!(done.ends_with("err=- lm_applied=1 lm_calls=1"), "{done}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_and_commit_lines_share_a_conversion_number() {
    let (e, dir) = logged_engine(0, LmBudget::default(), "log-conv");
    begin(&e, 7, false);
    let r = convert(&e, 7, READING, 0);
    let reply = send(
        &e,
        envelope::Body::Commit(pb::Commit {
            conversion_id: r.conversion_id,
            candidate_index: 0,
        }),
    );
    assert!(matches!(reply, envelope::Body::Ack(_)), "{reply:?}");
    let convert_line = last_convert_line(&e, &dir);
    let conv = format!("conv={}", r.conversion_id);
    assert!(convert_line.contains("session=7"), "{convert_line}");
    assert!(convert_line.contains(&conv), "{convert_line}");
    let commit_line = log_lines(&e, &dir, "commit").pop().expect("a commit line");
    assert!(commit_line.contains(&conv), "{commit_line}");
    assert!(!commit_line.contains("session="), "{commit_line}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_log_says_wait_hit_and_wait_miss() {
    let (e, dir) = logged_engine(10, LmBudget::default(), "log-wait-hit");
    begin(&e, 1, false);
    prefetch(&e, 1, READING, 0);
    let r = convert(&e, 1, READING, 0);
    assert!(r.lm_applied);
    let line = last_convert_line(&e, &dir);
    assert!(line.contains("cache=wait_hit lm_applied=1"), "{line}");
    let _ = std::fs::remove_dir_all(dir);

    let (e, dir) = logged_engine(200, LmBudget::default(), "log-wait-miss");
    begin(&e, 1, false);
    prefetch(&e, 1, READING, 0);
    let r = convert(&e, 1, READING, 0);
    assert!(!r.lm_applied);
    let line = last_convert_line(&e, &dir);
    assert!(line.contains("cache=wait_miss lm_applied=0"), "{line}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_log_tells_why_the_cache_missed() {
    let (e, dir) = logged_engine(0, LmBudget::default(), "log-miss");
    begin(&e, 1, false);
    prefetch_and_settle(&e, &dir, READING, 3);
    // 読みだけが違う
    convert(&e, 1, "ひ", 3);
    assert!(last_convert_line(&e, &dir).contains("cache=miss lm_applied=0 miss=reading"));
    // 文脈の番号だけが違う
    convert(&e, 1, READING, 4);
    assert!(last_convert_line(&e, &dir).contains("cache=miss lm_applied=0 miss=context"));
    // 近いものがない
    convert(&e, 1, "ひ", 9);
    assert!(last_convert_line(&e, &dir).contains("cache=miss lm_applied=0 miss=none"));
    // 秘密の入力は、キャッシュを引かない
    begin(&e, 2, true);
    send(&e, convert_req(2, READING, 3, 0));
    assert!(last_convert_line(&e, &dir).contains("cache=miss lm_applied=0 miss=none"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn engine_log_never_contains_the_text() {
    let (e, dir) = logged_engine(0, LmBudget::default(), "log-privacy");
    begin(&e, 1, false);
    prefetch_and_settle(&e, &dir, READING, 0);
    convert(&e, 1, READING, 0);
    convert(&e, 1, "ひ", 5);
    let text = log_text(&e, &dir);
    for secret in [READING, BOOSTED, BASE_TOP, "ひ"] {
        assert!(!text.contains(secret), "{secret} leaked: {text}");
    }
    // 注記は固定の語（key=value）だけ
    const KEYS: [&str; 22] = [
        "session",
        "conv",
        "app",
        "ctx",
        "segments",
        "cache",
        "lm_applied",
        "lm_calls",
        "miss",
        "err",
        "ctx_rev",
        "rev",
        "prev_rev",
        "left",
        "right",
        "avail",
        "dleft",
        "dright",
        "req",
        "reading",
        "lext",
        "tail",
    ];
    for line in text.lines() {
        for token in line.split_whitespace().skip(4) {
            let key = token.split('=').next().unwrap();
            assert!(KEYS.contains(&key), "unexpected token {token} in {line}");
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

fn surrounding(left: &str, right: &str, available: bool) -> Option<pb::SurroundingText> {
    Some(pb::SurroundingText {
        left: left.into(),
        right: right.into(),
        available,
    })
}

fn prefetch_with(e: &Engine, session: u64, rev: u64, context: Option<pb::SurroundingText>) {
    send(
        e,
        envelope::Body::Prefetch(pb::Prefetch {
            session_id: session,
            reading: READING.into(),
            context,
            context_rev: rev,
        }),
    );
}

/// 番号が進んだ理由（どれが変わったか、文字数の差）だけが `ctx_change` の行に出て、周辺テキストは出ない
#[test]
fn ctx_change_line_has_flags_and_deltas_only() {
    let (e, dir) = logged_engine(0, LmBudget::default(), "log-ctx-change");
    begin(&e, 1, false);
    prefetch_with(&e, 1, 1, surrounding("ひみつの左", "ひみつの右", true));
    // 番号が同じなら行は増えない
    prefetch_with(&e, 1, 1, surrounding("ひみつの左", "ひみつの右", true));
    assert!(log_lines(&e, &dir, "ctx_change").is_empty());
    // right だけが 2 文字増えた
    prefetch_with(&e, 1, 2, surrounding("ひみつの左", "ひみつの右あい", true));
    let lines = log_lines(&e, &dir, "ctx_change");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].ends_with("rev=2 prev_rev=1 left=0 right=1 avail=0 dleft=+0 dright=+2 req=prefetch reading=3 lext=0 tail=0"),
        "{}",
        lines[0]
    );
    // convert の行から対応が追える
    send(
        &e,
        envelope::Body::Convert(pb::Convert {
            session_id: 1,
            reading: READING.into(),
            context: surrounding("", "", false),
            context_rev: 3,
            page_size: 0,
        }),
    );
    let lines = log_lines(&e, &dir, "ctx_change");
    assert!(
        lines[1].ends_with("rev=3 prev_rev=2 left=1 right=1 avail=1 dleft=-5 dright=-7 req=convert reading=3 lext=0 tail=1"),
        "{}",
        lines[1]
    );
    // 周辺テキストが取れない要求には `ctx=0` が付く（ADR 0034 §4.3）
    assert!(last_convert_line(&e, &dir).ends_with("ctx_rev=3 ctx=0"));
    assert!(log_lines(&e, &dir, "prefetch")[0].ends_with("ctx_rev=1"));
    assert!(!log_text(&e, &dir).contains("ひみつ"));
    let _ = std::fs::remove_dir_all(dir);
}

/// 確定のあとに番号が進むと、`ctx_change` の行に確定からの経過 ms と確定の文字数が付く（#249）
#[test]
fn ctx_change_line_has_commit_timing_after_commit() {
    let (e, dir) = logged_engine(0, LmBudget::default(), "log-ctx-commit");
    begin(&e, 1, false);
    prefetch_with(&e, 1, 1, surrounding("あ", "", true));
    send(
        &e,
        envelope::Body::CommitText(pb::CommitText {
            session_id: 1,
            reading: "ひみつ".into(),
            surface: "ひみつ".into(),
        }),
    );
    prefetch_with(&e, 1, 2, surrounding("あひみつ", "", true));
    let lines = log_lines(&e, &dir, "ctx_change");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains(" since_commit_ms="), "{}", lines[0]);
    assert!(lines[0].ends_with(" commit_chars=3"), "{}", lines[0]);
    let _ = std::fs::remove_dir_all(dir);
}

/// パスワード欄では、番号も周辺テキストも記録しない（NFR-18）
#[test]
fn secure_sessions_leave_no_context_trace() {
    let (e, dir) = logged_engine(0, LmBudget::default(), "log-ctx-secure");
    begin(&e, 2, true);
    prefetch_with(&e, 2, 1, surrounding("あ", "い", true));
    prefetch_with(&e, 2, 2, surrounding("あう", "い", true));
    send(
        &e,
        envelope::Body::Convert(pb::Convert {
            session_id: 2,
            reading: READING.into(),
            context: surrounding("あうえ", "", true),
            context_rev: 3,
            page_size: 0,
        }),
    );
    assert!(log_lines(&e, &dir, "ctx_change").is_empty());
    assert!(!log_text(&e, &dir).contains("ctx_rev"));
    assert!(lock_mutex(&e.inner.ctx_trace).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// Space から数えて 30ms 待つ。Prefetch の受付から数える budget（ここでは 1ms）が過ぎていても待つ
#[test]
fn convert_waits_up_to_30ms_from_the_space() {
    let budget = LmBudget {
        base: Duration::from_millis(1),
        per_char: Duration::ZERO,
        max: Duration::from_millis(300),
    };
    let (e, dir) = logged_engine(8, budget, "wait-30-hit");
    begin(&e, 1, false);
    prefetch(&e, 1, READING, 0);
    let r = convert(&e, 1, READING, 0);
    assert!(r.lm_applied, "an 8ms computation fits in 30ms from Space");
    assert_eq!(surfaces(&r)[0], BOOSTED);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_never_waits_more_than_30ms() {
    let (e, dir) = logged_engine(200, LmBudget::default(), "wait-30-miss");
    begin(&e, 1, false);
    prefetch(&e, 1, READING, 0);
    let started = Instant::now();
    let r = convert(&e, 1, READING, 0);
    let waited = started.elapsed();
    assert!(!r.lm_applied);
    assert_eq!(surfaces(&r)[0], BASE_TOP);
    assert!(waited >= CONVERT_WAIT_MAX, "{waited:?}");
    // 30ms の待ちとその場の変換。余裕を見ても、計算の 300ms には届かない
    assert!(waited < Duration::from_millis(150), "{waited:?}");
    let _ = std::fs::remove_dir_all(dir);
}
