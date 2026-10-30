//! 殻の 1 本の口（ADR 0041 §4.4。ヘッダは `include/ikari_shell.h` の `ikari_shell_*`）。
//!
//! - `IkariShell *` は指す先のない番号（1 から増やし、使い回さない）。表（`SHELLS`）で `Slot` を引くので、
//!   free のあとの使用と二重の free は `IKARI_SHELL_ERR_INVALID_HANDLE` と「何もしない」になる。
//! - `Shell` は `Send` だが `Sync` ではないので `Mutex` に入れる。dispatch は `try_lock` で取り、
//!   コールバックの中からの呼び直しは `BUSY` にする（待つと自分を待って止まる）。
//! - 落ちた通知（`ikari_shell_engine_down`）は `Shell` の `Mutex` を通らず、`Arc<EngineLink>` を直接呼ぶ
//!   （`EngineLink` は `Sync`。主スレッドが `request` で待っている間にも届く。§4.3）。
//! - ホストの口（`IkariShellHost`）は `HostTable` に包み、`TextAccess` と `EngineTransport` として `shell-core` に渡す。

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::Duration;

use ikari_shell_core::{
    DownKind, EngineLink, EngineTransport, SelectedText, SettingsSource, Shell, ShellAction,
    ShellConfig, SurroundingText, SystemClock, TextAccess, TransportError, TransportHandle,
};

use crate::{IkariShellBuf, IkariStr, bytes, convert, guard, write_error};

// ---------------------------------------------------------------- 戻り値（ヘッダの IkariShellStatus と一致）

pub const IKARI_SHELL_OK: i32 = 0;
pub const IKARI_SHELL_ERR_NULL: i32 = 1;
pub const IKARI_SHELL_ERR_INVALID_HANDLE: i32 = 2;
pub const IKARI_SHELL_ERR_DECODE: i32 = 3;
pub const IKARI_SHELL_ERR_BUSY: i32 = 4;
pub const IKARI_SHELL_ERR_PANIC: i32 = 5;
pub const IKARI_SHELL_ERR_UNIMPLEMENTED: i32 = 6;

/// ヘッダの `IkariShellTransportResult`
pub const IKARI_SHELL_TRANSPORT_OK: i32 = 0;
pub const IKARI_SHELL_TRANSPORT_UNAVAILABLE: i32 = 1;
pub const IKARI_SHELL_TRANSPORT_TIMEOUT: i32 = 2;

/// ヘッダの `IkariShellDownKind`
pub const IKARI_SHELL_DOWN_INTERRUPTED: u8 = 0;
pub const IKARI_SHELL_DOWN_INVALIDATED: u8 = 1;

/// コールバックの `surrounding` / `selection` が「取れた」と言う値
const HOST_TEXT_OK: i32 = 0;
/// `engine_connect` の「つながらない」
const NO_CONNECTION: u64 = 0;

// ---------------------------------------------------------------- 型（ヘッダと一致）

/// 不透明なハンドル（C 側は `IkariShell *` だけを持つ。指す先はない）。
pub enum IkariShell {}

/// ヘッダの `IkariShellConfig`（借用）。
#[repr(C)]
pub struct IkariShellConfig {
    pub shell_version: IkariStr,
    pub settings_path: IkariStr,
    pub resources_dir: IkariStr,
}

pub type TextFn = unsafe extern "C" fn(ctx: *mut c_void, out: *mut IkariShellBuf) -> i32;
pub type ConnectFn = unsafe extern "C" fn(ctx: *mut c_void) -> u64;
pub type RequestFn = unsafe extern "C" fn(
    ctx: *mut c_void,
    handle: u64,
    bytes: IkariStr,
    timeout_ms: u32,
    reply: *mut IkariShellBuf,
) -> i32;
pub type PostFn = unsafe extern "C" fn(ctx: *mut c_void, handle: u64, bytes: IkariStr);
pub type CloseFn = unsafe extern "C" fn(ctx: *mut c_void, handle: u64);
pub type PidFn = unsafe extern "C" fn(ctx: *mut c_void, handle: u64) -> i32;
pub type ReleaseFn = unsafe extern "C" fn(ctx: *mut c_void);

/// ヘッダの `IkariShellHost`。関数ポインタは NULL を表せるよう `Option` にする（同じ表現）。
#[repr(C)]
pub struct IkariShellHost {
    pub ctx: *mut c_void,
    pub surrounding: Option<TextFn>,
    pub selection: Option<TextFn>,
    pub engine_connect: Option<ConnectFn>,
    pub engine_request: Option<RequestFn>,
    pub engine_post: Option<PostFn>,
    pub engine_close: Option<CloseFn>,
    pub engine_pid: Option<PidFn>,
    pub release: Option<ReleaseFn>,
}

// ---------------------------------------------------------------- ホストの口の包み

/// NULL でないことを確かめたホストの口。落とすときに `release` を 1 回呼ぶ。
struct HostTable {
    ctx: *mut c_void,
    surrounding: TextFn,
    selection: TextFn,
    engine_connect: ConnectFn,
    engine_request: RequestFn,
    engine_post: PostFn,
    engine_close: CloseFn,
    engine_pid: PidFn,
    release: Option<ReleaseFn>,
}

// SAFETY: ヘッダの約束で、engine_* と release はどのスレッドからでも並行に呼べ、ctx はそれに耐える。
// surrounding / selection は dispatch を呼んだスレッドからだけ呼ぶ（`HostText` が dispatch の間だけ借りる）。
unsafe impl Send for HostTable {}
unsafe impl Sync for HostTable {}

impl HostTable {
    /// 必須の口が揃っていなければ、`release` を呼んでから誤りを返す（所有権は成否にかかわらず移る）。
    fn new(host: IkariShellHost) -> Result<Self, &'static str> {
        let IkariShellHost {
            ctx,
            surrounding,
            selection,
            engine_connect,
            engine_request,
            engine_post,
            engine_close,
            engine_pid,
            release,
        } = host;
        match (
            surrounding,
            selection,
            engine_connect,
            engine_request,
            engine_post,
            engine_close,
            engine_pid,
        ) {
            (
                Some(surrounding),
                Some(selection),
                Some(engine_connect),
                Some(engine_request),
                Some(engine_post),
                Some(engine_close),
                Some(engine_pid),
            ) => Ok(Self {
                ctx,
                surrounding,
                selection,
                engine_connect,
                engine_request,
                engine_post,
                engine_close,
                engine_pid,
                release,
            }),
            _ => {
                if let Some(release) = release {
                    // SAFETY: ホストが渡した ctx と release の組。1 回だけ呼ぶ。
                    unsafe { release(ctx) };
                }
                Err("IkariShellHost has a NULL callback")
            }
        }
    }

    /// `f` に出力先を渡して呼び、入れられたバイト列を引き取る（戻り値に関係なく解放する）。
    fn call_with_out(&self, f: impl FnOnce(*mut IkariShellBuf) -> i32) -> (i32, Vec<u8>) {
        let mut out = IkariShellBuf::EMPTY;
        let status = f(&mut out);
        // SAFETY: ヘッダの約束で、*out に入るのは ikari_shell_buf_copy が作ったもの（または空）。ここで 1 回だけ引き取る。
        let taken = unsafe { out.into_vec() };
        (status, taken)
    }

    fn text(&self, f: TextFn) -> Option<Vec<u8>> {
        // SAFETY: ホストの口。ctx と out は約束どおり。
        let (status, out) = self.call_with_out(|out| unsafe { f(self.ctx, out) });
        (status == HOST_TEXT_OK).then_some(out)
    }
}

impl Drop for HostTable {
    fn drop(&mut self) {
        if let Some(release) = self.release {
            // SAFETY: ctx を使い終えた。1 回だけ呼ぶ（`HostTable` は 1 つだけで、`Arc` の最後で落ちる）。
            unsafe { release(self.ctx) };
        }
    }
}

/// `EngineTransport` としてのホストの口（`EngineLink` が持つ）。
struct HostTransport(Arc<HostTable>);

impl EngineTransport for HostTransport {
    fn connect(&self) -> Result<TransportHandle, TransportError> {
        // SAFETY: ホストの口
        match unsafe { (self.0.engine_connect)(self.0.ctx) } {
            NO_CONNECTION => Err(TransportError::Unavailable),
            handle => Ok(TransportHandle(handle)),
        }
    }

    fn request(
        &self,
        handle: TransportHandle,
        bytes: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, TransportError> {
        let timeout_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        let host = &self.0;
        let (status, reply) = host.call_with_out(|reply| {
            // SAFETY: ホストの口。bytes は呼び出しの間だけ借用させる。
            unsafe {
                (host.engine_request)(
                    host.ctx,
                    handle.0,
                    IkariStr::of_bytes(bytes),
                    timeout_ms,
                    reply,
                )
            }
        });
        match status {
            IKARI_SHELL_TRANSPORT_OK => Ok(reply),
            IKARI_SHELL_TRANSPORT_TIMEOUT => Err(TransportError::Timeout),
            _ => Err(TransportError::Unavailable),
        }
    }

    fn post(&self, handle: TransportHandle, bytes: &[u8]) {
        // SAFETY: ホストの口
        unsafe { (self.0.engine_post)(self.0.ctx, handle.0, IkariStr::of_bytes(bytes)) }
    }

    fn close(&self, handle: TransportHandle) {
        // SAFETY: ホストの口
        unsafe { (self.0.engine_close)(self.0.ctx, handle.0) }
    }

    fn pid(&self, handle: TransportHandle) -> Option<i32> {
        // SAFETY: ホストの口
        let pid = unsafe { (self.0.engine_pid)(self.0.ctx, handle.0) };
        (pid > 0).then_some(pid)
    }
}

/// `TextAccess` としてのホストの口（dispatch の間だけ借りる）。返事が読めなければ「取れない」にする。
struct HostText<'a>(&'a HostTable);

impl TextAccess for HostText<'_> {
    fn surrounding(&mut self) -> Option<SurroundingText> {
        let bytes = self.0.text(self.0.surrounding)?;
        convert::decode_surrounding(&bytes).ok()
    }

    fn selection(&mut self) -> Option<SelectedText> {
        let bytes = self.0.text(self.0.selection)?;
        convert::decode_selection(&bytes).ok()
    }
}

// ---------------------------------------------------------------- ハンドルの表

struct Slot {
    shell: Mutex<Shell>,
    /// 落ちた通知を `shell` の `Mutex` を通らずに届けるため
    engine: Arc<EngineLink>,
    host: Arc<HostTable>,
}

static SHELLS: Mutex<BTreeMap<u64, Arc<Slot>>> = Mutex::new(BTreeMap::new());
/// 次の番号。0 は NULL と同じなので使わない。使い回さない（u64 は使い切らない）
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// 表を取る。表の操作は panic しないので、毒が入っても中身はそのまま使える。
fn shells() -> MutexGuard<'static, BTreeMap<u64, Arc<Slot>>> {
    SHELLS.lock().unwrap_or_else(PoisonError::into_inner)
}

fn id_of(s: *const IkariShell) -> u64 {
    s.addr() as u64
}

fn lookup(s: *const IkariShell) -> Option<Arc<Slot>> {
    shells().get(&id_of(s)).cloned()
}

fn config_from(config: &IkariShellConfig) -> Result<ShellConfig, &'static str> {
    let text = |s: &IkariStr, what: &'static str| {
        // SAFETY: 借用の文字列（ヘッダの約束）
        std::str::from_utf8(unsafe { bytes(s.ptr, s.len) }).map_err(|_| what)
    };
    let shell_version = text(&config.shell_version, "shell_version is not valid UTF-8")?;
    let settings_path = text(&config.settings_path, "settings_path is not valid UTF-8")?;
    let resources_dir = text(&config.resources_dir, "resources_dir is not valid UTF-8")?;
    Ok(ShellConfig {
        shell_version: shell_version.to_owned(),
        settings: if settings_path.is_empty() {
            SettingsSource::Inline(String::new())
        } else {
            SettingsSource::File(PathBuf::from(settings_path))
        },
        resources_dir: PathBuf::from(resources_dir),
    })
}

fn new_shell(config: *const IkariShellConfig, host: IkariShellHost) -> Result<u64, String> {
    // 所有権を先に受け取る（config が NULL でも release を呼ぶ）
    let host = Arc::new(HostTable::new(host)?);
    // SAFETY: NULL か、読める IkariShellConfig（ヘッダの約束）
    let config = unsafe { config.as_ref() }.ok_or("config is NULL")?;
    let config = config_from(config)?;
    let shell = Shell::new(
        config,
        Box::new(HostTransport(Arc::clone(&host))),
        Arc::new(SystemClock::new()),
    )
    .map_err(|e| e.to_string())?;
    let slot = Arc::new(Slot {
        engine: Arc::clone(shell.engine()),
        shell: Mutex::new(shell),
        host,
    });
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    shells().insert(id, slot);
    Ok(id)
}

// ---------------------------------------------------------------- 入口

/// # Safety
/// `config` は NULL か読める `IkariShellConfig`。`err` は NULL か書き込めること。`host` はヘッダの約束どおり。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_shell_new(
    config: *const IkariShellConfig,
    host: IkariShellHost,
    err: *mut IkariShellBuf,
) -> *mut IkariShell {
    match catch_unwind(AssertUnwindSafe(|| new_shell(config, host))) {
        // 指す先のない番号（provenance を持たない）
        Ok(Ok(id)) => ptr::without_provenance_mut(id as usize),
        Ok(Err(message)) => {
            write_error(err, &message);
            ptr::null_mut()
        }
        Err(_) => {
            write_error(err, "panic in ikari_shell_new");
            ptr::null_mut()
        }
    }
}

/// 誤りの回に返す動作（キーをアプリに渡す。ADR 0041 §4.2）
fn fallback_actions() -> Vec<u8> {
    convert::encode_actions(&[ShellAction::PassThrough])
}

fn dispatch(s: *const IkariShell, event: &[u8]) -> (i32, Vec<u8>) {
    let Some(slot) = lookup(s) else {
        return (IKARI_SHELL_ERR_INVALID_HANDLE, fallback_actions());
    };
    let Ok(event) = convert::decode_event(event) else {
        return (IKARI_SHELL_ERR_DECODE, fallback_actions());
    };
    // 錠は catch_unwind の中で取る（panic したら錠に毒が入り、以後の dispatch も PANIC を返す）
    let ran = catch_unwind(AssertUnwindSafe(|| {
        let mut shell = match slot.shell.try_lock() {
            Ok(shell) => shell,
            Err(TryLockError::WouldBlock) => return Err(IKARI_SHELL_ERR_BUSY),
            Err(TryLockError::Poisoned(_)) => return Err(IKARI_SHELL_ERR_PANIC),
        };
        #[cfg(test)]
        tests::maybe_panic();
        Ok(shell.dispatch(event, &mut HostText(&slot.host)))
    }));
    match ran {
        Ok(Ok(actions)) => (IKARI_SHELL_OK, convert::encode_actions(&actions)),
        Ok(Err(status)) => (status, fallback_actions()),
        Err(_) => (IKARI_SHELL_ERR_PANIC, fallback_actions()),
    }
}

/// # Safety
/// `event` は借用の文字列（読める `len` バイト、または NULL）。`actions` は NULL か書き込めること。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_shell_dispatch(
    s: *mut IkariShell,
    event: IkariStr,
    actions: *mut IkariShellBuf,
) -> i32 {
    if actions.is_null() {
        return IKARI_SHELL_ERR_NULL;
    }
    // SAFETY: 借用の文字列（ヘッダの約束）
    let event = unsafe { bytes(event.ptr, event.len) };
    let (status, encoded) = guard((IKARI_SHELL_ERR_PANIC, Vec::new()), || dispatch(s, event));
    // SAFETY: actions は書き込める（上で NULL を除いた。ヘッダの約束）
    unsafe { actions.write(IkariShellBuf::from_vec(encoded)) };
    status
}

/// **どのスレッドからでも呼べる**（ADR 0041 §4.3）。
///
/// # Safety
/// 引数はどの値でもよい（`s` は番号として引くだけで、参照しない）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_shell_engine_down(s: *mut IkariShell, generation: u64, kind: u8) {
    guard((), || {
        let kind = match kind {
            IKARI_SHELL_DOWN_INTERRUPTED => DownKind::Interrupted,
            IKARI_SHELL_DOWN_INVALIDATED => DownKind::Invalidated,
            _ => return,
        };
        // 表の錠は番号を引く間だけ持つ（notify_down の間は持たない）
        let Some(engine) = lookup(s).map(|slot| Arc::clone(&slot.engine)) else {
            return;
        };
        engine.notify_down(TransportHandle(generation), kind);
    });
}

/// # Safety
/// 引数はどの値でもよい（NULL、解放ずみ、知らない番号は何もしない）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_shell_free(s: *mut IkariShell) {
    guard((), || {
        // 表から外すだけ。中身は、dispatch や engine_down が持つ最後の Arc が落ちたときに片付く
        let removed = shells().remove(&id_of(s));
        drop(removed);
    });
}

/// # Safety
/// `bytes` は借用の文字列（読める `len` バイト、または NULL）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_shell_buf_copy(bytes: IkariStr) -> IkariShellBuf {
    // SAFETY: 借用の文字列（ヘッダの約束）
    let raw = unsafe { crate::bytes(bytes.ptr, bytes.len) };
    guard(IkariShellBuf::EMPTY, || {
        IkariShellBuf::from_vec(raw.to_vec())
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::Cell;

    use super::*;

    thread_local! {
        static PANIC_IN_DISPATCH: Cell<bool> = const { Cell::new(false) };
    }

    /// dispatch の中で panic させる（境界で止まることを確かめる。テストだけ）
    pub(crate) fn maybe_panic() {
        if PANIC_IN_DISPATCH.with(Cell::get) {
            panic!("test panic in dispatch");
        }
    }

    unsafe extern "C" fn no_text(_: *mut c_void, _: *mut IkariShellBuf) -> i32 {
        1
    }
    unsafe extern "C" fn no_connect(_: *mut c_void) -> u64 {
        0
    }
    unsafe extern "C" fn no_request(
        _: *mut c_void,
        _: u64,
        _: IkariStr,
        _: u32,
        _: *mut IkariShellBuf,
    ) -> i32 {
        IKARI_SHELL_TRANSPORT_UNAVAILABLE
    }
    unsafe extern "C" fn no_post(_: *mut c_void, _: u64, _: IkariStr) {}
    unsafe extern "C" fn no_close(_: *mut c_void, _: u64) {}
    unsafe extern "C" fn no_pid(_: *mut c_void, _: u64) -> i32 {
        -1
    }

    fn host() -> IkariShellHost {
        IkariShellHost {
            ctx: ptr::null_mut(),
            surrounding: Some(no_text),
            selection: Some(no_text),
            engine_connect: Some(no_connect),
            engine_request: Some(no_request),
            engine_post: Some(no_post),
            engine_close: Some(no_close),
            engine_pid: Some(no_pid),
            release: None,
        }
    }

    fn empty_config() -> IkariShellConfig {
        IkariShellConfig {
            shell_version: IkariStr::of("test"),
            settings_path: IkariStr::of(""),
            resources_dir: IkariStr::of(""),
        }
    }

    fn call(s: *mut IkariShell, event: &[u8]) -> (i32, Vec<ShellAction>) {
        let mut out = IkariShellBuf::EMPTY;
        let status = unsafe { ikari_shell_dispatch(s, IkariStr::of_bytes(event), &mut out) };
        let bytes = unsafe { out.into_vec() };
        (status, convert::decode_actions(&bytes).unwrap())
    }

    #[test]
    fn a_panic_in_dispatch_is_stopped_at_the_boundary_and_poisons_the_shell() {
        let s = unsafe { ikari_shell_new(&empty_config(), host(), ptr::null_mut()) };
        assert!(!s.is_null());
        let event = convert::encode_event(&ikari_shell_core::ShellEvent::Deactivate);

        PANIC_IN_DISPATCH.with(|p| p.set(true));
        let (status, actions) = call(s, &event);
        PANIC_IN_DISPATCH.with(|p| p.set(false));
        assert_eq!(status, IKARI_SHELL_ERR_PANIC);
        assert_eq!(actions, vec![ShellAction::PassThrough]);

        // 一度 panic した shell は使わない（作り直す）
        let (status, actions) = call(s, &event);
        assert_eq!(status, IKARI_SHELL_ERR_PANIC);
        assert_eq!(actions, vec![ShellAction::PassThrough]);
        // 落ちた通知と free は、毒が入っても安全
        unsafe { ikari_shell_engine_down(s, 1, IKARI_SHELL_DOWN_INVALIDATED) };
        unsafe { ikari_shell_free(s) };

        let s = unsafe { ikari_shell_new(&empty_config(), host(), ptr::null_mut()) };
        assert_eq!(call(s, &event).0, IKARI_SHELL_OK);
        unsafe { ikari_shell_free(s) };
    }

    // ------------------------------------------------ ホストの口の包み（dispatch の stub が呼ばないので直接）

    fn copy_into(out: *mut IkariShellBuf, bytes: &[u8]) {
        unsafe { out.write(ikari_shell_buf_copy(IkariStr::of_bytes(bytes))) };
    }

    unsafe extern "C" fn text_ok(_: *mut c_void, out: *mut IkariShellBuf) -> i32 {
        let message = ikari_proto::shell::v1::SurroundingText {
            left: "左😀".into(),
            right: "右".into(),
            document_length: 4,
        };
        copy_into(out, &prost::Message::encode_to_vec(&message));
        HOST_TEXT_OK
    }
    /// 取れないと言いつつ *out に入れる（Rust が解放する。リークしない）
    unsafe extern "C" fn text_refused(_: *mut c_void, out: *mut IkariShellBuf) -> i32 {
        copy_into(out, b"ignored");
        1
    }
    unsafe extern "C" fn text_garbage(_: *mut c_void, out: *mut IkariShellBuf) -> i32 {
        copy_into(out, &[0xff, 0xff]);
        HOST_TEXT_OK
    }
    unsafe extern "C" fn connect_7(_: *mut c_void) -> u64 {
        7
    }
    unsafe extern "C" fn request_echo(
        _: *mut c_void,
        handle: u64,
        bytes: IkariStr,
        timeout_ms: u32,
        reply: *mut IkariShellBuf,
    ) -> i32 {
        match handle {
            1 => IKARI_SHELL_TRANSPORT_TIMEOUT,
            2 => 99, // 知らない値は unavailable
            _ => {
                let mut echoed = unsafe { crate::bytes(bytes.ptr, bytes.len) }.to_vec();
                echoed.extend_from_slice(&timeout_ms.to_le_bytes());
                copy_into(reply, &echoed);
                IKARI_SHELL_TRANSPORT_OK
            }
        }
    }
    unsafe extern "C" fn pid_of(_: *mut c_void, handle: u64) -> i32 {
        handle as i32
    }

    #[test]
    fn host_text_reads_protobuf_and_treats_refusal_and_garbage_as_unavailable() {
        let table = |surrounding: TextFn| {
            HostTable::new(IkariShellHost {
                surrounding: Some(surrounding),
                ..host()
            })
            .unwrap()
        };
        let ok = table(text_ok);
        let got = HostText(&ok).surrounding().unwrap();
        assert_eq!(got.left, "左😀");
        assert_eq!(got.right, "右");
        assert_eq!(got.document_length, ikari_shell_core::Utf16Len(4));
        assert!(HostText(&ok).selection().is_none()); // no_text
        assert!(HostText(&table(text_refused)).surrounding().is_none());
        assert!(HostText(&table(text_garbage)).surrounding().is_none());
    }

    #[test]
    fn host_transport_maps_results_to_transport_errors() {
        let transport = HostTransport(Arc::new(
            HostTable::new(IkariShellHost {
                engine_connect: Some(connect_7),
                engine_request: Some(request_echo),
                engine_pid: Some(pid_of),
                ..host()
            })
            .unwrap(),
        ));
        assert_eq!(transport.connect(), Ok(TransportHandle(7)));
        assert_eq!(
            transport.request(TransportHandle(7), b"ab", Duration::from_millis(1500)),
            Ok([b"ab".as_slice(), &1500u32.to_le_bytes()].concat())
        );
        // u32 を超える待ちは丸める
        assert_eq!(
            transport.request(TransportHandle(7), b"", Duration::from_secs(u64::MAX)),
            Ok(u32::MAX.to_le_bytes().to_vec())
        );
        assert_eq!(
            transport.request(TransportHandle(1), b"", Duration::ZERO),
            Err(TransportError::Timeout)
        );
        assert_eq!(
            transport.request(TransportHandle(2), b"", Duration::ZERO),
            Err(TransportError::Unavailable)
        );
        assert_eq!(transport.pid(TransportHandle(42)), Some(42));
        assert_eq!(transport.pid(TransportHandle(0)), None);

        let no_connection = HostTransport(Arc::new(HostTable::new(host()).unwrap()));
        assert_eq!(no_connection.connect(), Err(TransportError::Unavailable));
        assert_eq!(no_connection.pid(TransportHandle(1)), None);
    }
}
