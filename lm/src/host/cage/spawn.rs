//! 檻役が子を起動し、見張り、止めて回収する（macOS。ADR 0077 §3.2 の起動の規則）。
//!
//! - 子に付ける fd は [`CHILD_FDS`]（0・1・3）だけ（`POSIX_SPAWN_CLOEXEC_DEFAULT`）。環境変数は空。
//!   標準エラー（fd 2）には、親の fd を付けず、`/dev/null` を開く（空いたままだと、子が次に開くファイルが fd 2 になり、
//!   標準エラーへの書き込みがそのファイルへ向かう）。
//!   引数は [`child_args`] だけ。実行ファイルは絶対パスで、PATH から探さない。
//! - 子ごとに見張りのスレッドを 1 つ置き、終わったら回収する（ゾンビを残さない）。止めるときは、回収の前に
//!   印の錠の下で SIGKILL を送る（回収した pid には送らない）。

use std::ffi::{CStr, CString, OsStr};
use std::io;
use std::iter;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;

use crate::host::cage::{CHILD_FDS, CHILD_MODEL_FD, CageGpu, child_args};

/// 子の標準エラーの番号（[`CHILD_FDS`] に入れず、`/dev/null` を開く）
const CHILD_STDERR_FD: RawFd = 2;
/// 子の標準エラーに開くもの
const NULL_DEVICE: &CStr = c"/dev/null";

const REAPER_THREAD_NAME: &str = "ikari-lm-cage-reaper";
/// 受けた fd を、子の番号と重ならない番号へ写すときの下限（0・1・3 への写しが互いを壊さないように）
const STAGING_FD_FLOOR: RawFd = 10;
/// 終わりを待てなかったとき（`waitid` の誤り）に、確かめ直すまでの間隔。ふつうは使わない
const WAIT_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// 子が回収されたときに呼ぶもの（数の戻しとトランザクションの終わり）
pub(super) type OnReaped = Box<dyn FnOnce() + Send>;

/// 起動した子 1 つ
#[derive(Debug)]
pub(super) struct CagedChild {
    pid: libc::pid_t,
    reaped: Mutex<bool>,
    done: Condvar,
}

impl CagedChild {
    pub(super) fn pid(&self) -> u32 {
        self.pid.unsigned_abs()
    }

    fn guard(&self) -> MutexGuard<'_, bool> {
        self.reaped.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 回収済みか（子が自分で終わったあとも真）
    pub(super) fn is_reaped(&self) -> bool {
        *self.guard()
    }

    /// SIGKILL を送り、見張りが回収するまで待つ（すでに回収済みなら何もしない）
    pub(super) fn kill_and_reap(&self) {
        let mut reaped = self.guard();
        if !*reaped {
            // SAFETY: 錠の下で、まだ回収していない自分の子（終わっていてもゾンビで残り、pid は使い回されない）
            unsafe { libc::kill(self.pid, libc::SIGKILL) };
        }
        while !*reaped {
            reaped = self.done.wait(reaped).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// 見張りのスレッド: 終わるまで待ち（回収はしない）、錠の下で回収して印を立てる
    fn watch(self: Arc<Self>, on_reaped: OnReaped) {
        loop {
            // SAFETY: `info` はこの関数の中の変数。自分の子が終わるまで待つだけ（WNOWAIT で回収しない）
            let rc = unsafe {
                let mut info: libc::siginfo_t = std::mem::zeroed();
                libc::waitid(
                    libc::P_PID,
                    self.pid.unsigned_abs(),
                    &mut info,
                    libc::WEXITED | libc::WNOWAIT,
                )
            };
            if rc == -1 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            let mut reaped = self.guard();
            let mut status = 0;
            // SAFETY: 自分の子の pid。終わっていれば回収し、まだなら 0（WNOHANG）
            let got = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
            if got == 0 {
                // まだ終わっていない（待ちが早く戻った、または待てなかった）。錠を放して待ち直す
                drop(reaped);
                if rc == -1 {
                    thread::sleep(WAIT_RETRY_INTERVAL);
                }
                continue;
            }
            on_reaped();
            *reaped = true;
            drop(reaped);
            self.done.notify_all();
            return;
        }
    }
}

/// 子を起動する。
///
/// 契約: `Err` を返したときは `on_reaped` を呼ばない（子はいない。数やトランザクションを戻すのは呼び出し側）。
/// `Ok` のときだけ、子を回収した時に見張りのスレッドが 1 回呼ぶ。
///
/// `stdin`・`stdout`・`model` は、確かめ済みの fd（[`super::check_launch_fds`]）
pub(super) fn spawn(
    program: &Path,
    gpu: CageGpu,
    stdin: BorrowedFd<'_>,
    stdout: BorrowedFd<'_>,
    model: BorrowedFd<'_>,
    on_reaped: OnReaped,
) -> io::Result<Arc<CagedChild>> {
    let words: Vec<CString> = iter::once(program.as_os_str().to_owned())
        .chain(child_args(gpu))
        .map(|w| c_string(&w))
        .collect::<io::Result<_>>()?;
    let mut argv: Vec<*mut libc::c_char> = words.iter().map(|w| w.as_ptr().cast_mut()).collect();
    argv.push(ptr::null_mut());
    // 子の環境変数は固定の空
    let envp: [*mut libc::c_char; 1] = [ptr::null_mut()];

    let staged = [stage(stdin)?, stage(stdout)?, stage(model)?];
    let mut actions = FileActions::new()?;
    for (from, to) in staged.iter().zip(CHILD_FDS) {
        actions.dup2(from.as_raw_fd(), to)?;
    }
    debug_assert_eq!(CHILD_FDS[2], CHILD_MODEL_FD);
    actions.open_null(CHILD_STDERR_FD)?;
    let mut attr = SpawnAttr::new()?;
    attr.only_listed_fds()?;

    let mut pid: libc::pid_t = 0;
    // SAFETY: 渡すポインタは、どれもこの呼び出しの間生きている（`words`、`argv`、`envp`、`actions`、`attr`）。
    // `argv` と `envp` は NULL で終わる
    let rc = unsafe {
        libc::posix_spawn(
            &mut pid,
            argv[0],
            actions.as_ptr(),
            attr.as_ptr(),
            argv.as_ptr(),
            envp.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(io::Error::from_raw_os_error(rc));
    }
    drop(staged);
    let child = Arc::new(CagedChild {
        pid,
        reaped: Mutex::new(false),
        done: Condvar::new(),
    });
    let watcher = Arc::clone(&child);
    let started = thread::Builder::new()
        .name(REAPER_THREAD_NAME.to_owned())
        .spawn(move || watcher.watch(on_reaped));
    if let Err(e) = started {
        // 見張りを置けないなら、子を残さない。`on_reaped` は呼ばない（呼ぶのは呼び出し側の責任）
        // SAFETY: 今起動した、まだ回収していない自分の子
        unsafe {
            libc::kill(pid, libc::SIGKILL);
            let mut status = 0;
            while libc::waitpid(pid, &mut status, 0) == -1
                && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
            {}
        }
        return Err(e);
    }
    Ok(child)
}

/// fd を、子の番号と重ならない番号へ写す（close-on-exec。`posix_spawn` の写しで子に付く）
fn stage(fd: BorrowedFd<'_>) -> io::Result<OwnedFd> {
    // SAFETY: 開いている fd を写すだけ。返る fd の持ち主はこちら
    let copy = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, STAGING_FD_FLOOR) };
    if copy == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 今作った、ほかに持ち主のない fd
    Ok(unsafe { OwnedFd::from_raw_fd(copy) })
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

    /// 子の `fd` に `/dev/null` を書き込みだけで開く（親の fd は渡さない）
    fn open_null(&mut self, fd: RawFd) -> io::Result<()> {
        // SAFETY: 初期化済みの値に、操作を 1 つ足すだけ。パスは 'static の NUL で終わる文字列
        check(unsafe {
            libc::posix_spawn_file_actions_addopen(
                &mut self.0,
                fd,
                NULL_DEVICE.as_ptr(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsFd;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn failed_spawn_does_not_call_on_reaped() {
        let null = std::fs::File::open("/dev/null").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let hook = Arc::clone(&calls);
        let r = spawn(
            Path::new("/nonexistent/ikari-cage-test-child"),
            CageGpu::Off,
            null.as_fd(),
            null.as_fd(),
            null.as_fd(),
            Box::new(move || {
                hook.fetch_add(1, Ordering::SeqCst);
            }),
        );
        assert!(r.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
