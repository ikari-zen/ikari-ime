//! 今の入力欄が secure（パスワード、PIN）かを、TSF の入力スコープから読む（ADR 0040 §6 S8。NFR-18）。
//!
//! 読む値を `secure::is_secure` に渡すだけ。判定の表と、「読めなかったときは通常」の理由は `secure` にある。
//! 読んだ結果（secure かどうか、スコープの値）は**ログに出さない**。読み取りの失敗も、ふつうに起きる
//! （入力スコープを付けない欄が大半）ので出さない。
//!
//! 入力スコープ（`GUID_PROP_INPUTSCOPE`）の値を読むには編集セッションの `ec` が要る。このため、キーの処理の
//! 編集セッションの中（`service::dispatch_and_run_inner`）で、`dispatch` の前に呼ぶ。

use std::ffi::c_void;

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Variant::{VT_UNKNOWN, VariantClear};
use windows::Win32::UI::TextServices::{
    GUID_PROP_INPUTSCOPE, IS_ALPHANUMERIC_PIN, IS_ALPHANUMERIC_PIN_SET, IS_NUMERIC_PIN,
    IS_PASSWORD, IS_PRIVATE, ITfContext, ITfInputScope, InputScope,
};
use windows_core::{IUnknown, Interface};

use super::log;
use crate::secure::{self, ScopeReading};

/// 1 つの欄の入力スコープの数の上限（アプリが壊れた個数を返したときに、大きな領域を読まないため）
const SCOPE_COUNT_SANITY_LIMIT: u32 = 64;

// 判定の表の値が、OS の定義と同じであること（コンパイルで確かめる）
const _: () = {
    assert!(secure::IS_PASSWORD == IS_PASSWORD.0);
    assert!(secure::IS_NUMERIC_PIN == IS_NUMERIC_PIN.0);
    assert!(secure::IS_ALPHANUMERIC_PIN == IS_ALPHANUMERIC_PIN.0);
    assert!(secure::IS_ALPHANUMERIC_PIN_SET == IS_ALPHANUMERIC_PIN_SET.0);
    assert!(secure::IS_PRIVATE == IS_PRIVATE.0);
};

/// この文脈の欄は secure か。`ec` は今の編集セッションのもの。読めなければ通常（`false`）。
pub fn is_secure(ec: u32, context: &ITfContext) -> bool {
    let scopes = read_scopes(ec, context);
    if let Reading::Scopes(list) = &scopes {
        log::input_scopes(list);
    }
    secure::is_secure(match &scopes {
        Reading::Scopes(list) => ScopeReading::Scopes(list),
        Reading::NotSet => ScopeReading::NotSet,
        Reading::Unreadable => ScopeReading::Unreadable,
    })
}

enum Reading {
    Scopes(Vec<i32>),
    NotSet,
    Unreadable,
}

fn read_scopes(ec: u32, context: &ITfContext) -> Reading {
    // SAFETY: `context` は生きている。プロパティはこの文脈のもの
    let Ok(property) = (unsafe { context.GetProperty(&GUID_PROP_INPUTSCOPE) }) else {
        return Reading::Unreadable;
    };
    // SAFETY: `ec` はこのセッションのもの
    let Ok(start) = (unsafe { context.GetStart(ec) }) else {
        return Reading::Unreadable;
    };
    // SAFETY: `ec` はこのセッションのもの。`start` は生きている
    let Ok(mut variant) = (unsafe { property.GetValue(ec, &start) }) else {
        return Reading::Unreadable;
    };

    // SAFETY: `GetValue` が初期化した VARIANT。`vt` を見てから、対応する共用体の項目だけを読む
    let reading = unsafe {
        let inner = &variant.Anonymous.Anonymous;
        if inner.vt == VT_UNKNOWN {
            let unknown: Option<&IUnknown> = (*inner.Anonymous.punkVal).as_ref();
            match unknown {
                Some(unknown) => match unknown.cast::<ITfInputScope>() {
                    Ok(scope) => scopes_of(&scope),
                    Err(_) => Reading::Unreadable,
                },
                None => Reading::NotSet,
            }
        } else {
            // VT_EMPTY（付いていない）など
            Reading::NotSet
        }
    };
    // SAFETY: `variant` は `GetValue` が返したもの。持っている参照をここで 1 回だけ解放する
    let _ = unsafe { VariantClear(&mut variant) };
    reading
}

/// # Safety
/// `scope` は生きている `ITfInputScope`。
unsafe fn scopes_of(scope: &ITfInputScope) -> Reading {
    let mut list: *mut InputScope = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: `list` と `count` は書ける場所。成功すると、`list` は `count` 個の要素（CoTaskMemAlloc）を指す
    if unsafe { scope.GetInputScopes(&mut list, &mut count) }.is_err() {
        return Reading::Unreadable;
    }
    if list.is_null() {
        return Reading::NotSet;
    }
    let len = count.min(SCOPE_COUNT_SANITY_LIMIT) as usize;
    // SAFETY: 上のとおり、`list` は少なくとも `count` 個の要素を指す。`len` はそれ以下
    let values: Vec<i32> = unsafe { std::slice::from_raw_parts(list, len) }
        .iter()
        .map(|scope| scope.0)
        .collect();
    // SAFETY: `GetInputScopes` が CoTaskMemAlloc した領域を、1 回だけ解放する
    unsafe { CoTaskMemFree(Some(list as *const c_void)) };
    Reading::Scopes(values)
}
