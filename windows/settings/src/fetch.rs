//! 取得（`ikari-data.exe`）の起動、進み具合、取り消しの核（ADR 0076 §3.1、§8.1。D-2c）。**OS に依存しない**
//! （OS の呼び出しは `fetch_os`。ここは `Launcher` を差し替えてテストする）。
//!
//! - 引数は全部この核が作る（`model_fetch_args`）。`ikari-data` が同じ検査をやり直す（二重の守り）。
//! - 子の標準出力の行（`resource::fetch_report`）を、別のスレッドで読んで状態にまとめる。行は信じすぎない:
//!   長すぎる行、知らない行、終わりの行のあとの行は捨てる。
//! - 取り消しは、子の木を終わらせる（Windows は Job Object を閉じる）。次の起動の始めに、`ikari-data` が残りを消す。
//! - 文は `text.rs`。画面（C#）は文と数を描くだけ。

use std::io::{self, BufReader, Read};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use ikari_resource::fetch_report::{FetchErrorCode, Line, Report, ReportReader, read_line_capped};

/// 取得の頼み。値は画面が集める（確認の窓のあと）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchRequest {
    Preset {
        id: String,
    },
    Url {
        url: String,
        format: String,
        sha256: Option<String>,
        name: Option<String>,
    },
    /// 版の確認（`ikari-data version-check`。ADR 0076 §6。結果は `Snapshot::latest`）
    VersionCheck,
}

/// `ikari-data` の引数（`model fetch …`、`version-check`）。空の値は引数に出さない
pub fn model_fetch_args(request: &FetchRequest) -> Vec<String> {
    if *request == FetchRequest::VersionCheck {
        return vec!["version-check".to_owned()];
    }
    let mut args = vec!["model".to_owned(), "fetch".to_owned()];
    let mut push = |key: &str, value: &str| {
        args.push(format!("--{key}"));
        args.push(value.to_owned());
    };
    match request {
        FetchRequest::Preset { id } => push("preset", id),
        FetchRequest::Url {
            url,
            format,
            sha256,
            name,
        } => {
            push("url", url);
            push("format", format);
            if let Some(sha256) = sha256.as_deref().filter(|s| !s.is_empty()) {
                push("sha256", sha256);
            }
            if let Some(name) = name.as_deref().filter(|s| !s.is_empty()) {
                push("name", name);
            }
        }
        FetchRequest::VersionCheck => {}
    }
    args
}

/// 取得の段階
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Running,
    /// `done` を受けた（取り込みはエンジンが見回る）
    Done,
    Failed,
    Cancelled,
}

impl Phase {
    /// C の ABI の番号
    pub fn code(self) -> i32 {
        match self {
            Self::Running => 0,
            Self::Done => 1,
            Self::Failed => 2,
            Self::Cancelled => 3,
        }
    }
}

/// ある時点の様子
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub phase: Phase,
    pub received: u64,
    pub total: Option<u64>,
    pub error: Option<FetchErrorCode>,
    /// `done` の束の名前
    pub bundle: Option<String>,
    /// `done` の sha256（`ikari-data` が写しながら数えた値。sha256 なしで取ったときに確認の画面に出す。ADR 0076 §18.1）
    pub sha256: Option<String>,
    /// `latest` の版（版の確認。`数.数.数`）
    pub latest: Option<String>,
}

impl Snapshot {
    fn started() -> Self {
        Self {
            phase: Phase::Running,
            received: 0,
            total: None,
            error: None,
            bundle: None,
            sha256: None,
            latest: None,
        }
    }
}

/// 起こした子
pub struct Running {
    /// 子の標準出力
    pub stdout: Box<dyn Read + Send>,
    /// 子の木を終わらせる（何度呼んでもよい）
    pub kill: Box<dyn FnMut() + Send>,
    /// 終わるのを待つ。終了コード（取れなければ `None`）
    pub wait: Box<dyn FnOnce() -> Option<i32> + Send>,
}

/// 子を起こす部品（製品は `fetch_os`、テストは差し替える）
pub trait Launcher {
    fn launch(&self, args: &[String]) -> io::Result<Running>;
}

/// 走っている取得。捨てると取り消す
pub struct FetchTask {
    state: Arc<Mutex<Snapshot>>,
    kill: Mutex<Box<dyn FnMut() + Send>>,
    reader: Option<JoinHandle<()>>,
}

fn lock(state: &Mutex<Snapshot>) -> MutexGuard<'_, Snapshot> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

impl FetchTask {
    /// 起こして、読み始める。起こせなければ `Err`（文は `text::fetch_error(Tool)`）
    pub fn start(launcher: &dyn Launcher, request: &FetchRequest) -> Result<Self, FetchErrorCode> {
        let Running { stdout, kill, wait } = launcher
            .launch(&model_fetch_args(request))
            .map_err(|_| FetchErrorCode::Tool)?;
        let state = Arc::new(Mutex::new(Snapshot::started()));
        let reader = {
            let state = Arc::clone(&state);
            thread::spawn(move || read_reports(stdout, wait, &state))
        };
        Ok(Self {
            state,
            kill: Mutex::new(kill),
            reader: Some(reader),
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        lock(&self.state).clone()
    }

    /// 取り消す。走っているときだけ「取り消し」にする（終わったあとは変えない）
    pub fn cancel(&self) {
        {
            let mut s = lock(&self.state);
            if s.phase == Phase::Running {
                s.phase = Phase::Cancelled;
            }
        }
        (self.kill.lock().unwrap_or_else(|e| e.into_inner()))();
    }
}

impl Drop for FetchTask {
    fn drop(&mut self) {
        self.cancel();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// 子の行を読み切って、状態を更新する
fn read_reports(
    stdout: Box<dyn Read + Send>,
    wait: Box<dyn FnOnce() -> Option<i32> + Send>,
    state: &Mutex<Snapshot>,
) {
    let mut reader = BufReader::new(stdout);
    let mut reports = ReportReader::new();
    loop {
        match read_line_capped(&mut reader) {
            Ok(Line::Text(line)) => {
                if let Some(report) = reports.feed(&line) {
                    apply(&mut lock(state), report);
                }
            }
            Ok(Line::Dropped) => {}
            Ok(Line::Eof) | Err(_) => break,
        }
    }
    let exit = wait();
    let mut s = lock(state);
    if s.phase == Phase::Running {
        // 終わりの行がないまま終わった（落ちた）。成功の終了コードでも、`done` がなければ成功にしない
        s.phase = Phase::Failed;
        s.error = Some(if exit.is_none() {
            FetchErrorCode::Tool
        } else {
            FetchErrorCode::Internal
        });
    }
}

fn apply(s: &mut Snapshot, report: Report) {
    // 取り消したあとに届いた行は、状態を変えない
    if s.phase != Phase::Running {
        return;
    }
    match report {
        Report::Progress { received, total } => {
            s.received = received;
            s.total = total;
        }
        Report::Done { name, sha256 } => {
            s.phase = Phase::Done;
            s.bundle = Some(name);
            s.sha256 = Some(sha256);
        }
        Report::Error(code) => {
            s.phase = Phase::Failed;
            s.error = Some(code);
        }
        // 版の確認の終わり（比べるのは `ikari_resource::app_version::verdict`。文は `text`）
        Report::Latest { version } => {
            s.phase = Phase::Done;
            s.latest = Some(version);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    const SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    struct Fake {
        output: Vec<u8>,
        killed: Arc<AtomicBool>,
        exit: Option<i32>,
        fail_launch: bool,
    }

    impl Fake {
        fn new(output: &str) -> Self {
            Self {
                output: output.as_bytes().to_vec(),
                killed: Arc::new(AtomicBool::new(false)),
                exit: Some(0),
                fail_launch: false,
            }
        }
    }

    impl Launcher for Fake {
        fn launch(&self, args: &[String]) -> io::Result<Running> {
            if self.fail_launch {
                return Err(io::Error::other("no exe"));
            }
            assert_eq!(&args[..2], ["model", "fetch"]);
            let killed = Arc::clone(&self.killed);
            let exit = self.exit;
            Ok(Running {
                stdout: Box::new(Cursor::new(self.output.clone())),
                kill: Box::new(move || killed.store(true, Ordering::SeqCst)),
                wait: Box::new(move || exit),
            })
        }
    }

    fn preset() -> FetchRequest {
        FetchRequest::Preset {
            id: "zenz-small".into(),
        }
    }

    /// 読み終わるまで待って、最後の様子を返す（取り消しはしない）
    fn finish(fake: &Fake, request: &FetchRequest) -> Snapshot {
        let mut task = FetchTask::start(fake, request).unwrap();
        task.reader.take().unwrap().join().unwrap();
        task.snapshot()
    }

    #[test]
    fn args_are_built_by_the_core() {
        assert_eq!(
            model_fetch_args(&preset()),
            ["model", "fetch", "--preset", "zenz-small"]
        );
        let url = FetchRequest::Url {
            url: "https://e.com/m".into(),
            format: "zenz".into(),
            sha256: Some(String::new()),
            name: Some("n".into()),
        };
        assert_eq!(
            model_fetch_args(&url),
            [
                "model",
                "fetch",
                "--url",
                "https://e.com/m",
                "--format",
                "zenz",
                "--name",
                "n"
            ]
        );
    }

    #[test]
    fn progress_then_done() {
        let out = format!("progress 5 10\nprogress 10 10\ndone zenz-small {SHA}\n");
        let s = finish(&Fake::new(&out), &preset());
        assert_eq!(s.phase, Phase::Done);
        assert_eq!((s.received, s.total), (10, Some(10)));
        assert_eq!(s.bundle.as_deref(), Some("zenz-small"));
        assert_eq!(s.sha256.as_deref(), Some(SHA));
        assert_eq!(s.latest, None);
    }

    #[test]
    fn version_check_args_and_latest_line() {
        assert_eq!(
            model_fetch_args(&FetchRequest::VersionCheck),
            ["version-check"]
        );
        let state = Mutex::new(Snapshot::started());
        read_reports(
            Box::new(Cursor::new(b"latest 0.3.1\n".to_vec())),
            Box::new(|| Some(0)),
            &state,
        );
        let s = lock(&state).clone();
        assert_eq!(s.phase, Phase::Done);
        assert_eq!(s.latest.as_deref(), Some("0.3.1"));
        assert_eq!(s.bundle, None);
    }

    #[test]
    fn error_line_fails_with_its_word() {
        let s = finish(&Fake::new("progress 1 -\nerror checksum\n"), &preset());
        assert_eq!(s.phase, Phase::Failed);
        assert_eq!(s.error, Some(FetchErrorCode::Checksum));
        assert_eq!(s.total, None);
    }

    #[test]
    fn lines_after_the_final_line_and_garbage_are_ignored() {
        let out = format!("garbage\nerror network\ndone x {SHA}\nprogress 9 9\n");
        let s = finish(&Fake::new(&out), &preset());
        assert_eq!(s.phase, Phase::Failed);
        assert_eq!(s.error, Some(FetchErrorCode::Network));
        assert_eq!(s.received, 0);
    }

    #[test]
    fn ending_without_a_final_line_is_a_failure_even_with_exit_zero() {
        let s = finish(&Fake::new("progress 1 2\n"), &preset());
        assert_eq!(s.phase, Phase::Failed);
        assert_eq!(s.error, Some(FetchErrorCode::Internal));
    }

    #[test]
    fn launch_failure_is_tool() {
        let mut fake = Fake::new("");
        fake.fail_launch = true;
        assert_eq!(
            FetchTask::start(&fake, &preset()).err(),
            Some(FetchErrorCode::Tool)
        );
    }

    #[test]
    fn cancel_kills_and_sticks() {
        let fake = Fake::new("");
        let killed = Arc::clone(&fake.killed);
        let task = FetchTask::start(&fake, &preset()).unwrap();
        task.cancel();
        assert!(killed.load(Ordering::SeqCst));
        // 読み終わったあとに失敗の行や終了が来ても、取り消しのまま
        drop(task);
    }

    #[test]
    fn cancel_wins_over_late_reports() {
        let state = Mutex::new(Snapshot::started());
        lock(&state).phase = Phase::Cancelled;
        let out = format!("done x {SHA}\n");
        read_reports(
            Box::new(Cursor::new(out.into_bytes())),
            Box::new(|| Some(0)),
            &state,
        );
        assert_eq!(lock(&state).phase, Phase::Cancelled);
    }

    #[test]
    fn dropping_the_task_cancels_it() {
        let fake = Fake::new("");
        let killed = Arc::clone(&fake.killed);
        drop(FetchTask::start(&fake, &preset()).unwrap());
        assert!(killed.load(Ordering::SeqCst));
    }
}
