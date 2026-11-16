//! 絞った起動のテスト（Windows）。テストの実行ファイル自身を子として起動し、子の中と親の両方から確かめる。

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::mem::ManuallyDrop;
use std::os::windows::io::{AsHandle, AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{GetHandleInformation, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL,
    TOKEN_QUERY, TokenIntegrityLevel,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_CHAR, GetFileType};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_SYNCHRONIZE,
};

use super::*;

/// 子として起動されたときだけ動かすテスト
const PROBE_TEST: &str = "os::tests::probe_child";
const MIDDLE_TEST: &str = "os::tests::probe_middle";
/// 子に渡す、親が漏らしかけたハンドルの値（10 進）
const LEAK_ENV: &str = "IKARI_SANDBOX_TEST_LEAK";
/// 子に、止められるまで待たせる
const HOLD_ENV: &str = "IKARI_SANDBOX_TEST_HOLD";
const LEAK_BYTES: &[u8] = b"LEAK";
/// 低い整合性の RID（winnt.h の `SECURITY_MANDATORY_LOW_RID`）
const LOW_RID: u32 = 0x1000;
/// 子の行の印（libtest の出力と分ける）
const MARK: &str = "probe ";
/// 渡していない標準のハンドルの種類（`handle_kind`）
const NOT_PASSED: &str = "none";
const HOLD_FOR: Duration = Duration::from_secs(60);
/// 親が終わってから、孫が終わるまで待つ上限（ミリ秒）
const GRANDCHILD_EXIT_WAIT_MS: u32 = 10_000;

/// 子の中で動く側。渡されたハンドルの様子と、さらにプロセスを作れるかを、印つきの行で出す
#[test]
#[ignore = "ほかのテストが子として起動するときだけ動かす"]
fn probe_child() {
    let mut out = io::stdout();
    writeln!(
        out,
        "{MARK}stdin={}",
        handle_kind(io::stdin().as_raw_handle() as HANDLE)
    )
    .unwrap();
    writeln!(
        out,
        "{MARK}stderr={}",
        handle_kind(io::stderr().as_raw_handle() as HANDLE)
    )
    .unwrap();
    if let Some(value) = std::env::var(LEAK_ENV)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        // SAFETY: 受け継いでいなければ無効な値。書けるかを試すだけで、閉じない（`ManuallyDrop`）
        let mut file = ManuallyDrop::new(unsafe { File::from_raw_handle(value as HANDLE) });
        let wrote = file.write_all(LEAK_BYTES).is_ok();
        writeln!(out, "{MARK}leak={}", if wrote { "wrote" } else { "failed" }).unwrap();
    }
    let nested = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--list")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    writeln!(
        out,
        "{MARK}nested={}",
        if nested.is_ok() { "allowed" } else { "blocked" }
    )
    .unwrap();
    out.flush().unwrap();
    if std::env::var_os(HOLD_ENV).is_some() {
        std::thread::sleep(HOLD_FOR);
    }
}

/// 親の終了のテストの、まん中のプロセス。孫を起こして番号を出し、`Confined` を捨てずに終わる
#[test]
#[ignore = "the_child_ends_when_its_parent_ends が子として起動するときだけ動かす"]
fn probe_middle() {
    let (reader, writer) = io::pipe().unwrap();
    let writer = OwnedHandle::from(writer);
    let env = test_env(&[(HOLD_ENV, "1")]);
    let child = spawn(&Launch {
        program: &std::env::current_exe().unwrap(),
        args: &probe_args(PROBE_TEST),
        environment: Some(&env),
        integrity: Integrity::Low,
        job: JobLimits {
            max_processes: 1,
            ui_restricted: true,
        },
        stdio: Stdio {
            output: Some(writer.as_handle()),
            ..Stdio::default()
        },
    })
    .unwrap();
    drop(writer);
    // 孫が動き出した（行を出した）のを見てから
    let mut lines = BufReader::new(reader).lines();
    assert!(lines.next().is_some());
    println!("{MARK}grandchild={}", child.id());
    io::stdout().flush().unwrap();
    std::mem::forget(child);
    std::process::exit(0);
}

fn probe_args(test: &str) -> Vec<OsString> {
    [
        "--exact",
        test,
        "--ignored",
        "--nocapture",
        "--test-threads=1",
    ]
    .iter()
    .map(OsString::from)
    .collect()
}

/// 子に渡す環境。OS が動くのに要るものと、テストの変数だけ
fn test_env(extra: &[(&str, &str)]) -> Vec<u16> {
    let mut vars: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(k, _)| k.eq_ignore_ascii_case("SystemRoot"))
        .collect();
    vars.extend(
        extra
            .iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v))),
    );
    environment_block(vars).unwrap()
}

/// 印つきの行だけ
fn probe_lines(text: &str) -> Vec<&str> {
    // libtest の `test … ... ` が同じ行の前に付くことがある
    text.lines()
        .filter_map(|l| l.find(MARK).map(|at| &l[at + MARK.len()..]))
        .collect()
}

/// 子の標準のハンドルの種類。渡していないものは null か、子の新しいコンソール（`CREATE_NO_WINDOW` で OS が
/// 作る、窓のないコンソール）。親から受け継いだパイプやファイルなら `other`
fn handle_kind(handle: HANDLE) -> &'static str {
    if handle.is_null() {
        return NOT_PASSED;
    }
    // SAFETY: 種類を聞くだけ（無効な値なら FILE_TYPE_UNKNOWN が返る）
    match unsafe { GetFileType(handle) } {
        FILE_TYPE_CHAR => NOT_PASSED,
        _ => "other",
    }
}

/// プロセスのトークンの整合性の RID
fn integrity_rid(process: HANDLE) -> u32 {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: 生きているプロセスのハンドルと、書き込み先を渡すだけ
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
                raw(&token),
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

fn inherit_flag(handle: HANDLE) -> bool {
    let mut flags = 0u32;
    // SAFETY: 開いたハンドルの印を読むだけ
    assert_ne!(unsafe { GetHandleInformation(handle, &mut flags) }, 0);
    flags & HANDLE_FLAG_INHERIT != 0
}

#[test]
fn only_stdout_is_inherited_and_the_low_child_runs_in_the_job() {
    // 親が、継承できる印つきのハンドルを不注意に持っている形（一覧に入れなければ子に渡らない）
    let (mut leak_reader, leak_writer) = io::pipe().unwrap();
    let leak_writer = OwnedHandle::from(leak_writer);
    assert_ne!(
        // SAFETY: 持っているハンドルの印を変えるだけ
        unsafe {
            SetHandleInformation(raw(&leak_writer), HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT)
        },
        0
    );
    let leak_value = (raw(&leak_writer) as usize).to_string();
    let env = test_env(&[(LEAK_ENV, &leak_value)]);

    let (mut reader, writer) = io::pipe().unwrap();
    let writer = OwnedHandle::from(writer);
    let child = spawn(&Launch {
        program: &std::env::current_exe().unwrap(),
        args: &probe_args(PROBE_TEST),
        environment: Some(&env),
        integrity: Integrity::Low,
        job: JobLimits {
            max_processes: 1,
            ui_restricted: true,
        },
        stdio: Stdio {
            output: Some(writer.as_handle()),
            ..Stdio::default()
        },
    })
    .unwrap();
    // 渡したハンドルの継承の印は、起動のあとに外れている
    assert!(!inherit_flag(raw(&writer)));
    drop(writer);
    assert!(child.in_job().unwrap(), "子が Job に入っていない");
    assert_eq!(
        integrity_rid(child.process().as_raw_handle() as HANDLE),
        LOW_RID
    );

    let mut out = String::new();
    reader.read_to_string(&mut out).unwrap();
    assert_eq!(child.wait().unwrap(), 0, "{out}");
    let lines = probe_lines(&out);
    assert_eq!(
        lines,
        ["stdin=none", "stderr=none", "leak=failed", "nested=blocked"],
        "{out}"
    );
    // 子が書けていれば、ここに届く
    drop(leak_writer);
    let mut leaked = Vec::new();
    leak_reader.read_to_end(&mut leaked).unwrap();
    assert!(leaked.is_empty(), "子が一覧の外のハンドルを受け継いだ");
}

#[test]
fn two_processes_allow_one_nested_child_and_the_integrity_is_inherited() {
    let (mut reader, writer) = io::pipe().unwrap();
    let writer = OwnedHandle::from(writer);
    let child = spawn(&Launch {
        program: &std::env::current_exe().unwrap(),
        args: &probe_args(PROBE_TEST),
        environment: None,
        integrity: Integrity::Inherit,
        job: JobLimits {
            max_processes: 2,
            ui_restricted: false,
        },
        stdio: Stdio {
            output: Some(writer.as_handle()),
            ..Stdio::default()
        },
    })
    .unwrap();
    drop(writer);
    assert!(child.in_job().unwrap());
    // SAFETY: 自分のプロセスの擬似ハンドル
    let own = integrity_rid(unsafe { GetCurrentProcess() });
    assert_eq!(
        integrity_rid(child.process().as_raw_handle() as HANDLE),
        own
    );
    let mut out = String::new();
    reader.read_to_string(&mut out).unwrap();
    assert_eq!(child.wait().unwrap(), 0, "{out}");
    assert_eq!(
        probe_lines(&out),
        ["stdin=none", "stderr=none", "nested=allowed"],
        "{out}"
    );
}

#[test]
fn terminate_ends_the_child() {
    let (reader, writer) = io::pipe().unwrap();
    let writer = OwnedHandle::from(writer);
    let env = test_env(&[(HOLD_ENV, "1")]);
    let child = spawn(&Launch {
        program: &std::env::current_exe().unwrap(),
        args: &probe_args(PROBE_TEST),
        environment: Some(&env),
        integrity: Integrity::Low,
        job: JobLimits {
            max_processes: 1,
            ui_restricted: true,
        },
        stdio: Stdio {
            output: Some(writer.as_handle()),
            ..Stdio::default()
        },
    })
    .unwrap();
    drop(writer);
    let mut lines = BufReader::new(reader).lines();
    assert!(lines.next().is_some());
    let started = Instant::now();
    child.terminate();
    child.terminate();
    assert_eq!(child.wait().unwrap(), KILLED_EXIT_CODE);
    assert!(started.elapsed() < HOLD_FOR / 2);
}

#[test]
fn the_child_ends_when_its_parent_ends() {
    let mut middle = std::process::Command::new(std::env::current_exe().unwrap())
        .args(probe_args(MIDDLE_TEST))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut out = String::new();
    middle
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    middle.wait().unwrap();
    let pid: u32 = probe_lines(&out)
        .iter()
        .find_map(|l| l.strip_prefix("grandchild="))
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("{out}"));
    // SAFETY: 待つための権利だけで開く。失敗（もう無い）も、終わったとみなす
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if process.is_null() {
        return;
    }
    // SAFETY: 成功した `OpenProcess` が返したハンドル
    let process = unsafe { OwnedHandle::from_raw_handle(process) };
    // SAFETY: 開いたプロセスのハンドル
    let waited = unsafe { WaitForSingleObject(raw(&process), GRANDCHILD_EXIT_WAIT_MS) };
    assert_eq!(waited, WAIT_OBJECT_0, "親が終わっても孫が残った");
}

#[test]
fn environment_block_is_double_nul_terminated() {
    let os = OsString::from;
    let block = environment_block([(os("HTTPS_PROXY"), os("http://p:1"))]).unwrap();
    assert_eq!(
        String::from_utf16(&block).unwrap(),
        "HTTPS_PROXY=http://p:1\0\0"
    );
    assert_eq!(environment_block([]).unwrap(), [0, 0]);
    assert!(environment_block([(os("A"), os("b\0c"))]).is_err());
}

#[test]
fn a_bad_environment_block_is_refused_before_spawning() {
    let bad = [u16::from(b'A'), 0];
    let err = spawn(&Launch {
        program: &std::env::current_exe().unwrap(),
        args: &[],
        environment: Some(&bad),
        integrity: Integrity::Inherit,
        job: JobLimits {
            max_processes: 1,
            ui_restricted: true,
        },
        stdio: Stdio::default(),
    })
    .err()
    .unwrap();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}
