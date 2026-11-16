//! macOS の子の起動（`posix_spawn`）。
//!
//! - 子に渡す fd は、標準入出力の 3 つだけ（ADR 0068 §6.1）。`POSIX_SPAWN_CLOEXEC_DEFAULT` で、
//!   ファイルの操作（`dup2`、`open`）で作ったもの以外は、close-on-exec の印に関わらず子で閉じる。
//! - 実行ファイルは絶対パスで渡し、PATH から探さない（`posix_spawn`。`posix_spawnp` ではない）。
//! - 標準エラーは `/dev/null`。シグナルのマスクは空、SIGPIPE は既定の動きに戻す（`std::process` と同じ）。

use std::ffi::{CString, OsStr, OsString};
use std::io;
use std::iter;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::{ChildStdin, ChildStdout};
use std::ptr;

const NULL_DEVICE: &std::ffi::CStr = c"/dev/null";

/// 起動した子。`reap` は 1 回だけ効く（2 回目からは何もしない）
pub(super) struct Proc {
    pid: libc::pid_t,
    reaped: bool,
}

impl Proc {
    pub(super) fn id(&self) -> u32 {
        // `posix_spawn` が成功したときの pid は正の値
        self.pid.unsigned_abs()
    }

    /// 子を止めて回収する。すでに終わっていても失敗にしない
    pub(super) fn reap(&mut self) {
        if self.reaped {
            return;
        }
        // SAFETY: まだ回収していない自分の子の pid（終わっていればゾンビとして残っていて、ほかに使い回されない）
        unsafe { libc::kill(self.pid, libc::SIGKILL) };
        let mut status = 0;
        loop {
            // SAFETY: 自分の子の pid と、この関数の中の変数を渡すだけ
            let rc = unsafe { libc::waitpid(self.pid, &mut status, 0) };
            if rc == -1 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        self.reaped = true;
    }
}

pub(super) fn spawn(
    program: &Path,
    args: &[OsString],
) -> io::Result<(Proc, ChildStdin, ChildStdout)> {
    let words: Vec<CString> = iter::once(program.as_os_str())
        .chain(args.iter().map(OsString::as_os_str))
        .map(c_string)
        .collect::<io::Result<_>>()?;
    let mut argv: Vec<*mut libc::c_char> = words.iter().map(|w| w.as_ptr().cast_mut()).collect();
    argv.push(ptr::null_mut());

    // 子が読む側（標準入力）と、子が書く側（標準出力）
    let (child_in, parent_in) = io::pipe()?;
    let (parent_out, child_out) = io::pipe()?;
    let child_in = OwnedFd::from(child_in);
    let child_out = OwnedFd::from(child_out);

    let mut actions = FileActions::new()?;
    actions.dup2(child_in.as_raw_fd(), libc::STDIN_FILENO)?;
    actions.dup2(child_out.as_raw_fd(), libc::STDOUT_FILENO)?;
    actions.open_write(libc::STDERR_FILENO, NULL_DEVICE)?;
    let mut attr = SpawnAttr::new()?;
    attr.only_listed_fds()?;

    let mut pid: libc::pid_t = 0;
    // SAFETY: 渡すポインタは、どれもこの呼び出しの間生きている（`words`、`argv`、`actions`、`attr`）。
    // `argv` は NULL で終わる。環境は親のもの（`_NSGetEnviron` は、プロセスの環境の配列を指す）
    let rc = unsafe {
        libc::posix_spawn(
            &mut pid,
            argv[0],
            actions.as_ptr(),
            attr.as_ptr(),
            argv.as_ptr(),
            *libc::_NSGetEnviron(),
        )
    };
    if rc != 0 {
        return Err(io::Error::from_raw_os_error(rc));
    }
    // 子の側の端は、親では閉じる（閉じないと、子が終わっても読み取りが終わらない）
    drop((child_in, child_out));
    Ok((
        Proc { pid, reaped: false },
        ChildStdin::from(OwnedFd::from(parent_in)),
        ChildStdout::from(OwnedFd::from(parent_out)),
    ))
}

fn c_string(word: &OsStr) -> io::Result<CString> {
    CString::new(word.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "nul in command line"))
}

fn check(rc: libc::c_int) -> io::Result<()> {
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(rc))
    }
}

/// `posix_spawn_file_actions_t`。捨てると破棄する
struct FileActions(libc::posix_spawn_file_actions_t);

impl FileActions {
    fn new() -> io::Result<FileActions> {
        let mut raw: libc::posix_spawn_file_actions_t = ptr::null_mut();
        // SAFETY: 初期化する先を渡すだけ。失敗したら中身は使わない
        check(unsafe { libc::posix_spawn_file_actions_init(&mut raw) })?;
        Ok(FileActions(raw))
    }

    fn dup2(&mut self, from: RawFd, to: RawFd) -> io::Result<()> {
        // SAFETY: 初期化済みの値に、操作を 1 つ足すだけ
        check(unsafe { libc::posix_spawn_file_actions_adddup2(&mut self.0, from, to) })
    }

    fn open_write(&mut self, fd: RawFd, path: &std::ffi::CStr) -> io::Result<()> {
        // SAFETY: 初期化済みの値に、操作を 1 つ足すだけ。パスは 'static の文字列
        check(unsafe {
            libc::posix_spawn_file_actions_addopen(
                &mut self.0,
                fd,
                path.as_ptr(),
                libc::O_WRONLY,
                0,
            )
        })
    }

    fn as_ptr(&self) -> *const libc::posix_spawn_file_actions_t {
        &self.0
    }
}

impl Drop for FileActions {
    fn drop(&mut self) {
        // SAFETY: `new` で初期化した値を 1 回だけ破棄する
        unsafe { libc::posix_spawn_file_actions_destroy(&mut self.0) };
    }
}

/// `posix_spawnattr_t`。捨てると破棄する
struct SpawnAttr(libc::posix_spawnattr_t);

impl SpawnAttr {
    fn new() -> io::Result<SpawnAttr> {
        let mut raw: libc::posix_spawnattr_t = ptr::null_mut();
        // SAFETY: 初期化する先を渡すだけ。失敗したら中身は使わない
        check(unsafe { libc::posix_spawnattr_init(&mut raw) })?;
        Ok(SpawnAttr(raw))
    }

    /// ファイルの操作で作った fd だけを子に残す。シグナルのマスクを空に、SIGPIPE を既定の動きにする
    fn only_listed_fds(&mut self) -> io::Result<()> {
        let flags = libc::POSIX_SPAWN_CLOEXEC_DEFAULT
            | libc::POSIX_SPAWN_SETSIGDEF
            | libc::POSIX_SPAWN_SETSIGMASK;
        let flags = libc::c_short::try_from(flags)
            .map_err(|_| io::Error::other("posix_spawn flags out of range"))?;
        // SAFETY: `sigset_t` は、`sigemptyset` で初期化してから使う
        unsafe {
            let mut empty: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut empty);
            let mut default = empty;
            libc::sigaddset(&mut default, libc::SIGPIPE);
            check(libc::posix_spawnattr_setsigmask(&mut self.0, &empty))?;
            check(libc::posix_spawnattr_setsigdefault(&mut self.0, &default))?;
            check(libc::posix_spawnattr_setflags(&mut self.0, flags))
        }
    }

    fn as_ptr(&self) -> *const libc::posix_spawnattr_t {
        &self.0
    }
}

impl Drop for SpawnAttr {
    fn drop(&mut self) {
        // SAFETY: `new` で初期化した値を 1 回だけ破棄する
        unsafe { libc::posix_spawnattr_destroy(&mut self.0) };
    }
}

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
