//! 常駐のエンジンの起動と終了の手順（ADR 0040 §2.2、§7 の W5-a）。OS に依存しない（Mac でテストする）。
//!
//! macOS の `IkariEngineService`（`main.swift`）に相当する。`main` は、ここの関数を呼んで、`cfg(windows)` の
//! 部分（パイプの受け口、Ctrl+C の受け取り）をつなぐだけ。判断の置き場所: Rust。判断は engine。ここは起動と配線だけ。
//!
//! - データのディレクトリ: `--data-dir`、`IKARI_USER_DIR`、`%LOCALAPPDATA%\ikari-ime` の順（`UserDir::resolve_with`）。
//!   辞書は `<データ>/dict/`、モデルは `<データ>/models/`、ログは `<データ>/logs/`、設定は `<データ>/settings.tsv`
//!   （どれも `Engine::open` が決める配置。ここでは別の場所を持たない）。
//! - 二重起動の防止: `Engine::open` が `engine.lock`（`EngineLock`）を取る。取れなければ、すでに動いているとして
//!   静かに終わる（終了コード 0）。
//! - 失敗: 開けなければ（辞書がないなど）標準エラーと `logs/start-failure.log` に書いて終了コード 2。
//!   窓のない起動（`hide-console`）では標準エラーが見えないので、ファイルにも書く。
//! - 終了: 受け付けを止め、残りの接続を閉じ、エンジンを閉じる（学習の保存）。ロックはエンジンが持つので、
//!   エンジンを落とすと手放される。

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use ikari_core::user::{UserDir, create_private_dir_all};
use ikari_engine::{Engine, EngineConfig, EngineError, LockError, SETTINGS_FILE_NAME};
use ikari_pipe::{Listener, Shutdown};
use ikari_shell_core::settings::SettingsStore;

use crate::serve;
use crate::settings_pipe::{SettingsHost, serve_settings};

/// 正常な終了、または、すでに別のエンジンが動いている
pub const EXIT_OK: u8 = 0;
/// 非 Windows で起動した
pub const EXIT_UNSUPPORTED: u8 = 1;
/// エンジンを開けない（データのディレクトリが決まらない、辞書がない、など）、受け口を作れない
pub const EXIT_START_FAILED: u8 = 2;
/// 引数が不正（`sysexits.h` の `EX_USAGE`）
pub const EXIT_USAGE: u8 = 64;

/// 引数の名前
pub const ARG_DATA_DIR: &str = "--data-dir";
/// 起動の失敗を残すファイル（`<データ>/logs/`）。窓のない起動で標準エラーが見えないため。最後の 1 回分だけ持つ
pub const START_FAILURE_LOG: &str = "start-failure.log";
/// ログのディレクトリ名（`engine.log` と同じ場所。`ikari_engine` の `LOG_DIR_NAME` は非公開）
const LOGS_DIR_NAME: &str = "logs";
/// 非 Windows で出す文
pub const UNSUPPORTED_MESSAGE: &str = "ikari-engine: Windows 専用です";
/// LM のモデルが置かれていれば、使うかは設定の `engine.lm` が決める。Windows の既定は LM なし（ADR 0040 §2.6）
const ENABLE_LM_DEFAULT: bool = false;
/// 使われないモデルを手放すまでの秒数（macOS の `IkariEngineService` と同じ）
const IDLE_UNLOAD_SECS: u32 = 300;

/// 起動の引数
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// データのディレクトリの明示の指定
    pub data_dir: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum UsageError {
    /// 値の要る引数に値がない
    MissingValue(&'static str),
    Unknown(String),
}

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UsageError::MissingValue(name) => write!(f, "{name} needs a value"),
            UsageError::Unknown(arg) => write!(f, "unknown argument: {arg}"),
        }
    }
}

/// `args` は、実行ファイルの名前を除いたもの。
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Options, UsageError> {
    let mut options = Options::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            ARG_DATA_DIR => {
                let value = args.next().ok_or(UsageError::MissingValue(ARG_DATA_DIR))?;
                options.data_dir = Some(PathBuf::from(value));
            }
            _ => return Err(UsageError::Unknown(arg)),
        }
    }
    Ok(options)
}

/// 起動できなかった理由。
#[derive(Debug)]
pub enum StartError {
    /// データのディレクトリが決まらない（`%LOCALAPPDATA%` がない、など）
    NoDataDir,
    /// 別のエンジンが `engine.lock` を持っている。失敗ではなく、静かに終わる
    AlreadyRunning,
    /// 開けない（辞書がない、ディレクトリを作れない、受け口を作れない、など）。理由の文
    Failed(String),
}

impl StartError {
    pub fn exit_code(&self) -> u8 {
        match self {
            StartError::AlreadyRunning => EXIT_OK,
            StartError::NoDataDir | StartError::Failed(_) => EXIT_START_FAILED,
        }
    }
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartError::NoDataDir => write!(
                f,
                "cannot decide the data directory (set {} or pass {ARG_DATA_DIR})",
                ikari_core::user::USER_DIR_ENV
            ),
            StartError::AlreadyRunning => write!(f, "another engine is already running"),
            StartError::Failed(reason) => write!(f, "{reason}"),
        }
    }
}

/// データのディレクトリを決める（`UserDir::resolve`。引数、`IKARI_USER_DIR`、`%LOCALAPPDATA%\ikari-ime` の順）。
/// 環境と OS を差し込む `resolve_with` は、`HostOs` が `ikari_core::user` から出ていないので使えない
/// （各分岐は core のテストが見ている）。
pub fn resolve_data_dir(explicit: Option<&Path>) -> Result<UserDir, StartError> {
    UserDir::resolve(explicit).ok_or(StartError::NoDataDir)
}

/// `Engine::open` に渡す設定。辞書、モデル、設定、ログの場所は、データのディレクトリから `Engine::open` が決める。
pub fn engine_config(dir: &UserDir) -> EngineConfig {
    EngineConfig {
        data_dir: dir.root().to_string_lossy().into_owned(),
        enable_lm: ENABLE_LM_DEFAULT,
        idle_unload_secs: IDLE_UNLOAD_SECS,
    }
}

/// 開いたエンジンと、設定画面の口が共有するもの。
pub struct Started {
    pub engine: Arc<Engine>,
    pub settings: SettingsHost,
}

/// データのディレクトリを（なければ）作り、エンジンを開く（ロックを取る）。
pub fn start(dir: &UserDir) -> Result<Started, StartError> {
    create_private_dir_all(dir.root())
        .map_err(|e| StartError::Failed(format!("cannot create the data directory: {e}")))?;
    let engine = match Engine::open(&engine_config(dir)) {
        Ok(engine) => Arc::new(engine),
        Err(e) => return Err(classify_open_error(&e)),
    };
    let (store, _issues) = SettingsStore::open(dir.root().join(SETTINGS_FILE_NAME));
    let settings = SettingsHost::new(Arc::clone(&engine), store, env!("CARGO_PKG_VERSION"));
    Ok(Started { engine, settings })
}

/// ロックを取れなかった（別のエンジンが動いている）かどうかで、失敗と見分ける。
/// `Engine::open` は理由を文で返すので、`LockError::Held` の文で見分ける（エンジンの型は変えない）。
fn classify_open_error(error: &EngineError) -> StartError {
    match error {
        EngineError::Open(message) if message.contains(&LockError::Held.to_string()) => {
            StartError::AlreadyRunning
        }
        other => StartError::Failed(format!("cannot open the engine: {other}")),
    }
}

/// 失敗を `logs/start-failure.log` に書く（最後の 1 回分。時刻は UNIX 秒）。`AlreadyRunning` は何も書かない。
/// 書けなくても、起動の失敗の扱いは変えない。書くのは固定の語と、エンジンが返した理由の文だけ
/// （入力の内容は、この時点では持っていない）。
pub fn record_failure(dir: &UserDir, error: &StartError) {
    if matches!(error, StartError::AlreadyRunning) {
        return;
    }
    let logs = dir.root().join(LOGS_DIR_NAME);
    if create_private_dir_all(&logs).is_err() {
        return;
    }
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let _ = fs::write(
        logs.join(START_FAILURE_LOG),
        format!("unix_time={secs} engine-host start failed: {error}\n"),
    );
}

/// 止める合図を送る側（Ctrl+C の受け取りなど、別のスレッドから呼べる）。
#[derive(Clone)]
pub struct StopHandle(Sender<Event>);

impl StopHandle {
    /// 止めるよう頼む。すでに止まっていれば何も起きない。
    pub fn stop(&self) {
        let _ = self.0.send(Event::Stop);
    }
}

enum Event {
    Stop,
    /// 受け付けのスレッドが終わった（受け口が閉じた、または失敗した）
    Ended,
}

/// `run_host` が止まる合図を待つ側。
pub struct StopWait {
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl StopWait {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx }
    }

    pub fn handle(&self) -> StopHandle {
        StopHandle(self.tx.clone())
    }
}

impl Default for StopWait {
    fn default() -> Self {
        Self::new()
    }
}

/// エンジン用と設定画面用の 2 つの受け口を別のスレッドで受け付け、止まるまで待つ。
///
/// 止まるのは、`StopHandle::stop` が呼ばれたとき、または、どちらかの受け付けが終わったとき
/// （片方が止まったまま動き続けない）。止まったら、`stoppers`（受け口を閉じる手段）を呼び、両方のスレッドを待つ。
/// 戻るとき、`started` が持つ `Engine` の参照は、ここで作ったものも含めてすべて落ちている
/// （学習の保存とロックの解放が済む）。受け付けのどちらかが失敗していれば、その最初のエラーを返す。
pub fn run_host<E, S>(
    engine_listener: E,
    settings_listener: S,
    started: Started,
    stoppers: &[Arc<dyn Shutdown>],
    stop: StopWait,
) -> io::Result<()>
where
    E: Listener + 'static,
    S: Listener + 'static,
{
    let Started { engine, settings } = started;
    let engine_thread = spawn_accept("ikari-engine-pipe", &stop.tx, move || {
        serve(engine_listener, engine)
    })?;
    let settings_thread = match spawn_accept("ikari-settings-pipe", &stop.tx, move || {
        serve_settings(settings_listener, settings)
    }) {
        Ok(thread) => thread,
        Err(e) => {
            close_all(stoppers);
            let _ = join(engine_thread);
            return Err(e);
        }
    };

    // 最初の合図（止める、または受け付けが終わった）を待つ。送り手が全部なくなることはない（`stop.tx` を持つ）
    let _ = stop.rx.recv();
    close_all(stoppers);
    let engine_result = join(engine_thread);
    let settings_result = join(settings_thread);
    engine_result.and(settings_result)
}

fn close_all(stoppers: &[Arc<dyn Shutdown>]) {
    for stopper in stoppers {
        stopper.shutdown();
    }
}

fn spawn_accept(
    name: &str,
    tx: &Sender<Event>,
    serve: impl FnOnce() -> io::Result<()> + Send + 'static,
) -> io::Result<JoinHandle<io::Result<()>>> {
    let tx = tx.clone();
    thread::Builder::new().name(name.to_owned()).spawn(move || {
        let result = serve();
        let _ = tx.send(Event::Ended);
        result
    })
}

fn join(thread: JoinHandle<io::Result<()>>) -> io::Result<()> {
    thread
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("an accept thread panicked")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn parses_the_data_dir_argument() {
        assert_eq!(parse_args(args(&[])), Ok(Options::default()));
        assert_eq!(
            parse_args(args(&["--data-dir", "/x"])),
            Ok(Options {
                data_dir: Some(PathBuf::from("/x"))
            })
        );
        assert_eq!(
            parse_args(args(&["--data-dir"])),
            Err(UsageError::MissingValue(ARG_DATA_DIR))
        );
        assert_eq!(
            parse_args(args(&["--nope"])),
            Err(UsageError::Unknown("--nope".into()))
        );
    }

    #[test]
    fn exit_codes_follow_the_kind_of_failure() {
        assert_eq!(StartError::AlreadyRunning.exit_code(), EXIT_OK);
        assert_eq!(StartError::NoDataDir.exit_code(), EXIT_START_FAILED);
        assert_eq!(
            StartError::Failed("x".into()).exit_code(),
            EXIT_START_FAILED
        );
    }
}
