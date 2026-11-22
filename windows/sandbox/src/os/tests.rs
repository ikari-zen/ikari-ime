//! 絞った起動のテスト（Windows）。テストの実行ファイル自身を子として起動し、子の中と親の両方から確かめる。

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::mem::ManuallyDrop;
use std::os::windows::io::{AsHandle, AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{GetHandleInformation, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    DACL_SECURITY_INFORMATION, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel,
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
        job: JobLimits { max_processes: 1 },
        stdio: Stdio {
            output: Some(writer.as_handle()),
            ..Stdio::default()
        },
        inherit: &[],
        container: None,
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
        job: JobLimits { max_processes: 1 },
        stdio: Stdio {
            output: Some(writer.as_handle()),
            ..Stdio::default()
        },
        inherit: &[],
        container: None,
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
        ["stdin=none", "stderr=none", "leak=failed", "nested=blocked",],
        "{out}"
    );
    // 子が書けていれば、ここに届く
    drop(leak_writer);
    let mut leaked = Vec::new();
    leak_reader.read_to_end(&mut leaked).unwrap();
    assert!(leaked.is_empty(), "子が一覧の外のハンドルを受け継いだ");
}

#[test]
fn a_handle_in_inherit_reaches_the_child_together_with_stdout() {
    // `probe_child` は、`LEAK_ENV` の値のハンドルに書いてみる。`inherit` に入れたので、今度は届く
    let (mut extra_reader, extra_writer) = io::pipe().unwrap();
    let extra_writer = OwnedHandle::from(extra_writer);
    let value = (raw(&extra_writer) as usize).to_string();
    let env = test_env(&[(LEAK_ENV, &value)]);
    let (mut reader, writer) = io::pipe().unwrap();
    let writer = OwnedHandle::from(writer);
    let child = spawn(&Launch {
        program: &std::env::current_exe().unwrap(),
        args: &probe_args(PROBE_TEST),
        environment: Some(&env),
        integrity: Integrity::Low,
        job: JobLimits { max_processes: 1 },
        stdio: Stdio {
            output: Some(writer.as_handle()),
            ..Stdio::default()
        },
        inherit: &[extra_writer.as_handle()],
        container: None,
    })
    .unwrap();
    // どちらの継承の印も、起動のあとに外れている
    assert!(!inherit_flag(raw(&writer)));
    assert!(!inherit_flag(raw(&extra_writer)));
    drop(writer);
    let mut out = String::new();
    reader.read_to_string(&mut out).unwrap();
    assert_eq!(child.wait().unwrap(), 0, "{out}");
    assert!(probe_lines(&out).contains(&"leak=wrote"), "{out}");
    drop(extra_writer);
    let mut got = Vec::new();
    extra_reader.read_to_end(&mut got).unwrap();
    assert_eq!(got, LEAK_BYTES);
}

#[test]
fn repeated_or_overlapping_inherit_handles_are_refused_before_spawning() {
    let (_reader, writer) = io::pipe().unwrap();
    let writer = OwnedHandle::from(writer);
    let (_other_reader, other) = io::pipe().unwrap();
    let other = OwnedHandle::from(other);
    let launch = |stdio: Stdio<'_>, inherit: &[BorrowedHandle<'_>]| {
        spawn(&Launch {
            program: &std::env::current_exe().unwrap(),
            args: &probe_args(EXIT_TEST),
            environment: None,
            integrity: Integrity::Inherit,
            job: JobLimits { max_processes: 1 },
            stdio,
            inherit,
            container: None,
        })
        .err()
        .map(|e| e.kind())
    };
    let with_stdout = Stdio {
        output: Some(writer.as_handle()),
        ..Stdio::default()
    };
    assert_eq!(
        launch(with_stdout, &[writer.as_handle()]),
        Some(io::ErrorKind::InvalidInput),
        "標準出力と重なる"
    );
    assert_eq!(
        launch(Stdio::default(), &[other.as_handle(), other.as_handle()]),
        Some(io::ErrorKind::InvalidInput),
        "同じものが 2 度"
    );
    // 断ったときも、印は付けたままにしない
    assert!(!inherit_flag(raw(&writer)));
    assert!(!inherit_flag(raw(&other)));
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
        job: JobLimits { max_processes: 2 },
        stdio: Stdio {
            output: Some(writer.as_handle()),
            ..Stdio::default()
        },
        inherit: &[],
        container: None,
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
        job: JobLimits { max_processes: 1 },
        stdio: Stdio {
            output: Some(writer.as_handle()),
            ..Stdio::default()
        },
        inherit: &[],
        container: None,
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
        job: JobLimits { max_processes: 1 },
        stdio: Stdio::default(),
        inherit: &[],
        container: None,
    })
    .err()
    .unwrap();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}

/// 入れ子の起動（設定画面の核 → `ikari-data` → `transfer` の形）の、まん中の Job の中で動く側
const NESTED_MIDDLE_TEST: &str = "os::tests::probe_nested_middle";
/// 実機で手で流す `transfer_runs_inside_both_jobs` の、`ikari-data.exe` の場所、URL、出力先（親が追記だけで新しく作り、
/// ハンドルで渡す）。`ikari-data.exe` の場所が無ければ、まん中は孫として `probe_child` を起こす
const TRANSFER_EXE_ENV: &str = "IKARI_SANDBOX_TEST_DATA_EXE";
const TRANSFER_URL_ENV: &str = "IKARI_SANDBOX_TEST_URL";
const TRANSFER_OUT_ENV: &str = "IKARI_SANDBOX_TEST_OUT";
const TRANSFER_MAX_SIZE: &str = "16777216";

/// `ikari-data` の役。自分の Job の中から、孫を子 1 の Job に入れて起こし、孫の行と終了コードを印つきで出す。
/// 孫が `transfer` なら低い整合性、`probe_child` なら親と同じ（UI の制限が整合性でなく Job から来ることを見るため）
#[test]
#[ignore = "入れ子のテストが子として起動するときだけ動かす"]
fn probe_nested_middle() {
    let integrity = if std::env::var_os(TRANSFER_EXE_ENV).is_some() {
        Integrity::Low
    } else {
        Integrity::Inherit
    };
    // `transfer` の書く先（追記だけで開いたファイル）。`--out-handle` で知らせる（ADR 0078 §3.1）
    let mut out_file = None;
    let (program, args) = match std::env::var(TRANSFER_EXE_ENV) {
        Ok(exe) => {
            let var = |name: &str| std::env::var(name).unwrap();
            let file = std::fs::OpenOptions::new()
                .append(true)
                .create_new(true)
                .open(var(TRANSFER_OUT_ENV))
                .unwrap();
            let args = [
                "transfer".to_owned(),
                "--url".to_owned(),
                var(TRANSFER_URL_ENV),
                "--max-size".to_owned(),
                TRANSFER_MAX_SIZE.to_owned(),
                "--out-handle".to_owned(),
                (file.as_raw_handle() as usize).to_string(),
            ];
            out_file = Some(file);
            (
                std::path::PathBuf::from(exe),
                args.into_iter().map(OsString::from).collect(),
            )
        }
        Err(_) => (std::env::current_exe().unwrap(), probe_args(PROBE_TEST)),
    };
    let inherit: Vec<BorrowedHandle<'_>> = out_file.iter().map(|f| f.as_handle()).collect();
    let (mut reader, writer) = io::pipe().unwrap();
    let writer = OwnedHandle::from(writer);
    let spawned = spawn(&Launch {
        program: &program,
        args: &args,
        environment: None,
        integrity,
        job: JobLimits { max_processes: 1 },
        stdio: Stdio {
            output: Some(writer.as_handle()),
            ..Stdio::default()
        },
        inherit: &inherit,
        container: None,
    });
    // 標準エラーは親に渡っていないので、起こせなかった理由は印つきの行で出す
    let child = match spawned {
        Ok(child) => child,
        Err(e) => {
            println!("{MARK}spawn-error={e}");
            return;
        }
    };
    drop(writer);
    let mut out = String::new();
    reader.read_to_string(&mut out).unwrap();
    let code = child.wait().unwrap();
    for line in out.lines() {
        // 孫が `probe_child` なら、印のある行だけ（libtest の行は捨てる）。`transfer` なら全部の行に印を付ける
        match line.find(MARK) {
            Some(at) => println!("{}", &line[at..]),
            None if integrity == Integrity::Low => println!("{MARK}{line}"),
            None => {}
        }
    }
    println!("{MARK}exit={code}");
}

/// 外の Job（子 2）にまん中を入れて起こし、まん中の印つきの行を返す
fn run_nested(environment: Option<&[u16]>) -> String {
    let (mut reader, writer) = io::pipe().unwrap();
    let writer = OwnedHandle::from(writer);
    let child = spawn(&Launch {
        program: &std::env::current_exe().unwrap(),
        args: &probe_args(NESTED_MIDDLE_TEST),
        environment,
        integrity: Integrity::Inherit,
        job: JobLimits { max_processes: 2 },
        stdio: Stdio {
            output: Some(writer.as_handle()),
            ..Stdio::default()
        },
        inherit: &[],
        container: None,
    })
    .unwrap();
    drop(writer);
    let mut out = String::new();
    reader.read_to_string(&mut out).unwrap();
    child.wait().unwrap();
    out
}

/// UI の制限つきの Job の中から、さらに絞った子（UI の制限つきの Job に入れる形）を起こせること。
/// 孫に UI の制限が効いていること（外の Job の制限が入れ子の中にもかかること）は、このテストでは確かめていない
#[test]
fn a_nested_child_starts_inside_a_ui_restricted_job() {
    let env = test_env(&[]);
    let out = run_nested(Some(&env));
    assert_eq!(
        probe_lines(&out),
        ["stdin=none", "stderr=none", "nested=blocked", "exit=0"],
        "{out}"
    );
}

#[test]
#[ignore = "実機で手で流す（通信する。環境変数が要る）"]
fn transfer_runs_inside_both_jobs() {
    let out = run_nested(None);
    println!("{out}");
    let lines = probe_lines(&out);
    assert!(lines.contains(&"exit=0"), "{out}");
    assert!(lines.iter().any(|l| l.starts_with("progress ")), "{out}");
}

/// すぐ終わる子の終了コード
const QUICK_EXIT_CODE: u32 = 7;
const EXIT_TEST: &str = "os::tests::probe_exit";
/// 長く動く子を止められなかったときに、勝手に終わるまでの回数（`ping -n`。止められれば使わない）
const LONG_PING_COUNT: &str = "30";

fn system32(exe: &str) -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").expect("SystemRoot");
    std::path::Path::new(&root).join("System32").join(exe)
}

fn spawn_quick(integrity: Integrity) -> Confined {
    spawn(&Launch {
        program: &std::env::current_exe().unwrap(),
        args: &probe_args(EXIT_TEST),
        environment: None,
        integrity,
        job: JobLimits { max_processes: 1 },
        stdio: Stdio::default(),
        inherit: &[],
        container: None,
    })
    .unwrap()
}

/// 子として起動されたときだけ、決めた終了コードで終わる（cmd.exe は引用符つきの引数を読めないので使わない）
#[test]
#[ignore = "ほかのテストが子として起動するときだけ動かす"]
fn probe_exit() {
    std::process::exit(QUICK_EXIT_CODE as i32);
}

fn spawn_long(integrity: Integrity) -> Confined {
    let args: Vec<OsString> = ["-n", LONG_PING_COUNT, "127.0.0.1"]
        .iter()
        .map(OsString::from)
        .collect();
    spawn(&Launch {
        program: &system32("PING.EXE"),
        args: &args,
        environment: None,
        integrity,
        job: JobLimits { max_processes: 1 },
        stdio: Stdio::default(),
        inherit: &[],
        container: None,
    })
    .unwrap()
}

/// 子のプロセスの DACL を SDDL の文字列で返す
fn dacl_sddl(process: HANDLE) -> String {
    let mut descriptor: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: 生きているプロセスのハンドル（作成時のもの。READ_CONTROL がある）と、書き込み先
    let status = unsafe {
        GetSecurityInfo(
            process,
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    assert_eq!(status, 0, "GetSecurityInfo");
    let mut text: *mut u16 = ptr::null_mut();
    // SAFETY: `GetSecurityInfo` が返した記述子と、書き込み先
    let ok = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            SDDL_REVISION,
            DACL_SECURITY_INFORMATION,
            &mut text,
            ptr::null_mut(),
        )
    };
    assert_ne!(
        ok, 0,
        "ConvertSecurityDescriptorToStringSecurityDescriptorW"
    );
    // SAFETY: 成功した変換が返した、NUL で終わる文字列
    let sddl = unsafe {
        let mut len = 0;
        while *text.add(len) != 0 {
            len += 1;
        }
        String::from_utf16(std::slice::from_raw_parts(text, len)).unwrap()
    };
    // SAFETY: それぞれ OS が `LocalAlloc` で確保したものを 1 回だけ解放する
    unsafe {
        LocalFree(text.cast());
        LocalFree(descriptor);
    }
    sddl
}

/// SDDL の ACE を（種類、権利、SID）に分ける。権利は 16 進（`0x…`）か、`GA`・`RC` の記号
fn aces(sddl: &str) -> Vec<(String, String, String)> {
    sddl.split('(')
        .skip(1)
        .map(|ace| {
            let fields: Vec<&str> = ace.trim_end_matches(')').split(';').collect();
            assert_eq!(fields.len(), 6, "{sddl}");
            (
                fields[0].to_owned(),
                fields[2].to_owned(),
                fields[5].to_owned(),
            )
        })
        .collect()
}

/// 権利が、全権（`GA`、または 0x1fffff）か
fn is_full(rights: &str) -> bool {
    rights == "GA" || rights.eq_ignore_ascii_case("0x1fffff")
}

fn check_child_dacl(integrity: Integrity) {
    let child = spawn_long(integrity);
    let sddl = dacl_sddl(child.process().as_raw_handle() as HANDLE);
    child.terminate();
    child.wait().unwrap();
    assert!(sddl.starts_with("D:"), "{sddl}");
    let aces = aces(&sddl);
    assert_eq!(aces.len(), 3, "{sddl}");
    for (kind, rights, sid) in &aces {
        assert_eq!(kind, "A", "{sddl}");
        match sid.as_str() {
            "SY" | "BA" => assert!(is_full(rights), "{sddl}"),
            "OW" => assert!(
                rights == "RC" || rights.eq_ignore_ascii_case("0x20000"),
                "{sddl}"
            ),
            other => panic!("想定外の SID {other}: {sddl}"),
        }
    }
}

#[test]
fn a_low_child_with_a_job_returns_its_exit_code() {
    let child = spawn_quick(Integrity::Low);
    assert!(child.in_job().unwrap());
    assert_eq!(child.wait().unwrap(), QUICK_EXIT_CODE);
}

#[test]
fn a_normal_child_with_a_job_returns_its_exit_code() {
    let child = spawn_quick(Integrity::Inherit);
    assert!(child.in_job().unwrap());
    assert_eq!(child.wait().unwrap(), QUICK_EXIT_CODE);
}

#[test]
fn a_long_running_child_is_stopped_through_the_job() {
    for integrity in [Integrity::Low, Integrity::Inherit] {
        let child = spawn_long(integrity);
        child.terminate();
        assert_eq!(child.wait().unwrap(), KILLED_EXIT_CODE, "{integrity:?}");
    }
}

#[test]
fn a_long_running_child_is_stopped_through_its_handle() {
    for integrity in [Integrity::Low, Integrity::Inherit] {
        let child = spawn_long(integrity);
        // SAFETY: 親が持つ、生きているプロセスのハンドル（作成時の権利に終了がある）
        let ok = unsafe {
            TerminateProcess(child.process().as_raw_handle() as HANDLE, KILLED_EXIT_CODE)
        };
        assert_ne!(ok, 0, "{integrity:?}");
        assert_eq!(child.wait().unwrap(), KILLED_EXIT_CODE, "{integrity:?}");
    }
}

#[test]
fn the_low_child_process_dacl_is_restricted() {
    check_child_dacl(Integrity::Low);
}

#[test]
fn the_normal_child_process_dacl_is_restricted() {
    check_child_dacl(Integrity::Inherit);
}

/// AppContainer のテストのプロファイル（製品のプロファイルに触れない。最後に消す）
const TEST_PROFILE: crate::appcontainer::ContainerProfile = crate::appcontainer::ContainerProfile {
    name: "io.github.ikari-zen.sandbox-test",
    display_name: "ikari sandbox test",
};

/// 環境の切り分けのテストのプロファイル（並べて流れるほかのテストの削除とぶつからないように分ける）
const ENV_TEST_PROFILE: crate::appcontainer::ContainerProfile =
    crate::appcontainer::ContainerProfile {
        name: "io.github.ikari-zen.sandbox-env-test",
        display_name: "ikari sandbox test",
    };

/// AppContainer のプロファイルを使うテストを、1 つずつ流す鍵（#522）。テストは並べて流れるので、同じプロファイルを
/// 2 つのテストが同時に作る・消すと、片方の `spawn` が `ERROR_FILE_NOT_FOUND`（プロファイルが無い）で落ちる
/// （CI の windows ジョブで 3 回とも、`TEST_PROFILE` を使う 2 つのテストが同じ時刻に流れていた）。名前を分けても、
/// プロファイルの作成と削除が同時に流れることは変わらないので、AppContainer を使うテストは全部この鍵を持つ。
/// テストが落ちて鍵が汚れても（poison）、次のテストは流す
fn appcontainer_serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// GitHub Actions の runner では、AppContainer のプロファイルが作れない（`CreateAppContainerProfile` が一定しない
/// エラーで失敗する。VM と実機では動く。ADR 0072 §12、`lm` の段 2 のテストと同じ扱い）。CI（`GITHUB_ACTIONS` が
/// 立っている）で作れないときだけ `None`（テストを飛ばす）。手元・VM・実機では、作れなければ落とす
fn test_container(profile: &crate::appcontainer::ContainerProfile) -> Option<Container> {
    use crate::appcontainer::Capability;
    match Container::ensure(profile, &[Capability::InternetClient]) {
        Ok(container) => Some(container),
        Err(e) if std::env::var_os("GITHUB_ACTIONS").is_some() => {
            eprintln!("skip: CI の runner では AppContainer のプロファイルが作れない（{e}）");
            None
        }
        Err(e) => panic!("AppContainer のプロファイルが作れない: {e}"),
    }
}

/// プロセスのトークンが AppContainer か
fn is_appcontainer(process: HANDLE) -> bool {
    use windows_sys::Win32::Security::TokenIsAppContainer;
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: 生きているプロセスのハンドルと、書き込み先を渡すだけ
    assert_ne!(
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) },
        0
    );
    // SAFETY: 成功した `OpenProcessToken` が返したハンドル
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut value = 0u32;
    let mut len = 0u32;
    // SAFETY: `value` は渡す大きさの、書き込める領域
    assert_ne!(
        unsafe {
            GetTokenInformation(
                raw(&token),
                TokenIsAppContainer,
                ptr::from_mut(&mut value).cast(),
                size_of::<u32>() as u32,
                &mut len,
            )
        },
        0
    );
    value != 0
}

#[test]
fn an_appcontainer_child_writes_to_the_inherited_handle_and_runs_in_the_job() {
    let _serial = appcontainer_serial();
    let Some(container) = test_container(&TEST_PROFILE) else {
        return;
    };
    let (mut extra_reader, extra_writer) = io::pipe().unwrap();
    let extra_writer = OwnedHandle::from(extra_writer);
    let value = (raw(&extra_writer) as usize).to_string();
    let env = env_with(APPCONTAINER_ENV, &[(LEAK_ENV, &value)]);
    let (mut reader, writer) = io::pipe().unwrap();
    let writer = OwnedHandle::from(writer);
    let child = spawn(&Launch {
        program: &std::env::current_exe().unwrap(),
        args: &probe_args(PROBE_TEST),
        environment: Some(&env),
        integrity: Integrity::Low,
        job: JobLimits { max_processes: 1 },
        stdio: Stdio {
            output: Some(writer.as_handle()),
            ..Stdio::default()
        },
        inherit: &[extra_writer.as_handle()],
        container: Some(&container),
    })
    .unwrap();
    assert!(!inherit_flag(raw(&writer)));
    assert!(!inherit_flag(raw(&extra_writer)));
    drop(writer);
    let process = child.process().as_raw_handle() as HANDLE;
    assert!(child.in_job().unwrap(), "子が Job に入っていない");
    assert!(is_appcontainer(process), "子が AppContainer でない");
    assert_eq!(integrity_rid(process), LOW_RID);
    let mut out = String::new();
    reader.read_to_string(&mut out).unwrap();
    assert_eq!(child.wait().unwrap(), 0, "{out}");
    assert_eq!(
        probe_lines(&out),
        ["stdin=none", "stderr=none", "leak=wrote", "nested=blocked"],
        "{out}"
    );
    drop(extra_writer);
    let mut got = Vec::new();
    extra_reader.read_to_end(&mut got).unwrap();
    assert_eq!(got, LEAK_BYTES);
    drop(child);
    drop(container);
    crate::appcontainer::delete_profile(&TEST_PROFILE).unwrap();
}

/// AppContainer の子に渡す、親の環境の変数（`ikari-data` の `child::PASS_ENV` の、OS の分と同じ）
const APPCONTAINER_ENV: &[&str] = &["SystemRoot", "TEMP", "TMP", "LOCALAPPDATA"];

/// 親の環境から `names` の変数だけを選び、テストの変数を足した環境
fn env_with(names: &[&str], extra: &[(&str, &str)]) -> Vec<u16> {
    let mut vars: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(k, _)| names.iter().any(|n| k.eq_ignore_ascii_case(n)))
        .collect();
    vars.extend(
        extra
            .iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v))),
    );
    environment_block(vars).unwrap()
}

/// どの環境なら AppContainer の子が起きるか（CI の windows ジョブで、`SystemRoot` だけの環境では `spawn` が
/// `ERROR_ENVVAR_NOT_FOUND`（203）で失敗した。親の環境を受け継ぐ LM の子は起きた）。環境だけを替えて同じ起動をし、
/// どれかの形が期待と違えば、全部の結果をつけて落とす
#[test]
fn which_environment_an_appcontainer_child_needs() {
    let _serial = appcontainer_serial();
    let Some(container) = test_container(&ENV_TEST_PROFILE) else {
        return;
    };
    let launch = |environment: Option<&[u16]>| {
        spawn(&Launch {
            program: &std::env::current_exe().unwrap(),
            args: &probe_args(EXIT_TEST),
            environment,
            integrity: Integrity::Low,
            job: JobLimits { max_processes: 1 },
            stdio: Stdio::default(),
            inherit: &[],
            container: Some(&container),
        })
        .and_then(|child| child.wait())
        .map_err(|e| e.raw_os_error().unwrap_or(-1))
    };
    let cases: [(&str, Option<Vec<u16>>); 5] = [
        ("inherit", None),
        ("systemroot", Some(env_with(&["SystemRoot"], &[]))),
        (
            "+localappdata",
            Some(env_with(&["SystemRoot", "LOCALAPPDATA"], &[])),
        ),
        (
            "+userprofile",
            Some(env_with(&["SystemRoot", "USERPROFILE"], &[])),
        ),
        ("product", Some(env_with(APPCONTAINER_ENV, &[]))),
    ];
    let results: Vec<(&str, Result<u32, i32>)> = cases
        .iter()
        .map(|(name, env)| (*name, launch(env.as_deref())))
        .collect();
    let report = format!("{results:?}");
    eprintln!("appcontainer environments: {report}");
    let ok = |name: &str| results.iter().any(|(n, r)| *n == name && r.is_ok());
    assert!(ok("inherit"), "親の環境でも起きない: {report}");
    assert!(ok("product"), "製品の環境で起きない: {report}");
    // VM の実測（2026-11-20）: LOCALAPPDATA だけで足りる。USERPROFILE だけでは足りない
    assert!(
        ok("+localappdata"),
        "LOCALAPPDATA を足しても起きない: {report}"
    );
    assert!(
        !ok("+userprofile"),
        "USERPROFILE だけで起きた（LOCALAPPDATA は要らなくなった？ 製品の環境を見直す）: {report}"
    );
    drop(container);
    crate::appcontainer::delete_profile(&ENV_TEST_PROFILE).unwrap();
}

#[test]
fn an_appcontainer_child_must_be_low_integrity() {
    let _serial = appcontainer_serial();
    let Some(container) = test_container(&TEST_PROFILE) else {
        return;
    };
    let err = spawn(&Launch {
        program: &std::env::current_exe().unwrap(),
        args: &probe_args(EXIT_TEST),
        environment: None,
        integrity: Integrity::Inherit,
        job: JobLimits { max_processes: 1 },
        stdio: Stdio::default(),
        inherit: &[],
        container: Some(&container),
    })
    .err()
    .map(|e| e.kind());
    assert_eq!(err, Some(io::ErrorKind::InvalidInput));
    drop(container);
    crate::appcontainer::delete_profile(&TEST_PROFILE).unwrap();
}
