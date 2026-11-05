//! シグナルの処理（ADR 0051 §3.1、§4.2）。`cfg(unix)` のみ。
//!
//! 処理の中でするのは、起動のときに作っておいた固定のバイト列を `write(2)` で印に足し、
//! 前の処理に戻してシグナルを送り直す（または前の処理を呼ぶ）ことだけ。整形もメモリ確保もしない
//! （async-signal-safe な関数は `write`、`sigaction`、`raise` だけを使う）。

use std::os::fd::RawFd;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Once, OnceLock};

use crate::{EXIT_CRASH_LINE_PREFIX, EXIT_TERM_LINE_PREFIX};

/// 印の fd が無いことを表す値
const NO_FD: RawFd = -1;

/// 今の印の fd（`RunMark::begin` が置き、`RunMark` を落とすときに外す）。処理の中で読む
static MARK_FD: AtomicI32 = AtomicI32::new(NO_FD);

/// 終わらせるシグナル（`exit=term`。ADR 0051 §4.1）
const TERM_SIGNALS: [libc::c_int; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP];

/// 致命的なシグナル（`exit=crash`。ADR 0051 §4.1）
const CRASH_SIGNALS: [libc::c_int; 7] = [
    libc::SIGSEGV,
    libc::SIGBUS,
    libc::SIGILL,
    libc::SIGFPE,
    libc::SIGABRT,
    libc::SIGTRAP,
    libc::SIGSYS,
];

/// 代わりのスタックの大きさ。`SIGSTKSZ` より小さくしない
const ALT_STACK_BYTES: usize = 64 * 1024;

/// シグナルごとの、印に書くバイト列と、前の処理
struct Slot {
    sig: libc::c_int,
    /// `exit=term sig=15\n` など（起動のときに作る）
    line: Box<[u8]>,
    /// 入れる前の処理。処理の中で戻す
    prev: libc::sigaction,
    /// 処理を入れたか（前の処理が `SIG_IGN` なら入れない。無視していたシグナルで終わることはない）
    installed: bool,
}

/// 処理の中で読む表。入れる前に一度だけ作り、あとは読むだけ
static SLOTS: OnceLock<Vec<Slot>> = OnceLock::new();

static INSTALL: Once = Once::new();

pub(crate) fn set_mark_fd(fd: RawFd) {
    MARK_FD.store(fd, Ordering::SeqCst);
}

/// `fd` が今の印なら外す（別の `RunMark` が置き直していれば触らない）
pub(crate) fn clear_mark_fd(fd: RawFd) {
    let _ = MARK_FD.compare_exchange(fd, NO_FD, Ordering::SeqCst, Ordering::SeqCst);
}

pub(crate) fn install() {
    // 2 回入れると「前の処理」が自分になり、処理の中で自分を呼び続けるので、1 回に限る
    INSTALL.call_once(install_once);
}

fn install_once() {
    let mut slots = Vec::with_capacity(TERM_SIGNALS.len() + CRASH_SIGNALS.len());
    let groups = [
        (EXIT_TERM_LINE_PREFIX, &TERM_SIGNALS[..]),
        (EXIT_CRASH_LINE_PREFIX, &CRASH_SIGNALS[..]),
    ];
    for (prefix, sigs) in groups {
        for &sig in sigs {
            let Some(prev) = current_action(sig) else {
                continue;
            };
            slots.push(Slot {
                sig,
                line: format!("{prefix}{sig}\n").into_bytes().into_boxed_slice(),
                prev,
                installed: prev.sa_sigaction != libc::SIG_IGN,
            });
        }
    }
    let slots = SLOTS.get_or_init(|| slots);
    ensure_alt_stack();
    for slot in slots.iter().filter(|s| s.installed) {
        // SAFETY: `sigaction` に渡す構造体はこの関数の中で 0 で初期化してから埋める。
        // 処理の関数 `handle` は async-signal-safe な関数（write、sigaction、raise）だけを呼ぶ。
        // SA_RESETHAND: 処理に入ったら既定に戻る（処理の中でもう一度落ちても、既定の動作で終わる）。
        // SA_ONSTACK: スタックの溢れでも代わりのスタックで動ける（入口のスレッドだけ。§4.2）。
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = handle as *const () as libc::sighandler_t;
            action.sa_flags = libc::SA_SIGINFO | libc::SA_RESETHAND | libc::SA_ONSTACK;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(slot.sig, &action, std::ptr::null_mut());
        }
    }
}

/// 今の処理を読む（変えない）
fn current_action(sig: libc::c_int) -> Option<libc::sigaction> {
    // SAFETY: 新しい処理に null を渡すと読むだけになる。`old` は 0 で初期化してから OS が埋める
    unsafe {
        let mut old: libc::sigaction = std::mem::zeroed();
        (libc::sigaction(sig, std::ptr::null(), &mut old) == 0).then_some(old)
    }
}

/// 呼んだスレッドに代わりのスタックがなければ作る（Rust の std は自分のスレッドに作っているので、
/// あれば替えない）。作ったスタックはプロセスの終わりまで使うので、解放しない
fn ensure_alt_stack() {
    // SAFETY: `sigaltstack` に null を渡すと読むだけ。新しく置くスタックは leak したヒープで、
    // プロセスの終わりまで有効（解放しないので、処理の中で使っている間に消えることはない）
    unsafe {
        let mut old: libc::stack_t = std::mem::zeroed();
        if libc::sigaltstack(std::ptr::null(), &mut old) != 0 {
            return;
        }
        if old.ss_flags & libc::SS_DISABLE == 0 {
            return;
        }
        let size = ALT_STACK_BYTES.max(libc::SIGSTKSZ);
        let stack: &'static mut [u8] = Box::leak(vec![0u8; size].into_boxed_slice());
        let new = libc::stack_t {
            ss_sp: stack.as_mut_ptr().cast(),
            ss_size: size,
            ss_flags: 0,
        };
        libc::sigaltstack(&new, std::ptr::null_mut());
    }
}

/// シグナルの処理。async-signal-safe な範囲だけ（メモリ確保、ロック、整形をしない）
extern "C" fn handle(sig: libc::c_int, info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
    // `OnceLock::get` は原子的な読み出しだけで、ロックもメモリ確保もしない
    let Some(slot) = SLOTS
        .get()
        .and_then(|slots| slots.iter().find(|s| s.sig == sig))
    else {
        // 表がない（入れる途中）: 既定に戻して送り直す
        // SAFETY: SIG_DFL に戻して自分に送り直すだけ。どちらも async-signal-safe
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
            libc::raise(sig);
        }
        return;
    };
    let fd = MARK_FD.load(Ordering::SeqCst);
    if fd != NO_FD {
        // SAFETY: `write(2)` は async-signal-safe。`line` は入れる前に作った読むだけのバイト列。
        // 書けなくても（fd が閉じられた、など）何もしない
        unsafe {
            libc::write(fd, slot.line.as_ptr().cast(), slot.line.len());
        }
    }
    // SAFETY: 前の処理を戻すのは `sigaction`（async-signal-safe）。
    // 前の処理が既定なら、自分に送り直す。処理の中なので同じシグナルは止められていて、戻ったところで
    // 既定の動作（終了、クラッシュレポートの作成）が起きる。プロセスの終わり方は処理を入れる前と同じ。
    // 前の処理が関数なら、それを同じ引数で呼ぶ（Rust の std のスタックの溢れの検出などを壊さない）。
    unsafe {
        libc::sigaction(sig, &slot.prev, std::ptr::null_mut());
        let prev = slot.prev.sa_sigaction;
        if prev == libc::SIG_DFL {
            libc::raise(sig);
        } else if prev != libc::SIG_IGN {
            if slot.prev.sa_flags & libc::SA_SIGINFO != 0 {
                let f: extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void) =
                    std::mem::transmute(prev);
                f(sig, info, ctx);
            } else {
                let f: extern "C" fn(libc::c_int) = std::mem::transmute(prev);
                f(sig);
            }
        }
    }
}
