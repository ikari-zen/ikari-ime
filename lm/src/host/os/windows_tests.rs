//! Windows の子の絞りのテスト。テストの実行ファイル自身を子として起動し、子の中から確かめる。

use std::ffi::OsString;
use std::io::Read;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::process::{Command, Stdio};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL,
    TOKEN_QUERY, TokenIntegrityLevel,
};
use windows_sys::Win32::System::Threading::OpenProcessToken;

use super::{imp, spawn};

/// 子として起動されたときだけ動かすテストの名前（下の `probe_inside_the_job`）
const PROBE_TEST: &str = "host::os::windows_tests::probe_inside_the_job";
const NESTED_ALLOWED: &str = "nested=allowed";
const NESTED_BLOCKED: &str = "nested=blocked";
/// 低い整合性の RID（winnt.h の `SECURITY_MANDATORY_LOW_RID`）
const LOW_RID: u32 = 0x1000;

/// 子の中で動く側。さらにプロセスを作れるかを出力し、親が標準入力を閉じるまで待つ
#[test]
#[ignore = "the_child_runs_confined が子として起動するときだけ動かす"]
fn probe_inside_the_job() {
    let nested = Command::new(std::env::current_exe().unwrap())
        .arg("--list")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    println!(
        "{}",
        if nested.is_ok() {
            NESTED_ALLOWED
        } else {
            NESTED_BLOCKED
        }
    );
    let mut sink = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut sink);
}

/// 子のトークンの整合性の RID
fn integrity_rid(process: HANDLE) -> u32 {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: 生きている子のプロセスのハンドルと、書き込み先を渡すだけ
    assert_ne!(
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) },
        0
    );
    // SAFETY: 成功した `OpenProcessToken` が返したハンドル
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    // `TOKEN_MANDATORY_LABEL` と SID が入る大きさ。ポインタの境界に置く
    let mut buf = [0usize; 16];
    let mut len = 0u32;
    // SAFETY: `buf` は渡す大きさの、書き込める領域
    assert_ne!(
        unsafe {
            GetTokenInformation(
                token.as_raw_handle() as HANDLE,
                TokenIntegrityLevel,
                buf.as_mut_ptr().cast(),
                size_of_val(&buf) as u32,
                &mut len,
            )
        },
        0
    );
    // SAFETY: 成功した `GetTokenInformation` が `buf` に書いた `TOKEN_MANDATORY_LABEL` と、それが指す SID
    unsafe {
        let label = &*buf.as_ptr().cast::<TOKEN_MANDATORY_LABEL>();
        let count = *GetSidSubAuthorityCount(label.Label.Sid);
        *GetSidSubAuthority(label.Label.Sid, u32::from(count) - 1)
    }
}

#[test]
fn the_child_runs_confined() {
    let exe = std::env::current_exe().unwrap();
    let args: Vec<OsString> = ["--exact", PROBE_TEST, "--ignored", "--nocapture"]
        .iter()
        .map(OsString::from)
        .collect();
    let (proc, stdin, mut stdout) = spawn(&exe, &args).unwrap();
    if imp::LOW_INTEGRITY {
        assert_eq!(integrity_rid(proc.inner.process_handle()), LOW_RID);
    }
    drop(stdin);
    let mut out = String::new();
    stdout.read_to_string(&mut out).unwrap();
    assert!(out.contains(NESTED_BLOCKED), "{out}");
}
