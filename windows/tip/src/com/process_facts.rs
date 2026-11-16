//! 自分のプロセスのトークンから、種類の判定に使う事実を取る（ADR 0067 §5。判定は `process_kind::classify`）。
//!
//! `OpenProcessToken` で自分のトークンを開き、`GetTokenInformation` の `TokenIntegrityLevel`（必須ラベルの SID の
//! 最後の部分）と `TokenIsAppContainer` を読む。取れなかった項目は `None` にする（判定は素通しに倒れる）。
//! 種類は、プロセスの寿命の間に 1 回だけ決める（`current_kind`）。

use std::sync::OnceLock;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_INFORMATION_CLASS,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel, TokenIsAppContainer,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::log;
use crate::process_kind::{ProcessFacts, ProcessKind, classify};

/// 開いたトークン（落とすときに閉じる）
struct Token(HANDLE);

impl Drop for Token {
    fn drop(&mut self) {
        // SAFETY: `OpenProcessToken` が開いたハンドルで、ほかで閉じない
        if let Err(e) = unsafe { CloseHandle(self.0) } {
            log::error("CloseHandle (token) failed", &e);
        }
    }
}

fn open_own_token() -> Option<Token> {
    let mut handle = HANDLE::default();
    // SAFETY: `GetCurrentProcess` は閉じなくてよい疑似ハンドル。`handle` には成功したときだけ開いたトークンが書かれる
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) }.ok()?;
    Some(Token(handle))
}

/// 大きさの決まらない情報を読む。ポインタを含む構造体のために、ポインタの境界に揃ったバッファ（`usize` の並び）で受ける
fn read_variable(token: &Token, class: TOKEN_INFORMATION_CLASS) -> Option<Vec<usize>> {
    let mut needed = 0u32;
    // SAFETY: 大きさを聞くだけ（バッファなし、長さ 0）。失敗（ERROR_INSUFFICIENT_BUFFER）で `needed` が埋まる
    let _ = unsafe { GetTokenInformation(token.0, class, None, 0, &mut needed) };
    if needed == 0 {
        return None;
    }
    let mut buf = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
    let len = u32::try_from(buf.len() * size_of::<usize>()).ok()?;
    // SAFETY: `buf` は `len` バイトあり、`usize` の境界に揃っていて、この呼び出しの間生きている
    unsafe {
        GetTokenInformation(
            token.0,
            class,
            Some(buf.as_mut_ptr().cast()),
            len,
            &mut needed,
        )
    }
    .ok()?;
    Some(buf)
}

/// 整合性のレベル（必須ラベルの SID の最後の部分）
fn integrity_rid(token: &Token) -> Option<u32> {
    let buf = read_variable(token, TokenIntegrityLevel)?;
    if buf.len() * size_of::<usize>() < size_of::<TOKEN_MANDATORY_LABEL>() {
        return None;
    }
    // SAFETY: 成功したので、`buf` の先頭は `TOKEN_MANDATORY_LABEL`（揃いと大きさは上で確かめた）。SID は `buf` の中を指し、
    // `buf` はこの関数の終わりまで生きている
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid };
    if sid.is_invalid() {
        return None;
    }
    // SAFETY: `sid` は上の有効な SID。戻り値は SID の中の数を指す
    let count = unsafe { GetSidSubAuthorityCount(sid) };
    if count.is_null() {
        return None;
    }
    // SAFETY: null でない（上で確かめた）
    let count = u32::from(unsafe { *count });
    let last = count.checked_sub(1)?;
    // SAFETY: `last` は数の範囲内。戻り値は SID の中の値を指す
    let rid = unsafe { GetSidSubAuthority(sid, last) };
    if rid.is_null() {
        return None;
    }
    // SAFETY: null でない（上で確かめた）。`buf` が生きている
    Some(unsafe { *rid })
}

/// AppContainer の中か（`TokenIsAppContainer` は DWORD。0 でなければ真）
fn app_container(token: &Token) -> Option<bool> {
    let mut value = 0u32;
    let mut written = 0u32;
    // SAFETY: `value` は書ける u32 で、長さはその大きさ。この呼び出しの間生きている
    unsafe {
        GetTokenInformation(
            token.0,
            TokenIsAppContainer,
            Some((&raw mut value).cast()),
            size_of::<u32>() as u32,
            &mut written,
        )
    }
    .ok()?;
    (written as usize == size_of::<u32>()).then_some(value != 0)
}

/// 自分のプロセスの事実。トークンが開けなければ何も取れなかったものとして返す
pub(super) fn current_process_facts() -> ProcessFacts {
    let Some(token) = open_own_token() else {
        return ProcessFacts::UNKNOWN;
    };
    ProcessFacts {
        integrity_rid: integrity_rid(&token),
        app_container: app_container(&token),
    }
}

/// このプロセスの種類。最初に呼んだときに 1 回だけ決め、種類の語をログに出す
pub(super) fn current_kind() -> ProcessKind {
    static KIND: OnceLock<ProcessKind> = OnceLock::new();
    *KIND.get_or_init(|| {
        let kind = classify(&current_process_facts());
        log::process_kind(kind.as_str());
        kind
    })
}
