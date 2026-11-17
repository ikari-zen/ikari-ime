//! 自分の束の Info.plist の文字列を読む（macOS。CoreFoundation）。署名要件は、build.sh が束の Info.plist
//! （署名の対象）に書く（今のエンジンの `IkariPeerRequirement` と同じ形。ADR 0077 §3.2）。コンテナは読まない。

use std::ffi::{CString, c_char, c_void};
use std::ptr;

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFBundleRef = *mut c_void;
type CFIndex = isize;
type CFStringEncoding = u32;
type CFTypeID = usize;

const UTF8: CFStringEncoding = 0x0800_0100;
/// 署名要件の文字列の長さの上限（これより長い値は読まない）
const MAX_VALUE_BYTES: usize = 4096;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFBundleGetMainBundle() -> CFBundleRef;
    fn CFBundleGetValueForInfoDictionaryKey(bundle: CFBundleRef, key: CFStringRef) -> CFTypeRef;
    fn CFStringCreateWithCString(
        alloc: *const c_void,
        text: *const c_char,
        encoding: CFStringEncoding,
    ) -> CFStringRef;
    fn CFStringGetCString(
        text: CFStringRef,
        buffer: *mut c_char,
        size: CFIndex,
        encoding: CFStringEncoding,
    ) -> u8;
    fn CFStringGetTypeID() -> CFTypeID;
    fn CFGetTypeID(object: CFTypeRef) -> CFTypeID;
    fn CFRelease(object: CFTypeRef);
}

/// 主の束の Info.plist の `key` の文字列（無い・文字列でない・長すぎるときは None）
pub(crate) fn main_bundle_string(key: &str) -> Option<String> {
    let key = CString::new(key).ok()?;
    // SAFETY: NUL で終わる文字列から CFString を作る（作ったものは下で解放する）
    let cf_key = unsafe { CFStringCreateWithCString(ptr::null(), key.as_ptr(), UTF8) };
    if cf_key.is_null() {
        return None;
    }
    // SAFETY: 主の束は解放しない値（Get の規則）。値も束が持つ（Get の規則。解放しない）
    let value = unsafe {
        let bundle = CFBundleGetMainBundle();
        let value = if bundle.is_null() {
            ptr::null()
        } else {
            CFBundleGetValueForInfoDictionaryKey(bundle, cf_key)
        };
        CFRelease(cf_key);
        value
    };
    // SAFETY: 型を確かめるだけ
    if value.is_null() || unsafe { CFGetTypeID(value) != CFStringGetTypeID() } {
        return None;
    }
    let mut buffer = vec![0 as c_char; MAX_VALUE_BYTES];
    let size = CFIndex::try_from(buffer.len()).ok()?;
    // SAFETY: 型を確かめた CFString を、長さを渡した領域へ写す
    let ok = unsafe { CFStringGetCString(value, buffer.as_mut_ptr(), size, UTF8) };
    if ok == 0 {
        return None;
    }
    let bytes: Vec<u8> = buffer
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8(bytes).ok()
}
