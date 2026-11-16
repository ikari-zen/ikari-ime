//! 子プロセスの起動と回収の OS ごとの部分。`unsafe` はこのモジュール（`os/` の下を含む）に閉じる。
//!
//! - 回収: kill のあと必ず wait する（ゾンビを残さない）。`Drop` でも同じ。
//! - 子に渡すのは、標準入力・標準出力（パイプ）と標準エラー（捨て先）の 3 つだけ（ADR 0068 §6.1、0067 §4.3）。
//!   実行ファイルは、渡された絶対パスをそのまま使い、PATH から探さない。
//! - 親が死んだとき: 子は stdin が閉じたことで自分で終わる（`serve`）。Windows は、それに加えて Job Object に入れ、
//!   親のハンドルが閉じたら OS が子を終わらせる。
//! - 檻の段 1（ADR 0072 §3）: Windows は `windows` の説明のとおり（Job の制限、低い整合性）。macOS は、エンジンの
//!   App Sandbox を引き継ぐ（今のまま）。
//! - 絞りがかけられないときは、子を止めて失敗を返す。呼び出し側（`scorer`）は失敗に数え、LM なしで動く。
//! - macOS: 子の stdin のパイプに `F_SETNOSIGPIPE` を付ける。エンジンは Swift の XPC サービスの中の静的ライブラリで、
//!   Rust の `main` を通らないので SIGPIPE が無視されていない。付けないと、死んだ子への書き込みでエンジンが落ちる。
//!   ほかの unix は、Rust の実行時（`main`）が SIGPIPE を無視している前提（製品の対象ではない。テストと CLI だけ）。

use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::{ChildStdin, ChildStdout};

#[cfg(target_vendor = "apple")]
mod apple;
#[cfg(any(windows, test))]
mod cmdline;
#[cfg(windows)]
mod windows;

#[cfg(target_vendor = "apple")]
use apple as imp;
#[cfg(not(any(target_vendor = "apple", windows)))]
use other_unix as imp;
#[cfg(windows)]
use windows as imp;

/// 起動した子。捨てると kill して wait する
pub(super) struct ChildProc {
    inner: imp::Proc,
}

impl ChildProc {
    pub(super) fn id(&self) -> u32 {
        self.inner.id()
    }
}

impl Drop for ChildProc {
    fn drop(&mut self) {
        self.inner.reap();
    }
}

/// `program`（絶対パス）を、標準入出力をパイプにして起動する。標準エラーは捨てる
pub(super) fn spawn(
    program: &Path,
    args: &[OsString],
) -> io::Result<(ChildProc, ChildStdin, ChildStdout)> {
    let (inner, stdin, stdout) = imp::spawn(program, args)?;
    // ここから先は、失敗しても `proc` の Drop が子を回収する
    let proc = ChildProc { inner };
    #[cfg(target_vendor = "apple")]
    imp::no_sigpipe(&stdin)?;
    Ok((proc, stdin, stdout))
}

/// 製品の対象でない unix（テストと CLI だけ）。`std::process` で起動する。Rust の標準ライブラリが開く fd は
/// close-on-exec なので、子に渡るのは標準入出力だけ
#[cfg(not(any(target_vendor = "apple", windows)))]
mod other_unix {
    use std::ffi::OsString;
    use std::io;
    use std::path::Path;
    use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

    pub(super) struct Proc(Child);

    impl Proc {
        pub(super) fn id(&self) -> u32 {
            self.0.id()
        }

        pub(super) fn reap(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    pub(super) fn spawn(
        program: &Path,
        args: &[OsString],
    ) -> io::Result<(Proc, ChildStdin, ChildStdout)> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let pipes = (child.stdin.take(), child.stdout.take());
        let mut proc = Proc(child);
        match pipes {
            (Some(stdin), Some(stdout)) => Ok((proc, stdin, stdout)),
            _ => {
                proc.reap();
                Err(io::Error::other("child pipes unavailable"))
            }
        }
    }
}

#[cfg(all(test, target_vendor = "apple"))]
mod tests {
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    use super::*;

    /// 親で、close-on-exec の印なしに開いておく fd の番号（ほかのテストの fd と重ならない大きさ）
    const PROBE_FD: i32 = 200;

    fn probe_script() -> OsString {
        format!("if [ -e /dev/fd/{PROBE_FD} ]; then echo open; else echo closed; fi").into()
    }

    fn run(stdin: ChildStdin, mut stdout: ChildStdout) -> String {
        drop(stdin);
        let mut out = String::new();
        stdout.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn child_stdin_does_not_raise_sigpipe() {
        let (_proc, stdin, _stdout) = spawn(Path::new("/bin/cat"), &[]).unwrap();
        assert!(imp::has_no_sigpipe(&stdin));
    }

    #[test]
    fn the_child_gets_only_the_three_standard_fds() {
        let file = tempfile::tempfile().unwrap();
        // SAFETY: 開いた fd を、使われていない番号に複製するだけ（`dup2` の結果には close-on-exec が付かない）
        let fd = unsafe { libc::dup2(file.as_raw_fd(), PROBE_FD) };
        assert_eq!(fd, PROBE_FD);
        // SAFETY: いま開いた fd で、ほかに持ち主はいない
        let _probe = unsafe { OwnedFd::from_raw_fd(fd) };

        // 対照: `std::process` の子には渡る（このテストが意味を持つことの確かめ）
        let leaked = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(probe_script())
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&leaked.stdout), "open\n");

        let (_proc, stdin, stdout) =
            spawn(Path::new("/bin/sh"), &["-c".into(), probe_script()]).unwrap();
        assert_eq!(run(stdin, stdout), "closed\n");
    }

    #[test]
    fn stdin_and_stdout_reach_the_child() {
        let (_proc, mut stdin, stdout) = spawn(Path::new("/bin/cat"), &[]).unwrap();
        stdin.write_all(b"ping\n").unwrap();
        assert_eq!(run(stdin, stdout), "ping\n");
    }

    #[test]
    fn a_missing_program_is_an_error() {
        assert!(spawn(Path::new("/nonexistent/ikari-lm-host"), &[]).is_err());
    }

    #[test]
    fn dropping_the_child_kills_and_reaps_it() {
        let (proc, _stdin, _stdout) = spawn(Path::new("/bin/cat"), &[]).unwrap();
        let pid = proc.id() as libc::pid_t;
        drop(proc);
        // 回収済みなら、もう自分の子ではない
        // SAFETY: 状態を聞くだけ（WNOHANG）
        let rc = unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
        assert_eq!(rc, -1);
    }
}

#[cfg(all(test, windows))]
mod windows_tests;
