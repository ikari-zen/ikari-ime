//! FFI の境界を不正な入力で呼んで、落ちない・UB にならないことを確かめる（Issue #41）。
//!
//! 二重 close と close 後の使用は呼び出し側の契約違反で、Rust 側では検出できない（UB）。
//! そのためここでは、契約の範囲内で起こりうる入力（NULL、長さ 0、巨大な長さ、壊れた protobuf、
//! 深い入れ子、close と open の繰り返し）を調べる。解放の所有権は Swift 側のテストで確かめる。

use std::path::Path;
use std::ptr;
use std::slice;

use ikari_engine_ffi::{
    IkariBuf, IkariEngine, ikari_buf_free, ikari_engine_close, ikari_engine_handle,
    ikari_engine_open,
};
use ikari_proto::SCHEMA_VERSION;
use ikari_proto::v1::{EngineConfig, Envelope, ErrorCode, ErrorReply, Hello, envelope};
use prost::Message;

const EMPTY: IkariBuf = IkariBuf {
    ptr: ptr::null_mut(),
    len: 0,
};

/// 巨大な長さの目安。NULL と組み合わせるので、実際には読まれない。
const HUGE_LEN: usize = usize::MAX;
/// 入れ子の深さ。prost の再帰の上限（100）をはるかに超える。
const NEST_DEPTH: usize = 200_000;

fn fixture_dict(out: &Path) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../testdata/sudachi-mini");
    ikari_dict_builder::build(&ikari_dict_builder::BuildConfig {
        lex: vec![src.join("lex.csv")],
        matrix: src.join("matrix.def"),
        out_dir: out.to_path_buf(),
        unk_pos: "名詞,普通名詞,一般,*,*,*".to_owned(),
        unk_cost: 10_000,
        label: "ffi-boundary-test".to_owned(),
    })
    .expect("build the fixture dictionary");
}

fn open_in(dir: &Path) -> *mut IkariEngine {
    let raw = EngineConfig {
        data_dir: dir.to_string_lossy().into_owned(),
        enable_lm: false,
        idle_unload_secs: 300,
    }
    .encode_to_vec();
    let mut err = EMPTY;
    let e = unsafe { ikari_engine_open(raw.as_ptr(), raw.len(), &mut err) };
    assert!(!e.is_null(), "open failed");
    assert!(err.ptr.is_null());
    e
}

fn handle(e: *mut IkariEngine, req: *const u8, len: usize) -> Envelope {
    let buf = unsafe { ikari_engine_handle(e, req, len) };
    assert!(!buf.ptr.is_null());
    let env = Envelope::decode(unsafe { slice::from_raw_parts(buf.ptr, buf.len) })
        .expect("the reply is always a valid Envelope");
    unsafe { ikari_buf_free(buf) };
    env
}

fn is_error(env: &Envelope) -> bool {
    matches!(env.body, Some(envelope::Body::Error(_)))
}

fn hello() -> Vec<u8> {
    Envelope {
        schema_version: SCHEMA_VERSION,
        request_id: 7,
        body: Some(envelope::Body::Hello(Hello::default())),
    }
    .encode_to_vec()
}

/// データのディレクトリ（辞書つき）を作る。
fn data_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let dict = dir.path().join("dict");
    std::fs::create_dir_all(&dict).unwrap();
    fixture_dict(&dict);
    dir
}

/// `depth` 段の入れ子の group（フィールド 1。開始タグ 0x0b）。終了タグを閉じない。
fn nested_groups(depth: usize) -> Vec<u8> {
    vec![0x0b; depth]
}

/// `depth` 段の入れ子の length-delimited（フィールド 1、中身が次の段）。
fn nested_lengths(depth: usize) -> Vec<u8> {
    let mut body = Vec::new();
    for _ in 0..depth {
        let mut next = vec![0x0a];
        let mut len = body.len();
        loop {
            let byte = (len & 0x7f) as u8;
            len >>= 7;
            if len == 0 {
                next.push(byte);
                break;
            }
            next.push(byte | 0x80);
        }
        next.extend_from_slice(&body);
        body = next;
    }
    body
}

#[test]
fn open_with_null_or_huge_config_is_an_error_reply() {
    // NULL と巨大な長さ: NULL は長さに関係なく空の設定として読まれ、読み出しは起きない
    let mut err = EMPTY;
    assert!(unsafe { ikari_engine_open(ptr::null(), HUGE_LEN, &mut err) }.is_null());
    let reply = ErrorReply::decode(unsafe { slice::from_raw_parts(err.ptr, err.len) }).unwrap();
    assert_eq!(reply.code, ErrorCode::BadRequest as i32);
    unsafe { ikari_buf_free(err) };
    // 壊れた設定、深い入れ子の設定
    for config in [
        vec![0xffu8; 64],
        nested_groups(NEST_DEPTH),
        nested_lengths(NEST_DEPTH),
        vec![0x0a, 0xff, 0xff, 0xff, 0xff, 0x0f], // 長さが本体より大きい
    ] {
        let mut err = EMPTY;
        let e = unsafe { ikari_engine_open(config.as_ptr(), config.len(), &mut err) };
        assert!(e.is_null());
        assert!(!err.ptr.is_null());
        unsafe { ikari_buf_free(err) };
    }
}

#[test]
fn handle_survives_null_empty_and_broken_requests() {
    let dir = data_dir();
    let e = open_in(dir.path());

    // NULL と巨大な長さ（NULL なので読まれない）、NULL と 0、ポインタがあって長さ 0
    assert!(is_error(&handle(e, ptr::null(), HUGE_LEN)));
    assert!(is_error(&handle(e, ptr::null(), 0)));
    let one = [0u8; 1];
    assert!(is_error(&handle(e, one.as_ptr(), 0)));

    // 壊れた protobuf（途中で切れた長さ、不正なタグ、フィールド 0、終わらない varint）
    for req in [
        vec![0xffu8; 5],
        vec![0x0a],
        vec![0x0a, 0x10, 0x01],
        vec![0x00],
        vec![0x80; 32],
        vec![0x0a, 0xff, 0xff, 0xff, 0xff, 0x0f],
        vec![0x12, 0x05, 0x08],
    ] {
        assert!(is_error(&handle(e, req.as_ptr(), req.len())), "{req:?}");
    }

    // 正しい要求を全長より短く切ったもの（どの長さでも Envelope が返る）
    let good = hello();
    for len in 0..good.len() {
        handle(e, good.as_ptr(), len);
    }
    // バイトを反転したもの
    for at in 0..good.len() {
        let mut b = good.clone();
        b[at] ^= 0xff;
        handle(e, b.as_ptr(), b.len());
    }

    // 壊れたあとも、同じハンドルで正しい要求が通る
    let reply = handle(e, good.as_ptr(), good.len());
    assert_eq!(reply.request_id, 7);
    assert!(matches!(reply.body, Some(envelope::Body::HelloReply(_))));
    unsafe { ikari_engine_close(e) };
}

#[test]
fn handle_does_not_overflow_on_deeply_nested_protobuf() {
    let dir = data_dir();
    let e = open_in(dir.path());
    // prost の再帰の上限でデコードが失敗し、スタックを使い切らない
    for req in [nested_groups(NEST_DEPTH), nested_lengths(NEST_DEPTH)] {
        let reply = handle(e, req.as_ptr(), req.len());
        assert!(is_error(&reply));
    }
    // 管理の上限（64MB）に近い、巨大な未知フィールドを 1 つ持つ要求も、長さの検査だけで済む
    // （ここでは 8MB。メモリの確保は要求の大きさ分だけで、入れ子で増えない）
    let mut big = vec![0x7a]; // フィールド 15、length-delimited
    let payload = 8 * 1024 * 1024;
    let mut len = payload;
    loop {
        let byte = (len & 0x7f) as u8;
        len >>= 7;
        if len == 0 {
            big.push(byte);
            break;
        }
        big.push(byte | 0x80);
    }
    big.resize(big.len() + payload, 0xaa);
    handle(e, big.as_ptr(), big.len());
    // 入れ子のあとも、正しい要求が通る
    let good = hello();
    assert!(matches!(
        handle(e, good.as_ptr(), good.len()).body,
        Some(envelope::Body::HelloReply(_))
    ));
    unsafe { ikari_engine_close(e) };
}

#[test]
fn close_then_open_again_and_null_close_are_safe() {
    let dir = data_dir();
    // close はロックと学習の保存を解放する。続けて開き直せる
    for _ in 0..3 {
        let e = open_in(dir.path());
        let good = hello();
        assert!(matches!(
            handle(e, good.as_ptr(), good.len()).body,
            Some(envelope::Body::HelloReply(_))
        ));
        unsafe { ikari_engine_close(e) };
        // NULL の close は何度でも安全
        unsafe {
            ikari_engine_close(ptr::null_mut());
            ikari_engine_close(ptr::null_mut());
        }
    }
    // close のあと、NULL にしたハンドルで呼んでもエラーの Envelope が返る
    assert!(is_error(&handle(ptr::null_mut(), ptr::null(), 0)));
}

#[test]
fn buf_free_accepts_empty_and_zero_length_buffers() {
    unsafe {
        ikari_buf_free(EMPTY);
        ikari_buf_free(EMPTY);
        // ptr が NULL なら、len が何でも読まない
        ikari_buf_free(IkariBuf {
            ptr: ptr::null_mut(),
            len: HUGE_LEN,
        });
    }
    // 実際に返ってきたバッファを 1 回解放する
    let mut err = EMPTY;
    assert!(unsafe { ikari_engine_open(ptr::null(), 0, &mut err) }.is_null());
    unsafe { ikari_buf_free(err) };
}
