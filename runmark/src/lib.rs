//! 起動中の印と、前回の終わり方の判定（ADR 0051）。
//!
//! C0（契約）の段階: 型とシグネチャだけで、中身は安全な既定（何も書かず、何も記録しない）。
//! 書式・判定・シグナルの処理・panic の hook は R1 で入れる。
//!
//! 印の中身は固定の語と数だけ（NFR-19）。panic のメッセージは書かない。

use std::path::Path;

/// 場所を絞り込めなかったときの語（`sanitize_location` の戻り。ADR 0051 §5.3）
pub const EXTERNAL_LOCATION: &str = "external";

/// 場所（`<相対パス>.rs:<行>`）の最大バイト数（ADR 0051 §5.2）
pub const MAX_LOCATION_BYTES: usize = 96;

/// 前回の終わり方（ADR 0051 §3.1、§4.1）。`unknown` を `crash` と呼ばない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// `exit=clean`: 正常に終わった
    Clean,
    /// `exit=term`: 終わらせるシグナル（`SIGTERM` など）を受けた
    Term,
    /// `exit=crash`: 致命的なシグナル（`SIGSEGV` など）を受けた
    Crash,
    /// `exit=` の行がない、壊れている、版が違う（`SIGKILL`、電源断など）
    Unknown,
}

impl Exit {
    /// 診断ログと殻の事象に書く固定の語
    pub fn as_word(self) -> &'static str {
        match self {
            Exit::Clean => "clean",
            Exit::Term => "term",
            Exit::Crash => "crash",
            Exit::Unknown => "unknown",
        }
    }
}

/// 前回の判定
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrevExit {
    pub exit: Exit,
    /// 受けたシグナルの番号。`Term` と `Crash` のときだけ
    pub sig: Option<i32>,
    /// 動いていた秒数（印の mtime から）。`Unknown` のとき、時計が戻ったときは None
    pub up_s: Option<u64>,
}

/// 今回の起動中の印。落とすだけでは何も書かない（正常な終わりは `finish_clean`）。
#[derive(Debug)]
pub struct RunMark {
    // R1 で fd（シグナルの処理が書く先）を持つ
    _private: (),
}

impl RunMark {
    /// 前回の印（`path`）を読んで判定し、今回の印を作り直す。
    /// 印がない（初めての起動）、または書けないときは判定が `None`。
    ///
    /// C0 のスタブ: 何も読まず、何も書かず、`None` を返す。
    pub fn begin(path: &Path) -> (RunMark, Option<PrevExit>) {
        let _ = path;
        (RunMark { _private: () }, None)
    }

    /// 正常な終わりを印に書く（`exit=clean`）。C0 のスタブ: 何もしない。
    pub fn finish_clean(&self) {}
}

/// 終わらせるシグナルと致命的なシグナルの処理を入れる（`cfg(unix)` のみ。プロセスの入口で 1 回）。
/// C0 のスタブ: 何も入れない。
pub fn install_signal_handlers() {}

/// panic の記録の送り先。引数は `(thread の語, 絞り込んだ場所)`（メッセージは渡さない）
pub type PanicSink = Box<dyn Fn(&str, &str) + Send + Sync + 'static>;

/// panic の hook を入れる。`sink` へ `(thread の語, 絞り込んだ場所)` を渡す。前の hook は呼んでから記録する。
/// メッセージは `sink` に渡さない。C0 のスタブ: 何も入れない。
pub fn install_panic_hook(sink: PanicSink) {
    let _ = sink;
}

/// panic の場所を絞り込む。`[A-Za-z0-9_/.-]+\.rs:[0-9]+` で 96 バイトまで、`/` で始まらず、`..` を含まない
/// ものだけそのまま返す。ほかは `EXTERNAL_LOCATION`。
///
/// C0 のスタブ: 常に `EXTERNAL_LOCATION`（何も漏らさない側に倒す）。
pub fn sanitize_location(loc: &str) -> &str {
    let _ = loc;
    EXTERNAL_LOCATION
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    /// 前回の印の中身（`body`）を置いて `begin` した判定
    fn judge(body: Option<&str>) -> Option<PrevExit> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.run");
        if let Some(body) = body {
            fs::write(&path, body).unwrap();
        }
        RunMark::begin(&path).1
    }

    fn exit_of(body: Option<&str>) -> Option<Exit> {
        judge(body).map(|p| p.exit)
    }

    #[test]
    fn stub_records_nothing() {
        // C0 の既定: 前回の印があっても何も判定しない
        assert_eq!(exit_of(Some("v=1 start_ms=1\nexit=crash sig=11\n")), None);
        assert_eq!(sanitize_location("engine/src/lib.rs:10"), EXTERNAL_LOCATION);
    }

    #[test]
    fn words_are_fixed() {
        assert_eq!(Exit::Clean.as_word(), "clean");
        assert_eq!(Exit::Term.as_word(), "term");
        assert_eq!(Exit::Crash.as_word(), "crash");
        assert_eq!(Exit::Unknown.as_word(), "unknown");
    }

    #[test]
    #[ignore = "R1 で中身を入れたら外す（ADR 0051 §8）"]
    fn judgement_table() {
        // 印なし（初めての起動）→ 何も記録しない
        assert_eq!(exit_of(None), None);
        // 1 行目だけ → unknown（up_s は出さない）
        let p = judge(Some("v=1 start_ms=1793000000123\n")).unwrap();
        assert_eq!((p.exit, p.sig, p.up_s), (Exit::Unknown, None, None));
        assert_eq!(
            exit_of(Some("v=1 start_ms=1\nexit=clean\n")),
            Some(Exit::Clean)
        );
        let p = judge(Some("v=1 start_ms=1\nexit=term sig=15\n")).unwrap();
        assert_eq!((p.exit, p.sig), (Exit::Term, Some(15)));
        let p = judge(Some("v=1 start_ms=1\nexit=crash sig=11\n")).unwrap();
        assert_eq!((p.exit, p.sig), (Exit::Crash, Some(11)));
        // 壊れた行・別の版 → unknown
        for body in [
            "garbage\n",
            "v=2 start_ms=1\nexit=clean\n",
            "v=1 start_ms=1\nexit=crash sig=abc\n",
            "v=1 start_ms=1\nexit=explode\n",
            "",
        ] {
            assert_eq!(exit_of(Some(body)), Some(Exit::Unknown), "{body:?}");
        }
    }

    #[test]
    #[ignore = "R1 で中身を入れたら外す（ADR 0051 §8）"]
    fn begin_recreates_the_mark_and_finish_clean_appends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.run");
        let (mark, prev) = RunMark::begin(&path);
        assert_eq!(prev, None);
        let first = fs::read_to_string(&path).unwrap();
        assert!(first.starts_with("v=1 start_ms="), "{first:?}");
        mark.finish_clean();
        assert!(fs::read_to_string(&path).unwrap().ends_with("exit=clean\n"));
        let (_, prev) = RunMark::begin(&path);
        assert_eq!(prev.unwrap().exit, Exit::Clean);
    }

    #[test]
    #[ignore = "R1 で中身を入れたら外す（ADR 0051 §8）"]
    fn locations_are_narrowed_to_workspace_sources() {
        assert_eq!(
            sanitize_location("engine/src/lib.rs:477"),
            "engine/src/lib.rs:477"
        );
        assert_eq!(sanitize_location("a-b_c/d.e/f.rs:1"), "a-b_c/d.e/f.rs:1");
        for bad in [
            "/Users/someone/.cargo/registry/src/x/lib.rs:1",
            "/rustc/abc/library/core/src/option.rs:1",
            "../secret.rs:1",
            "a/../b.rs:1",
            "engine/src/lib.rs",
            "engine/src/lib.rs:",
            "engine/src/lib.rs:x",
            "engine/src/lib.txt:1",
            "エンジン/lib.rs:1",
            "",
        ] {
            assert_eq!(sanitize_location(bad), EXTERNAL_LOCATION, "{bad:?}");
        }
        let long = format!("{}.rs:1", "a".repeat(MAX_LOCATION_BYTES));
        assert_eq!(sanitize_location(&long), EXTERNAL_LOCATION);
    }

    // ---- 子のプロセス: テストの実行ファイルを環境変数つきで自分で起動する

    const CHILD_MODE: &str = "IKARI_RUNMARK_CHILD_MODE";
    const CHILD_PATH: &str = "IKARI_RUNMARK_CHILD_PATH";
    const READY: &str = "ready";

    /// 子として呼ばれたときの本体（親から起動されたときだけ動く。普通のテストの実行では何もしない）
    #[test]
    fn child_entry() {
        let Ok(mode) = std::env::var(CHILD_MODE) else {
            return;
        };
        let path = PathBuf::from(std::env::var(CHILD_PATH).unwrap());
        let (_mark, _) = RunMark::begin(&path);
        install_signal_handlers();
        println!("{READY}");
        match mode.as_str() {
            "segv" => unsafe { std::ptr::null_mut::<u8>().write_volatile(0) },
            "abort" => std::process::abort(),
            _ => loop {
                std::thread::sleep(std::time::Duration::from_millis(50));
            },
        }
    }

    /// 子を起動し、`signal`（`kill -<signal>`。None なら子が自分で落ちる）のあとの次の `begin` の判定を返す
    fn judge_after_child(mode: &str, signal: Option<&str>) -> Exit {
        use std::io::{BufRead, BufReader};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("child.run");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tests::child_entry", "--nocapture"])
            .env(CHILD_MODE, mode)
            .env(CHILD_PATH, &path)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        while let Some(Ok(line)) = lines.next() {
            if line.trim() == READY {
                break;
            }
        }
        if let Some(signal) = signal {
            Command::new("kill")
                .arg(format!("-{signal}"))
                .arg(child.id().to_string())
                .status()
                .unwrap();
        }
        child.wait().unwrap();
        RunMark::begin(&path).1.unwrap().exit
    }

    #[test]
    #[ignore = "R1 で中身を入れたら外す（ADR 0051 §8）"]
    fn fatal_signals_are_recorded_as_crash() {
        assert_eq!(judge_after_child("segv", None), Exit::Crash);
        assert_eq!(judge_after_child("abort", None), Exit::Crash);
    }

    #[test]
    #[ignore = "R1 で中身を入れたら外す（ADR 0051 §8）"]
    fn sigterm_is_term_and_sigkill_is_unknown() {
        assert_eq!(judge_after_child("wait", Some("TERM")), Exit::Term);
        assert_eq!(judge_after_child("wait", Some("KILL")), Exit::Unknown);
    }
}
