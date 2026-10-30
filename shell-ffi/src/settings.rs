//! 設定アプリ（別のプロセス）用の口（ADR 0041 §2.3、§4.4。ヘッダの `ikari_settings_*`）。
//! 状態を持たないので、どのスレッドからでも呼べる。

use std::panic::{AssertUnwindSafe, catch_unwind};

use ikari_shell_core::SettingsError;
use ikari_shell_core::settings;

use crate::shell::{
    IKARI_SHELL_ERR_DECODE, IKARI_SHELL_ERR_NULL, IKARI_SHELL_ERR_PANIC,
    IKARI_SHELL_ERR_UNIMPLEMENTED, IKARI_SHELL_OK,
};
use crate::{IkariShellBuf, IkariStr, bytes, convert};

/// 出力を書き、状態を返す。`OK` 以外では空を書く。
fn finish(out: *mut IkariShellBuf, result: std::thread::Result<Result<Vec<u8>, i32>>) -> i32 {
    let (status, buf) = match result {
        Ok(Ok(encoded)) => (IKARI_SHELL_OK, IkariShellBuf::from_vec(encoded)),
        Ok(Err(status)) => (status, IkariShellBuf::EMPTY),
        Err(_) => (IKARI_SHELL_ERR_PANIC, IkariShellBuf::EMPTY),
    };
    // SAFETY: 呼び出し側で NULL を除いた。書き込める（ヘッダの約束）
    unsafe { out.write(buf) };
    status
}

fn utf8(raw: &[u8]) -> Result<&str, i32> {
    std::str::from_utf8(raw).map_err(|_| IKARI_SHELL_ERR_DECODE)
}

fn status_of(error: &SettingsError) -> i32 {
    match error {
        // `rewrite` の `IssuesAdded` は誤りではなく結果（SettingsRewrite.issues_added）。ここへは来ない
        SettingsError::IssuesAdded { .. } | SettingsError::Unimplemented => {
            IKARI_SHELL_ERR_UNIMPLEMENTED
        }
    }
}

/// # Safety
/// `text` は借用の文字列（読める `len` バイト、または NULL）。`out` は NULL か書き込めること。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_validate(text: IkariStr, out: *mut IkariShellBuf) -> i32 {
    if out.is_null() {
        return IKARI_SHELL_ERR_NULL;
    }
    // SAFETY: 借用の文字列（ヘッダの約束）
    let raw = unsafe { bytes(text.ptr, text.len) };
    let result = catch_unwind(AssertUnwindSafe(|| {
        let validation = settings::validate(utf8(raw)?).map_err(|e| status_of(&e))?;
        Ok(convert::encode_settings_validation(&validation))
    }));
    finish(out, result)
}

/// # Safety
/// `file` と `changes` は借用の文字列（読める `len` バイト、または NULL）。`out` は NULL か書き込めること。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_rewrite(
    file: IkariStr,
    changes: IkariStr,
    out: *mut IkariShellBuf,
) -> i32 {
    if out.is_null() {
        return IKARI_SHELL_ERR_NULL;
    }
    // SAFETY: 借用の文字列（ヘッダの約束）
    let (file, changes) = unsafe { (bytes(file.ptr, file.len), bytes(changes.ptr, changes.len)) };
    let result = catch_unwind(AssertUnwindSafe(|| {
        let file = utf8(file)?;
        let changes =
            convert::decode_settings_changes(changes).map_err(|_| IKARI_SHELL_ERR_DECODE)?;
        match settings::rewrite(file, &changes) {
            Ok(text) => Ok(convert::encode_settings_rewrite(Ok(&text))),
            Err(SettingsError::IssuesAdded { issues }) => {
                Ok(convert::encode_settings_rewrite(Err(&issues)))
            }
            Err(error) => Err(status_of(&error)),
        }
    }));
    finish(out, result)
}
