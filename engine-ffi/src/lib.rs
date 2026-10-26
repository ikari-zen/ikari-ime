//! ikari-engine-ffi: エンジンの C ABI（ADR 0004 §4.2。ヘッダは `include/ikari_engine.h`）。
//!
//! メッセージは protobuf のバイト列。すべての入口で `catch_unwind` を使い、panic を FFI の外へ出さない。
//! ヘッダと一致させること（シグネチャの変更は契約の変更）。

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::slice;

use ikari_engine::{Engine, EngineConfig, EngineError, error_envelope};
use ikari_proto::SCHEMA_VERSION;
use ikari_proto::v1::{ErrorCode, ErrorReply};
use prost::Message;

/// 不透明なハンドル（C 側は `IkariEngine *` だけを持つ）。
pub struct IkariEngine {
    engine: Engine,
}

/// Rust が確保したバイト列。`ikari_buf_free` で解放する。
#[repr(C)]
pub struct IkariBuf {
    pub ptr: *mut u8,
    pub len: usize,
}

impl IkariBuf {
    fn from_vec(v: Vec<u8>) -> IkariBuf {
        let boxed = v.into_boxed_slice();
        let len = boxed.len();
        IkariBuf {
            ptr: Box::into_raw(boxed) as *mut u8,
            len,
        }
    }
}

/// `(ptr, len)` を借用する。NULL か len 0 は空。
///
/// # Safety
/// `ptr` が非 NULL なら、`len` バイトが読めること。
unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(ptr, len) }
    }
}

fn error_reply(code: ErrorCode, message: &str) -> Vec<u8> {
    ErrorReply {
        code: code as i32,
        message: message.to_owned(),
    }
    .encode_to_vec()
}

/// # Safety
/// `config` は `len` バイト読めること（または NULL）。`err` は NULL か、書き込める `IkariBuf` を指すこと。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_engine_open(
    config: *const u8,
    len: usize,
    err: *mut IkariBuf,
) -> *mut IkariEngine {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let raw = unsafe { bytes(config, len) };
        let cfg = EngineConfig::decode(raw)
            .map_err(|_| error_reply(ErrorCode::BadRequest, "invalid EngineConfig"))?;
        Engine::open(&cfg)
            .map(|engine| Box::into_raw(Box::new(IkariEngine { engine })))
            .map_err(|e| match e {
                EngineError::Config(_) => {
                    error_reply(ErrorCode::BadRequest, "invalid engine config")
                }
                EngineError::Open(_) => {
                    error_reply(ErrorCode::EngineNotReady, "engine open failed")
                }
            })
    }));
    let failure = match result {
        Ok(Ok(p)) => return p,
        Ok(Err(bytes)) => bytes,
        Err(_) => error_reply(ErrorCode::Internal, "panic in ikari_engine_open"),
    };
    if !err.is_null() {
        unsafe { err.write(IkariBuf::from_vec(failure)) };
    }
    ptr::null_mut()
}

/// # Safety
/// `e` は `ikari_engine_open` が返した、まだ close していないハンドル（または NULL）。
/// `req` は `len` バイト読めること（または NULL）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_engine_handle(
    e: *mut IkariEngine,
    req: *const u8,
    len: usize,
) -> IkariBuf {
    let reply = catch_unwind(AssertUnwindSafe(|| {
        let raw = unsafe { bytes(req, len) };
        match unsafe { e.as_ref() } {
            Some(handle) => handle.engine.handle(raw),
            None => error_envelope(0, ErrorCode::BadRequest, "null engine"),
        }
    }))
    .unwrap_or_else(|_| error_envelope(0, ErrorCode::Internal, "panic in ikari_engine_handle"));
    IkariBuf::from_vec(reply)
}

/// # Safety
/// `buf` は Rust が返したものを1回だけ渡すこと。`ptr == NULL` は何もしない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_buf_free(buf: IkariBuf) {
    if buf.ptr.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        drop(unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(buf.ptr, buf.len)) });
    }));
}

/// # Safety
/// `e` は `ikari_engine_open` が返したハンドル（または NULL）。close のあとは使わない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_engine_close(e: *mut IkariEngine) {
    if e.is_null() {
        return;
    }
    // Engine の Drop が学習を保存し、スレッドを止める
    let _ = catch_unwind(AssertUnwindSafe(|| drop(unsafe { Box::from_raw(e) })));
}

#[unsafe(no_mangle)]
pub extern "C" fn ikari_engine_schema_version() -> u32 {
    SCHEMA_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikari_proto::v1::{Envelope, envelope};

    const EMPTY: IkariBuf = IkariBuf {
        ptr: ptr::null_mut(),
        len: 0,
    };

    #[test]
    fn header_declares_every_exported_function() {
        let header = include_str!("../include/ikari_engine.h");
        for name in [
            "ikari_engine_open",
            "ikari_engine_handle",
            "ikari_buf_free",
            "ikari_engine_close",
            "ikari_engine_schema_version",
        ] {
            assert!(
                header.contains(name),
                "{name} is missing from ikari_engine.h"
            );
        }
    }

    fn call(e: *mut IkariEngine, req: &[u8]) -> Envelope {
        let buf = unsafe { ikari_engine_handle(e, req.as_ptr(), req.len()) };
        let env = Envelope::decode(unsafe { slice::from_raw_parts(buf.ptr, buf.len) }).unwrap();
        unsafe { ikari_buf_free(buf) };
        env
    }

    fn open_error(config: &EngineConfig) -> ErrorReply {
        let raw = config.encode_to_vec();
        let mut err = EMPTY;
        let e = unsafe { ikari_engine_open(raw.as_ptr(), raw.len(), &mut err) };
        assert!(e.is_null());
        let reply = ErrorReply::decode(unsafe { slice::from_raw_parts(err.ptr, err.len) }).unwrap();
        unsafe { ikari_buf_free(err) };
        reply
    }

    #[test]
    fn open_failures_are_error_replies() {
        // 空の設定
        let reply = open_error(&EngineConfig::default());
        assert_eq!(reply.code, ErrorCode::BadRequest as i32);
        // 辞書がない（NULL の config も空の設定として扱う）
        let dir = std::env::temp_dir().join(format!("ikari-ffi-open-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let reply = open_error(&EngineConfig {
            data_dir: dir.to_string_lossy().into_owned(),
            ..EngineConfig::default()
        });
        assert_eq!(reply.code, ErrorCode::EngineNotReady as i32);
        let _ = std::fs::remove_dir_all(dir);
        let mut err = EMPTY;
        assert!(unsafe { ikari_engine_open(ptr::null(), 0, &mut err) }.is_null());
        unsafe { ikari_buf_free(err) };
        // err が NULL でも落ちない
        assert!(unsafe { ikari_engine_open(ptr::null(), 0, ptr::null_mut()) }.is_null());
    }

    /// 実辞書で open -> handle -> close（`IKARI_DICT_DIR` を指定したときだけ）。
    #[test]
    fn open_handle_close_roundtrip_with_a_real_dictionary() {
        let Some(dict) = std::env::var_os("IKARI_DICT_DIR").filter(|v| !v.is_empty()) else {
            eprintln!("IKARI_DICT_DIR is not set; skipping");
            return;
        };
        let dir = std::env::temp_dir().join(format!("ikari-ffi-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(dict, dir.join("dict")).unwrap();
        let raw = EngineConfig {
            data_dir: dir.to_string_lossy().into_owned(),
            enable_lm: false,
            idle_unload_secs: 300,
        }
        .encode_to_vec();
        let mut err = EMPTY;
        let e = unsafe { ikari_engine_open(raw.as_ptr(), raw.len(), &mut err) };
        assert!(!e.is_null());
        assert!(err.ptr.is_null());
        let hello = Envelope {
            schema_version: SCHEMA_VERSION,
            request_id: 3,
            body: Some(envelope::Body::Hello(ikari_proto::v1::Hello::default())),
        }
        .encode_to_vec();
        let reply = call(e, &hello);
        assert_eq!(reply.request_id, 3);
        assert!(matches!(reply.body, Some(envelope::Body::HelloReply(_))));
        // 壊れた要求も Envelope で返る
        assert!(matches!(
            call(e, &[0xff; 5]).body,
            Some(envelope::Body::Error(_))
        ));
        unsafe { ikari_engine_close(e) };
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn open_rejects_garbage_config_with_error_reply() {
        let garbage = [0xffu8; 4];
        let mut err = EMPTY;
        let e = unsafe { ikari_engine_open(garbage.as_ptr(), garbage.len(), &mut err) };
        assert!(e.is_null());
        let reply = ErrorReply::decode(unsafe { slice::from_raw_parts(err.ptr, err.len) }).unwrap();
        assert_eq!(reply.code, ErrorCode::BadRequest as i32);
        unsafe { ikari_buf_free(err) };
    }

    #[test]
    fn null_engine_and_null_request_do_not_crash() {
        let reply = call(ptr::null_mut(), &[]);
        assert!(matches!(reply.body, Some(envelope::Body::Error(_))));
        unsafe {
            ikari_engine_close(ptr::null_mut());
            ikari_buf_free(EMPTY);
        }
        assert_eq!(ikari_engine_schema_version(), SCHEMA_VERSION);
    }
}
