//! 子のプロセスの起動と、檻の確認（ADR 0075 §6）。
//!
//! - ふつう: `std::process::Command`（標準出力だけを受け取る。標準入力と標準エラーは捨てる）
//! - 低い整合性・AppContainer（能力なし）: Windows だけ（`sandbox/windows.rs`）。`lm/src/host/os/windows.rs` の流儀
//!   （渡すハンドルは標準入出力の 3 つだけ、窓を出さない）に合わせる
//!
//! AppContainer のプロファイルは、確認の間だけ作って、終わったら消す。落ちて残ったときは `--cleanup` で消す。
//! ツールのフォルダに、AppContainer の SID の読みと実行の許可を足す（消すときに外す）。

use std::ffi::OsString;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(windows)]
mod windows;

/// AppContainer のプロファイルの名前
#[cfg_attr(not(windows), allow(dead_code))]
pub const PROFILE_NAME: &str = "ikari-npu-probe";
/// 終わったかを見る間隔
const POLL: Duration = Duration::from_millis(50);

/// 子の結果（標準出力と終了コード）
#[derive(Debug)]
pub struct Captured {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub timed_out: bool,
}

/// 子を起動する檻
pub enum Confine<'a> {
    Normal,
    LowIntegrity,
    AppContainer(#[cfg_attr(not(windows), allow(dead_code))] &'a Profile),
}

impl Confine<'_> {
    pub fn name(&self) -> &'static str {
        match self {
            Confine::Normal => "ふつう",
            Confine::LowIntegrity => "低い整合性",
            Confine::AppContainer(_) => "AppContainer",
        }
    }
}

/// `program` を `confine` で起動し、終わるまで（`timeout` まで）待つ
pub fn run(
    program: &Path,
    args: &[OsString],
    confine: &Confine<'_>,
    timeout: Duration,
) -> io::Result<Captured> {
    match confine {
        Confine::Normal => run_normal(program, args, timeout),
        #[cfg(windows)]
        Confine::LowIntegrity => windows::run(program, args, None, timeout),
        #[cfg(windows)]
        Confine::AppContainer(p) => windows::run(program, args, Some(&p.0), timeout),
        #[cfg(not(windows))]
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "檻の確認は Windows だけ",
        )),
    }
}

fn run_normal(program: &Path, args: &[OsString], timeout: Duration) -> io::Result<Captured> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut out = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("no stdout"))?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if started.elapsed() > timeout {
            timed_out = true;
            let _ = child.kill();
            break child.wait()?;
        }
        std::thread::sleep(POLL);
    };
    let stdout = reader.join().unwrap_or_default();
    Ok(Captured {
        code: status.code(),
        stdout,
        timed_out,
    })
}

/// AppContainer のプロファイル（確認の間だけ）
pub struct Profile(#[cfg(windows)] windows::Profile, #[cfg(not(windows))] ());

impl Profile {
    /// プロファイルを作り（あれば使い）、`dir` に読みと実行の許可を足す
    pub fn create(dir: &Path) -> io::Result<Profile> {
        #[cfg(windows)]
        return windows::Profile::create(PROFILE_NAME, dir).map(Profile);
        #[cfg(not(windows))]
        {
            let _ = dir;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "AppContainer は Windows だけ",
            ))
        }
    }

    /// 許可を外し、プロファイルを消す
    pub fn remove(self) -> io::Result<()> {
        #[cfg(windows)]
        return self.0.remove();
        #[cfg(not(windows))]
        Ok(())
    }
}

/// 残ったプロファイルと許可を消す（`--cleanup`）。消したら真、無ければ偽
pub fn cleanup(dir: &Path) -> io::Result<bool> {
    #[cfg(windows)]
    return windows::cleanup(PROFILE_NAME, dir);
    #[cfg(not(windows))]
    {
        let _ = dir;
        Ok(false)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn normal_captures_stdout_and_code() {
        let args = vec![OsString::from("-c"), OsString::from("printf hello; exit 3")];
        let c = run(
            Path::new("/bin/sh"),
            &args,
            &Confine::Normal,
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(c.stdout, b"hello");
        assert_eq!(c.code, Some(3));
        assert!(!c.timed_out);
    }

    #[test]
    fn normal_times_out() {
        let args = vec![OsString::from("-c"), OsString::from("sleep 5")];
        let c = run(
            Path::new("/bin/sh"),
            &args,
            &Confine::Normal,
            Duration::from_millis(100),
        )
        .unwrap();
        assert!(c.timed_out);
    }
}
