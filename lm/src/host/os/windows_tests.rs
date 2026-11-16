//! Windows の子の絞りのテスト。テストの実行ファイル自身を子として起動し、子の中から確かめる。
//! Windows でしか動かない（CI の Windows と VM）。段 2 のテストは、利用者ごとのテスト用のプロファイル
//! （`appcontainer::PROFILE_NAME`。単体テストでは製品と別の名前）を作り、最後に消す。

use std::ffi::OsString;
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, EqualSid, GetSidSubAuthority, GetSidSubAuthorityCount,
    GetTokenInformation, PSECURITY_DESCRIPTOR, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
    TokenIntegrityLevel, TokenIsAppContainer,
};
use windows_sys::Win32::System::Threading::OpenProcessToken;

use super::appcontainer;
use super::{Cage, CageState, imp, spawn};

/// 子として起動されたときだけ動かすテストの名前（下の `probe_inside_the_job`）
const PROBE_TEST: &str = "host::os::windows_tests::probe_inside_the_job";
const NESTED_ALLOWED: &str = "nested=allowed";
const NESTED_BLOCKED: &str = "nested=blocked";
/// 段 2 の子として起動されたときだけ動かすテストの名前（下の `probe_inside_the_appcontainer`）
const AC_PROBE_TEST: &str = "host::os::windows_tests::probe_inside_the_appcontainer";
/// 段 2 の子が、つなぎに行く時間の上限（拒まれるなら、すぐ返る見込み。VM では待たずに拒まれた）
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
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

/// 段 2 の子の中で動く側。標準入力から、読めるはずのファイル、読めないはずのファイル、つなぎに行く番号を
/// 1 行ずつ受け、それぞれの結果と、さらにプロセスを作れるかを出力する
#[test]
#[ignore = "the_appcontainer_child_reads_only_the_granted_file が子として起動するときだけ動かす"]
fn probe_inside_the_appcontainer() {
    let mut lines = std::io::stdin().lock().lines();
    let mut next = || lines.next().unwrap().unwrap();
    let (granted, other, port) = (next(), next(), next());
    let read = |path: &str| match std::fs::read(path) {
        Ok(_) => "ok",
        Err(_) => "denied",
    };
    println!("granted={}", read(&granted));
    println!("other={}", read(&other));
    let write = std::fs::write(Path::new(&granted).with_extension("new"), b"x");
    println!("write={}", if write.is_ok() { "ok" } else { "denied" });
    let addr = format!("127.0.0.1:{port}").parse().unwrap();
    let net = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT);
    println!("net={}", if net.is_ok() { "ok" } else { "denied" });
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
}

fn open_token(process: HANDLE) -> OwnedHandle {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: 生きている子のプロセスのハンドルと、書き込み先を渡すだけ
    assert_ne!(
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) },
        0
    );
    // SAFETY: 成功した `OpenProcessToken` が返したハンドル
    unsafe { OwnedHandle::from_raw_handle(token) }
}

/// 子のトークンが AppContainer か
fn is_appcontainer(process: HANDLE) -> bool {
    let token = open_token(process);
    let mut value = 0u32;
    let mut len = 0u32;
    // SAFETY: `value` は渡す大きさの、書き込める領域
    assert_ne!(
        unsafe {
            GetTokenInformation(
                token.as_raw_handle() as HANDLE,
                TokenIsAppContainer,
                std::ptr::from_mut(&mut value).cast(),
                size_of::<u32>() as u32,
                &mut len,
            )
        },
        0
    );
    value != 0
}

/// 子のトークンの整合性の RID
fn integrity_rid(process: HANDLE) -> u32 {
    let token = open_token(process);
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
    let (proc, stdin, mut stdout) = spawn(&exe, &args, Cage::Standard).unwrap();
    assert!(!is_appcontainer(proc.inner.process_handle()));
    if imp::low_integrity_enabled() {
        assert_eq!(proc.cage(), CageState::LowIntegrity);
        assert_eq!(integrity_rid(proc.inner.process_handle()), LOW_RID);
    } else {
        assert_eq!(proc.cage(), CageState::None);
    }
    drop(stdin);
    let mut out = String::new();
    stdout.read_to_string(&mut out).unwrap();
    assert!(out.contains(NESTED_BLOCKED), "{out}");
}

#[test]
fn the_appcontainer_child_reads_only_the_granted_file() {
    let dir = tempfile::tempdir().unwrap();
    let granted = dir.path().join("granted.gguf");
    let other = dir.path().join("other.txt");
    std::fs::write(&granted, b"model").unwrap();
    std::fs::write(&other, b"not for the child").unwrap();
    // 親の側では、どちらも読める（子が読めないのは、檻のせい）
    assert!(std::fs::read(&other).is_ok());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let exe = std::env::current_exe().unwrap();
    let args: Vec<OsString> = ["--exact", AC_PROBE_TEST, "--ignored", "--nocapture"]
        .iter()
        .map(OsString::from)
        .collect();
    let (proc, mut stdin, mut stdout) =
        spawn(&exe, &args, Cage::AppContainer { readable: &granted }).unwrap();
    assert_eq!(proc.cage(), CageState::AppContainer);
    assert!(is_appcontainer(proc.inner.process_handle()));
    assert_eq!(integrity_rid(proc.inner.process_handle()), LOW_RID);
    writeln!(stdin, "{}\n{}\n{port}", granted.display(), other.display()).unwrap();
    drop(stdin);
    let mut out = String::new();
    stdout.read_to_string(&mut out).unwrap();
    drop(proc);
    for expected in [
        "granted=ok",
        "other=denied",
        "write=denied",
        "net=denied",
        NESTED_BLOCKED,
    ] {
        assert!(out.contains(expected), "{expected}: {out}");
    }
    appcontainer::delete_profile().unwrap();
}

/// ファイルの DACL の ACE の数
fn ace_count(path: &Path) -> u16 {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: NUL で終わるパスと、書き込み先を渡すだけ
    let rc = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    assert_eq!(rc, ERROR_SUCCESS);
    assert!(!dacl.is_null(), "DACL がない");
    // SAFETY: 成功した呼び出しが返した DACL（`sd` の中。null でないと確かめた）を読み、`sd` を 1 回だけ解放する
    unsafe {
        let count = (*dacl).AceCount;
        LocalFree(sd);
        count
    }
}

#[test]
fn the_profile_and_the_grant_are_idempotent() {
    let first = appcontainer::ensure_profile().unwrap();
    let second = appcontainer::ensure_profile().unwrap();
    // SAFETY: どちらも、生きている SID
    assert_ne!(unsafe { EqualSid(first.as_psid(), second.as_psid()) }, 0);

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("model.gguf");
    std::fs::write(&file, b"model").unwrap();
    let before = ace_count(&file);
    appcontainer::grant_read(&file, &first).unwrap();
    let once = ace_count(&file);
    appcontainer::grant_read(&file, &first).unwrap();
    assert_eq!(once, before + 1);
    assert_eq!(ace_count(&file), once);
}
