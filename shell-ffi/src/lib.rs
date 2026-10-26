//! ikari-shell-ffi: 殻が使う入力処理の C ABI（ADR 0004 §4.3。ヘッダは `include/ikari_shell.h`）。
//!
//! すべての入口で `catch_unwind` を使い、panic を FFI の外へ出さない。
//! ヘッダと一致させること（シグネチャの変更は契約の変更）。

use std::mem::ManuallyDrop;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{self, NonNull};
use std::slice;

use ikari_input::{JisKanaComposer, JisKey, RomajiComposer, RomajiTable, Script, convert_script};

/// ローマ字の composer。`RomajiComposer<'t>` は表を借用するので、表をヒープに置いて固定し、
/// composer がその表を指す（自己参照）。表は composer より後に解放する。
struct RomajiState {
    composer: ManuallyDrop<RomajiComposer<'static>>,
    table: NonNull<RomajiTable>,
}

impl RomajiState {
    fn new(table: RomajiTable) -> Self {
        let table = NonNull::from(Box::leak(Box::new(table)));
        Self {
            composer: ManuallyDrop::new(Self::composer_for(table)),
            table,
        }
    }

    fn composer_for(table: NonNull<RomajiTable>) -> RomajiComposer<'static> {
        // SAFETY: 表は `Drop` まで解放しない固定のヒープにあり、composer は表より先に落とす。
        RomajiComposer::new(unsafe { table.as_ref() })
    }

    fn clear(&mut self) {
        // SAFETY: 古い composer を落としてから、同じ表を指す新しい composer に置き換える。
        unsafe { ManuallyDrop::drop(&mut self.composer) };
        self.composer = ManuallyDrop::new(Self::composer_for(self.table));
    }
}

impl Drop for RomajiState {
    fn drop(&mut self) {
        // SAFETY: composer を先に、表をあとに解放する。どちらも 1 回だけ。
        unsafe {
            ManuallyDrop::drop(&mut self.composer);
            drop(Box::from_raw(self.table.as_ptr()));
        }
    }
}

enum Inner {
    Romaji(RomajiState),
    Jis(JisKanaComposer),
}

/// 不透明なハンドル（C 側は `IkariComposer *` だけを持つ）。
pub struct IkariComposer {
    inner: Inner,
}

impl IkariComposer {
    fn kana(&self) -> &str {
        match &self.inner {
            Inner::Romaji(r) => r.composer.kana(),
            Inner::Jis(j) => j.kana(),
        }
    }

    fn pending(&self) -> &str {
        match &self.inner {
            Inner::Romaji(r) => r.composer.pending(),
            Inner::Jis(_) => "",
        }
    }
}

/// 借用の文字列。
#[repr(C)]
pub struct IkariStr {
    pub ptr: *const u8,
    pub len: usize,
}

/// Rust が確保したバイト列。`ikari_shell_buf_free` で解放する。
#[repr(C)]
pub struct IkariShellBuf {
    pub ptr: *mut u8,
    pub len: usize,
}

impl IkariStr {
    const EMPTY: IkariStr = IkariStr {
        ptr: ptr::null(),
        len: 0,
    };

    fn of(s: &str) -> IkariStr {
        IkariStr {
            ptr: s.as_ptr(),
            len: s.len(),
        }
    }
}

impl IkariShellBuf {
    const EMPTY: IkariShellBuf = IkariShellBuf {
        ptr: ptr::null_mut(),
        len: 0,
    };

    fn from_vec(v: Vec<u8>) -> IkariShellBuf {
        let boxed = v.into_boxed_slice();
        let len = boxed.len();
        IkariShellBuf {
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

/// panic を FFI の外へ出さない。panic したら `fallback` を返す。
fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

fn write_error(err: *mut IkariShellBuf, message: &str) {
    if !err.is_null() {
        // SAFETY: 呼び出し側が書き込める `IkariShellBuf` を渡す（契約）。
        unsafe { err.write(IkariShellBuf::from_vec(message.as_bytes().to_vec())) };
    }
}

/// # Safety
/// `table_tsv` は `len` バイト読めること（または NULL）。`err` は NULL か書き込めること。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_composer_new_romaji(
    table_tsv: *const u8,
    len: usize,
    err: *mut IkariShellBuf,
) -> *mut IkariComposer {
    let made = catch_unwind(AssertUnwindSafe(|| {
        let raw = unsafe { bytes(table_tsv, len) };
        let table = if raw.is_empty() {
            RomajiTable::builtin()
        } else {
            let text = std::str::from_utf8(raw)
                .map_err(|_| "romaji table is not valid UTF-8".to_owned())?;
            RomajiTable::parse(text).map_err(|e| e.to_string())?
        };
        Ok::<_, String>(Box::into_raw(Box::new(IkariComposer {
            inner: Inner::Romaji(RomajiState::new(table)),
        })))
    }));
    match made {
        Ok(Ok(p)) => p,
        Ok(Err(message)) => {
            write_error(err, &message);
            ptr::null_mut()
        }
        Err(_) => {
            write_error(err, "panic in ikari_composer_new_romaji");
            ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn ikari_composer_new_jis() -> *mut IkariComposer {
    guard(ptr::null_mut(), || {
        Box::into_raw(Box::new(IkariComposer {
            inner: Inner::Jis(JisKanaComposer::new()),
        }))
    })
}

/// # Safety
/// `c` は `ikari_composer_new_*` が返したハンドル（または NULL）。以下の composer の関数も同じ。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_composer_push_char(c: *mut IkariComposer, codepoint: u32) {
    guard((), || {
        let (Some(c), Some(ch)) = (unsafe { c.as_mut() }, char::from_u32(codepoint)) else {
            return;
        };
        if let Inner::Romaji(r) = &mut c.inner {
            r.composer.push(ch);
        }
    });
}

/// # Safety
/// `ikari_composer_push_char` を参照。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_composer_push_jis(c: *mut IkariComposer, jis_key: u8, shift: bool) {
    guard((), || {
        let (Some(c), Some(key)) = (unsafe { c.as_mut() }, JisKey::from_index(jis_key)) else {
            return;
        };
        if let Inner::Jis(j) = &mut c.inner {
            j.push(key, shift);
        }
    });
}

/// # Safety
/// `ikari_composer_push_char` を参照。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_composer_backspace(c: *mut IkariComposer) -> bool {
    guard(false, || {
        let Some(c) = (unsafe { c.as_mut() }) else {
            return false;
        };
        match &mut c.inner {
            Inner::Romaji(r) => r.composer.backspace(),
            Inner::Jis(j) => j.backspace(),
        }
    })
}

/// # Safety
/// `ikari_composer_push_char` を参照。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_composer_flush(c: *mut IkariComposer) {
    guard((), || {
        if let Some(IkariComposer {
            inner: Inner::Romaji(r),
        }) = unsafe { c.as_mut() }
        {
            r.composer.flush();
        }
    });
}

/// # Safety
/// `ikari_composer_push_char` を参照。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_composer_clear(c: *mut IkariComposer) {
    guard((), || match unsafe { c.as_mut() } {
        Some(IkariComposer {
            inner: Inner::Romaji(r),
        }) => r.clear(),
        Some(IkariComposer {
            inner: Inner::Jis(j),
        }) => *j = JisKanaComposer::new(),
        None => {}
    });
}

/// # Safety
/// `ikari_composer_push_char` を参照。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_composer_kana(c: *const IkariComposer) -> IkariStr {
    guard(IkariStr::EMPTY, || {
        unsafe { c.as_ref() }.map_or(IkariStr::EMPTY, |c| IkariStr::of(c.kana()))
    })
}

/// # Safety
/// `ikari_composer_push_char` を参照。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_composer_pending(c: *const IkariComposer) -> IkariStr {
    guard(IkariStr::EMPTY, || {
        unsafe { c.as_ref() }.map_or(IkariStr::EMPTY, |c| IkariStr::of(c.pending()))
    })
}

/// # Safety
/// `c` は1回だけ free すること（または NULL）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_composer_free(c: *mut IkariComposer) {
    if c.is_null() {
        return;
    }
    guard((), || drop(unsafe { Box::from_raw(c) }));
}

/// # Safety
/// `s` は `len` バイト読めること（または NULL）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_convert_script(
    s: *const u8,
    len: usize,
    script: u32,
) -> IkariShellBuf {
    let raw = unsafe { bytes(s, len) };
    let converted = catch_unwind(AssertUnwindSafe(|| {
        match (Script::from_u32(script), std::str::from_utf8(raw)) {
            (Some(to), Ok(text)) => convert_script(text, to).into_bytes(),
            // 範囲外の字種と UTF-8 でない入力は、そのまま返す
            _ => raw.to_vec(),
        }
    }));
    match converted {
        Ok(v) => IkariShellBuf::from_vec(v),
        Err(_) => IkariShellBuf::EMPTY,
    }
}

/// # Safety
/// `buf` は Rust が返したものを1回だけ渡すこと。`ptr == NULL` は何もしない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_shell_buf_free(buf: IkariShellBuf) {
    if buf.ptr.is_null() {
        return;
    }
    guard((), || {
        drop(unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(buf.ptr, buf.len)) });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: IkariStr) -> String {
        if s.ptr.is_null() {
            return String::new();
        }
        String::from_utf8(unsafe { slice::from_raw_parts(s.ptr, s.len) }.to_vec()).unwrap()
    }

    fn type_romaji(c: *mut IkariComposer, keys: &str) {
        for ch in keys.chars() {
            unsafe { ikari_composer_push_char(c, ch as u32) };
        }
    }

    fn new_romaji() -> *mut IkariComposer {
        let c = unsafe { ikari_composer_new_romaji(ptr::null(), 0, ptr::null_mut()) };
        assert!(!c.is_null());
        c
    }

    #[test]
    fn header_declares_every_exported_function() {
        let header = include_str!("../include/ikari_shell.h");
        for name in [
            "ikari_composer_new_romaji",
            "ikari_composer_new_jis",
            "ikari_composer_push_char",
            "ikari_composer_push_jis",
            "ikari_composer_backspace",
            "ikari_composer_flush",
            "ikari_composer_clear",
            "ikari_composer_kana",
            "ikari_composer_pending",
            "ikari_composer_free",
            "ikari_convert_script",
            "ikari_shell_buf_free",
        ] {
            assert!(
                header.contains(name),
                "{name} is missing from ikari_shell.h"
            );
        }
    }

    #[test]
    fn jis_key_count_matches_the_header() {
        assert!(include_str!("../include/ikari_shell.h").contains("IKARI_JIS_KEY_COUNT = 48"));
        assert_eq!(ikari_input::JisKey::ALL.len(), 48);
    }

    #[test]
    fn script_numbers_match_the_header() {
        let header = include_str!("../include/ikari_shell.h");
        for (name, script) in [
            ("IKARI_SCRIPT_HIRAGANA = 0", Script::Hiragana),
            (
                "IKARI_SCRIPT_FULLWIDTH_KATAKANA = 1",
                Script::FullwidthKatakana,
            ),
            (
                "IKARI_SCRIPT_HALFWIDTH_KATAKANA = 2",
                Script::HalfwidthKatakana,
            ),
            ("IKARI_SCRIPT_FULLWIDTH_ALNUM = 3", Script::FullwidthAlnum),
            ("IKARI_SCRIPT_HALFWIDTH_ALNUM = 4", Script::HalfwidthAlnum),
        ] {
            assert!(header.contains(name), "{name}");
            assert_eq!(Script::from_u32(script as u32), Some(script));
        }
    }

    #[test]
    fn romaji_composer_follows_the_rust_composer() {
        let c = new_romaji();
        type_romaji(c, "kyou");
        assert_eq!(text(unsafe { ikari_composer_kana(c) }), "きょう");
        assert_eq!(text(unsafe { ikari_composer_pending(c) }), "");
        type_romaji(c, "n");
        assert_eq!(text(unsafe { ikari_composer_pending(c) }), "n");
        // pending から先に消える
        assert!(unsafe { ikari_composer_backspace(c) });
        assert_eq!(text(unsafe { ikari_composer_kana(c) }), "きょう");
        type_romaji(c, "n");
        unsafe { ikari_composer_flush(c) };
        assert_eq!(text(unsafe { ikari_composer_kana(c) }), "きょうん");
        unsafe { ikari_composer_clear(c) };
        assert_eq!(text(unsafe { ikari_composer_kana(c) }), "");
        assert!(!unsafe { ikari_composer_backspace(c) });
        // clear のあとも使える
        type_romaji(c, "ka");
        assert_eq!(text(unsafe { ikari_composer_kana(c) }), "か");
        unsafe { ikari_composer_free(c) };
    }

    #[test]
    fn custom_table_replaces_the_builtin() {
        let tsv = "a\tお\n";
        let c = unsafe { ikari_composer_new_romaji(tsv.as_ptr(), tsv.len(), ptr::null_mut()) };
        assert!(!c.is_null());
        type_romaji(c, "a");
        assert_eq!(text(unsafe { ikari_composer_kana(c) }), "お");
        unsafe { ikari_composer_free(c) };
    }

    #[test]
    fn bad_table_returns_null_and_an_error_text() {
        let tsv = "a\tお\na\tあ\n";
        let mut err = IkariShellBuf::EMPTY;
        let c = unsafe { ikari_composer_new_romaji(tsv.as_ptr(), tsv.len(), &mut err) };
        assert!(c.is_null());
        let message =
            String::from_utf8(unsafe { slice::from_raw_parts(err.ptr, err.len) }.to_vec()).unwrap();
        assert!(message.contains("duplicate"), "{message}");
        unsafe { ikari_shell_buf_free(err) };

        let invalid = [0xffu8, 0xfe];
        let mut err = IkariShellBuf::EMPTY;
        assert!(
            unsafe { ikari_composer_new_romaji(invalid.as_ptr(), invalid.len(), &mut err) }
                .is_null()
        );
        assert!(!err.ptr.is_null());
        unsafe { ikari_shell_buf_free(err) };
        // err が NULL でも落ちない
        assert!(
            unsafe { ikari_composer_new_romaji(invalid.as_ptr(), invalid.len(), ptr::null_mut()) }
                .is_null()
        );
    }

    #[test]
    fn jis_composer_combines_dakuten_and_ignores_romaji_input() {
        let c = ikari_composer_new_jis();
        assert!(!c.is_null());
        let ka = JisKey::from_legend('t').expect("か is on the T key");
        let dakuten = JisKey::from_legend('@').expect("゛ is on the @ key");
        unsafe {
            ikari_composer_push_char(c, 'a' as u32);
            ikari_composer_push_jis(c, ka as u8, false);
            ikari_composer_push_jis(c, dakuten as u8, false);
        }
        assert_eq!(text(unsafe { ikari_composer_kana(c) }), "が");
        assert_eq!(text(unsafe { ikari_composer_pending(c) }), "");
        unsafe { ikari_composer_push_jis(c, 200, false) }; // 範囲外は無視
        unsafe { ikari_composer_flush(c) };
        assert_eq!(text(unsafe { ikari_composer_kana(c) }), "が");
        assert!(unsafe { ikari_composer_backspace(c) });
        assert_eq!(text(unsafe { ikari_composer_kana(c) }), "");
        unsafe { ikari_composer_push_jis(c, ka as u8, false) };
        unsafe { ikari_composer_clear(c) };
        assert_eq!(text(unsafe { ikari_composer_kana(c) }), "");
        unsafe { ikari_composer_free(c) };
    }

    #[test]
    fn null_handles_and_invalid_arguments_are_ignored() {
        unsafe {
            ikari_composer_push_char(ptr::null_mut(), 'a' as u32);
            ikari_composer_push_jis(ptr::null_mut(), 0, false);
            assert!(!ikari_composer_backspace(ptr::null_mut()));
            ikari_composer_flush(ptr::null_mut());
            ikari_composer_clear(ptr::null_mut());
            assert!(ikari_composer_kana(ptr::null()).ptr.is_null());
            assert_eq!(ikari_composer_pending(ptr::null()).len, 0);
            ikari_composer_free(ptr::null_mut());
            ikari_shell_buf_free(IkariShellBuf::EMPTY);
        }
        let c = new_romaji();
        // サロゲートは無視
        unsafe { ikari_composer_push_char(c, 0xD800) };
        assert_eq!(text(unsafe { ikari_composer_kana(c) }), "");
        unsafe { ikari_composer_free(c) };
    }

    fn script(input: &[u8], to: u32) -> Vec<u8> {
        let buf = unsafe { ikari_convert_script(input.as_ptr(), input.len(), to) };
        let out = if buf.ptr.is_null() {
            Vec::new()
        } else {
            unsafe { slice::from_raw_parts(buf.ptr, buf.len) }.to_vec()
        };
        unsafe { ikari_shell_buf_free(buf) };
        out
    }

    #[test]
    fn convert_script_round_trip() {
        assert_eq!(script("がっこう".as_bytes(), 1), "ガッコウ".as_bytes());
        assert_eq!(script("がっこう".as_bytes(), 2), "ｶﾞｯｺｳ".as_bytes());
        assert_eq!(script("きょう".as_bytes(), 4), b"kyou");
        assert_eq!(script(b"", 1), b"");
        // 範囲外の字種と、UTF-8 でない入力はそのまま
        assert_eq!(script("あ".as_bytes(), 9), "あ".as_bytes());
        assert_eq!(script(&[0xff, 0x41], 1), vec![0xff, 0x41]);
        // NULL と len 0
        let buf = unsafe { ikari_convert_script(ptr::null(), 0, 1) };
        assert_eq!(buf.len, 0);
        unsafe { ikari_shell_buf_free(buf) };
    }
}
