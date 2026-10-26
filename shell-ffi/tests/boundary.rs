//! 殻の FFI の境界を不正な入力で呼んで、落ちない・UB にならないことを確かめる（Issue #41）。
//!
//! 二重 free と free 後の使用は呼び出し側の契約違反で、Rust 側では検出できない（UB）ので調べない。
//! NULL、長さ 0、巨大な長さ（NULL と組み合わせる）、範囲外の値、大量の入力、生成と解放の繰り返しを調べる。

use std::ptr;
use std::slice;

use ikari_shell_ffi::*;

const HUGE_LEN: usize = usize::MAX;
const MANY: usize = 200_000;
const BIG_BYTES: usize = 8 * 1024 * 1024;

fn text(s: IkariStr) -> String {
    if s.ptr.is_null() {
        return String::new();
    }
    String::from_utf8(unsafe { slice::from_raw_parts(s.ptr, s.len) }.to_vec()).unwrap()
}

fn take(buf: IkariShellBuf) -> Vec<u8> {
    let out = if buf.ptr.is_null() {
        Vec::new()
    } else {
        unsafe { slice::from_raw_parts(buf.ptr, buf.len) }.to_vec()
    };
    unsafe { ikari_shell_buf_free(buf) };
    out
}

#[test]
fn new_romaji_with_null_and_huge_length_uses_the_builtin_table() {
    // NULL は長さに関係なく空（組み込みの表）。読み出しは起きない
    let c = unsafe { ikari_composer_new_romaji(ptr::null(), HUGE_LEN, ptr::null_mut()) };
    assert!(!c.is_null());
    for ch in "ka".chars() {
        unsafe { ikari_composer_push_char(c, ch as u32) };
    }
    assert_eq!(text(unsafe { ikari_composer_kana(c) }), "か");
    unsafe { ikari_composer_free(c) };
    // ポインタがあって長さ 0 も組み込みの表
    let byte = [0xffu8];
    let c = unsafe { ikari_composer_new_romaji(byte.as_ptr(), 0, ptr::null_mut()) };
    assert!(!c.is_null());
    unsafe { ikari_composer_free(c) };
}

#[test]
fn new_romaji_rejects_broken_and_huge_tables_without_crashing() {
    let mut cases: Vec<Vec<u8>> = vec![
        vec![0xff; 16],           // UTF-8 でない
        b"\t\t\t\n\n\n".to_vec(), // 空のキーと値
        b"a".to_vec(),            // タブがない
        "a\t\u{0}\n".as_bytes().to_vec(),
        vec![0u8; 1024], // NUL だらけ
    ];
    // 巨大な 1 行と、大量の行（重複で失敗するか、読み込めるかのどちらか。落ちない）
    let mut long = b"a".to_vec();
    long.extend(std::iter::repeat_n(b'b', BIG_BYTES));
    long.extend_from_slice("\tあ\n".as_bytes());
    cases.push(long);
    cases.push("a\tあ\n".repeat(MANY).into_bytes());
    for tsv in cases {
        let mut err = IkariShellBuf {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let c = unsafe { ikari_composer_new_romaji(tsv.as_ptr(), tsv.len(), &mut err) };
        if c.is_null() {
            assert!(!err.ptr.is_null(), "an error text is returned");
            take(err);
        } else {
            unsafe { ikari_composer_free(c) };
        }
    }
}

#[test]
fn composer_survives_extreme_codepoints_and_many_keys() {
    let c = unsafe { ikari_composer_new_romaji(ptr::null(), 0, ptr::null_mut()) };
    for cp in [0, 0x7f, 0xD800, 0xDFFF, 0x10FFFF, 0x110000, u32::MAX] {
        unsafe { ikari_composer_push_char(c, cp) };
    }
    unsafe { ikari_composer_clear(c) };
    for _ in 0..MANY {
        unsafe { ikari_composer_push_char(c, 'k' as u32) };
        unsafe { ikari_composer_push_char(c, 'a' as u32) };
    }
    assert_eq!(
        text(unsafe { ikari_composer_kana(c) }).chars().count(),
        MANY
    );
    // 余分な backspace は false を返すだけ
    for _ in 0..MANY + 10 {
        unsafe { ikari_composer_backspace(c) };
    }
    assert!(!unsafe { ikari_composer_backspace(c) });
    assert_eq!(text(unsafe { ikari_composer_kana(c) }), "");
    // 種類の違う関数（ローマ字の composer に JIS のキー）は無視される
    unsafe { ikari_composer_push_jis(c, 0, true) };
    unsafe { ikari_composer_free(c) };
}

#[test]
fn jis_composer_survives_out_of_range_keys() {
    let c = ikari_composer_new_jis();
    for key in [0u8, 1, 100, 200, 255] {
        unsafe { ikari_composer_push_jis(c, key, false) };
        unsafe { ikari_composer_push_jis(c, key, true) };
    }
    for _ in 0..MANY {
        unsafe { ikari_composer_push_jis(c, 0, false) };
    }
    unsafe { ikari_composer_flush(c) };
    unsafe { ikari_composer_clear(c) };
    assert_eq!(text(unsafe { ikari_composer_kana(c) }), "");
    assert_eq!(text(unsafe { ikari_composer_pending(c) }), "");
    unsafe { ikari_composer_free(c) };
}

#[test]
fn create_clear_and_free_repeatedly() {
    // 自己参照（表と composer）の組み立てと解放を繰り返す。clear は composer を作り直す
    for _ in 0..2000 {
        let c = unsafe { ikari_composer_new_romaji(ptr::null(), 0, ptr::null_mut()) };
        unsafe {
            ikari_composer_push_char(c, 'k' as u32);
            ikari_composer_clear(c);
            ikari_composer_push_char(c, 'k' as u32);
            ikari_composer_clear(c);
            ikari_composer_free(c);
        }
        let j = ikari_composer_new_jis();
        unsafe {
            ikari_composer_clear(j);
            ikari_composer_free(j);
        }
    }
}

#[test]
fn convert_script_handles_null_huge_and_out_of_range_arguments() {
    // NULL と巨大な長さ（NULL なので読まれない）
    assert!(take(unsafe { ikari_convert_script(ptr::null(), HUGE_LEN, 1) }).is_empty());
    for script in [0u32, 1, 2, 3, 4, 5, 9, 1000, u32::MAX] {
        // 範囲外でも、UTF-8 でなくても落ちない
        let _ = take(unsafe { ikari_convert_script("あいう".as_ptr(), 9, script) });
        let bad = [0xffu8, 0xfe, 0x41];
        let _ = take(unsafe { ikari_convert_script(bad.as_ptr(), bad.len(), script) });
        let _ = take(unsafe { ikari_convert_script(ptr::null(), 0, script) });
    }
    // 大きな入力
    let big = "あ".repeat(BIG_BYTES / 3);
    let out = take(unsafe { ikari_convert_script(big.as_ptr(), big.len(), 1) });
    assert_eq!(out, "ア".repeat(BIG_BYTES / 3).into_bytes());
}

#[test]
fn free_functions_accept_null_and_empty() {
    unsafe {
        ikari_composer_free(ptr::null_mut());
        ikari_composer_free(ptr::null_mut());
        ikari_shell_buf_free(IkariShellBuf {
            ptr: ptr::null_mut(),
            len: 0,
        });
        // ptr が NULL なら、len が何でも読まない
        ikari_shell_buf_free(IkariShellBuf {
            ptr: ptr::null_mut(),
            len: HUGE_LEN,
        });
    }
}
