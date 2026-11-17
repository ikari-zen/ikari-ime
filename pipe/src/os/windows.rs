//! Win32 の名前付きパイプで `Duplex` と `Listener` を実装する（ADR 0040 §2.2。W1-b）。
//!
//! - サーバー（`NamedPipeListener`）: `FILE_FLAG_FIRST_PIPE_INSTANCE`（先に別のプロセスが同じ名前で作っていたら
//!   失敗する。なりすましの対策）、`PIPE_REJECT_REMOTE_CLIENTS`、自分のユーザーと SYSTEM だけの DACL。
//!   受け付けのたびに次のインスタンスを先に作り、名前を手放さない。
//! - クライアント（`connect`）: `CreateFileW` で開き、`GetNamedPipeServerProcessId` で相手の PID を取って
//!   `Client::server_pid()` に返す。返す前に、パイプの持ち主（受ける側のプロセスの利用者とセッション）が自分と
//!   同じかを確かめ、違えば・取れなければ断つ（`peer::verify_pipe_owner`。ADR 0068 §2 の 2）。PID が正しい相手か
//!   （インストール先のパスと一致するか）は口ごとに違う判断なので、使う側（TIP、設定画面）が決める。
//! - 流れ（`NamedPipeDuplex`）: 重ね合わせ（overlapped）の I/O。読みと書きは別の `OVERLAPPED` とイベントを
//!   使うので、同じハンドルでも直列にならない。`Shutdown` は、閉じる指示のイベントで待ちを起こし、
//!   進行中の操作を `CancelIoEx` で取り消してから、ハンドルを閉じる（相手には切断が伝わる）。
//!
//! `unsafe` はこのファイルだけに置く。ハンドルは `OwnedHandle`（標準ライブラリ）で包み、閉じ忘れを防ぐ。
//! 重ね合わせの操作は `run_overlapped` の中で始めて、終わるまで戻らない（`OVERLAPPED` とバッファが、
//! 操作の間ずっと同じ場所で生きていることを、この 1 か所で保証する）。

use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard};

use windows_sys::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_INSUFFICIENT_BUFFER, ERROR_IO_PENDING, ERROR_NO_DATA,
    ERROR_OPERATION_ABORTED, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED,
    GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL, INVALID_HANDLE_VALUE, LocalFree, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, PSECURITY_DESCRIPTOR,
    SECURITY_ATTRIBUTES, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_USER, TokenIntegrityLevel,
    TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_SHARE_NONE, GetLongPathNameW, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    GetNamedPipeServerProcessId, GetNamedPipeServerSessionId, NAMED_PIPE_MODE, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    WaitNamedPipeW,
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, GetCurrentProcessId, INFINITE, OpenProcess, OpenProcessToken,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, ResetEvent,
    SetEvent, WaitForMultipleObjects,
};

use crate::client::Client;
use crate::duplex::{Duplex, Halves, Listener, Shutdown, WRITE_DEADLINE};
use crate::elevation::{self, Refusal};
use crate::name::pipe_name;
use crate::peer::{PeerRejection, SessionUser, verify_pipe_owner};

/// サーバーのインスタンスの開き方。読み書きの両方向、重ね合わせの I/O。
const SERVER_OPEN_MODE: FILE_FLAGS_AND_ATTRIBUTES = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
/// パイプの型。バイトの流れにする。枠（`frame`）が自分で長さを持つので、メッセージの境目は要らない。
/// MESSAGE にすると、読みのバッファより長い本体で `ERROR_MORE_DATA` を扱う必要が出て、`Read` の上で
/// 枠を `read_exact` で読む今の形と合わない。ネットワークの向こうのクライアントは受けない。
const PIPE_MODE: NAMED_PIPE_MODE =
    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
/// インスタンスの数の上限。アプリごとに TIP が 1 本つなぐので、決めた数で切らない（相手は自分のユーザーだけ）。
const MAX_INSTANCES: u32 = PIPE_UNLIMITED_INSTANCES;
/// パイプの送りと受けのバッファの大きさ（OS への目安。上限ではなく、超えた書きは相手が読むまで待つ）。
/// キー 1 回の要求と返事（数 KiB）が 1 回で収まる大きさ。計測で決めた値ではない（S1 で見直す）。
const PIPE_BUFFER_BYTES: u32 = 64 * 1024;
/// `WaitNamedPipeW` に `NMPWAIT_USE_DEFAULT_WAIT` を渡したときの待ち。0 は OS の既定（50ms）。
/// クライアントは自分の待ち（`CONNECT_BUSY_WAIT_MS`）を渡すので、使われない。
const PIPE_DEFAULT_TIMEOUT_MS: u32 = 0;

/// クライアントの開き方。読み書き。
const CLIENT_ACCESS: u32 = GENERIC_READ | GENERIC_WRITE;
/// クライアントの開き方。重ね合わせの I/O。サーバーにはクライアントの「識別」までしか許さない
/// （なりすました相手が、クライアントの権限で動けないようにする）。
const CLIENT_FLAGS: FILE_FLAGS_AND_ATTRIBUTES =
    FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION;
/// すべてのインスタンスがふさがっている（`ERROR_PIPE_BUSY`）ときに、待ってやり直す回数。
/// サーバーは受け付けた直後に次のインスタンスを作るので、ふさがるのは一瞬のはず。何度も待たない。
const CONNECT_BUSY_RETRIES: u32 = 3;
/// ふさがっているときに 1 回待つ長さ（ms）。TIP はアプリのキー入力の中でつなぐので、合計
/// （`CONNECT_BUSY_RETRIES` × これ）を 0.3 秒ほどに抑える。計測で決めた値ではない（W2 の実機で見直す）。
const CONNECT_BUSY_WAIT_MS: u32 = 100;

/// Win32 の `BOOL` の偽と真。
const FALSE: i32 = 0;
const TRUE: i32 = 1;

fn is_code(e: &io::Error, code: u32) -> bool {
    e.raw_os_error() == Some(code as i32)
}

/// 相手が閉じた、または切り離したことを示す誤り。
fn is_disconnected(e: &io::Error) -> bool {
    is_code(e, ERROR_BROKEN_PIPE)
        || is_code(e, ERROR_PIPE_NOT_CONNECTED)
        || is_code(e, ERROR_NO_DATA)
}

fn broken_pipe() -> io::Error {
    io::Error::from(io::ErrorKind::BrokenPipe)
}

/// NUL で終わる UTF-16。途中に NUL がある名前は受けない（名前が途中で切れるため）。
fn to_wide(s: &str) -> io::Result<Vec<u16>> {
    if s.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "name contains NUL",
        ));
    }
    Ok(s.encode_utf16().chain(std::iter::once(0)).collect())
}

/// 直前の Win32 の呼び出しが返したハンドルを所有する。null と `INVALID_HANDLE_VALUE` は失敗として、
/// その呼び出しの誤り（`GetLastError`）を返す。呼び出しの直後に呼ぶこと。
fn own_handle(raw: HANDLE) -> io::Result<OwnedHandle> {
    if raw.is_null() || raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: raw は直前の呼び出しが返した、開いている有効なハンドルで、ほかに所有者はいない
    // （ここで 1 回だけ包み、閉じるのは OwnedHandle の Drop だけ）。
    Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
}

/// 手動で戻すイベント（重ね合わせの I/O の完了と、閉じる指示に使う）。
struct Event(OwnedHandle);

impl Event {
    fn new() -> io::Result<Event> {
        // SAFETY: セキュリティ属性と名前は null（既定、名前なし）。手動で戻す、最初は立っていない。
        // 返り値は own_handle が確かめて所有する。
        let raw = unsafe { CreateEventW(ptr::null(), TRUE, FALSE, ptr::null()) };
        own_handle(raw).map(Event)
    }

    fn raw(&self) -> HANDLE {
        self.0.as_raw_handle()
    }

    /// 立てる。失敗はハンドルが無効なときだけで、自分で持っているハンドルなので起きない。
    fn set(&self) {
        // SAFETY: self.0 は自分が所有する、開いているイベントのハンドル
        unsafe { SetEvent(self.raw()) };
    }

    fn reset(&self) -> io::Result<()> {
        // SAFETY: self.0 は自分が所有する、開いているイベントのハンドル
        if unsafe { ResetEvent(self.raw()) } == FALSE {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// `LocalAlloc` で OS が確保したもの（SID の文字列、セキュリティ記述子）。Drop で `LocalFree` する。
struct LocalMem(HLOCAL);

impl Drop for LocalMem {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: self.0 は OS が LocalAlloc で返したもので、ほかに解放する者はいない（1 回だけ解放する）
            unsafe { LocalFree(self.0) };
        }
    }
}

/// 重ね合わせの操作の終わり方。
enum Completion {
    /// 終わった（運んだバイト数）
    Done(u32),
    /// 閉じる指示で取り消した
    Cancelled,
    /// 締め切りを超えたので取り消した
    TimedOut,
}

/// 締め切りなし（読みと接続の待ち）。
const NO_DEADLINE_MS: u32 = INFINITE;

/// `WRITE_DEADLINE` をミリ秒（`WaitForMultipleObjects` の単位）にしたもの。
fn write_deadline_ms() -> u32 {
    u32::try_from(WRITE_DEADLINE.as_millis()).unwrap_or(NO_DEADLINE_MS - 1)
}

/// 重ね合わせの操作を 1 つ行い、終わるまで待つ。`cancel` が立つか、`timeout_ms` を超えたら取り消す。
///
/// `start` は `OVERLAPPED` を受けて、`handle` への操作（`ReadFile` など）を始め、その返り値（`BOOL`）を返す。
/// この関数は、操作を始めたら、終わる（または取り消しが済む）まで戻らない。
///
/// # Safety
/// - `handle` は、この呼び出しの間ずっと開いている、`FILE_FLAG_OVERLAPPED` で開いたハンドル。
/// - `start` は `handle` への操作を 1 つだけ始め、渡されたポインタを `OVERLAPPED` として渡す。
/// - `start` が OS に渡すバッファは、この呼び出しの間ずっと生きていて、ほかから触られない。
/// - `event` を、この呼び出しの間ほかの操作に使わない。
unsafe fn run_overlapped(
    handle: HANDLE,
    event: &Event,
    cancel: &Event,
    timeout_ms: u32,
    start: impl FnOnce(*mut OVERLAPPED) -> i32,
) -> io::Result<Completion> {
    event.reset()?;
    let mut timed_out = false;
    // ov はこのフレームに置き、下の GetOverlappedResult（完了まで待つ）を通らずに抜ける道を作らない。
    // そのため、OS が ov を使っている間、ov は動かず生きている。
    let mut ov = OVERLAPPED {
        hEvent: event.raw(),
        ..Default::default()
    };
    let ov_ptr: *mut OVERLAPPED = &mut ov;
    if start(ov_ptr) == FALSE {
        let err = io::Error::last_os_error();
        if !is_code(&err, ERROR_IO_PENDING) {
            // 操作は始まらなかった（ERROR_PIPE_CONNECTED など）。OS は ov を使っていない
            return Err(err);
        }
        let handles = [event.raw(), cancel.raw()];
        // SAFETY: handles の 2 つは、借りている event と cancel のハンドルで、この呼び出しの間開いている
        let waited = unsafe {
            WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), FALSE, timeout_ms)
        };
        if waited != WAIT_OBJECT_0 {
            timed_out = waited == WAIT_TIMEOUT;
            // 締め切り、閉じる指示（WAIT_OBJECT_0 + 1）、または待ちの失敗。どちらも取り消し、終わるのを下で待つ。
            // SAFETY: handle は開いていて、ov_ptr は進行中の操作の OVERLAPPED（このフレームの ov）。
            // 失敗（ERROR_NOT_FOUND）は、すでに終わっていたこと。どちらでも下の待ちで結果を受ける。
            unsafe { CancelIoEx(handle, ov_ptr) };
        }
    }
    let mut transferred = 0u32;
    // SAFETY: handle は開いていて、ov_ptr はこの操作の OVERLAPPED。bWait が TRUE なので、操作が終わるまで
    // 戻らない。ここを抜けたあと、OS は ov にもバッファにも触れない。
    let ok = unsafe { GetOverlappedResult(handle, ov_ptr, &mut transferred, TRUE) };
    if ok == FALSE {
        let err = io::Error::last_os_error();
        if is_code(&err, ERROR_OPERATION_ABORTED) {
            // 締め切りの取り消しが、完了と行き違いになったときは、上の Done になる（書けたものは書けた）
            return Ok(if timed_out {
                Completion::TimedOut
            } else {
                Completion::Cancelled
            });
        }
        return Err(err);
    }
    Ok(Completion::Done(transferred))
}

/// パイプのハンドルと、受け口に「まだ生きている接続がある」ことを知らせる印。
struct PipeSlot {
    // 宣言の順に落ちる。印を先に落とし、ハンドルをあとで閉じる。逆だと、ハンドルが閉じて名前が空いたのに
    // 受け口が「生きている接続がある」と見て、FIRST_PIPE_INSTANCE なしで作る隙ができる。この順なら、隙は
    // 「印が消えたのにハンドルが残っている」側になり、FIRST_PIPE_INSTANCE つきの作成が失敗するだけで済む。
    _alive: Option<Arc<()>>,
    handle: OwnedHandle,
}

/// 名前付きパイプの接続 1 本（サーバー側、クライアント側のどちらにも使う）。
pub struct NamedPipeDuplex {
    slot: PipeSlot,
    cancel: Event,
    read_event: Event,
    write_event: Event,
}

impl NamedPipeDuplex {
    fn new(handle: OwnedHandle, alive: Option<Arc<()>>) -> io::Result<NamedPipeDuplex> {
        Ok(NamedPipeDuplex {
            slot: PipeSlot {
                _alive: alive,
                handle,
            },
            cancel: Event::new()?,
            read_event: Event::new()?,
            write_event: Event::new()?,
        })
    }
}

/// 分けたあとの、読む側、書く側、閉じる手段が共有するもの。
struct Conn {
    /// `None` は閉じたあと。読みと書きは共有のロックの間だけハンドルを使い、閉じる側は排他のロックで取り出す
    /// （使っている最中のハンドルを閉じない）。
    pipe: RwLock<Option<PipeSlot>>,
    /// 閉じる指示。立てると、待っている読みと書きが起きて、操作を取り消す
    cancel: Event,
    closed: AtomicBool,
}

impl Conn {
    fn pipe(&self) -> RwLockReadGuard<'_, Option<PipeSlot>> {
        self.pipe.read().unwrap_or_else(|e| e.into_inner())
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

impl Shutdown for Conn {
    fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        // 先に起こす。待っている読みと書きは取り消して共有のロックを手放すので、下の排他のロックが取れる
        self.cancel.set();
        let slot = self.pipe.write().unwrap_or_else(|e| e.into_inner()).take();
        // ハンドルを閉じる。相手の読みは EOF（ERROR_BROKEN_PIPE）、書きは誤りになる
        drop(slot);
    }
}

struct PipeReader {
    conn: Arc<Conn>,
    event: Event,
}

impl Read for PipeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.conn.is_closed() {
            return Ok(0);
        }
        let guard = self.conn.pipe();
        let Some(slot) = guard.as_ref() else {
            return Ok(0);
        };
        let handle = slot.handle.as_raw_handle();
        // 一度に読むのは u32 で表せる分まで（残りは次の read で読む）
        let len = u32::try_from(out.len()).unwrap_or(u32::MAX);
        let buf = out.as_mut_ptr();
        // SAFETY: handle は開いたパイプ、buf は len バイト書ける（out を借りている）。ov は run_overlapped が
        // 渡すもの。
        let start = |ov| unsafe { ReadFile(handle, buf, len, ptr::null_mut(), ov) };
        // SAFETY: handle は guard が閉じさせない（閉じる側は排他のロックを待つ）。buf（out）はこの呼び出しの
        // 間借りていて生きている。self.event はこの読み専用（&mut self で直列）。
        let done = unsafe {
            run_overlapped(
                handle,
                &self.event,
                &self.conn.cancel,
                NO_DEADLINE_MS,
                start,
            )
        };
        match done {
            Ok(Completion::Done(n)) => Ok(n as usize),
            Ok(Completion::Cancelled | Completion::TimedOut) => Ok(0),
            Err(e) if is_disconnected(&e) => Ok(0),
            Err(e) => Err(e),
        }
    }
}

struct PipeWriter {
    conn: Arc<Conn>,
    event: Event,
}

impl Write for PipeWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        if self.conn.is_closed() {
            return Err(broken_pipe());
        }
        let guard = self.conn.pipe();
        let Some(slot) = guard.as_ref() else {
            return Err(broken_pipe());
        };
        let handle = slot.handle.as_raw_handle();
        let len = u32::try_from(data.len()).unwrap_or(u32::MAX);
        let buf = data.as_ptr();
        // SAFETY: handle は開いたパイプ、buf は len バイト読める（data を借りている）。ov は run_overlapped が
        // 渡すもの。
        let start = |ov| unsafe { WriteFile(handle, buf, len, ptr::null_mut(), ov) };
        // SAFETY: handle は guard が閉じさせない。buf（data）はこの呼び出しの間借りていて生きている。
        // self.event はこの書き専用（&mut self で直列）。
        let done = unsafe {
            run_overlapped(
                handle,
                &self.event,
                &self.conn.cancel,
                write_deadline_ms(),
                start,
            )
        };
        match done {
            Ok(Completion::Done(n)) => Ok(n as usize),
            Ok(Completion::Cancelled) => Err(broken_pipe()),
            // 相手が読まずにバッファが埋まった。呼び出し側（Client、Server）が接続を切る
            Ok(Completion::TimedOut) => Err(io::Error::from(io::ErrorKind::TimedOut)),
            Err(e) if is_disconnected(&e) => Err(broken_pipe()),
            Err(e) => Err(e),
        }
    }

    /// 重ね合わせの書きは、終わった時点でパイプのバッファに入っている。待つものはない
    /// （`FlushFileBuffers` は相手が読み終えるまで止まるので、使わない）。
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Duplex for NamedPipeDuplex {
    fn split(self) -> Halves {
        let conn = Arc::new(Conn {
            pipe: RwLock::new(Some(self.slot)),
            cancel: self.cancel,
            closed: AtomicBool::new(false),
        });
        (
            Box::new(PipeReader {
                conn: Arc::clone(&conn),
                event: self.read_event,
            }),
            Box::new(PipeWriter {
                conn: Arc::clone(&conn),
                event: self.write_event,
            }),
            conn,
        )
    }
}

/// `ConvertStringSecurityDescriptorToSecurityDescriptorW` が作ったセキュリティ記述子。
struct SecurityDescriptor(LocalMem);

impl SecurityDescriptor {
    /// `sddl` は NUL で終わる UTF-16。
    fn from_sddl(sddl: &[u16]) -> io::Result<SecurityDescriptor> {
        debug_assert_eq!(sddl.last(), Some(&0));
        let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: sddl は NUL で終わり、この呼び出しの間生きている。sd には OS が LocalAlloc した記述子を
        // 書く（解放は LocalMem の Drop）。大きさは要らないので null。
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                ptr::null_mut(),
            )
        };
        if ok == FALSE {
            return Err(io::Error::last_os_error());
        }
        Ok(SecurityDescriptor(LocalMem(sd)))
    }

    fn as_ptr(&self) -> PSECURITY_DESCRIPTOR {
        (self.0).0
    }
}

/// 受け口を閉じる手段（別のスレッドから呼べる）。`accept` の待ちを起こし、以降の `accept` は `Ok(None)`。
pub struct ListenerCloser(Arc<ListenerClose>);

struct ListenerClose {
    cancel: Event,
    closed: AtomicBool,
}

impl Shutdown for ListenerCloser {
    fn shutdown(&self) {
        self.0.closed.store(true, Ordering::SeqCst);
        self.0.cancel.set();
    }
}

/// 名前付きパイプの受け口（サーバー側）。
///
/// 待っているインスタンスを常に 1 つ持ち、名前を手放さない（別のプロセスに同じ名前で作らせない）。
/// 閉じたあとも、そのインスタンスは受け口を落とすまで残る。
pub struct NamedPipeListener {
    name: Vec<u16>,
    sddl: Vec<u16>,
    /// 次の接続を待つインスタンス
    next: Option<OwnedHandle>,
    connect_event: Event,
    close: Arc<ListenerClose>,
    /// 受け付けた接続が持つ印。数が 1（自分だけ）なら、この名前のインスタンスは `next` のほかにない
    alive: Arc<()>,
    /// つないできた相手の実行ファイルのパスを照らす規則（`require_client`。無ければ照らさない）
    client_rule: Option<ClientRule>,
}

/// 受け口が、つないできた相手の実行ファイルのパス（OS が起動のときに記録したフルパス。8.3 の短い名前は展開済み）を
/// 照らす規則。判断は `peer::verify_peer_path`（OS に依存しない）で、ここは呼ぶだけ。
pub type ClientRule = Box<dyn Fn(&str) -> Result<(), PeerRejection> + Send>;

impl NamedPipeListener {
    /// `name`（`\\.\pipe\…`）で受け口を作る。最初のインスタンスを `FILE_FLAG_FIRST_PIPE_INSTANCE` で作るので、
    /// 同じ名前のパイプがすでにあれば（別のプロセスが先に作っていれば）失敗する（`ERROR_ACCESS_DENIED`）。
    /// DACL は、このプロセスのユーザーと SYSTEM だけに読み書きを許す。
    pub fn bind(name: &str) -> io::Result<NamedPipeListener> {
        let sid = current_user_sid()?;
        let sddl = super::pipe_dacl_sddl(&sid).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "unexpected user SID format")
        })?;
        let mut listener = NamedPipeListener {
            name: to_wide(name)?,
            sddl: to_wide(&sddl)?,
            next: None,
            connect_event: Event::new()?,
            close: Arc::new(ListenerClose {
                cancel: Event::new()?,
                closed: AtomicBool::new(false),
            }),
            alive: Arc::new(()),
            client_rule: None,
        };
        listener.next = Some(listener.create_instance(true)?);
        Ok(listener)
    }

    /// 受け付けるたびに、最初の枠を読む前に、相手の実行ファイルのパスを `rule` で照らす（設定の口の S3。
    /// ADR 0068 §4）。PID やパスが取れないときも断る。断った接続は、何も読まず・書かずに切る。
    pub fn require_client(mut self, rule: ClientRule) -> NamedPipeListener {
        self.client_rule = Some(rule);
        self
    }

    /// このプロセスのユーザーとセッションの名前（`current_pipe_name`）で受け口を作る。
    pub fn bind_current_user() -> io::Result<NamedPipeListener> {
        NamedPipeListener::bind(&current_pipe_name()?)
    }

    /// 受け口を閉じる手段。
    pub fn closer(&self) -> ListenerCloser {
        ListenerCloser(Arc::clone(&self.close))
    }

    fn has_live_connections(&self) -> bool {
        Arc::strong_count(&self.alive) > 1
    }

    /// インスタンスを 1 つ作る。`first` なら `FILE_FLAG_FIRST_PIPE_INSTANCE` を付ける（同じ名前のインスタンスが
    /// 1 つでもあれば失敗する）。
    fn create_instance(&self, first: bool) -> io::Result<OwnedHandle> {
        let sd = SecurityDescriptor::from_sddl(&self.sddl)?;
        let attrs = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.as_ptr(),
            bInheritHandle: FALSE,
        };
        let open_mode = if first {
            SERVER_OPEN_MODE | FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            SERVER_OPEN_MODE
        };
        // SAFETY: self.name は NUL で終わる UTF-16。attrs と、それが指す sd は、この呼び出しの間生きている
        // （OS は中身を写すので、呼び出しのあとは要らない）。返り値は own_handle が確かめて所有する。
        let raw = unsafe {
            CreateNamedPipeW(
                self.name.as_ptr(),
                open_mode,
                PIPE_MODE,
                MAX_INSTANCES,
                PIPE_BUFFER_BYTES,
                PIPE_BUFFER_BYTES,
                PIPE_DEFAULT_TIMEOUT_MS,
                &attrs,
            )
        };
        own_handle(raw)
    }
}

impl Listener for NamedPipeListener {
    type Conn = NamedPipeDuplex;

    fn accept(&mut self) -> io::Result<Option<NamedPipeDuplex>> {
        loop {
            match self.accept_one()? {
                Accepted::Conn(conn) => return Ok(Some(conn)),
                Accepted::Stopped => return Ok(None),
                // 断った。次を待つ（誤りにしない。続けて誤りが出ると、受け付けを諦めるため）
                Accepted::Refused => {}
            }
        }
    }
}

/// `accept_one` の結果
enum Accepted {
    Conn(NamedPipeDuplex),
    Stopped,
    Refused,
}

impl NamedPipeListener {
    /// 1 回の受け付け。高い権限の相手は、何も読まずに切る（ADR 0068 §2 の 5）。
    fn accept_one(&mut self) -> io::Result<Accepted> {
        if self.close.closed.load(Ordering::SeqCst) {
            return Ok(Accepted::Stopped);
        }
        let instance = match self.next.take() {
            Some(handle) => handle,
            // 前の受け付けで次のインスタンスを作れなかった。ほかに生きたインスタンスがなければ、名前が空いて
            // いるので、FIRST_PIPE_INSTANCE で作り直す（その間に別のプロセスが作っていたら失敗にする）
            None => self.create_instance(!self.has_live_connections())?,
        };
        let handle = instance.as_raw_handle();
        // SAFETY: handle は開いたサーバーのインスタンス。ov は run_overlapped が渡すもの。
        let start = |ov| unsafe { ConnectNamedPipe(handle, ov) };
        // SAFETY: handle は instance が持っていて、この呼び出しの間閉じない。バッファは渡さない。
        // connect_event はこの受け付け専用（&mut self で直列）。
        let connected = unsafe {
            run_overlapped(
                handle,
                &self.connect_event,
                &self.close.cancel,
                NO_DEADLINE_MS,
                start,
            )
        };
        match connected {
            Ok(Completion::Done(_)) => {}
            // CreateNamedPipeW と ConnectNamedPipe の間に相手がつないだ。つながっている
            Err(e) if is_code(&e, ERROR_PIPE_CONNECTED) => {}
            Ok(Completion::Cancelled | Completion::TimedOut) => {
                self.next = Some(instance);
                return Ok(Accepted::Stopped);
            }
            Err(e) if is_code(&e, ERROR_NO_DATA) => {
                // 相手がつないですぐ閉じた。切り離して、このインスタンスを次に使う（名前を手放さない）。
                // SAFETY: handle は開いたサーバーのインスタンス。失敗しても、次の ConnectNamedPipe が誤りで知らせる
                unsafe { DisconnectNamedPipe(handle) };
                self.next = Some(instance);
                return Err(e);
            }
            Err(e) => return Err(e),
        }
        if self.close.closed.load(Ordering::SeqCst) {
            return Ok(Accepted::Stopped);
        }
        // 相手のプロセスは 1 回だけ開き、権限とパスの両方をそのハンドルで見る（PID の使い回しで、別々のプロセスを
        // 見ないため。ADR 0068 §10）
        let client = open_client_process(&instance);
        let refusal = match check_client(client.as_ref()) {
            Err(refusal) => Some(refusal.as_str()),
            Ok(()) => self
                .client_rule
                .as_ref()
                .and_then(|rule| check_client_path(client.as_ref(), rule).err())
                .map(PeerRejection::as_str),
        };
        drop(client);
        if let Some(reason) = refusal {
            // 理由の語だけを書く（番号やパスは出さない）
            eprintln!("ikari-pipe: refused a connection ({reason})");
            // SAFETY: handle は開いたサーバーのインスタンス。失敗しても、次の ConnectNamedPipe が誤りで知らせる
            unsafe { DisconnectNamedPipe(handle) };
            self.next = Some(instance);
            return Ok(Accepted::Refused);
        }
        // 渡す前に次のインスタンスを作り、名前を手放さない。作れなければ、この接続も渡さない（相手には切断に
        // 見える）。次の accept が作り直す
        self.next = Some(self.create_instance(false)?);
        NamedPipeDuplex::new(instance, Some(Arc::clone(&self.alive))).map(Accepted::Conn)
    }
}

/// つないできた相手（クライアント）のプロセス。PID が取れなければ `NoPeerPid`、開けなければ `NoPeerPath`。
fn open_client_process(instance: &OwnedHandle) -> Result<OwnedHandle, PeerRejection> {
    let mut pid = 0u32;
    // SAFETY: instance は接続済みのパイプのサーバー側。pid は書ける u32 で、この呼び出しの間生きている。
    let ok = unsafe { GetNamedPipeClientProcessId(instance.as_raw_handle(), &mut pid) };
    if ok == FALSE {
        return Err(PeerRejection::NoPeerPid);
    }
    open_process(pid).ok_or(PeerRejection::NoPeerPath)
}

/// `pid` のプロセスを、情報を読むだけの権利で開く。開けなければ `None`。
fn open_process(pid: u32) -> Option<OwnedHandle> {
    // SAFETY: 引数は値だけ。返り値は own_handle が確かめて所有する（開けなければ None）。
    own_handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) }).ok()
}

/// つないできた相手（クライアント）のプロセスが、高い権限でないかを確かめる。取れなければ断る。
fn check_client(client: Result<&OwnedHandle, &PeerRejection>) -> Result<(), Refusal> {
    let peer_rid = client
        .ok()
        .and_then(|process| integrity_rid_of(process.as_raw_handle()));
    elevation::check_peer(current_integrity_rid(), peer_rid)
}

/// つないできた相手（クライアント）の実行ファイルのパスを `rule` で照らす。呼ぶ側が相手のプロセスのハンドルを
/// 開いたまま、そこからパスを取って比べる（PID の使い回しで、別のプロセスのパスを見ないため。ADR 0068 §10）。
fn check_client_path(
    client: Result<&OwnedHandle, &PeerRejection>,
    rule: &ClientRule,
) -> Result<(), PeerRejection> {
    let process = client.map_err(|reason| *reason)?;
    let path = image_path_of(process).ok_or(PeerRejection::NoPeerPath)?;
    rule(&long_path_name(&path))
}

/// パスを取るときの最初のバッファの長さ（UTF-16 の単位。ふつうのパスはこれに収まる）
const INITIAL_PATH_LEN: usize = 260;
/// パスの長さの上限（UTF-16 の単位。Windows の長いパスの上限と NUL）
const MAX_LONG_PATH: usize = 32_768;

/// プロセス `pid` の実行ファイルのフルパス（Win32 の形。OS が起動のときに記録したもの。8.3 の短い名前は展開する）。
/// 取れなければ `None`。設定画面の核が、設定の口の相手（engine-host）を照らすときに使う。
pub fn process_image_path(pid: u32) -> Option<String> {
    let process = open_process(pid)?;
    image_path_of(&process).map(|path| long_path_name(&path))
}

/// 開いたプロセスの実行ファイルのフルパス
fn image_path_of(process: &OwnedHandle) -> Option<String> {
    let mut buffer = vec![0u16; INITIAL_PATH_LEN];
    loop {
        let mut len = u32::try_from(buffer.len()).ok()?;
        // SAFETY: process は開いたプロセスのハンドル（呼ぶ側が生かしている）。buffer は len 個の u16 を書ける。
        // 成功すると len に NUL を除いた長さが入る。
        let ok = unsafe {
            QueryFullProcessImageNameW(
                process.as_raw_handle(),
                PROCESS_NAME_WIN32,
                buffer.as_mut_ptr(),
                &mut len,
            )
        };
        if ok != FALSE {
            let len = usize::try_from(len).ok()?.min(buffer.len());
            return String::from_utf16(&buffer[..len]).ok();
        }
        let err = io::Error::last_os_error();
        if !is_code(&err, ERROR_INSUFFICIENT_BUFFER) || buffer.len() >= MAX_LONG_PATH {
            return None;
        }
        buffer.resize((buffer.len() * 2).min(MAX_LONG_PATH), 0);
    }
}

/// 8.3 の短い名前を長い名前に直す（`GetLongPathNameW`）。直せなければそのまま返す（そのままでは期待のパスと
/// 一致しないので、照合は拒む側に倒れる）。各段の名前を同じフォルダの中の長い名前に置き換えるだけで、リンクはたどらない。
pub fn long_path_name(path: &str) -> String {
    let Ok(wide) = to_wide(path) else {
        return path.to_owned();
    };
    let mut buffer = vec![0u16; INITIAL_PATH_LEN];
    loop {
        let Ok(capacity) = u32::try_from(buffer.len()) else {
            return path.to_owned();
        };
        // SAFETY: wide は NUL で終わる UTF-16 で、この呼び出しの間生きている。buffer は capacity 個まで書ける。
        let written =
            unsafe { GetLongPathNameW(wide.as_ptr(), buffer.as_mut_ptr(), capacity) } as usize;
        if written == 0 {
            return path.to_owned();
        }
        if written < buffer.len() {
            return String::from_utf16(&buffer[..written]).unwrap_or_else(|_| path.to_owned());
        }
        // 足りないときは、NUL を含めた要る長さが返る
        if written > MAX_LONG_PATH {
            return path.to_owned();
        }
        buffer.resize(written, 0);
    }
}

/// このプロセスのトークンの整合性のレベル（`SECURITY_MANDATORY_*_RID`）。取れなければ `None`。
pub fn current_integrity_rid() -> Option<u32> {
    // SAFETY: GetCurrentProcess は引数を取らず、閉じなくてよい疑似ハンドルを返す。
    integrity_rid_of(unsafe { GetCurrentProcess() })
}

/// `process` のトークンの整合性のレベル。トークンを開けない（高い権限のプロセスは、ふつうの権限からは開けない）、
/// 読めないときは `None`。
fn integrity_rid_of(process: HANDLE) -> Option<u32> {
    let mut raw_token: HANDLE = ptr::null_mut();
    // SAFETY: process は有効なプロセスのハンドル（呼ぶ側が生かしている）。raw_token には開いたトークンが書かれる
    // （成功したときだけ。所有は下の own_handle）。
    let ok = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw_token) };
    if ok == FALSE {
        return None;
    }
    let token = own_handle(raw_token).ok()?;

    let mut needed = 0u32;
    // SAFETY: 大きさを聞くだけ（バッファは null、長さ 0）。失敗（ERROR_INSUFFICIENT_BUFFER）で needed が埋まる。
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenIntegrityLevel,
            ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    if (needed as usize) < size_of::<TOKEN_MANDATORY_LABEL>() {
        return None;
    }
    // TOKEN_MANDATORY_LABEL はポインタを含むので、ポインタの境界に揃ったバッファ（usize の並び）で受ける
    let mut buf = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
    // SAFETY: buf は needed バイト以上あり、usize（ポインタ）の境界に揃っていて、この呼び出しの間生きている。
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenIntegrityLevel,
            buf.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    };
    if ok == FALSE {
        return None;
    }
    // SAFETY: 成功したので、buf の先頭は TOKEN_MANDATORY_LABEL（揃いと大きさは上で確かめた）。Sid は buf の
    // 中を指し、buf は下で使い終わるまで生きている。
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid };
    if sid.is_null() {
        return None;
    }
    // SAFETY: sid は上の有効な SID。返りは SID の中を指す（解放しない）。
    let count = unsafe { *GetSidSubAuthorityCount(sid) };
    if count == 0 {
        return None;
    }
    // SAFETY: 添字は count - 1（0 以上 count 未満）。返りは SID の中を指す（解放しない）。
    Some(unsafe { *GetSidSubAuthority(sid, u32::from(count) - 1) })
}

/// `name`（`\\.\pipe\…`）につなぐ。相手の PID が取れたら `Client::server_pid()` に入る。
///
/// エンジンが動いていない（`ERROR_FILE_NOT_FOUND`）ときは、すぐ誤りで戻る（起動するかは使う側が決める）。
/// DACL で拒まれたら `ERROR_ACCESS_DENIED`。パイプの持ち主が自分と違う・確かめられないときは、何も読み書きせずに
/// 閉じて `PermissionDenied`（理由は `owner_rejection` で取れる）。
pub fn connect(name: &str) -> io::Result<Client> {
    let (duplex, server_pid) = connect_duplex(name)?;
    Ok(Client::new(duplex, server_pid))
}

/// `connect` の、流れと相手の PID を返す形（`Client` を作らない）。
pub fn connect_duplex(name: &str) -> io::Result<(NamedPipeDuplex, Option<u32>)> {
    let wide = to_wide(name)?;
    let mut busy_retries = 0u32;
    let handle = loop {
        // SAFETY: wide は NUL で終わる UTF-16 で、この呼び出しの間生きている。セキュリティ属性とテンプレートは
        // null（使わない）。返り値は own_handle が確かめて所有する。
        let raw = unsafe {
            CreateFileW(
                wide.as_ptr(),
                CLIENT_ACCESS,
                FILE_SHARE_NONE,
                ptr::null(),
                OPEN_EXISTING,
                CLIENT_FLAGS,
                ptr::null_mut(),
            )
        };
        match own_handle(raw) {
            Ok(handle) => break handle,
            Err(e) if is_code(&e, ERROR_PIPE_BUSY) && busy_retries < CONNECT_BUSY_RETRIES => {
                busy_retries += 1;
                // SAFETY: wide は NUL で終わる UTF-16。待ちの失敗（時間切れ、パイプが消えた）は、次の
                // CreateFileW が誤りで知らせるので、ここでは見ない。
                unsafe { WaitNamedPipeW(wide.as_ptr(), CONNECT_BUSY_WAIT_MS) };
            }
            Err(e) => return Err(e),
        }
    };
    let server_pid = server_process_id(&handle);
    if let Err(reason) = check_server_owner(&handle, server_pid) {
        // 何も読み書きせずに閉じる（相手には切断に見える）
        drop(handle);
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, reason));
    }
    Ok((NamedPipeDuplex::new(handle, None)?, server_pid))
}

/// `connect` と `connect_duplex` が、パイプの持ち主の確認で断ったときの理由。ほかの誤りは `None`。
pub fn owner_rejection(e: &io::Error) -> Option<PeerRejection> {
    e.get_ref()?.downcast_ref::<PeerRejection>().copied()
}

/// つないだパイプの持ち主（受ける側のプロセスのトークンの利用者の SID と、セッション番号）が、自分と同じかを
/// 確かめる（ADR 0068 §2 の 2）。判断は `peer::verify_pipe_owner` で、ここは OS から取るだけ。
///
/// 比べるのはトークンの「ユーザー」の SID なので、UAC で昇格したつなぐ側と、昇格していない受ける側の組み合わせでも
/// 同じになる。相手のトークンを開けないとき（別の利用者のプロセスなど）は、取れないとして断る。
fn check_server_owner(handle: &OwnedHandle, server_pid: Option<u32>) -> Result<(), PeerRejection> {
    let own_sid = current_user_sid().ok();
    let own = SessionUser {
        user_sid: own_sid.as_deref(),
        session: current_session_id().ok(),
    };
    let server_sid = server_pid
        .and_then(open_process)
        .and_then(|process| user_sid_of(process.as_raw_handle()).ok());
    let server = SessionUser {
        user_sid: server_sid.as_deref(),
        session: server_session_id(handle),
    };
    verify_pipe_owner(own, server)
}

/// 相手（サーバー）のセッション番号。取れなければ `None`。
fn server_session_id(handle: &OwnedHandle) -> Option<u32> {
    let mut session = 0u32;
    // SAFETY: handle は開いたパイプのクライアント側。session は書ける u32 で、この呼び出しの間生きている。
    let ok = unsafe { GetNamedPipeServerSessionId(handle.as_raw_handle(), &mut session) };
    (ok != FALSE).then_some(session)
}

/// 相手（サーバー）のプロセスの番号。取れなければ `None`（相手を確かめられないので、使う側が判断する）。
fn server_process_id(handle: &OwnedHandle) -> Option<u32> {
    let mut pid = 0u32;
    // SAFETY: handle は開いたパイプのクライアント側。pid は書ける u32 で、この呼び出しの間生きている。
    let ok = unsafe { GetNamedPipeServerProcessId(handle.as_raw_handle(), &mut pid) };
    (ok != FALSE).then_some(pid)
}

/// このプロセスのユーザーの SID（`S-1-5-21-…`）。
pub fn current_user_sid() -> io::Result<String> {
    // SAFETY: GetCurrentProcess は引数を取らず、閉じなくてよい疑似ハンドルを返す。
    user_sid_of(unsafe { GetCurrentProcess() })
}

/// `process` のトークンのユーザーの SID の文字列。トークンを開けない・読めないときは誤り。
fn user_sid_of(process: HANDLE) -> io::Result<String> {
    let mut raw_token: HANDLE = ptr::null_mut();
    // SAFETY: process は有効なプロセスのハンドル（呼ぶ側が生かしている。疑似ハンドルでもよい）。raw_token には
    // 開いたトークンが書かれる（成功したときだけ。所有は下の own_handle）。
    let ok = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw_token) };
    if ok == FALSE {
        return Err(io::Error::last_os_error());
    }
    let token = own_handle(raw_token)?;

    let mut needed = 0u32;
    // SAFETY: 大きさを聞くだけ（バッファは null、長さ 0）。失敗（ERROR_INSUFFICIENT_BUFFER）で needed が埋まる。
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    if (needed as usize) < size_of::<TOKEN_USER>() {
        return Err(io::Error::last_os_error());
    }
    // TOKEN_USER はポインタを含むので、ポインタの境界に揃ったバッファ（usize の並び）で受ける
    let mut buf = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
    // SAFETY: buf は needed バイト以上あり、usize（ポインタ）の境界に揃っていて、この呼び出しの間生きている。
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buf.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    };
    if ok == FALSE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 成功したので、buf の先頭は TOKEN_USER（揃いと大きさは上で確かめた）。Sid は buf の中を指し、
    // buf は下で使い終わるまで生きている。
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };

    let mut text: *mut u16 = ptr::null_mut();
    // SAFETY: sid は上の有効な SID（buf が生きている）。text には OS が LocalAlloc した文字列を書く
    // （成功したときだけ。解放は下の LocalMem）。
    let ok = unsafe { ConvertSidToStringSidW(sid, &mut text) };
    if ok == FALSE {
        return Err(io::Error::last_os_error());
    }
    let _free = LocalMem(text.cast());
    let mut len = 0usize;
    // SAFETY: text は NUL で終わる UTF-16（ConvertSidToStringSidW の約束）で、_free が落ちるまで生きている。
    // NUL より先は読まない。
    while unsafe { *text.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: text から len 個は上で読めた範囲で、_free が落ちるまで生きていて、ほかから書かれない。
    let units = unsafe { std::slice::from_raw_parts(text, len) };
    String::from_utf16(units)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "SID is not valid UTF-16"))
}

/// このプロセスの Windows のセッション番号。
pub fn current_session_id() -> io::Result<u32> {
    let mut session = 0u32;
    // SAFETY: GetCurrentProcessId は引数を取らない。session は書ける u32 で、この呼び出しの間生きている。
    let ok = unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) };
    if ok == FALSE {
        return Err(io::Error::last_os_error());
    }
    Ok(session)
}

/// このプロセスのユーザーとセッションのパイプの名前（`pipe_name`）。
pub fn current_pipe_name() -> io::Result<String> {
    pipe_name(&current_user_sid()?, current_session_id()?)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "unexpected user SID format"))
}

// Windows の CI（W1-d）か実機で流れる。Mac では流れない。
#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ClientError;
    use crate::server::Server;
    use std::sync::atomic::AtomicU32;
    use std::thread;
    use std::time::Duration;
    use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
    };
    use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;

    static NEXT_NAME: AtomicU32 = AtomicU32::new(0);

    /// テストごとに別の名前（動いているエンジンの名前とぶつけない）
    fn unique_name() -> String {
        format!(
            r"\\.\pipe\ikari-ime-test-{}-{}",
            std::process::id(),
            NEXT_NAME.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn long() -> Duration {
        ikari_testtime::ms(10_000)
    }

    /// 本体をそのまま返すサーバー。閉じる手段と、受け付けのスレッドを返す。
    fn echo_server(name: &str) -> (ListenerCloser, thread::JoinHandle<io::Result<()>>) {
        let listener = NamedPipeListener::bind(name).unwrap();
        let closer = listener.closer();
        let thread = thread::spawn(move || {
            Server::new(listener).serve(|| |body: &[u8]| Some(body.to_vec()))
        });
        (closer, thread)
    }

    /// 受け付けを 1 回だけ行うスレッド。
    fn accept_once(name: &str) -> thread::JoinHandle<NamedPipeDuplex> {
        let mut listener = NamedPipeListener::bind(name).unwrap();
        thread::spawn(move || listener.accept().unwrap().unwrap())
    }

    #[test]
    fn round_trip_and_server_pid() {
        let name = unique_name();
        let (closer, server) = echo_server(&name);
        let client = connect(&name).unwrap();
        assert_eq!(client.request(b"ping", long()).unwrap(), b"ping");
        assert_eq!(client.server_pid(), Some(std::process::id()));
        // 2 本目（受け付けのあと次のインスタンスがあること）
        let second = connect(&name).unwrap();
        assert_eq!(second.request(b"pong", long()).unwrap(), b"pong");
        drop(client);
        drop(second);
        closer.shutdown();
        server.join().unwrap().unwrap();
    }

    #[test]
    fn concurrent_requests_on_one_connection() {
        let name = unique_name();
        let (closer, server) = echo_server(&name);
        let client = Arc::new(connect(&name).unwrap());
        let workers: Vec<_> = (0..8u8)
            .map(|i| {
                let client = Arc::clone(&client);
                thread::spawn(move || {
                    let body = vec![i; 1000 + usize::from(i)];
                    assert_eq!(client.request(&body, long()).unwrap(), body);
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        drop(client);
        closer.shutdown();
        server.join().unwrap().unwrap();
    }

    #[test]
    fn first_instance_rejects_a_second_listener_with_the_same_name() {
        let name = unique_name();
        let _first = NamedPipeListener::bind(&name).unwrap();
        let err = NamedPipeListener::bind(&name).err().unwrap();
        assert!(is_code(&err, ERROR_ACCESS_DENIED), "{err:?}");
    }

    #[test]
    fn rejects_remote_clients() {
        assert_ne!(PIPE_MODE & PIPE_REJECT_REMOTE_CLIENTS, 0);
        let name = unique_name();
        let _listener = NamedPipeListener::bind(&name).unwrap();
        // 同じ機械でも、`\\localhost\pipe\…` は SMB を通るのでリモートとして扱われ、拒まれる。
        // （SMB のサーバーが止まっている機械でも、開けないことに変わりはない）
        let remote = name.replacen(r"\\.\", r"\\localhost\", 1);
        assert!(connect_duplex(&remote).is_err());
    }

    #[test]
    fn dacl_allows_only_the_user_and_system() {
        let name = unique_name();
        let server = accept_once(&name);
        let (client, _) = connect_duplex(&name).unwrap();
        let _server_side = server.join().unwrap();

        let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: クライアントのハンドルは GENERIC_READ（READ_CONTROL を含む）で開いている。sd には OS が
        // LocalAlloc した記述子を書く（解放は下の LocalMem）。所有者などは要らないので null。
        let status = unsafe {
            GetSecurityInfo(
                client.slot.handle.as_raw_handle(),
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut sd,
            )
        };
        assert_eq!(status, 0);
        let _sd = LocalMem(sd);
        let mut text: *mut u16 = ptr::null_mut();
        // SAFETY: sd は上の有効な記述子。text には OS が LocalAlloc した文字列を書く（解放は下の LocalMem）。
        let ok = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                sd,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                ptr::null_mut(),
            )
        };
        assert_ne!(ok, FALSE);
        let _text = LocalMem(text.cast());
        let mut len = 0usize;
        // SAFETY: text は NUL で終わる UTF-16 で、_text が落ちるまで生きている。
        while unsafe { *text.add(len) } != 0 {
            len += 1;
        }
        // SAFETY: text から len 個は上で読めた範囲。
        let sddl = String::from_utf16(unsafe { std::slice::from_raw_parts(text, len) }).unwrap();

        let sid = current_user_sid().unwrap();
        assert!(sddl.starts_with("D:P"), "{sddl}");
        assert_eq!(sddl.matches('(').count(), 2, "{sddl}");
        // SDDL の文字列化は、よく知られた SID を別名に省略する。組み込みの管理者（RID 500。GitHub の
        // ランナーのユーザーがこれ）は `LA`。SID のままの形か、RID 500 のときの `LA` のどちらかを許す
        let is_builtin_admin = sid.ends_with("-500");
        assert!(
            sddl.contains(&format!(";;;{sid})")) || (is_builtin_admin && sddl.contains(";;;LA)")),
            "{sddl}"
        );
        assert!(sddl.contains(";;;SY)"), "{sddl}");
    }

    #[test]
    fn server_closing_disconnects_the_client() {
        let name = unique_name();
        let server = accept_once(&name);
        let client = connect(&name).unwrap();
        let (_reader, _writer, shutdown) = server.join().unwrap().split();
        shutdown.shutdown();
        assert_eq!(
            client.request(b"x", long()).unwrap_err(),
            ClientError::Disconnected
        );
    }

    #[test]
    fn peer_drop_gives_eof() {
        let name = unique_name();
        let server = accept_once(&name);
        let (client, _) = connect_duplex(&name).unwrap();
        let (mut reader, _writer, _shutdown) = server.join().unwrap().split();
        let (_client_reader, mut client_writer, client_shutdown) = client.split();
        client_writer.write_all(b"xy").unwrap();
        drop(client_writer);
        client_shutdown.shutdown();
        let mut all = Vec::new();
        reader.read_to_end(&mut all).unwrap();
        assert_eq!(all, b"xy");
    }

    /// 相手のパスの規則（設定の口の S3）: 断った接続は切られ、受け口は次の接続を待ち続ける。規則が受けるのは、
    /// 相手（ここでは同じテストの exe）のフルパス
    #[test]
    fn require_client_refuses_by_the_peer_path_and_keeps_listening() {
        let name = unique_name();
        let own = long_path_name(&std::env::current_exe().unwrap().to_string_lossy());
        let calls = Arc::new(AtomicU32::new(0));
        let seen = Arc::clone(&calls);
        let expected = own.clone();
        let mut listener = NamedPipeListener::bind(&name)
            .unwrap()
            .require_client(Box::new(move |peer| {
                assert!(peer.eq_ignore_ascii_case(&expected), "{peer} vs {expected}");
                // 最初の 1 本は断り、次は通す
                if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(PeerRejection::NotThePeer)
                } else {
                    Ok(())
                }
            }));
        let server = thread::spawn(move || listener.accept().unwrap().unwrap());
        let (refused, _) = connect_duplex(&name).unwrap();
        let (mut refused_reader, _w, _s) = refused.split();
        let mut rest = Vec::new();
        // 切られたので、何も来ずに終わる（EOF か切断の誤り）
        let _ = refused_reader.read_to_end(&mut rest);
        assert!(rest.is_empty());
        let (accepted, _) = connect_duplex(&name).unwrap();
        let (mut reader, _writer, _shutdown) = server.join().unwrap().split();
        let (_client_reader, mut client_writer, client_shutdown) = accepted.split();
        client_writer.write_all(b"ok").unwrap();
        drop(client_writer);
        client_shutdown.shutdown();
        let mut all = Vec::new();
        reader.read_to_end(&mut all).unwrap();
        assert_eq!(all, b"ok");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn process_image_path_of_self_is_the_current_exe() {
        let own = long_path_name(&std::env::current_exe().unwrap().to_string_lossy());
        let path = process_image_path(std::process::id()).unwrap();
        assert!(path.eq_ignore_ascii_case(&own), "{path} vs {own}");
    }

    #[test]
    fn shutdown_wakes_a_blocked_reader_and_fails_writes() {
        let name = unique_name();
        let server = accept_once(&name);
        let (client, _) = connect_duplex(&name).unwrap();
        let _server_side = server.join().unwrap();
        let (mut reader, mut writer, shutdown) = client.split();
        let blocked = thread::spawn(move || {
            let mut buf = [0u8; 1];
            reader.read(&mut buf).unwrap()
        });
        // 読みが待ちに入るのを少し待つ（入る前に閉じても、閉じる指示のイベントで同じく起きる）
        thread::sleep(ikari_testtime::ms(100));
        shutdown.shutdown();
        assert_eq!(blocked.join().unwrap(), 0);
        assert_eq!(
            writer.write(b"x").unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        // 何度呼んでもよい
        shutdown.shutdown();
    }

    #[test]
    fn closer_ends_a_waiting_accept() {
        let name = unique_name();
        let mut listener = NamedPipeListener::bind(&name).unwrap();
        let closer = listener.closer();
        let waiting = thread::spawn(move || listener.accept().map(|c| c.is_none()));
        thread::sleep(ikari_testtime::ms(100));
        closer.shutdown();
        assert!(waiting.join().unwrap().unwrap());
    }

    #[test]
    fn write_gives_up_at_the_deadline_when_the_peer_never_reads() {
        let name = unique_name();
        let server = accept_once(&name);
        let (client, _) = connect_duplex(&name).unwrap();
        // 相手はつないだまま読まない（固まったエンジンの役）
        let _server_side = server.join().unwrap();
        let (_reader, mut writer, _shutdown) = client.split();
        let chunk = vec![0u8; PIPE_BUFFER_BYTES as usize];
        let started = std::time::Instant::now();
        // バッファが埋まるまでは進む。埋まったら 1 回の書きが締め切りで TimedOut になる
        let err = loop {
            if let Err(e) = writer.write(&chunk) {
                break e;
            }
            assert!(started.elapsed() < long(), "write never blocked");
        };
        assert_eq!(err.kind(), io::ErrorKind::TimedOut, "{err:?}");
        // 下限は締め切りの半分にとどめる。Windows の待ちはタイマーの粒度（約 15.6ms）で
        // 締め切りより少し早く戻ることがあり、厳密に比べると揺れる（#427）。
        // 見たいのは「待たずに即失敗しない」ことと「締め切りの数倍以内で返る」こと
        // （上は、書きが固まらないことをループ内の long() の検査が受け持つ）
        assert!(started.elapsed() >= WRITE_DEADLINE / 2);
    }

    #[test]
    fn missing_pipe_fails_fast() {
        let err = connect_duplex(&unique_name()).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::NotFound, "{err:?}");
    }

    /// 同じ利用者・同じセッションの受け口には、持ち主の確認を通ってつながる（自分のプロセスどうし）
    #[test]
    fn the_owner_of_an_own_pipe_is_accepted() {
        let name = unique_name();
        let server = accept_once(&name);
        let (client, pid) = connect_duplex(&name).unwrap();
        let _server_side = server.join().unwrap();
        assert_eq!(pid, Some(std::process::id()));
        assert_eq!(
            check_server_owner(&client.slot.handle, pid),
            Ok(()),
            "same user and session"
        );
        // 相手の PID が取れなければ、持ち主を確かめられないので断る
        assert_eq!(
            check_server_owner(&client.slot.handle, None),
            Err(PeerRejection::NoPeerOwner)
        );
    }

    #[test]
    fn owner_rejection_reads_only_the_owner_refusal() {
        let refused = io::Error::new(io::ErrorKind::PermissionDenied, PeerRejection::OtherUser);
        assert_eq!(owner_rejection(&refused), Some(PeerRejection::OtherUser));
        assert_eq!(
            owner_rejection(&io::Error::from(io::ErrorKind::NotFound)),
            None
        );
    }

    #[test]
    fn current_user_names() {
        let sid = current_user_sid().unwrap();
        assert!(sid.starts_with("S-1-"), "{sid}");
        let session = current_session_id().unwrap();
        let name = current_pipe_name().unwrap();
        assert!(name.ends_with(&format!("{sid}-s{session}")), "{name}");
    }

    #[test]
    fn names_with_nul_are_rejected() {
        assert_eq!(
            to_wide("a\0b").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
