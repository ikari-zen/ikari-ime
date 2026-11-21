//! macOS で `ikari-data` を起こす部分（ADR 0079 §3.1、§3.6。Windows の `fetch_os.rs` に当たる）。
//!
//! - exe の場所は、自分（設定アプリ）の exe の場所から決める。`<X>/ikari.app/Contents/Helpers/IkariSettings.app/Contents/MacOS/<exe>`
//!   の形でなければ起こさない。起こすのは `<X>/ikari.app/Contents/Helpers/ikari-data`（絶対パス。PATH を使わない。シェルを通さない）。
//! - 標準入力と標準エラーは `/dev/null`、標準出力はパイプ。環境はプロキシの変数だけ（`HOME` も渡さない。ADR 0076 §3.4）。
//! - 取り消しは子に `SIGKILL`（子は孫を作らない）。待ちは `wait`。
//!
//! 判断の置き場所: Rust。

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use ikari_shell_core::settings::fetch::{Launcher, Running};

const DATA_EXE: &str = "ikari-data";
const APP_DIR: &str = "ikari.app";
const CONTENTS_DIR: &str = "Contents";
const HELPERS_DIR: &str = "Helpers";
const SETTINGS_APP_DIR: &str = "IkariSettings.app";
const MACOS_DIR: &str = "MacOS";

/// 子に渡す環境（ADR 0076 §3.4。通信の設定で読むのはプロキシだけ）
const PROXY_VARS: [&str; 8] = [
    "ALL_PROXY",
    "all_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "NO_PROXY",
    "no_proxy",
];

/// `wait` が子の終わりを見る間隔（`kill` が子の確認の間に入れるようにする）
const WAIT_POLL: Duration = Duration::from_millis(10);

fn name_is(path: &Path, expected: &str) -> bool {
    path.file_name().is_some_and(|n| n == expected)
}

/// 自分の exe の場所から、`ikari-data` の場所を決める。形が違えば `None`（開発用の置き場所からは起こさない）。
/// `own` は正規化済みであること
pub fn data_exe_path(own: &Path) -> Option<PathBuf> {
    let macos = own.parent()?;
    let settings_app = macos.parent()?.parent()?;
    let helpers = settings_app.parent()?;
    let contents = helpers.parent()?;
    let app = contents.parent()?;
    let shape_ok = name_is(macos, MACOS_DIR)
        && name_is(macos.parent()?, CONTENTS_DIR)
        && name_is(settings_app, SETTINGS_APP_DIR)
        && name_is(helpers, HELPERS_DIR)
        && name_is(contents, CONTENTS_DIR)
        && name_is(app, APP_DIR);
    shape_ok.then(|| helpers.join(DATA_EXE))
}

/// 子に渡す環境（親にあるプロキシの変数だけ）
fn child_environment() -> Vec<(OsString, OsString)> {
    PROXY_VARS
        .iter()
        .filter_map(|k| std::env::var_os(k).map(|v| (OsString::from(*k), v)))
        .collect()
}

fn lock(child: &Mutex<Child>) -> MutexGuard<'_, Child> {
    child.lock().unwrap_or_else(|e| e.into_inner())
}

/// `program` を、標準出力だけをパイプにして起こす（`environment` 以外の環境は渡さない）。製品もテストもこれを通る
pub fn spawn_running(
    program: &Path,
    args: &[String],
    environment: &[(OsString, OsString)],
) -> io::Result<Running> {
    let mut child = Command::new(program)
        .args(args)
        .env_clear()
        .envs(environment.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other("child stdout unavailable"));
    };
    let child = Arc::new(Mutex::new(child));
    let kill_child = Arc::clone(&child);
    Ok(Running {
        stdout: Box::new(stdout),
        // `Child::kill` は `SIGKILL`。終わって回収したあとは何もしない（pid の使い回しに送らない）
        kill: Box::new(move || {
            let _ = lock(&kill_child).kill();
        }),
        wait: Box::new(move || {
            loop {
                match lock(&child).try_wait() {
                    Ok(Some(status)) => return status.code(),
                    Ok(None) => {}
                    Err(_) => return None,
                }
                thread::sleep(WAIT_POLL);
            }
        }),
    })
}

/// 製品の起こし方
pub struct ProductLauncher;

impl Launcher for ProductLauncher {
    fn launch(&self, args: &[String]) -> io::Result<Running> {
        let own = std::env::current_exe()?.canonicalize()?;
        let exe = data_exe_path(&own)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not installed"))?;
        spawn_running(&exe, args, &child_environment())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::Read;

    use super::*;

    #[test]
    fn only_the_bundled_layout_is_accepted() {
        let ok = Path::new(
            "/Applications/ikari.app/Contents/Helpers/IkariSettings.app/Contents/MacOS/IkariSettings",
        );
        assert_eq!(
            data_exe_path(ok),
            Some(PathBuf::from(
                "/Applications/ikari.app/Contents/Helpers/ikari-data"
            ))
        );
        for bad in [
            "/Applications/ikari.app/Contents/Helpers/IkariSettings.app/Contents/MacOS",
            "/Applications/ikari.app/Contents/MacOS/ikari",
            "/x/other.app/Contents/Helpers/IkariSettings.app/Contents/MacOS/IkariSettings",
            "/x/ikari.app/Contents/Helpers/Other.app/Contents/MacOS/IkariSettings",
            "/x/ikari.app/Contents/Helpers/IkariSettings.app/MacOS/IkariSettings",
            "/x/target/debug/deps/test",
            "test",
            "/",
        ] {
            assert_eq!(data_exe_path(Path::new(bad)), None, "{bad}");
        }
    }

    #[test]
    fn product_launcher_refuses_outside_the_bundle() {
        // テストの exe は束の中にない
        let err = ProductLauncher.launch(&[]).err().expect("must refuse");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    fn sh(script: &str, env: &[(OsString, OsString)]) -> Running {
        spawn_running(
            Path::new("/bin/sh"),
            &["-c".to_owned(), script.to_owned()],
            env,
        )
        .unwrap()
    }

    fn read_all(running: &mut Running) -> String {
        let mut out = String::new();
        running.stdout.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn stdout_lines_and_exit_code_come_back() {
        let mut r = sh("echo a; echo b; exit 3", &[]);
        assert_eq!(read_all(&mut r), "a\nb\n");
        assert_eq!((r.wait)(), Some(3));
    }

    #[test]
    fn only_the_given_environment_reaches_the_child() {
        let env = [(OsString::from("HTTPS_PROXY"), OsString::from("http://p:1"))];
        let mut r = sh("echo \"$HTTPS_PROXY|$HOME|$PATH_X\"", &env);
        assert_eq!(read_all(&mut r), "http://p:1||\n");
        let _ = (r.wait)();
    }

    #[test]
    fn stdin_is_closed_and_stderr_is_discarded() {
        let mut r = sh("cat; echo err >&2; echo done", &[]);
        assert_eq!(read_all(&mut r), "done\n");
        assert_eq!((r.wait)(), Some(0));
    }

    #[test]
    fn kill_stops_a_running_child_and_is_repeatable() {
        let mut r = sh("echo up; exec sleep 60", &[]);
        let mut first = [0u8; 3];
        r.stdout.read_exact(&mut first).unwrap();
        (r.kill)();
        (r.kill)();
        // SIGKILL で終わると終了コードは無い
        assert_eq!((r.wait)(), None);
        (r.kill)();
    }

    #[test]
    fn missing_program_is_an_error() {
        let err = spawn_running(Path::new("/nonexistent/ikari-data"), &[], &[]).err();
        assert!(err.is_some());
    }
}
