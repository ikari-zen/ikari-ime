//! macOS の取得の子の起動（`posix_spawn`。ADR 0068 §6.1。`lm/src/host/os/apple.rs` と同じ流儀）。
//!
//! - 子に渡す fd は、標準入出力の 3 つだけ。`POSIX_SPAWN_CLOEXEC_DEFAULT` で、ファイルの操作（`dup2`、`open`）で
//!   作ったもの以外は、close-on-exec の印に関わらず子で閉じる（設定アプリの Swift が開いたものも渡らない）。
//! - 標準入力と標準エラーは `/dev/null`、標準出力はパイプ。環境は渡されたものだけ。
//! - 実行ファイルは絶対パスで渡し、PATH から探さない（`posix_spawn`。`posix_spawnp` ではない）。
//! - シグナルのマスクは空、SIGPIPE は既定の動きに戻す（`std::process` と同じ）。
//! - 止めるのは `SIGKILL`。回収したあとは送らない（pid の使い回しに送らない）。

use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs::File;
use std::io;
use std::iter;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use ikari_shell_core::settings::fetch::Running;

const NULL_DEVICE: &CStr = c"/dev/null";
/// パイプの子の端を、0・1・2 と重ならない番号へ写すときの下限（写しが標準入出力を壊さないように）
const STAGING_FD_FLOOR: RawFd = 10;
/// `wait` が子の終わりを見る間隔（`kill` が子の確認の間に入れるようにする）
const WAIT_POLL: Duration = Duration::from_millis(10);

/// 起こした子の状態。`exit` が `Some` なら回収済み（中身は終了コード。シグナルで終われば `None`）
struct Proc {
    pid: libc::pid_t,
    exit: Option<Option<i32>>,
}

fn lock(proc: &Mutex<Proc>) -> MutexGuard<'_, Proc> {
    proc.lock().unwrap_or_else(|e| e.into_inner())
}

impl Proc {
    /// まだ回収していなければ `SIGKILL` を送る（何度呼んでもよい）
    fn kill(&self) {
        if self.exit.is_none() {
            // SAFETY: 錠の下で、まだ回収していない自分の子（終わっていてもゾンビで残り、pid は使い回されない）
            unsafe { libc::kill(self.pid, libc::SIGKILL) };
        }
    }

    /// 終わっていれば回収して終了コードを返す。まだなら `None`
    fn try_reap(&mut self) -> Option<Option<i32>> {
        if let Some(exit) = self.exit {
            return Some(exit);
        }
        loop {
            let mut status = 0;
            // SAFETY: 自分の子の pid と、この関数の中の変数を渡すだけ（WNOHANG で待たない）
            let got = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
            if got == 0 {
                return None;
            }
            if got == -1 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            // 回収できた。または回収できない誤り（もう子ではない）。どちらも終わりとして扱い、以後は送らない
            let exit =
                (got == self.pid && libc::WIFEXITED(status)).then(|| libc::WEXITSTATUS(status));
            self.exit = Some(exit);
            return Some(exit);
        }
    }
}

/// `program` を、標準出力だけをパイプにして起こす（`environment` 以外の環境は渡さない）
pub fn spawn_running(
    program: &Path,
    args: &[String],
    environment: &[(OsString, OsString)],
) -> io::Result<Running> {
    let words: Vec<CString> = iter::once(program.as_os_str())
        .chain(args.iter().map(OsStr::new))
        .map(c_string)
        .collect::<io::Result<_>>()?;
    let mut argv: Vec<*mut libc::c_char> = words.iter().map(|w| w.as_ptr().cast_mut()).collect();
    argv.push(ptr::null_mut());
    let pairs: Vec<CString> = environment
        .iter()
        .map(|(k, v)| env_pair(k, v))
        .collect::<io::Result<_>>()?;
    let mut envp: Vec<*mut libc::c_char> = pairs.iter().map(|p| p.as_ptr().cast_mut()).collect();
    envp.push(ptr::null_mut());

    // 子が書く側（標準出力）
    let (parent_out, child_out) = io::pipe()?;
    let child_out = stage(&OwnedFd::from(child_out))?;

    let mut actions = FileActions::new()?;
    actions.open_null(libc::STDIN_FILENO, libc::O_RDONLY)?;
    actions.dup2(child_out.as_raw_fd(), libc::STDOUT_FILENO)?;
    actions.open_null(libc::STDERR_FILENO, libc::O_WRONLY)?;
    let mut attr = SpawnAttr::new()?;
    attr.only_listed_fds()?;

    let mut pid: libc::pid_t = 0;
    // SAFETY: 渡すポインタは、どれもこの呼び出しの間生きている（`words`、`argv`、`pairs`、`envp`、`actions`、`attr`）。
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
    // 子の側の端は、親では閉じる（閉じないと、子が終わっても読み取りが終わらない）
    drop(child_out);

    let proc = Arc::new(Mutex::new(Proc { pid, exit: None }));
    let kill_proc = Arc::clone(&proc);
    Ok(Running {
        stdout: Box::new(File::from(OwnedFd::from(parent_out))),
        kill: Box::new(move || lock(&kill_proc).kill()),
        wait: Box::new(move || {
            loop {
                if let Some(exit) = lock(&proc).try_reap() {
                    return exit;
                }
                thread::sleep(WAIT_POLL);
            }
        }),
    })
}

/// fd を、標準入出力の番号と重ならない番号へ写す（close-on-exec。`posix_spawn` の写しで子に付く）
fn stage(fd: &OwnedFd) -> io::Result<OwnedFd> {
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

/// 環境の 1 つ（`名前=値`）。名前に `=` があれば誤り
fn env_pair(key: &OsStr, value: &OsStr) -> io::Result<CString> {
    if key.is_empty() || key.as_bytes().contains(&b'=') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bad environment name",
        ));
    }
    let mut bytes = Vec::with_capacity(key.len() + 1 + value.len());
    bytes.extend_from_slice(key.as_bytes());
    bytes.push(b'=');
    bytes.extend_from_slice(value.as_bytes());
    CString::new(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "nul in environment"))
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

    /// 子の `fd` に `/dev/null` を開く（親の fd は渡さない）
    fn open_null(&mut self, fd: RawFd, mode: libc::c_int) -> io::Result<()> {
        // SAFETY: 初期化済みの値に、操作を 1 つ足すだけ。パスは 'static の NUL で終わる文字列
        check(unsafe {
            libc::posix_spawn_file_actions_addopen(&mut self.0, fd, NULL_DEVICE.as_ptr(), mode, 0)
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
    use std::io::Read;

    use super::*;

    /// 親で、close-on-exec の印なしに開いておく fd の番号（ほかのテストの fd と重ならない大きさ）
    const PROBE_FD: RawFd = 201;

    #[test]
    fn the_child_gets_only_the_three_standard_fds() {
        let file = File::open("/dev/null").unwrap();
        // SAFETY: 開いた fd を、使われていない番号に複製するだけ（`dup2` の結果には close-on-exec が付かない）
        let fd = unsafe { libc::dup2(file.as_raw_fd(), PROBE_FD) };
        assert_eq!(fd, PROBE_FD);
        // SAFETY: いま開いた fd で、ほかに持ち主はいない
        let _probe = unsafe { OwnedFd::from_raw_fd(fd) };
        let script = format!(
            "for n in 0 1 2 {PROBE_FD}; do if [ -e /dev/fd/$n ]; then printf 'o'; else printf 'c'; fi; done"
        );

        // 対照: `std::process` の子には渡る（このテストが意味を持つことの確かめ）
        let leaked = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&script)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&leaked.stdout), "oooo");

        let mut running =
            spawn_running(Path::new("/bin/sh"), &["-c".to_owned(), script], &[]).unwrap();
        let mut out = String::new();
        running.stdout.read_to_string(&mut out).unwrap();
        assert_eq!(out, "oooc");
        assert_eq!((running.wait)(), Some(0));
    }

    #[test]
    fn a_bad_environment_name_is_refused() {
        let env = [(OsString::from("A=B"), OsString::from("x"))];
        let err = spawn_running(Path::new("/bin/sh"), &[], &env).err();
        assert_eq!(err.map(|e| e.kind()), Some(io::ErrorKind::InvalidInput));
    }
}
