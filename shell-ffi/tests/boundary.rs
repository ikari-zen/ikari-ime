//! 殻の FFI の境界を不正な入力で呼んで、落ちない・UB にならないことを確かめる（Issue #41）。
//!
//! 二重 free と free 後の使用は呼び出し側の契約違反で、Rust 側では検出できない（UB）ので調べない。
//! `ikari_shell_*`・`ikari_settings_*` の NULL と壊れた入力は `shell_abi.rs` が、`ikari_panel_*` は
//! `src/panel.rs` が調べる。ここは共通の解放の口だけ（旧い `ikari_composer_*`・`ikari_convert_script` は M5 で消した）。

use std::ptr;

use ikari_shell_ffi::*;

const HUGE_LEN: usize = usize::MAX;

#[test]
fn free_accepts_null_and_empty() {
    unsafe {
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

#[test]
fn copy_and_free_repeatedly() {
    // 複製した列は 1 回だけ解放する。空の入力（NULL と長さ 0）も扱える
    for len in [0usize, 1, 1024, 1 << 20] {
        let bytes = vec![0x41u8; len];
        let buf = unsafe { ikari_shell_buf_copy(IkariStr::of_bytes(&bytes)) };
        assert_eq!(buf.len, len);
        unsafe { ikari_shell_buf_free(buf) };
    }
    let empty = unsafe { ikari_shell_buf_copy(IkariStr::EMPTY) };
    assert_eq!(empty.len, 0);
    unsafe { ikari_shell_buf_free(empty) };
}
