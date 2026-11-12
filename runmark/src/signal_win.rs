//! クラッシュと終了の記録（ADR 0051、ADR 0040 W4）。`cfg(windows)` のみ。`signal`（Unix）の Windows 版。
//!
//! - 致命的な例外（`SetUnhandledExceptionFilter`）: `exit=crash sig=N`。N は例外コード（NTSTATUS）の下位 28 bit
//!   （`0xC0000005` なら 5、スタックの溢れ `0xC00000FD` なら 253）。上位 4 bit は重大度で、致命的な例外では
//!   いつも `C` なので落とす（`sig` は `i32` で読むため、符号つきにならない範囲にする）。
//! - 終わらせる通知（`SetConsoleCtrlHandler`）: `exit=term sig=N`。N は `CTRL_C_EVENT`（0）、`CTRL_BREAK_EVENT`（1）、
//!   `CTRL_CLOSE_EVENT`（2）、`CTRL_LOGOFF_EVENT`（5）、`CTRL_SHUTDOWN_EVENT`（6）。コンソールのない常駐では
//!   Ctrl+C などは来ない（窓のない終了は W5 で決める。メモ）。
//!
//! 処理の中でするのは、起動のときに作っておいた固定のバイト列を `WriteFile` で印に足すことだけ。整形もメモリ確保もしない。
//! 例外は前の処理（OS の既定。WER のクラッシュレポート）へ `EXCEPTION_CONTINUE_SEARCH` で回す。

use std::os::windows::io::RawHandle;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::{Once, OnceLock};

use windows_sys::Win32::Foundation::{FALSE, HANDLE};
use windows_sys::Win32::Storage::FileSystem::WriteFile;
use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
use windows_sys::Win32::System::Diagnostics::Debug::{
    EXCEPTION_POINTERS, SetUnhandledExceptionFilter,
};

use crate::{EXIT_CRASH_LINE_PREFIX, EXIT_TERM_LINE_PREFIX};

/// 印のハンドルが無いことを表す値
const NO_HANDLE: isize = 0;

/// 例外コードのうち、`sig` に使う下位のビット（上位 4 bit は重大度）
const CODE_MASK: u32 = 0x0FFF_FFFF;

/// `EXCEPTION_CONTINUE_SEARCH`（前の処理へ回す）
const CONTINUE_SEARCH: i32 = 0;

/// 終わらせる通知（`CTRL_C_EVENT`、`CTRL_BREAK_EVENT`、`CTRL_CLOSE_EVENT`、`CTRL_LOGOFF_EVENT`、`CTRL_SHUTDOWN_EVENT`）
const TERM_EVENTS: [u32; 5] = [0, 1, 2, 5, 6];

/// 今の印のハンドル（`RunMark::begin` が置き、`RunMark` を落とすときに外す）。処理の中で読む
static MARK_HANDLE: AtomicIsize = AtomicIsize::new(NO_HANDLE);

/// `exit=crash sig=` の行を作るための、最後に起きた例外コード。処理の中で 1 回だけ書く（再入を防ぐ）
static CRASHING: AtomicU32 = AtomicU32::new(0);

/// 通知ごとの、印に書くバイト列（起動のときに作る）
static TERM_LINES: OnceLock<Vec<(u32, Box<[u8]>)>> = OnceLock::new();

static INSTALL: Once = Once::new();

pub(crate) fn set_mark_handle(handle: RawHandle) {
    MARK_HANDLE.store(handle as isize, Ordering::SeqCst);
}

/// `handle` が今の印なら外す（別の `RunMark` が置き直していれば触らない）
pub(crate) fn clear_mark_handle(handle: RawHandle) {
    let _ = MARK_HANDLE.compare_exchange(
        handle as isize,
        NO_HANDLE,
        Ordering::SeqCst,
        Ordering::SeqCst,
    );
}

pub(crate) fn install() {
    // 2 回入れると前の処理が自分になるので、1 回に限る
    INSTALL.call_once(install_once);
}

fn install_once() {
    let lines = TERM_EVENTS
        .iter()
        .map(|&event| {
            (
                event,
                format!("{EXIT_TERM_LINE_PREFIX}{event}\n")
                    .into_bytes()
                    .into_boxed_slice(),
            )
        })
        .collect();
    let _ = TERM_LINES.set(lines);
    // SAFETY: どちらも、渡す関数は `extern "system"` で、プロセスの終わりまで有効。
    // 前の例外の処理は返り値で受けるが、呼ばない（`EXCEPTION_CONTINUE_SEARCH` で OS に回すので、
    // 前の処理が先にいても、後に OS の既定（WER）が動く）。コンソールの処理は FALSE を返して次の処理へ回す。
    unsafe {
        SetUnhandledExceptionFilter(Some(on_exception));
        SetConsoleCtrlHandler(Some(on_console_event), 1);
    }
}

/// 印に `bytes` を足す。書けなくても何もしない
fn append(bytes: &[u8]) {
    let handle = MARK_HANDLE.load(Ordering::SeqCst);
    if handle == NO_HANDLE {
        return;
    }
    let len = u32::try_from(bytes.len()).unwrap_or(0);
    let mut written = 0u32;
    // SAFETY: `handle` は `RunMark` が持つ開いたファイル（落とすときに先に外す）。`bytes` は読むだけの有効な領域。
    // 追記で開いているので、末尾に書かれる。OVERLAPPED は使わない（null）
    unsafe {
        WriteFile(
            handle as HANDLE,
            bytes.as_ptr(),
            len,
            &mut written,
            std::ptr::null_mut(),
        );
    }
}

/// 致命的な例外の処理。メモリ確保をしない範囲で、整数を十進にして固定の頭に足す
unsafe extern "system" fn on_exception(info: *const EXCEPTION_POINTERS) -> i32 {
    // SAFETY: OS が渡す `info` は、この関数の間は有効。null なら何も書かない
    let code = unsafe {
        info.as_ref()
            .and_then(|i| i.ExceptionRecord.as_ref())
            .map(|r| r.ExceptionCode as u32)
    };
    // 処理の中でもう一度落ちても、2 行目を書かない
    if let Some(code) = code
        && CRASHING
            .compare_exchange(0, code | 1 << 31, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    {
        // 固定長の領域に整形する（`EXIT_CRASH_LINE_PREFIX` + 十進 + 改行。ヒープを使わない）
        let mut buf = [0u8; 48];
        let n = format_line(
            &mut buf,
            EXIT_CRASH_LINE_PREFIX.as_bytes(),
            code & CODE_MASK,
        );
        append(&buf[..n]);
    }
    CONTINUE_SEARCH
}

/// 終わらせる通知の処理。次の処理にも回す（FALSE）
unsafe extern "system" fn on_console_event(event: u32) -> i32 {
    if let Some(lines) = TERM_LINES.get()
        && let Some((_, line)) = lines.iter().find(|(e, _)| *e == event)
    {
        append(line);
    }
    FALSE
}

/// `prefix` + `n` の十進 + 改行を `buf` に書いて、長さを返す
fn format_line(buf: &mut [u8; 48], prefix: &[u8], mut n: u32) -> usize {
    buf[..prefix.len()].copy_from_slice(prefix);
    let mut pos = prefix.len();
    let mut digits = [0u8; 10];
    let mut count = 0;
    loop {
        digits[count] = b'0' + (n % 10) as u8;
        count += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    while count > 0 {
        count -= 1;
        buf[pos] = digits[count];
        pos += 1;
    }
    buf[pos] = b'\n';
    pos + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_line_writes_prefix_decimal_and_newline() {
        let mut buf = [0u8; 48];
        let n = format_line(&mut buf, b"exit=crash sig=", 253);
        assert_eq!(&buf[..n], b"exit=crash sig=253\n");
        let n = format_line(&mut buf, b"exit=crash sig=", 0);
        assert_eq!(&buf[..n], b"exit=crash sig=0\n");
        let n = format_line(&mut buf, b"exit=crash sig=", CODE_MASK);
        assert_eq!(&buf[..n], b"exit=crash sig=268435455\n");
    }

    #[test]
    fn code_mask_keeps_it_readable_as_i32() {
        for code in [0xC000_0005u32, 0xC000_00FD, 0xC000_0409] {
            assert!(i32::try_from(code & CODE_MASK).is_ok());
        }
        assert_eq!(0xC000_0005u32 & CODE_MASK, 5);
    }
}
