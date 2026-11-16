//! 止める口の OS 層（`cfg(windows)`。Windows の CI か実機で確かめる）。名前つきのイベントを作り、合図を待つ。
//!
//! `unsafe` はこのファイルの Win32 の呼び出しだけ。

use std::ptr;
use std::thread;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, FALSE, GetLastError, HANDLE, HLOCAL, LocalFree,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};

use super::{StopPortError, WaitOutcome, stop_event_spec, watch};
use crate::run::StopHandle;

/// 作ったイベントのハンドル（閉じるのは落ちるとき）
struct EventHandle(HANDLE);

// SAFETY: イベントのハンドルは、どのスレッドからでも待てる（カーネルのオブジェクト）。所有は 1 つで、複製しない。
unsafe impl Send for EventHandle {}

impl Drop for EventHandle {
    fn drop(&mut self) {
        // SAFETY: 自分が持つ有効なハンドルを 1 回だけ閉じる
        unsafe { CloseHandle(self.0) };
    }
}

/// `LocalAlloc` された記述子の解放
struct LocalMem(PSECURITY_DESCRIPTOR);

impl Drop for LocalMem {
    fn drop(&mut self) {
        // SAFETY: `ConvertStringSecurityDescriptorToSecurityDescriptorW` が LocalAlloc したものを 1 回だけ解放する
        unsafe { LocalFree(self.0 as HLOCAL) };
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// イベントを作る。名前がすでにあれば（作ったのが自分でなければ）使わない。
fn create(name: &str, sddl: &str) -> Result<EventHandle, StopPortError> {
    let (name, sddl) = (to_wide(name), to_wide(sddl));
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: sddl は NUL で終わる UTF-16。descriptor には成功したときだけ LocalAlloc したものが書かれる（解放は下の LocalMem）
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    };
    if ok == FALSE {
        return Err(StopPortError::Create);
    }
    let _free = LocalMem(descriptor);
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: FALSE,
    };
    // SAFETY: attributes と name は呼び出しの間生きている。手動リセット、最初は合図なし
    let raw = unsafe { CreateEventW(&attributes, 1, 0, name.as_ptr()) };
    // SAFETY: 直前の呼び出しのエラー値（ほかの呼び出しを挟まない）
    let last_error = unsafe { GetLastError() };
    if raw.is_null() {
        return Err(StopPortError::Create);
    }
    let handle = EventHandle(raw);
    if last_error == ERROR_ALREADY_EXISTS {
        // 先に作ったのは自分でない。DACL も自分のものではないので使わない（落ちるときにハンドルを閉じる）
        return Err(StopPortError::NameTaken);
    }
    Ok(handle)
}

/// 止める口を作り、待つスレッドを起こす。
pub(super) fn start(stop: StopHandle) -> Result<(), StopPortError> {
    let sid = ikari_pipe::os::windows::current_user_sid().map_err(|_| StopPortError::Identity)?;
    let session =
        ikari_pipe::os::windows::current_session_id().map_err(|_| StopPortError::Identity)?;
    let (name, sddl) = stop_event_spec(&sid, session).map_err(|_| StopPortError::Identity)?;
    let event = create(&name, &sddl)?;
    thread::Builder::new()
        .name("ikari-stop-port".into())
        .spawn(move || {
            let event = event;
            let wait = || {
                // SAFETY: event は有効なハンドルで、この呼び出しの間生きている
                if unsafe { WaitForSingleObject(event.0, INFINITE) } == WAIT_OBJECT_0 {
                    WaitOutcome::Signaled
                } else {
                    WaitOutcome::Failed
                }
            };
            watch(wait, || stop.stop());
        })
        .map(|_| ())
        .map_err(|_| StopPortError::Spawn)
}
