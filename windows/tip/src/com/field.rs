//! 今の入力欄が secure（パスワード、PIN）かを、TSF の入力スコープから読む（ADR 0040 §6 S8。NFR-18）。
//!
//! 読む値を `secure::is_secure` に渡すだけ。判定の表と、「読めなかったときは通常」の理由は `secure` にある。
//!
//! # 読み方（#369）
//! 入力スコープはアプリ（テキストストア）が出す**アプリのプロパティ**なので、`ITfContext::GetAppProperty` で読む
//! （Mozc の `TipRangeUtil::GetInputScopes` と同じ）。以前は `ITfContext::GetProperty` で読んでいたが、こちらは TIP が
//! 値を置くプロパティの口で、アプリの入力スコープが入っている保証がない（VM の Edge と Win32 の欄で、1 度も値が読めて
//! いなかった）。`GetAppProperty` で読めなかったときだけ、以前の読み方も試す。範囲はキャレット（既定の選択）で、
//! 取れなければ文書の先頭。
//!
//! # 診断のログ
//! 読み方ごとの結果（入力スコープの値、値なし、失敗した呼び出しと HRESULT）と、文脈の
//! `GUID_COMPARTMENT_KEYBOARD_DISABLED`・`GUID_COMPARTMENT_EMPTYCONTEXT` の値を、前回と違うときに 1 行出す
//! （`log::input_scope`。最初の 1 回は空でも出る）。数値と固定の文だけで、欄の内容は出さない。DISABLED と EMPTYCONTEXT は
//! **読むだけで判定には使わない**（Chromium は password 欄の文脈にこの 2 つを立てる。使うかはユーザーの決定待ち）。
//!
//! 入力スコープの値を読むには編集セッションの `ec` が要る。このため、キーの処理の編集セッションの中
//! （`service::dispatch_and_run_inner`）で、`dispatch` の前に呼ぶ。

use std::ffi::c_void;

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Variant::{VARIANT, VT_UNKNOWN, VariantClear};
use windows::Win32::UI::TextServices::{
    GUID_COMPARTMENT_EMPTYCONTEXT, GUID_COMPARTMENT_KEYBOARD_DISABLED, GUID_PROP_INPUTSCOPE,
    IS_ALPHANUMERIC_PIN, IS_ALPHANUMERIC_PIN_SET, IS_NUMERIC_PIN, IS_PASSWORD, IS_PRIVATE,
    ITfCompartmentMgr, ITfContext, ITfInputScope, ITfRange, ITfReadOnlyProperty, InputScope,
};
use windows_core::{GUID, IUnknown, Interface};

use super::edit;
use super::log::{self, ScopeDiag, ScopeOutcome};
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
    let (range_kind, range) = match edit::default_selection(ec, context) {
        Some(range) => ("selection", Some(range)),
        // SAFETY: `ec` はこのセッションのもの
        None => match unsafe { context.GetStart(ec) } {
            Ok(range) => ("start", Some(range)),
            Err(_) => ("none", None),
        },
    };
    let (app, tip) = match &range {
        Some(range) => {
            // SAFETY: `context` は生きている
            let app = read_with(ec, range, "GetAppProperty", || unsafe {
                context.GetAppProperty(&GUID_PROP_INPUTSCOPE)
            });
            let tip = if matches!(app, ScopeOutcome::Scopes(_)) {
                ScopeOutcome::Skipped
            } else {
                // SAFETY: 同上
                read_with(ec, range, "GetProperty", || unsafe {
                    context
                        .GetProperty(&GUID_PROP_INPUTSCOPE)
                        .and_then(|property| property.cast::<ITfReadOnlyProperty>())
                })
            };
            (app, tip)
        }
        None => (ScopeOutcome::Skipped, ScopeOutcome::Skipped),
    };
    log::input_scope(&ScopeDiag {
        range: range_kind,
        app: &app,
        tip: &tip,
        disabled: context_flag(context, &GUID_COMPARTMENT_KEYBOARD_DISABLED),
        empty: context_flag(context, &GUID_COMPARTMENT_EMPTYCONTEXT),
    });
    // 読めた方の値で判定する（両方読めなければ、`app` の結果で「通常」）
    let chosen = match (&app, &tip) {
        (ScopeOutcome::Scopes(_), _) => &app,
        (_, ScopeOutcome::Scopes(_)) => &tip,
        _ => &app,
    };
    secure::is_secure(match chosen {
        ScopeOutcome::Scopes(list) => ScopeReading::Scopes(list),
        ScopeOutcome::NotSet { .. } => ScopeReading::NotSet,
        ScopeOutcome::Failed { .. } | ScopeOutcome::Skipped => ScopeReading::Unreadable,
    })
}

/// 文脈の compartment の値（診断用。読めなければ `None`）
fn context_flag(context: &ITfContext, guid: &GUID) -> Option<i32> {
    let compartments: ITfCompartmentMgr = context.cast().ok()?;
    // SAFETY: `guid` は定数への参照
    let compartment = unsafe { compartments.GetCompartment(guid) }.ok()?;
    // SAFETY: COM の呼び出し。値がなければ空の VARIANT が返る（`None`）
    let variant = unsafe { compartment.GetValue() }.ok()?;
    i32::try_from(&variant).ok()
}

/// `get` で取ったプロパティから、`range` の入力スコープを読む
fn read_with(
    ec: u32,
    range: &ITfRange,
    step: &'static str,
    get: impl FnOnce() -> windows_core::Result<ITfReadOnlyProperty>,
) -> ScopeOutcome {
    let property = match get() {
        Ok(property) => property,
        Err(e) => return ScopeOutcome::Failed { step, hr: e.code() },
    };
    // SAFETY: `ec` はこのセッションのもの。`range` は生きている
    let mut variant = match unsafe { property.GetValue(ec, range) } {
        Ok(variant) => variant,
        Err(e) => {
            return ScopeOutcome::Failed {
                step: "GetValue",
                hr: e.code(),
            };
        }
    };
    let outcome = scopes_in(&variant);
    // SAFETY: `variant` は `GetValue` が返したもの。持っている参照をここで 1 回だけ解放する
    let _ = unsafe { VariantClear(&mut variant) };
    outcome
}

fn scopes_in(variant: &VARIANT) -> ScopeOutcome {
    // SAFETY: `GetValue` が初期化した VARIANT。`vt` を見てから、対応する共用体の項目だけを読む
    unsafe {
        let inner = &variant.Anonymous.Anonymous;
        if inner.vt != VT_UNKNOWN {
            // VT_EMPTY（付いていない）など
            return ScopeOutcome::NotSet { vt: inner.vt.0 };
        }
        let unknown: Option<&IUnknown> = (*inner.Anonymous.punkVal).as_ref();
        match unknown {
            Some(unknown) => match unknown.cast::<ITfInputScope>() {
                Ok(scope) => scopes_of(&scope),
                Err(e) => ScopeOutcome::Failed {
                    step: "QueryInterface(ITfInputScope)",
                    hr: e.code(),
                },
            },
            None => ScopeOutcome::NotSet { vt: inner.vt.0 },
        }
    }
}

/// # Safety
/// `scope` は生きている `ITfInputScope`。
unsafe fn scopes_of(scope: &ITfInputScope) -> ScopeOutcome {
    let mut list: *mut InputScope = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: `list` と `count` は書ける場所。成功すると、`list` は `count` 個の要素（CoTaskMemAlloc）を指す
    if let Err(e) = unsafe { scope.GetInputScopes(&mut list, &mut count) } {
        return ScopeOutcome::Failed {
            step: "GetInputScopes",
            hr: e.code(),
        };
    }
    if list.is_null() {
        return ScopeOutcome::Scopes(Vec::new());
    }
    let len = count.min(SCOPE_COUNT_SANITY_LIMIT) as usize;
    // SAFETY: 上のとおり、`list` は少なくとも `count` 個の要素を指す。`len` はそれ以下
    let values: Vec<i32> = unsafe { std::slice::from_raw_parts(list, len) }
        .iter()
        .map(|scope| scope.0)
        .collect();
    // SAFETY: `GetInputScopes` が CoTaskMemAlloc した領域を、1 回だけ解放する
    unsafe { CoTaskMemFree(Some(list as *const c_void)) };
    ScopeOutcome::Scopes(values)
}
