//! 候補窓（`ikari-candidate.exe`）を起動して見張る（ADR 0062 §5.4 の決定 1 と 4。#414）。
//!
//! 判断の置き場所: Rust。ここは OS に依存しない部分（実行ファイルの場所、起動し直す間隔と回数の上限、見張りの繰り返し）。
//! 子を作って待つ OS 層は `os`（`cfg(windows)`）。
//!
//! - 順序（ここを呼ぶ場所。`main.rs`）: 別のエンジンが動いていないことを確かめたあと、エンジンを開く前に呼ぶ。
//!   窓の寿命は、エンジンが開けたかどうかにつなげない（開けなくても engine-host は常駐し、窓はかなの候補を出す）。
//! - 窓の寿命は engine-host のプロセスにつなぐ（Job Object の「閉じたら終了」。決定 4）。`CandidateWindow` を落とすと、
//!   見張りをやめて窓を終わらせる。engine-host が落ちても、OS が Job を閉じて窓を終わらせる。
//! - 起動の規則（ADR 0067 §4.3）: 実行ファイルは engine-host の exe の場所から決めた絶対パス
//!   （`<インストール先>\candidate\ikari-candidate.exe`。TIP が照らす場所と同じ。ADR 0068 §5）。引数なし、
//!   ハンドルは継承させない、カレントディレクトリは実行ファイルのフォルダ。engine-host は高い権限では起動しない
//!   （`run::check_launch`）ので、窓も高い権限にならない（窓の核も自分で確かめる）。
//! - 落ちたら起動し直す。回数の上限は、エンジンの落ちた回数の数え方（`shell-core::engine` の `CRASH_LIMIT`、
//!   `CRASH_WINDOW`、`SUSPENSION`）と同じ値を使う。実行ファイルが無いときは見張りをやめる（窓なしで入力を続ける。
//!   ADR 0062 §5.4 の決定 2）。
//! - ログ（標準エラー）には固定の語と数だけを出す（候補の文字は、このプロセスを通らない）。

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ikari_pipe::peer::{CANDIDATE_DIR_NAME, CANDIDATE_EXE_NAME};
use ikari_shell_core::engine::{CRASH_LIMIT, CRASH_WINDOW, SUSPENSION};

#[cfg(windows)]
#[allow(unsafe_code)]
mod os;

/// 窓が終わってから起動し直すまでの間隔。**仮置き（計測なし）**: 落ちてすぐ起動し直すと、起動の直後に落ちる不具合で
/// CPU を回し続けるので、少し間を置く。窓がない間は、下線つきの 1 位で入力が続く（ADR 0062 §5.4）
pub const RESTART_DELAY: Duration = Duration::from_secs(1);

/// engine-host の実行ファイル `engine_exe` から決まる、候補窓の実行ファイルの絶対パス。
/// `engine_exe` が絶対パスでない・親のフォルダがないときは `None`（起動しない）。
pub fn candidate_exe(engine_exe: &Path) -> Option<PathBuf> {
    if !engine_exe.is_absolute() {
        return None;
    }
    let dir = engine_exe.parent()?;
    Some(dir.join(CANDIDATE_DIR_NAME).join(CANDIDATE_EXE_NAME))
}

/// 起動し直す間隔と回数の上限（純粋。Mac でテストする）。
#[derive(Debug, Default)]
pub struct RestartPolicy {
    /// `CRASH_WINDOW` の中で窓が終わった時刻
    exits: VecDeque<Instant>,
}

impl RestartPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    /// 窓が終わった（または起動できなかった）。次に起動するまで待つ長さを返す。
    /// `CRASH_WINDOW` の中で `CRASH_LIMIT` 回に達したら `SUSPENSION` 休み、数え直す。
    pub fn after_exit(&mut self, now: Instant) -> Duration {
        while let Some(&first) = self.exits.front() {
            if now.saturating_duration_since(first) > CRASH_WINDOW {
                self.exits.pop_front();
            } else {
                break;
            }
        }
        self.exits.push_back(now);
        if self.exits.len() >= CRASH_LIMIT {
            self.exits.clear();
            SUSPENSION
        } else {
            RESTART_DELAY
        }
    }
}

/// 窓のプロセスを作って待つ手段（Windows は `os`、テストは偽物）。
pub trait Launcher: Send + 'static {
    type Child: Send;

    /// 子を起動する。
    fn launch(&mut self) -> io::Result<Self::Child>;
    /// 子が終わるまで待ち、終了コードを返す（取れなければ `None`）。
    fn wait(&mut self, child: Self::Child) -> Option<u32>;
}

/// 見張りを止める合図（待ちを起こせる）。
#[derive(Debug, Default)]
pub struct StopSignal {
    stopped: Mutex<bool>,
    wake: Condvar,
}

impl StopSignal {
    pub fn stop(&self) {
        *self.stopped.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.wake.notify_all();
    }

    pub fn is_stopped(&self) -> bool {
        *self.stopped.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `delay` だけ待つ。途中で止められたら `false`。
    pub fn sleep(&self, delay: Duration) -> bool {
        let deadline = Instant::now() + delay;
        let mut stopped = self.stopped.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if *stopped {
                return false;
            }
            let now = Instant::now();
            if now >= deadline {
                return true;
            }
            stopped = self
                .wake
                .wait_timeout(stopped, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// 見張りの繰り返しで起きたこと（ログとテスト用。固定の語と数だけ）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    Started,
    /// 終わった（終了コード）
    Exited(Option<u32>),
    /// 起動できなかった（`io::ErrorKind` の名前）
    LaunchFailed(String),
    /// 実行ファイルが無いので、見張りをやめた
    Missing,
    /// 次に起動するまで待つ
    Waiting(Duration),
}

/// 起動し、終わったら間隔を置いて起動し直す。止められるか、実行ファイルが無ければ戻る。
/// `clock` は今の時刻、`report` は起きたことの知らせ（ログ）。
pub fn supervise<L: Launcher>(
    mut launcher: L,
    stop: &StopSignal,
    clock: impl Fn() -> Instant,
    mut report: impl FnMut(WatchEvent),
) {
    let mut policy = RestartPolicy::new();
    while !stop.is_stopped() {
        match launcher.launch() {
            Ok(child) => {
                report(WatchEvent::Started);
                let code = launcher.wait(child);
                report(WatchEvent::Exited(code));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                report(WatchEvent::Missing);
                return;
            }
            Err(e) => report(WatchEvent::LaunchFailed(format!("{:?}", e.kind()))),
        }
        if stop.is_stopped() {
            return;
        }
        let delay = policy.after_exit(clock());
        report(WatchEvent::Waiting(delay));
        if !stop.sleep(delay) {
            return;
        }
    }
}

/// 起動した候補窓。engine-host が終わるまで持つ（落とすと見張りをやめて、窓を終わらせる）。
#[derive(Debug)]
pub struct CandidateWindow {
    stop: Arc<StopSignal>,
    #[cfg(windows)]
    job: Option<Arc<os::Job>>,
}

impl Drop for CandidateWindow {
    fn drop(&mut self) {
        self.stop.stop();
        #[cfg(windows)]
        if let Some(job) = &self.job {
            job.terminate();
        }
    }
}

/// ログの 1 行（固定の語と数だけ）
fn log_event(event: &WatchEvent) {
    match event {
        WatchEvent::Started => eprintln!("ikari-engine: candidate window started"),
        WatchEvent::Exited(code) => match code {
            Some(code) => eprintln!("ikari-engine: candidate window exited (code {code})"),
            None => eprintln!("ikari-engine: candidate window exited"),
        },
        WatchEvent::LaunchFailed(kind) => {
            eprintln!("ikari-engine: cannot start the candidate window ({kind})")
        }
        WatchEvent::Missing => {
            eprintln!(
                "ikari-engine: no candidate window next to the engine (typing goes on without it)"
            )
        }
        WatchEvent::Waiting(delay) => eprintln!(
            "ikari-engine: candidate window restarts in {} s",
            delay.as_secs()
        ),
    }
}

/// 候補窓を起動して見張る。起動に失敗しても engine-host は動き続ける（窓なしでも、下線つきの 1 位で入力を続けられる。
/// ADR 0062 §5.4 の決定 2）。Windows 以外では何もしない。
pub fn launch() -> CandidateWindow {
    let stop = Arc::new(StopSignal::default());
    #[cfg(windows)]
    {
        let job = start_watching(Arc::clone(&stop));
        CandidateWindow { stop, job }
    }
    #[cfg(not(windows))]
    {
        CandidateWindow { stop }
    }
}

#[cfg(windows)]
fn start_watching(stop: Arc<StopSignal>) -> Option<Arc<os::Job>> {
    let Some(exe) = std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(candidate_exe)
    else {
        eprintln!("ikari-engine: cannot find the candidate window path");
        return None;
    };
    let job = match os::Job::new() {
        Ok(job) => Arc::new(job),
        Err(e) => {
            eprintln!(
                "ikari-engine: cannot create the job for the candidate window ({:?})",
                e.kind()
            );
            return None;
        }
    };
    let launcher = os::JobLauncher::new(exe, Arc::clone(&job));
    let spawned = std::thread::Builder::new()
        .name("candidate-window".to_owned())
        .spawn(move || supervise(launcher, &stop, Instant::now, |e| log_event(&e)));
    if let Err(e) = spawned {
        eprintln!(
            "ikari-engine: cannot watch the candidate window ({:?})",
            e.kind()
        );
        return None;
    }
    Some(job)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque as Queue;

    #[test]
    fn the_candidate_exe_is_in_the_candidate_folder_next_to_the_engine() {
        let engine = if cfg!(windows) {
            PathBuf::from(r"C:\Program Files\ikari-ime\ikari-engine-host.exe")
        } else {
            PathBuf::from("/opt/ikari-ime/ikari-engine-host.exe")
        };
        let exe = candidate_exe(&engine).unwrap();
        assert_eq!(
            exe,
            engine
                .parent()
                .unwrap()
                .join("candidate")
                .join("ikari-candidate.exe")
        );
        assert_eq!(candidate_exe(Path::new("ikari-engine-host.exe")), None);
    }

    #[test]
    fn restarts_after_a_short_delay_and_rests_after_the_crash_limit() {
        let mut policy = RestartPolicy::new();
        let t0 = Instant::now();
        let step = Duration::from_secs(1);
        for i in 0..CRASH_LIMIT - 1 {
            assert_eq!(policy.after_exit(t0 + step * i as u32), RESTART_DELAY);
        }
        assert_eq!(
            policy.after_exit(t0 + step * (CRASH_LIMIT as u32 - 1)),
            SUSPENSION
        );
        // 休んだあとは数え直す
        let later = t0 + SUSPENSION + CRASH_WINDOW;
        assert_eq!(policy.after_exit(later), RESTART_DELAY);
    }

    #[test]
    fn exits_spread_over_more_than_the_window_do_not_rest() {
        let mut policy = RestartPolicy::new();
        let t0 = Instant::now();
        let apart = CRASH_WINDOW + Duration::from_secs(1);
        for i in 0..(CRASH_LIMIT as u32 * 2) {
            assert_eq!(policy.after_exit(t0 + apart * i), RESTART_DELAY);
        }
    }

    /// 決めた順に結果を返す偽の起動の手段。尽きたら見張りを止める
    struct Script {
        launches: Queue<io::Result<()>>,
        stop: Arc<StopSignal>,
        exits: Arc<Mutex<u32>>,
    }

    impl Launcher for Script {
        type Child = ();

        fn launch(&mut self) -> io::Result<()> {
            let next = self.launches.pop_front();
            if self.launches.is_empty() {
                self.stop.stop();
            }
            next.unwrap_or_else(|| Err(io::Error::other("no more")))
        }

        fn wait(&mut self, _child: ()) -> Option<u32> {
            *self.exits.lock().unwrap() += 1;
            Some(3)
        }
    }

    fn run(launches: Vec<io::Result<()>>) -> Vec<WatchEvent> {
        let stop = Arc::new(StopSignal::default());
        let script = Script {
            launches: launches.into(),
            stop: Arc::clone(&stop),
            exits: Arc::default(),
        };
        let mut events = Vec::new();
        supervise(script, &stop, Instant::now, |e| events.push(e));
        events
    }

    #[test]
    fn a_missing_exe_stops_watching() {
        let events = run(vec![Err(io::Error::from(io::ErrorKind::NotFound)), Ok(())]);
        assert_eq!(events, vec![WatchEvent::Missing]);
    }

    #[test]
    fn an_exit_after_stop_does_not_restart() {
        let events = run(vec![Ok(())]);
        assert_eq!(
            events,
            vec![WatchEvent::Started, WatchEvent::Exited(Some(3))]
        );
    }

    #[test]
    fn the_window_is_restarted_after_it_exits() {
        let stop = Arc::new(StopSignal::default());
        let exits = Arc::new(Mutex::new(0));
        let script = Script {
            launches: vec![Ok(()), Ok(())].into(),
            stop: Arc::clone(&stop),
            exits: Arc::clone(&exits),
        };
        let mut events = Vec::new();
        supervise(script, &stop, Instant::now, |e| events.push(e));
        assert_eq!(
            events,
            vec![
                WatchEvent::Started,
                WatchEvent::Exited(Some(3)),
                WatchEvent::Waiting(RESTART_DELAY),
                WatchEvent::Started,
                WatchEvent::Exited(Some(3)),
            ]
        );
        assert_eq!(*exits.lock().unwrap(), 2);
    }

    #[test]
    fn stop_wakes_the_sleep() {
        let stop = Arc::new(StopSignal::default());
        let other = Arc::clone(&stop);
        let waiter = std::thread::spawn(move || other.sleep(Duration::from_secs(600)));
        stop.stop();
        assert!(!waiter.join().unwrap());
    }

    #[test]
    fn dropping_the_window_stops_watching() {
        let window = launch();
        let stop = Arc::clone(&window.stop);
        drop(window);
        assert!(stop.is_stopped());
    }
}
