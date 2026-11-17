//! fd の種類と開き方を取る（unix。檻役が受けた fd の検査に使う。FC-2。判断は `cage::check_launch_fds`）。

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};

use crate::host::cage::{FdAccess, FdFacts, FdKind};

/// `fstat` の種類と、`fcntl(F_GETFL)` の読み書きの別。fd の中身は読まない
pub(crate) fn fd_facts(fd: BorrowedFd<'_>) -> io::Result<FdFacts> {
    let raw = fd.as_raw_fd();
    // SAFETY: `stat` はこの関数の中の変数。`fd` は呼び出しの間、開いている（`BorrowedFd`）
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: 同上。状態を読むだけ
    if unsafe { libc::fstat(raw, &mut stat) } == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 開いている fd の状態の印を読むだけ
    let flags = unsafe { libc::fcntl(raw, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    let kind = match stat.st_mode & libc::S_IFMT {
        libc::S_IFREG => FdKind::Regular,
        libc::S_IFIFO => FdKind::Pipe,
        libc::S_IFSOCK => FdKind::Socket,
        libc::S_IFDIR => FdKind::Directory,
        _ => FdKind::Other,
    };
    let access = match flags & libc::O_ACCMODE {
        libc::O_RDONLY => FdAccess::Read,
        libc::O_WRONLY => FdAccess::Write,
        _ => FdAccess::ReadWrite,
    };
    Ok(FdFacts { kind, access })
}
