//! 子プロセスの起動と回収の OS ごとの部分。`unsafe` はこのファイルに閉じる。
//!
//! - 回収: kill のあと必ず wait する（ゾンビを残さない）。`Drop` でも同じ。
//! - 親が死んだとき: 子は stdin が閉じたことで自分で終わる（`serve`）。Windows は、それに加えて Job Object
//!   （`KILL_ON_JOB_CLOSE`）に入れ、親のハンドルが閉じたら OS が子を終わらせる。
//! - macOS: 子の stdin のパイプに `F_SETNOSIGPIPE` を付ける。エンジンは Swift の XPC サービスの中の静的ライブラリで、
//!   Rust の `main` を通らないので SIGPIPE が無視されていない。付けないと、死んだ子への書き込みでエンジンが落ちる。
//!   ほかの unix は、Rust の実行時（`main`）が SIGPIPE を無視している前提（製品の対象ではない。テストと CLI だけ）。
//! - Windows: `CREATE_NO_WINDOW`（窓なしのエンジンから起動しても、コンソールの窓を出さない）。

use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// 起動した子。捨てると kill して wait する
pub(super) struct ChildProc {
    child: Child,
    #[cfg(windows)]
    _job: Option<windows::Job>,
}

impl ChildProc {
    pub(super) fn id(&self) -> u32 {
        self.child.id()
    }

    /// 子を止めて回収する。すでに終わっていても失敗にしない
    fn reap(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for ChildProc {
    fn drop(&mut self) {
        self.reap();
    }
}

/// `program` を、標準入出力をパイプにして起動する。標準エラーは捨てる
pub(super) fn spawn(
    program: &Path,
    args: &[OsString],
) -> io::Result<(ChildProc, ChildStdin, ChildStdout)> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    windows::no_window(&mut command);
    let mut child = command.spawn()?;
    let pipes = (child.stdin.take(), child.stdout.take());
    #[cfg(windows)]
    let job = windows::Job::kill_on_close(&child);
    // ここから先は、失敗しても `proc` の Drop が子を回収する
    let proc = ChildProc {
        child,
        #[cfg(windows)]
        _job: job,
    };
    let (Some(stdin), Some(stdout)) = pipes else {
        return Err(io::Error::other("child pipes unavailable"));
    };
    #[cfg(target_vendor = "apple")]
    apple::no_sigpipe(&stdin)?;
    Ok((proc, stdin, stdout))
}

#[cfg(target_vendor = "apple")]
mod apple {
    use std::io;
    use std::os::fd::AsRawFd;
    use std::process::ChildStdin;

    /// この fd への書き込みで SIGPIPE を出さない（`EPIPE` のエラーだけが返る）
    pub(super) fn no_sigpipe(stdin: &ChildStdin) -> io::Result<()> {
        // SAFETY: `stdin` が持つ開いた fd に、フラグを 1 つ設定するだけ（メモリには触れない）
        let rc = unsafe { libc::fcntl(stdin.as_raw_fd(), libc::F_SETNOSIGPIPE, 1) };
        if rc == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn has_no_sigpipe(stdin: &ChildStdin) -> bool {
        // SAFETY: 開いた fd のフラグを読むだけ
        unsafe { libc::fcntl(stdin.as_raw_fd(), libc::F_GETNOSIGPIPE) == 1 }
    }
}

#[cfg(windows)]
mod windows {
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command};

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    pub(super) fn no_window(command: &mut Command) {
        command.creation_flags(CREATE_NO_WINDOW);
    }

    /// 閉じると中のプロセスが終わる Job Object。親（エンジン）が死ぬとハンドルが閉じ、子も終わる
    pub(super) struct Job(HANDLE);

    // SAFETY: Job Object のハンドルは、どのスレッドから閉じてもよい（`CloseHandle` だけに使う）
    unsafe impl Send for Job {}

    impl Job {
        /// Job を作って `child` を入れる。作れない・入れられないときは None（子は stdin が閉じたことで終わる
        /// 経路だけになる。起動は続ける）
        pub(super) fn kill_on_close(child: &Child) -> Option<Job> {
            // SAFETY: 名前なし・既定のセキュリティで作る。戻り値は検査する
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return None;
            }
            let job = Job(handle);
            // SAFETY: すべて 0 の値は、この構造体として正しい（制限なし）
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: `info` はこの呼び出しの間だけ生きていて、大きさを正しく渡す
            let set = unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    std::ptr::from_ref(&info).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if set == 0 {
                return None;
            }
            // SAFETY: `child` は生きていて、そのハンドルは `Child` が持っている
            let assigned =
                unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle() as HANDLE) };
            (assigned != 0).then_some(job)
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: `CreateJobObjectW` が返した、まだ閉じていないハンドル
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(all(test, target_vendor = "apple"))]
mod tests {
    use super::*;

    #[test]
    fn child_stdin_does_not_raise_sigpipe() {
        let (_proc, stdin, _stdout) = spawn(Path::new("/bin/cat"), &[]).unwrap();
        assert!(apple::has_no_sigpipe(&stdin));
    }
}
