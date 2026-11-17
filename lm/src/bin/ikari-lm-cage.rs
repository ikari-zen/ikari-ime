//! `ikari-lm-cage`: macOS の LM の檻役（XPC サービス `IkariLMCage.xpc` の実行ファイル。ADR 0077）。
//!
//! 殻が名前の口で匿名の口をもらい、エンジンに渡す。エンジンは匿名の口で、子（`ikari-lm-host`。この束の中）の起動と
//! 止めるを頼む。檻役は判断を持たず、決まった子を決まった引数で起動し、止めて回収するだけ（`ikari_lm::host::cage`）。
//! 子は檻役の App Sandbox を引き継ぐ。macOS 以外では何もせずに終わる。
//!
//! 相手の署名要件は、自分の束の Info.plist（署名の対象。build.sh が書く）から読む。コンテナは読まない。
//! CoreFoundation を使うのはこの bin だけ（ライブラリはエンジンの静的ライブラリにも入るので、リンクの一覧を増やさない）。

#[cfg(target_vendor = "apple")]
fn main() {
    use ikari_lm::host::cage::{
        INFO_KEY_ENGINE_PEER_REQUIREMENT, INFO_KEY_SHELL_PEER_REQUIREMENT, run_service,
    };
    let shell = info::main_bundle_string(INFO_KEY_SHELL_PEER_REQUIREMENT);
    let engine = info::main_bundle_string(INFO_KEY_ENGINE_PEER_REQUIREMENT);
    if shell.is_none() || engine.is_none() {
        eprintln!("ikari-lm-cage: no peer requirement; rejecting those connections");
    }
    run_service(shell.as_deref(), engine.as_deref())
}

#[cfg(not(target_vendor = "apple"))]
fn main() -> std::process::ExitCode {
    eprintln!("ikari-lm-cage: macOS only");
    std::process::ExitCode::FAILURE
}

/// 自分の束の Info.plist の文字列を読む（CoreFoundation）
#[cfg(target_vendor = "apple")]
mod info {
    use std::ffi::{CString, c_char, c_void};
    use std::ptr;

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFBundleRef = *mut c_void;
    type CFIndex = isize;
    type CFStringEncoding = u32;
    type CFTypeID = usize;

    /// `kCFStringEncodingUTF8`
    const UTF8: CFStringEncoding = 0x0800_0100;
    /// 署名要件の文字列の長さの上限（これより長い値は読まない）
    const MAX_VALUE_BYTES: usize = 4096;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFBundleGetMainBundle() -> CFBundleRef;
        fn CFBundleGetValueForInfoDictionaryKey(bundle: CFBundleRef, key: CFStringRef)
        -> CFTypeRef;
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
    pub(super) fn main_bundle_string(key: &str) -> Option<String> {
        let key = CString::new(key).ok()?;
        // SAFETY: NUL で終わる文字列から CFString を作る（作ったものは下で解放する）
        let cf_key = unsafe { CFStringCreateWithCString(ptr::null(), key.as_ptr(), UTF8) };
        if cf_key.is_null() {
            return None;
        }
        // SAFETY: 主の束と、その値は束が持つ（Get の規則。解放しない）。作ったキーだけを解放する
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
        // SAFETY: 値の型を確かめるだけ
        if value.is_null() || unsafe { CFGetTypeID(value) != CFStringGetTypeID() } {
            return None;
        }
        let mut buffer = vec![0 as c_char; MAX_VALUE_BYTES];
        let size = CFIndex::try_from(buffer.len()).ok()?;
        // SAFETY: 型を確かめた CFString を、長さを渡した領域へ写す
        if unsafe { CFStringGetCString(value, buffer.as_mut_ptr(), size, UTF8) } == 0 {
            return None;
        }
        let bytes: Vec<u8> = buffer
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as u8)
            .collect();
        String::from_utf8(bytes).ok()
    }
}
