//! 起動中の印と、前回の終わり方の判定（ADR 0051）。
//!
//! 書式（ADR 0051 §3.1）:
//!
//! ```text
//! v=1 start_ms=1793000000123
//! exit=crash sig=11
//! ```
//!
//! 1 行目は `begin` で作り直し、2 行目からは追記（`finish_clean`、シグナルの処理）。
//!
//! 印の中身は固定の語と数だけ（NFR-19）。panic のメッセージは書かない。

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
mod signal;

/// 場所を絞り込めなかったときの語（`sanitize_location` の戻り。ADR 0051 §5.3）
pub const EXTERNAL_LOCATION: &str = "external";

/// 場所（`<相対パス>.rs:<行>`）の最大バイト数（ADR 0051 §5.2）
pub const MAX_LOCATION_BYTES: usize = 96;

/// 印の書式の版（1 行目の `v=`）。書式を変えたら上げる。違う版は `unknown`
const FORMAT_VERSION: u32 = 1;

/// 印の権限（`ikari_core::user::PRIVATE_FILE_MODE` と同じ。この crate は core に依存しない）
#[cfg(unix)]
const PRIVATE_FILE_MODE: u32 = 0o600;

/// 印の親のディレクトリの権限（`ikari_core::user::PRIVATE_DIR_MODE` と同じ）
#[cfg(unix)]
const PRIVATE_DIR_MODE: u32 = 0o700;

/// 読む印の大きさの上限。これより大きい印は壊れているとみなす（`unknown`）
const MAX_MARK_BYTES: u64 = 4096;

const VERSION_KEY: &str = "v=";
const START_KEY: &str = "start_ms=";
const EXIT_KEY: &str = "exit=";
const SIG_KEY: &str = "sig=";

/// 正常な終わりの行
const EXIT_CLEAN_LINE: &[u8] = b"exit=clean\n";
/// 終わらせるシグナルの行の頭（このあとにシグナルの番号と改行）
#[cfg_attr(not(unix), allow(dead_code))]
const EXIT_TERM_LINE_PREFIX: &str = "exit=term sig=";
/// 致命的なシグナルの行の頭
#[cfg_attr(not(unix), allow(dead_code))]
const EXIT_CRASH_LINE_PREFIX: &str = "exit=crash sig=";

/// panic の hook が渡すスレッドの語（名前のないスレッド、形の外れた名前）
const OTHER_THREAD: &str = "other";
/// Rust の main のスレッドの名前
const MAIN_THREAD: &str = "main";
/// そのまま渡すスレッドの名前の頭（ワークスペースで付けている名前。`ikari-log` など）
const IKARI_THREAD_PREFIX: &str = "ikari-";
/// そのまま渡すスレッドの名前の最大バイト数
const MAX_THREAD_WORD_BYTES: usize = 32;

const MS_PER_S: u64 = 1000;

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

impl PrevExit {
    const UNKNOWN: PrevExit = PrevExit {
        exit: Exit::Unknown,
        sig: None,
        up_s: None,
    };
}

/// 今回の起動中の印。落とすだけでは何も書かない（正常な終わりは `finish_clean`）。
#[derive(Debug)]
pub struct RunMark {
    /// 追記で開いた印。作れなかったときは None（何も書かない）
    file: Option<File>,
}

impl RunMark {
    /// 前回の印（`path`）を読んで判定し、今回の印を作り直す。
    /// 印がない（初めての起動）、または書けないときは判定が `None`。
    /// 親のディレクトリがなければ作る（権限は自分だけ）。
    ///
    /// シグナルの処理（`install_signal_handlers`）の書き先は、最後に `begin` した印になる。
    pub fn begin(path: &Path) -> (RunMark, Option<PrevExit>) {
        let prev = read_prev(path);
        let Some(file) = recreate(path) else {
            return (RunMark { file: None }, None);
        };
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            signal::set_mark_fd(file.as_raw_fd());
        }
        (RunMark { file: Some(file) }, prev)
    }

    /// 正常な終わりを印に書く（`exit=clean`）。書けなくても何もしない（終わる途中で止めない）。
    pub fn finish_clean(&self) {
        if let Some(mut file) = self.file.as_ref() {
            let _ = file.write_all(EXIT_CLEAN_LINE);
        }
    }
}

impl Drop for RunMark {
    fn drop(&mut self) {
        // 閉じた fd の番号はほかのファイルに使い回されうるので、シグナルの処理の書き先から先に外す
        #[cfg(unix)]
        if let Some(file) = &self.file {
            use std::os::fd::AsRawFd;
            signal::clear_mark_fd(file.as_raw_fd());
        }
    }
}

/// 前回の印を読んで判定する。印がなければ None、読めない・壊れていれば `Unknown`
fn read_prev(path: &Path) -> Option<PrevExit> {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => return Some(PrevExit::UNKNOWN),
    };
    let mtime = file.metadata().and_then(|m| m.modified()).ok();
    let mut body = String::new();
    let read = (&mut file)
        .take(MAX_MARK_BYTES + 1)
        .read_to_string(&mut body);
    if read.is_err() || body.len() as u64 > MAX_MARK_BYTES {
        return Some(PrevExit::UNKNOWN);
    }
    Some(judge(&body, mtime))
}

/// 印の中身から判定する（ADR 0051 §3.1）。最後の `exit=` の行で決め、なければ `Unknown`。
/// 形の外れた行が 1 つでもあれば `Unknown`
fn judge(body: &str, mtime: Option<SystemTime>) -> PrevExit {
    let mut lines = body.lines();
    let Some(start_ms) = lines.next().and_then(parse_header) else {
        return PrevExit::UNKNOWN;
    };
    let mut last = None;
    for line in lines {
        match parse_exit(line) {
            Some(exit) => last = Some(exit),
            None => return PrevExit::UNKNOWN,
        }
    }
    let Some((exit, sig)) = last else {
        return PrevExit::UNKNOWN;
    };
    // 時計が戻って終わりが始まりより前なら書かない（ADR 0051 §9）
    let up_s = mtime
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|end| u64::try_from(end.as_millis()).ok())
        .and_then(|end_ms| end_ms.checked_sub(start_ms))
        .map(|ms| ms / MS_PER_S);
    PrevExit { exit, sig, up_s }
}

/// 1 行目 `v=1 start_ms=<数>` の起動の時刻。版が違えば None
fn parse_header(line: &str) -> Option<u64> {
    let (v, start) = line.split_once(' ')?;
    let v: u32 = parse_digits(v.strip_prefix(VERSION_KEY)?)?;
    if v != FORMAT_VERSION {
        return None;
    }
    parse_digits(start.strip_prefix(START_KEY)?)
}

/// `exit=clean` / `exit=term sig=N` / `exit=crash sig=N`
fn parse_exit(line: &str) -> Option<(Exit, Option<i32>)> {
    let rest = line.strip_prefix(EXIT_KEY)?;
    if rest == Exit::Clean.as_word() {
        return Some((Exit::Clean, None));
    }
    let (word, sig) = rest.split_once(' ')?;
    let exit = if word == Exit::Term.as_word() {
        Exit::Term
    } else if word == Exit::Crash.as_word() {
        Exit::Crash
    } else {
        return None;
    };
    let sig: i32 = parse_digits(sig.strip_prefix(SIG_KEY)?)?;
    Some((exit, Some(sig)))
}

/// 符号なしの十進の数だけ（`+` や空白を許さない）
fn parse_digits<T: std::str::FromStr>(s: &str) -> Option<T> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// 今回の印を作り直し、1 行目を書く。以後の書き込みは追記
fn recreate(path: &Path) -> Option<File> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty())
        && !parent.is_dir()
    {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(PRIVATE_DIR_MODE);
        }
        builder.create(parent).ok()?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(PRIVATE_FILE_MODE);
    }
    let mut file = options.open(path).ok()?;
    let start_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0);
    file.write_all(format!("{VERSION_KEY}{FORMAT_VERSION} {START_KEY}{start_ms}\n").as_bytes())
        .ok()?;
    // 1 行目のあとは追記にする（シグナルの処理の `write(2)` も、`finish_clean` も末尾に足す）。
    // O_TRUNC と O_APPEND を同時に付けられないので、開き直す
    drop(file);
    OpenOptions::new().append(true).open(path).ok()
}

/// 終わらせるシグナルと致命的なシグナルの処理を入れる（`cfg(unix)` のみ。プロセスの入口で 1 回）。
/// 2 回目からは何もしない。処理は今の印（最後に `begin` した `RunMark`）に `exit=term sig=N` /
/// `exit=crash sig=N` を足し、前の処理に戻して終わり方を変えない（クラッシュレポートも作られる）。
pub fn install_signal_handlers() {
    #[cfg(unix)]
    signal::install();
}

/// panic の記録の送り先。引数は `(thread の語, 絞り込んだ場所)`（メッセージは渡さない）
pub type PanicSink = Box<dyn Fn(&str, &str) + Send + Sync + 'static>;

/// panic の hook を入れる。`sink` へ `(thread の語, 絞り込んだ場所)` を渡す。前の hook は呼んでから記録する。
/// メッセージは `sink` に渡さない。呼ぶたびに hook が重なるので、プロセスの入口で 1 回だけ呼ぶ。
///
/// thread の語: Rust の main のスレッドは `main`、`ikari-` で始まる名前（英小文字・数字・`-`、
/// 32 バイトまで）はそのまま、ほか（名前なし、Swift などが作ったスレッド）は `other`。
/// 語を ADR 0051 §5.1・§5.2 の一覧に寄せるのは呼び出し側（`sink`）で行う。
pub fn install_panic_hook(sink: PanicSink) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        prev(info);
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()));
        let loc = loc.as_deref().map_or(EXTERNAL_LOCATION, sanitize_location);
        let thread = std::thread::current();
        sink(thread_word(thread.name()), loc);
    }));
}

/// スレッドの名前を固定の語に絞る（形の外れたもの、名前のないものは `other`）
fn thread_word(name: Option<&str>) -> &str {
    match name {
        Some(MAIN_THREAD) => MAIN_THREAD,
        Some(n)
            if n.len() <= MAX_THREAD_WORD_BYTES
                && n.len() > IKARI_THREAD_PREFIX.len()
                && n.starts_with(IKARI_THREAD_PREFIX)
                && n.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') =>
        {
            n
        }
        _ => OTHER_THREAD,
    }
}

/// panic の場所を絞り込む。`[A-Za-z0-9_/.-]+\.rs:[0-9]+` で 96 バイトまで、`/` で始まらず、`..` を含まない
/// ものだけそのまま返す。ほかは `EXTERNAL_LOCATION`。
///
/// 場所の形の検査はここを正とする（エンジンの許可リストもこれを使う。ADR 0051 §5.3）。
pub fn sanitize_location(loc: &str) -> &str {
    if is_workspace_location(loc) {
        loc
    } else {
        EXTERNAL_LOCATION
    }
}

fn is_workspace_location(loc: &str) -> bool {
    if loc.len() > MAX_LOCATION_BYTES || loc.starts_with('/') || loc.contains("..") {
        return false;
    }
    let Some((path, line)) = loc.rsplit_once(".rs:") else {
        return false;
    };
    !path.is_empty()
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'/' | b'.' | b'-'))
        && !line.is_empty()
        && line.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    // 子プロセスを使うテストは Unix だけ（シグナルの処理が cfg(unix) のため）
    #[cfg(unix)]
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
    fn words_are_fixed() {
        assert_eq!(Exit::Clean.as_word(), "clean");
        assert_eq!(Exit::Term.as_word(), "term");
        assert_eq!(Exit::Crash.as_word(), "crash");
        assert_eq!(Exit::Unknown.as_word(), "unknown");
    }

    #[test]
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
    fn last_exit_line_wins_and_junk_lines_are_unknown() {
        let p = judge(Some("v=1 start_ms=1\nexit=clean\nexit=term sig=15\n")).unwrap();
        assert_eq!((p.exit, p.sig), (Exit::Term, Some(15)));
        for body in [
            "v=1 start_ms=1\nexit=clean\nnoise\n",
            "v=1 start_ms=-1\nexit=clean\n",
            "v=1 start_ms=1\nexit=term\n",
            "v=1 start_ms=1\nexit=clean sig=1\n",
            "v=1 start_ms=1\nexit=crash sig=+11\n",
            "v=1\nexit=clean\n",
        ] {
            assert_eq!(exit_of(Some(body)), Some(Exit::Unknown), "{body:?}");
        }
        // 大きすぎる印は壊れているとみなす
        let huge = format!("v=1 start_ms=1\n{}", "exit=clean\n".repeat(1000));
        assert_eq!(exit_of(Some(&huge)), Some(Exit::Unknown));
    }

    #[test]
    fn up_s_comes_from_mtime() {
        let start_ms = 1_000_000_000_000u64;
        let mtime = UNIX_EPOCH + std::time::Duration::from_millis(start_ms + 5_310_999);
        let body = format!("v=1 start_ms={start_ms}\nexit=crash sig=11\n");
        assert_eq!(super::judge(&body, Some(mtime)).up_s, Some(5310));
        // 時計が戻った（終わりが始まりより前）→ 書かない
        let early = UNIX_EPOCH + std::time::Duration::from_millis(start_ms - 1);
        assert_eq!(super::judge(&body, Some(early)).up_s, None);
        assert_eq!(super::judge(&body, None).up_s, None);
        // unknown は up_s を出さない
        let only_header = format!("v=1 start_ms={start_ms}\n");
        assert_eq!(super::judge(&only_header, Some(mtime)).up_s, None);
    }

    #[test]
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

    #[cfg(unix)]
    #[test]
    fn mark_is_private_and_parent_is_created() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs").join("test.run");
        let (_mark, prev) = RunMark::begin(&path);
        assert_eq!(prev, None);
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), PRIVATE_FILE_MODE);
        assert_eq!(mode(path.parent().unwrap()), PRIVATE_DIR_MODE);
    }

    #[test]
    fn unwritable_mark_gives_no_judgement() {
        let dir = tempfile::tempdir().unwrap();
        // 親がファイルなので印を作れない
        let blocker = dir.path().join("file");
        fs::write(&blocker, "x").unwrap();
        let (mark, prev) = RunMark::begin(&blocker.join("test.run"));
        assert_eq!(prev, None);
        mark.finish_clean(); // 何もしない（落ちない）
    }

    #[test]
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
        // ちょうど上限のバイト数は残す
        let edge = format!("{}.rs:1", "a".repeat(MAX_LOCATION_BYTES - ".rs:1".len()));
        assert_eq!(sanitize_location(&edge), edge);
    }

    #[test]
    fn thread_words_are_fixed_shapes() {
        assert_eq!(thread_word(Some("main")), "main");
        assert_eq!(thread_word(Some("ikari-log")), "ikari-log");
        assert_eq!(thread_word(Some("ikari-lm-warmup")), "ikari-lm-warmup");
        for other in [
            None,
            Some("ikari-"),
            Some("ikari-Log"),
            Some("ikari-ログ"),
            Some("tokio-runtime-worker"),
            Some(&*format!("ikari-{}", "a".repeat(MAX_THREAD_WORD_BYTES))),
        ] {
            assert_eq!(thread_word(other), "other", "{other:?}");
        }
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
            "abort" => {
                // 終わりの時刻（印の更新時刻）は、Linux ではカーネルの粗い時計で付き、起動の時刻（start_ms）より
                // 数 ms 前になることがある。すぐ落ちると up_s が None になり、テストが揺れるので、少し待つ
                std::thread::sleep(std::time::Duration::from_millis(50));
                std::process::abort()
            }
            "panic" => {
                install_panic_hook(Box::new(|thread, loc| {
                    println!("panic thread={thread} loc={loc}");
                }));
                std::thread::Builder::new()
                    .name("ikari-test".to_owned())
                    .spawn(|| panic!("secret message"))
                    .unwrap()
                    .join()
                    .ok();
                std::process::exit(0);
            }
            _ => loop {
                std::thread::sleep(std::time::Duration::from_millis(50));
            },
        }
    }

    /// 子を起動して `ready` まで待つ。戻りは子と、`ready` のあとの標準出力の行
    #[cfg(unix)]
    fn spawn_child(
        mode: &str,
        path: &Path,
    ) -> (std::process::Child, impl Iterator<Item = String> + use<>) {
        use std::io::{BufRead, BufReader};
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tests::child_entry", "--nocapture"])
            .env(CHILD_MODE, mode)
            .env(CHILD_PATH, path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap())
            .lines()
            .map_while(Result::ok);
        for line in lines.by_ref() {
            if line.trim() == READY {
                break;
            }
        }
        (child, lines)
    }

    /// 子を起動し、`signal`（`kill -<signal>`。None なら子が自分で落ちる）のあとの次の `begin` の判定を返す
    #[cfg(unix)]
    fn judge_after_child(mode: &str, signal: Option<&str>) -> PrevExit {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("child.run");
        let (mut child, _lines) = spawn_child(mode, &path);
        if let Some(signal) = signal {
            Command::new("kill")
                .arg(format!("-{signal}"))
                .arg(child.id().to_string())
                .status()
                .unwrap();
        }
        let status = child.wait().unwrap();
        // 終わり方は処理を入れる前と同じ（シグナルで終わる）
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            assert!(status.signal().is_some(), "{mode}: {status:?}");
        }
        let _ = status;
        RunMark::begin(&path).1.unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn fatal_signals_are_recorded_as_crash() {
        let p = judge_after_child("segv", None);
        assert_eq!(p.exit, Exit::Crash);
        // null への書き込みは macOS では SIGSEGV か SIGBUS
        assert!(matches!(p.sig, Some(libc::SIGSEGV | libc::SIGBUS)), "{p:?}");
        let p = judge_after_child("abort", None);
        assert_eq!((p.exit, p.sig), (Exit::Crash, Some(libc::SIGABRT)));
        assert!(p.up_s.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn sigterm_is_term_and_sigkill_is_unknown() {
        let p = judge_after_child("wait", Some("TERM"));
        assert_eq!((p.exit, p.sig), (Exit::Term, Some(libc::SIGTERM)));
        let p = judge_after_child("wait", Some("KILL"));
        assert_eq!((p.exit, p.sig, p.up_s), (Exit::Unknown, None, None));
    }

    #[cfg(unix)]
    #[test]
    fn panic_hook_passes_thread_and_location_but_not_the_message() {
        let dir = tempfile::tempdir().unwrap();
        let (mut child, lines) = spawn_child("panic", &dir.path().join("child.run"));
        let out: Vec<String> = lines.collect();
        child.wait().unwrap();
        let panics: Vec<&String> = out.iter().filter(|l| l.starts_with("panic ")).collect();
        assert_eq!(panics.len(), 1, "{out:?}");
        // ワークスペースの中の相対パスと行は残る
        let rest = panics[0]
            .strip_prefix("panic thread=ikari-test loc=runmark/src/lib.rs:")
            .unwrap_or_else(|| panic!("{out:?}"));
        assert!(rest.parse::<u32>().is_ok(), "{out:?}");
        assert!(!out.iter().any(|l| l.contains("secret")), "{out:?}");
    }
}
