//! libxpc の呼び出し（macOS。檻役とエンジンの側の両方が使う。ADR 0077 §3.4）。`unsafe` は、このファイルと
//! `spawn.rs`、`bundle.rs` に閉じる。
//!
//! - [`Obj`] は、xpc の値 1 つへの参照（retain を 1 つ持ち、捨てると release する）。
//! - ハンドラは `block2` の block で渡す。libxpc は block を写して持つので、こちらの `RcBlock` は渡したあと捨ててよい。
//! - 辞書の値は、型を確かめてから読む（違う型・無いキーは None）。

#![allow(non_upper_case_globals)]

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::ptr::{self, NonNull};

use block2::{Block, RcBlock};

/// libxpc の値（`xpc_object_t`）
pub(super) type RawObj = *mut c_void;
type XpcType = *const c_void;
type DispatchQueue = *mut c_void;
type Handler = Block<dyn Fn(RawObj)>;
/// `xpc_main` に渡す、名前の口への新しい接続のハンドラ
pub(super) type ConnectionHandler = extern "C" fn(RawObj);

/// 型と誤りの印（アドレスだけを使う）
#[repr(C)]
struct Opaque {
    _private: [u8; 0],
}

unsafe extern "C" {
    static _xpc_type_dictionary: Opaque;
    static _xpc_type_error: Opaque;
    static _xpc_type_connection: Opaque;
    static _xpc_type_endpoint: Opaque;
    static _xpc_type_uint64: Opaque;
    static _xpc_type_string: Opaque;
    static _xpc_error_connection_invalid: Opaque;
    static _xpc_error_connection_interrupted: Opaque;
    static _xpc_error_peer_code_signing_requirement: Opaque;

    fn xpc_retain(object: RawObj) -> RawObj;
    fn xpc_release(object: RawObj);
    fn xpc_get_type(object: RawObj) -> XpcType;

    fn xpc_dictionary_create(
        keys: *const *const c_char,
        values: *const RawObj,
        count: usize,
    ) -> RawObj;
    fn xpc_dictionary_create_reply(original: RawObj) -> RawObj;
    fn xpc_dictionary_set_string(dictionary: RawObj, key: *const c_char, value: *const c_char);
    fn xpc_dictionary_set_uint64(dictionary: RawObj, key: *const c_char, value: u64);
    fn xpc_dictionary_set_fd(dictionary: RawObj, key: *const c_char, fd: c_int);
    fn xpc_dictionary_set_value(dictionary: RawObj, key: *const c_char, value: RawObj);
    fn xpc_dictionary_get_value(dictionary: RawObj, key: *const c_char) -> RawObj;
    fn xpc_dictionary_dup_fd(dictionary: RawObj, key: *const c_char) -> c_int;
    fn xpc_uint64_get_value(object: RawObj) -> u64;
    fn xpc_string_get_string_ptr(object: RawObj) -> *const c_char;

    fn xpc_connection_create(name: *const c_char, queue: DispatchQueue) -> RawObj;
    fn xpc_connection_create_from_endpoint(endpoint: RawObj) -> RawObj;
    fn xpc_endpoint_create(connection: RawObj) -> RawObj;
    fn xpc_connection_set_event_handler(connection: RawObj, handler: &Handler);
    fn xpc_connection_set_peer_code_signing_requirement(
        connection: RawObj,
        requirement: *const c_char,
    ) -> c_int;
    fn xpc_connection_resume(connection: RawObj);
    fn xpc_connection_cancel(connection: RawObj);
    fn xpc_connection_send_message(connection: RawObj, message: RawObj);
    fn xpc_connection_send_message_with_reply(
        connection: RawObj,
        message: RawObj,
        queue: DispatchQueue,
        handler: &Handler,
    );
    fn xpc_main(handler: ConnectionHandler) -> !;
    fn xpc_transaction_begin();
    fn xpc_transaction_end();

    fn dispatch_get_global_queue(identifier: isize, flags: usize) -> DispatchQueue;
}

/// `DISPATCH_QUEUE_PRIORITY_DEFAULT`
const GLOBAL_QUEUE_DEFAULT: isize = 0;

/// xpc の値の種類（ハンドラに届いたものを分ける）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Dictionary,
    Connection,
    Endpoint,
    /// 接続が無効になった（相手が終わった、捨てた、取り消した。これが最後の知らせ）
    ConnectionInvalid,
    /// 接続が途切れた（名前の口の相手が終わった。取り直せる）
    ConnectionInterrupted,
    /// 相手が署名要件を満たさない（libxpc が伝言を落とした）
    PeerRequirement,
    OtherError,
    Other,
}

fn type_of(raw: RawObj) -> XpcType {
    // SAFETY: `raw` は呼び出し側が持っている（または、ハンドラの間、libxpc が持っている）生きた値
    unsafe { xpc_get_type(raw) }
}

fn same(raw: RawObj, mark: *const Opaque) -> bool {
    ptr::eq(raw.cast_const().cast::<Opaque>(), mark)
}

/// ハンドラに届いた値（libxpc が持っている間だけ有効）の種類。型と誤りの印は libxpc の中の不変の値で、
/// アドレスだけを比べる
pub(super) fn kind_of(raw: RawObj) -> Kind {
    let t = type_of(raw);
    if ptr::eq(t, (&raw const _xpc_type_dictionary).cast()) {
        Kind::Dictionary
    } else if ptr::eq(t, (&raw const _xpc_type_connection).cast()) {
        Kind::Connection
    } else if ptr::eq(t, (&raw const _xpc_type_endpoint).cast()) {
        Kind::Endpoint
    } else if ptr::eq(t, (&raw const _xpc_type_error).cast()) {
        if same(raw, &raw const _xpc_error_connection_invalid) {
            Kind::ConnectionInvalid
        } else if same(raw, &raw const _xpc_error_connection_interrupted) {
            Kind::ConnectionInterrupted
        } else if same(raw, &raw const _xpc_error_peer_code_signing_requirement) {
            Kind::PeerRequirement
        } else {
            Kind::OtherError
        }
    } else {
        Kind::Other
    }
}

/// xpc の値 1 つへの参照（retain を 1 つ持つ）
pub(super) struct Obj(NonNull<c_void>);

// SAFETY: libxpc の値の参照の数え方はスレッドをまたいで安全。辞書を組むのは作ったスレッドだけで、送ったあとは書き換えない
unsafe impl Send for Obj {}
// SAFETY: 同上。共有して行うのは、読み取りと、接続への送信と取り消し（libxpc はスレッドをまたいで呼べる）だけ
unsafe impl Sync for Obj {}

impl Drop for Obj {
    fn drop(&mut self) {
        // SAFETY: この値が持つ retain を 1 つ返す
        unsafe { xpc_release(self.0.as_ptr()) };
    }
}

impl Clone for Obj {
    fn clone(&self) -> Obj {
        // SAFETY: 生きた値の retain を 1 つ足す
        unsafe { xpc_retain(self.0.as_ptr()) };
        Obj(self.0)
    }
}

impl std::fmt::Debug for Obj {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Obj({:?})", self.kind())
    }
}

fn key(name: &str) -> CString {
    // 取り決めのキーは固定の語（NUL を含まない）
    CString::new(name).unwrap_or_default()
}

impl Obj {
    /// 作る関数が返した値（retain を 1 つ持っている）を受け取る。NULL なら None
    fn owned(raw: RawObj) -> Option<Obj> {
        NonNull::new(raw).map(Obj)
    }

    /// 借りている値（ハンドラの引数など）に retain を足して持つ
    pub(super) fn retain(raw: RawObj) -> Option<Obj> {
        let raw = NonNull::new(raw)?;
        // SAFETY: 呼び出し側が生きていることを保証する値（libxpc が持っている間）に retain を足す
        unsafe { xpc_retain(raw.as_ptr()) };
        Some(Obj(raw))
    }

    pub(super) fn as_raw(&self) -> RawObj {
        self.0.as_ptr()
    }

    pub(super) fn kind(&self) -> Kind {
        kind_of(self.as_raw())
    }

    /// 空の辞書
    pub(super) fn dictionary() -> io::Result<Obj> {
        // SAFETY: 空の配列（長さ 0）を渡すだけ
        let raw = unsafe { xpc_dictionary_create(ptr::null(), ptr::null(), 0) };
        Obj::owned(raw).ok_or_else(|| io::Error::other("xpc dictionary"))
    }

    /// 届いた要求への返事の辞書（相手が返事を待っていなければ None）
    pub(super) fn reply_to(&self) -> Option<Obj> {
        // SAFETY: 生きた辞書を渡すだけ。返る値は retain つき（または NULL）
        Obj::owned(unsafe { xpc_dictionary_create_reply(self.as_raw()) })
    }

    pub(super) fn set_str(&self, name: &str, value: &str) {
        let (k, v) = (key(name), CString::new(value).unwrap_or_default());
        // SAFETY: 生きた辞書に、NUL で終わる文字列を写して入れる
        unsafe { xpc_dictionary_set_string(self.as_raw(), k.as_ptr(), v.as_ptr()) };
    }

    pub(super) fn set_u64(&self, name: &str, value: u64) {
        let k = key(name);
        // SAFETY: 生きた辞書に値を入れる
        unsafe { xpc_dictionary_set_uint64(self.as_raw(), k.as_ptr(), value) };
    }

    /// fd を入れる（libxpc が写しを作る。`fd` はこちらが持ったまま）
    pub(super) fn set_fd(&self, name: &str, fd: BorrowedFd<'_>) {
        let k = key(name);
        // SAFETY: 生きた辞書と、呼び出しの間開いている fd
        unsafe { xpc_dictionary_set_fd(self.as_raw(), k.as_ptr(), fd.as_raw_fd()) };
    }

    pub(super) fn set_value(&self, name: &str, value: &Obj) {
        let k = key(name);
        // SAFETY: 生きた辞書に、生きた値を入れる（辞書が retain する）
        unsafe { xpc_dictionary_set_value(self.as_raw(), k.as_ptr(), value.as_raw()) };
    }

    fn get_raw(&self, name: &str) -> Option<RawObj> {
        let k = key(name);
        // SAFETY: 生きた辞書から借りるだけ（辞書が生きている間有効）
        let raw = unsafe { xpc_dictionary_get_value(self.as_raw(), k.as_ptr()) };
        (!raw.is_null()).then_some(raw)
    }

    /// キーがあるか（型は問わない）
    pub(super) fn has(&self, name: &str) -> bool {
        self.get_raw(name).is_some()
    }

    pub(super) fn get_str(&self, name: &str) -> Option<String> {
        let raw = self.get_raw(name)?;
        if !ptr::eq(type_of(raw), (&raw const _xpc_type_string).cast()) {
            return None;
        }
        // SAFETY: 文字列の値の、NUL で終わる中身（辞書が生きている間有効。すぐに写す）
        let text = unsafe { CStr::from_ptr(xpc_string_get_string_ptr(raw)) };
        text.to_str().ok().map(str::to_owned)
    }

    pub(super) fn get_u64(&self, name: &str) -> Option<u64> {
        let raw = self.get_raw(name)?;
        if !ptr::eq(type_of(raw), (&raw const _xpc_type_uint64).cast()) {
            return None;
        }
        // SAFETY: 型を確かめた値を読む
        Some(unsafe { xpc_uint64_get_value(raw) })
    }

    /// fd の写しを取る（無い・fd でなければ None）。写しは close-on-exec
    pub(super) fn dup_fd(&self, name: &str) -> Option<OwnedFd> {
        let k = key(name);
        // SAFETY: 生きた辞書から、新しい fd を作ってもらう（持ち主はこちら）
        let fd = unsafe { xpc_dictionary_dup_fd(self.as_raw(), k.as_ptr()) };
        // SAFETY: `fd` は今作られた、ほかに持ち主のない fd
        (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

/// 接続 1 本の持ち手。捨てると取り消す（`xpc_connection_cancel`）。接続の操作は [`Obj`] にある
#[derive(Debug)]
pub(super) struct Connection(Obj);

impl Drop for Connection {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

impl std::ops::Deref for Connection {
    type Target = Obj;

    fn deref(&self) -> &Obj {
        &self.0
    }
}

/// ハンドラの block を作る（libxpc が写して持つ）
fn handler(f: impl Fn(RawObj) + Send + Sync + 'static) -> RcBlock<dyn Fn(RawObj)> {
    RcBlock::new(f)
}

impl Connection {
    /// 匿名の受け口（プロセスの中だけの口。`endpoint` で口を作って渡す）
    pub(super) fn anonymous_listener() -> io::Result<Connection> {
        // SAFETY: 名前なし・既定の列で作る
        let raw = unsafe { xpc_connection_create(ptr::null(), ptr::null_mut()) };
        Obj::owned(raw)
            .map(Connection)
            .ok_or_else(|| io::Error::other("xpc anonymous listener"))
    }

    /// 匿名の口へつなぐ接続（まだ始めていない）
    pub(super) fn to_endpoint(endpoint: &Obj) -> io::Result<Connection> {
        if endpoint.kind() != Kind::Endpoint {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not an xpc endpoint",
            ));
        }
        // SAFETY: 型を確かめた生きた口から、新しい接続を作る（retain つき）
        let raw = unsafe { xpc_connection_create_from_endpoint(endpoint.as_raw()) };
        Obj::owned(raw)
            .map(Connection)
            .ok_or_else(|| io::Error::other("xpc connection from endpoint"))
    }
}

/// 接続の操作（`self` は接続の値であること）
impl Obj {
    /// 相手の署名要件（始める前に呼ぶ）。合わない相手の伝言は libxpc が落とす
    pub(super) fn require_peer(&self, requirement: &CStr) -> io::Result<()> {
        // SAFETY: 生きた接続と、NUL で終わる文字列（libxpc が写す）
        let rc = unsafe {
            xpc_connection_set_peer_code_signing_requirement(self.as_raw(), requirement.as_ptr())
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(rc))
        }
    }

    /// 届いたものを受けるハンドラ（始める前に 1 回だけ呼ぶ）。接続ごとに 1 つずつ順に呼ばれる
    pub(super) fn on_event(&self, f: impl Fn(RawObj) + Send + Sync + 'static) {
        let block = handler(f);
        // SAFETY: 生きた接続に、block を渡す（libxpc が写して持つ）
        unsafe { xpc_connection_set_event_handler(self.as_raw(), &block) };
    }

    pub(super) fn resume(&self) {
        // SAFETY: ハンドラを付けた生きた接続を始める（1 回だけ呼ぶ）
        unsafe { xpc_connection_resume(self.as_raw()) };
    }

    /// 取り消す（何度呼んでもよい。最後に、ハンドラに「無効」が届く）
    pub(super) fn cancel(&self) {
        // SAFETY: 生きた接続を取り消す
        unsafe { xpc_connection_cancel(self.as_raw()) };
    }

    pub(super) fn send(&self, message: &Obj) {
        // SAFETY: 生きた接続に、生きた辞書を送る（libxpc が retain する）
        unsafe { xpc_connection_send_message(self.as_raw(), message.as_raw()) };
    }

    /// 送って、返事（または誤り）を `f` で受ける。`f` は共有の列で 1 回だけ呼ばれる
    pub(super) fn send_with_reply(
        &self,
        message: &Obj,
        f: impl Fn(RawObj) + Send + Sync + 'static,
    ) {
        let block = handler(f);
        // SAFETY: 生きた接続・辞書と、プロセスの共有の列（解放しない）と、block（libxpc が写して持つ）
        unsafe {
            let queue = dispatch_get_global_queue(GLOBAL_QUEUE_DEFAULT, 0);
            xpc_connection_send_message_with_reply(self.as_raw(), message.as_raw(), queue, &block);
        }
    }

    /// この受け口への匿名の口
    pub(super) fn endpoint(&self) -> io::Result<Obj> {
        // SAFETY: 生きた受け口から口を作る（retain つき）
        Obj::owned(unsafe { xpc_endpoint_create(self.as_raw()) })
            .ok_or_else(|| io::Error::other("xpc endpoint"))
    }
}

/// XPC サービスとして動く（名前の口への接続ごとに `handler` が呼ばれる）。戻らない
pub(super) fn run_main(handler: ConnectionHandler) -> ! {
    // SAFETY: プロセスの入口から 1 回だけ呼ぶ
    unsafe { xpc_main(handler) }
}

/// 子がいる間、XPC サービスを終わらせない印（`end` と対で呼ぶ）
pub(super) fn transaction_begin() {
    // SAFETY: 引数なしの呼び出し
    unsafe { xpc_transaction_begin() };
}

pub(super) fn transaction_end() {
    // SAFETY: 引数なしの呼び出し（`begin` と対）
    unsafe { xpc_transaction_end() };
}
