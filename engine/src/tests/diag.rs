//! 診断ログのエンジン側（ADR 0034 §4）: `DiagEvent` の許可リスト、secure の捨て、
//! `app=` `rw=` `ctx=0` `lm_load` `lm_unload` `stat`

use super::*;
use crate::diag::STAT_INTERVAL;

const POSTAL_SAMPLE: &str = include_str!("../../../dict-format/tests/fixtures/postal.sample.tsv");
const ADDRESS: &str = "東京都千代田区千代田";
const BUNDLE_ID: &str = "com.example.Editor";

/// モックの LM を持ち、ログを書くエンジン
fn logged(tag: &str) -> (Engine, Arc<MockLm>, PathBuf) {
    let dir = temp_dir(tag);
    let mock = MockLm::new(Duration::ZERO);
    let e = Engine::from_parts(EngineParts {
        stack: mock_stack(&mock, None),
        rebuild: None,
        data_dir: Some(dir.clone()),
        lm_requested: true,
        lm_budget: LmBudget::default(),
        convert_wait: DEFAULT_CONVERT_WAIT,
        lock: None,
    });
    (e, mock, dir)
}

fn log_of(e: &Engine, dir: &Path) -> String {
    e.inner.log.flush_for_test();
    std::fs::read_to_string(dir.join("logs").join("engine.log")).unwrap_or_default()
}

fn lines_of(e: &Engine, dir: &Path, kind: &str) -> Vec<String> {
    log_of(e, dir)
        .lines()
        .filter(|l| l.split_whitespace().nth(1) == Some(kind))
        .map(str::to_owned)
        .collect()
}

fn begin_app(e: &Engine, session: u64, bundle: &str, secure: bool) {
    let r = send(
        e,
        envelope::Body::BeginSession(pb::BeginSession {
            session_id: session,
            client_bundle_id: bundle.into(),
            secure,
        }),
    );
    assert!(matches!(r, envelope::Body::Ack(_)), "{r:?}");
}

fn event(session: u64, kind: &str, nums: &[(&str, i64)], tags: &[(&str, &str)]) -> envelope::Body {
    envelope::Body::Diag(pb::DiagEvent {
        session_id: session,
        kind: kind.into(),
        nums: nums
            .iter()
            .map(|(k, v)| pb::DiagNum {
                key: (*k).into(),
                value: *v,
            })
            .collect(),
        tags: tags
            .iter()
            .map(|(k, v)| pb::DiagTag {
                key: (*k).into(),
                value: (*v).into(),
            })
            .collect(),
    })
}

fn send_ack(e: &Engine, body: envelope::Body) {
    let r = send(e, body);
    assert!(matches!(r, envelope::Body::Ack(_)), "{r:?}");
}

#[test]
fn listed_event_is_written_in_the_adr_line_shape() {
    let (e, _, dir) = logged("diag-ok");
    begin_app(&e, 7, BUNDLE_ID, false);
    send_ack(
        &e,
        event(
            7,
            "sh_episode",
            &[("keys", 9), ("dur_ms", 2140)],
            &[("mode", "romaji"), ("end", "commit_cand")],
        ),
    );
    let lines = lines_of(&e, &dir, "sh_episode");
    assert_eq!(lines.len(), 1, "{lines:?}");
    // {ms} {kind} 0us lm={状態} err=- session=N k=v …
    let words: Vec<&str> = lines[0].split_whitespace().collect();
    assert_eq!(words[2], "0us");
    assert!(words[3].starts_with("lm="), "{}", lines[0]);
    assert_eq!(words[4], "err=-");
    assert_eq!(
        words[5..],
        [
            "session=7",
            "keys=9",
            "dur_ms=2140",
            "mode=romaji",
            "end=commit_cand"
        ]
    );
    // 要求そのものの `diag` の行は重ねて書かない
    assert!(lines_of(&e, &dir, "diag").is_empty());
    assert!(lines_of(&e, &dir, "diag_reject").is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unlisted_event_is_rejected_with_a_reason_and_no_content() {
    let (e, _, dir) = logged("diag-reject");
    begin_app(&e, 1, BUNDLE_ID, false);
    // 値が一覧にない
    send_ack(
        &e,
        event(1, "sh_episode", &[], &[("mode", "ひみつの文字列")]),
    );
    // キーが一覧にない
    send_ack(&e, event(1, "sh_episode", &[("reading", 3)], &[]));
    // kind が一覧にない
    send_ack(&e, event(1, "ひみつの種類", &[], &[]));
    // 長すぎる値
    let long = "a".repeat(64);
    send_ack(&e, event(1, "sh_mode", &[], &[("via", long.as_str())]));
    let text = log_of(&e, &dir);
    for secret in ["ひみつ", "reading", &long] {
        assert!(!text.contains(secret), "{secret} leaked: {text}");
    }
    let lines = lines_of(&e, &dir, "diag_reject");
    let tails: Vec<String> = lines
        .iter()
        .map(|l| l.split_whitespace().skip(5).collect::<Vec<_>>().join(" "))
        .collect();
    assert_eq!(
        tails,
        [
            "n=1 reason=tag_value kind=sh_episode",
            "n=1 reason=num_key kind=sh_episode",
            "n=1 reason=kind",
            "n=1 reason=too_long kind=sh_mode",
        ]
    );
    assert!(lines_of(&e, &dir, "sh_episode").is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn secure_session_events_are_dropped_silently() {
    let (e, _, dir) = logged("diag-secure");
    begin_app(&e, 2, BUNDLE_ID, true);
    // 通る事象も、通らない事象も、何も書かない（`diag_reject` も書かない。NFR-18）
    send_ack(&e, event(2, "sh_episode", &[("keys", 9)], &[]));
    send_ack(&e, event(2, "sh_episode", &[], &[("mode", "ひみつ")]));
    let text = log_of(&e, &dir);
    assert!(!text.contains("sh_episode"), "{text}");
    assert!(!text.contains("diag_reject"), "{text}");
    assert!(!text.contains("diag "), "{text}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unknown_session_is_counted_not_reported_as_an_error() {
    let (e, _, dir) = logged("diag-nosession");
    let r = send(&e, event(99, "sh_episode", &[("keys", 1)], &[]));
    assert!(matches!(r, envelope::Body::Ack(_)), "{r:?}");
    let text = log_of(&e, &dir);
    assert!(!text.contains("UNKNOWN_SESSION"), "{text}");
    assert!(!text.contains("sh_episode"), "{text}");
    let lines = lines_of(&e, &dir, "diag_reject");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].ends_with("n=1 reason=no_session"), "{}", lines[0]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn paused_session_events_are_still_written() {
    let (e, _, dir) = logged("diag-paused");
    begin_app(&e, 3, BUNDLE_ID, false);
    send_ack(
        &e,
        envelope::Body::EndSession(pb::EndSession { session_id: 3 }),
    );
    send_ack(&e, event(3, "sh_mode", &[], &[("to", "en")]));
    let lines = lines_of(&e, &dir, "sh_mode");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].ends_with("session=3 to=en"), "{}", lines[0]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bundle_id_is_written_only_on_the_begin_session_and_convert_lines() {
    let (e, _, dir) = logged("diag-app");
    begin_app(&e, 1, BUNDLE_ID, false);
    begin_app(&e, 2, "com.example.あ", false);
    begin_app(&e, 3, "", false);
    // secure は Bundle ID を書かない（NFR-18）
    begin_app(&e, 4, "com.example.Bank", true);
    prefetch(&e, 1, READING, 0);
    convert(&e, 1, READING, 0);
    send_ack(&e, event(1, "sh_mode", &[], &[("to", "ja")]));
    let begins = lines_of(&e, &dir, "begin_session");
    let tails: Vec<String> = begins
        .iter()
        .map(|l| l.split_whitespace().skip(5).collect::<Vec<_>>().join(" "))
        .collect();
    assert_eq!(
        tails,
        [
            "session=1 app=com.example.Editor",
            "session=2 app=invalid",
            "session=3 app=none",
            "session=4",
        ]
    );
    let text = log_of(&e, &dir);
    // begin_session の行と convert の行だけ（prefetch、sh_mode には書かない）
    assert_eq!(count_lines(&text, BUNDLE_ID), 2, "{text}");
    assert!(!text.contains("Bank"), "{text}");
    for l in text.lines().filter(|l| l.contains(BUNDLE_ID)) {
        let kind = l.split_whitespace().nth(1);
        assert!(matches!(kind, Some("begin_session" | "convert")), "{l}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// convert の行の `app=` の終わりの語（なければ None）
fn convert_app(e: &Engine, dir: &Path) -> Vec<Option<String>> {
    lines_of(e, dir, "convert")
        .iter()
        .map(|l| {
            l.split_whitespace()
                .find_map(|w| w.strip_prefix("app="))
                .map(str::to_owned)
        })
        .collect()
}

#[test]
fn convert_line_carries_app_except_secure() {
    let (e, _, dir) = logged("diag-convert-app");
    begin_app(&e, 1, BUNDLE_ID, false);
    begin_app(&e, 2, "", false);
    begin_app(&e, 3, "com.example.あ b", false);
    begin_app(&e, 4, "com.example.Bank", true);
    for s in 1..=4 {
        convert(&e, s, READING, 0);
    }
    assert_eq!(
        convert_app(&e, &dir),
        [
            Some(BUNDLE_ID.to_owned()),
            Some("none".to_owned()),
            Some("invalid".to_owned()),
            None,
        ]
    );
    let text = log_of(&e, &dir);
    assert!(!text.contains("Bank") && !text.contains("あ b"), "{text}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_app_follows_a_resumed_session() {
    let (e, _, dir) = logged("diag-convert-resume");
    begin_app(&e, 1, BUNDLE_ID, false);
    convert(&e, 1, READING, 0);
    // secure に替わると、Bundle ID を持ち越さない
    begin_app(&e, 1, "com.example.Bank", true);
    convert(&e, 1, READING, 0);
    assert_eq!(convert_app(&e, &dir), [Some(BUNDLE_ID.to_owned()), None]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ctx_zero_is_added_only_when_surrounding_text_is_missing() {
    let (e, _, dir) = logged("diag-ctx");
    begin(&e, 1, false);
    let with = |available: bool, rev: u64| {
        send(
            &e,
            envelope::Body::Prefetch(pb::Prefetch {
                session_id: 1,
                reading: READING.into(),
                context: Some(pb::SurroundingText {
                    left: String::new(),
                    right: String::new(),
                    available,
                }),
                context_rev: rev,
            }),
        );
    };
    with(true, 1);
    with(false, 2);
    send(
        &e,
        envelope::Body::Convert(pb::Convert {
            session_id: 1,
            reading: READING.into(),
            context: None,
            context_rev: 3,
            page_size: 0,
        }),
    );
    let prefetches = lines_of(&e, &dir, "prefetch");
    assert!(prefetches[0].ends_with("ctx_rev=1"), "{}", prefetches[0]);
    assert!(
        prefetches[1].ends_with("ctx_rev=2 ctx=0"),
        "{}",
        prefetches[1]
    );
    let converts = lines_of(&e, &dir, "convert");
    assert!(converts[0].ends_with("ctx_rev=3 ctx=0"), "{}", converts[0]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn lm_load_unload_and_stat_lines() {
    let (e, mock, dir) = logged("diag-lm");
    // 起動の暖機が終わると `lm_load`（所要 us、成功なので err=-）
    wait_until(|| !lines_of(&e, &dir, "lm_load").is_empty());
    let load = lines_of(&e, &dir, "lm_load").remove(0);
    let words: Vec<&str> = load.split_whitespace().collect();
    assert!(words[2].ends_with("us"), "{load}");
    assert_eq!(words[3], "lm=ready", "{load}");
    assert_eq!(words[4], "err=-", "{load}");

    // アイドル解放（モデルが読み込まれていない状態に戻った）は見回りで見つかる
    e.inner.tick_diag(Instant::now());
    mock.loaded.store(false, Ordering::Release);
    e.inner.tick_diag(Instant::now());
    e.inner.tick_diag(Instant::now());
    let unloads = lines_of(&e, &dir, "lm_unload");
    assert_eq!(unloads.len(), 1, "{unloads:?}");
    assert!(unloads[0].ends_with("reason=idle"), "{}", unloads[0]);

    // stat: 前回から要求があったときだけ。間隔が経つまで書かない
    begin(&e, 1, false);
    e.inner.tick_diag(Instant::now());
    assert!(lines_of(&e, &dir, "stat").is_empty());
    let later = Instant::now() + STAT_INTERVAL + Duration::from_secs(1);
    e.inner.tick_diag(later);
    let stats = lines_of(&e, &dir, "stat");
    assert_eq!(stats.len(), 1, "{stats:?}");
    assert!(stats[0].contains("sessions=1"), "{}", stats[0]);
    // `ps` が使える環境なら RSS は数値
    if let Some(rss) = stats[0]
        .split_whitespace()
        .find_map(|t| t.strip_prefix("rss_kb="))
    {
        assert!(rss.parse::<u64>().is_ok_and(|kb| kb > 0), "{}", stats[0]);
    }
    // 要求がなければ、間隔が経っても書かない
    e.inner.tick_diag(later + STAT_INTERVAL * 2);
    assert_eq!(lines_of(&e, &dir, "stat").len(), 1);

    // 閉じるとき、読み込み済みなら `reason=shutdown`
    mock.loaded.store(true, Ordering::Release);
    drop(e);
    let text = std::fs::read_to_string(dir.join("logs").join("engine.log")).unwrap();
    assert!(
        text.lines()
            .any(|l| l.contains(" lm_unload ") && l.ends_with("reason=shutdown")),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn stack_swap_that_drops_a_loaded_lm_is_logged_as_settings() {
    let (e, mock, dir) = logged("diag-swap");
    wait_until(|| mock.loaded.load(Ordering::Acquire));
    // 同じ LM を使い回す差し替えは解放ではない
    let old = e.inner.stack();
    e.inner.note_stack_swap(&old, &old);
    assert!(lines_of(&e, &dir, "lm_unload").is_empty());
    // LM のない Stack に替わる
    let mut next = mock_stack(&MockLm::new(Duration::ZERO), None);
    next.lm = None;
    e.inner.note_stack_swap(&old, &next);
    let unloads = lines_of(&e, &dir, "lm_unload");
    assert_eq!(unloads.len(), 1, "{unloads:?}");
    assert!(unloads[0].ends_with("reason=settings"), "{}", unloads[0]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn commit_line_names_the_rewriter_of_the_chosen_candidate() {
    let mut t = Real::new();
    std::fs::write(crate::dispatch::postal_tsv_path_in(&t.dir), POSTAL_SAMPLE).unwrap();
    t.engine = None;
    t.open();
    begin(t.e(), 1, false);
    let conv = convert(t.e(), 1, "100-0001", 0);
    let index = surfaces(&conv)
        .iter()
        .position(|s| *s == ADDRESS)
        .expect("the postal candidate");
    let commit = |conversion_id, candidate_index| {
        send_ack(
            t.e(),
            envelope::Body::Commit(pb::Commit {
                conversion_id,
                candidate_index: u32::try_from(candidate_index).unwrap(),
            }),
        );
    };
    commit(conv.conversion_id, index);
    // 辞書の候補（Rewriter でない）には付かない
    let plain = convert(t.e(), 1, "100-0001", 0);
    let dict_index = plain
        .candidates
        .iter()
        .position(|c| c.kind != pb::CandidateKind::Rewriter as i32)
        .expect("a non-rewriter candidate");
    commit(plain.conversion_id, dict_index);
    let log = t.log();
    let commits: Vec<&str> = log
        .lines()
        .filter(|l| l.split_whitespace().nth(1) == Some("commit"))
        .collect();
    assert_eq!(commits.len(), 2, "{log}");
    assert!(commits[0].ends_with("rw=postal"), "{}", commits[0]);
    assert!(!commits[1].contains("rw="), "{}", commits[1]);
    assert!(!log.contains(ADDRESS), "{log}");
}

#[test]
fn end_session_and_unknown_session_lines_carry_the_session_number() {
    let (e, _, dir) = logged("diag-session-note");
    begin_app(&e, 5, BUNDLE_ID, false);
    send_ack(
        &e,
        envelope::Body::EndSession(pb::EndSession { session_id: 5 }),
    );
    let r = send(
        &e,
        envelope::Body::Convert(pb::Convert {
            session_id: 42,
            reading: "あ".into(),
            ..Default::default()
        }),
    );
    assert_eq!(error_code(&r), ErrorCode::UnknownSession);
    let ends = lines_of(&e, &dir, "end_session");
    assert_eq!(ends.len(), 1, "{ends:?}");
    assert!(ends[0].ends_with("err=- session=5"), "{}", ends[0]);
    let converts = lines_of(&e, &dir, "convert");
    assert_eq!(converts.len(), 1, "{converts:?}");
    assert!(
        converts[0].ends_with("err=UNKNOWN_SESSION session=42"),
        "{}",
        converts[0]
    );
    let _ = std::fs::remove_dir_all(dir);
}
