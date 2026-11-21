//! 書く先のハンドル渡しのテスト（Windows。ADR 0078 §6 の段 1 の (a)〜(d)）と、AppContainer の子のテスト（段 2）。
//! テストの実行ファイル自身を、`transfer` と同じ起こし方（`spawn_transfer`。低い整合性か AppContainer、Job、追記だけの
//! 写し）で子にし、子の中と親の両方から確かめる。cmd.exe は引用符つきの引数を読めないので使わない。
//! AppContainer のテストは、テスト用のプロファイル（`TRANSFER_PROFILE`。単体テストでは製品と別の名前）を作り、最後に
//! 消す。GitHub Actions の runner ではプロファイルが作れないので飛ばす（VM と実機で流す）。

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::time::{Duration, Instant};

use super::*;

/// 子として起動されたときだけ動かすテスト
const PROBE_TEST: &str = "spawn_windows::tests::probe_out_handle";
/// 子の行の印（libtest の出力と分ける）
const MARK: &str = "probe ";
/// 子の仕事の選び方（libtest の名前の絞り込みとして渡す。どのテストの名前にも合わない）
const MODE_PREFIX: &str = "mode=";
/// 子にパスで開かせてみるファイル（利用者のファイルの代わり。同じく名前の絞り込みとして渡す）
const READ_PREFIX: &str = "read=";
const MODE_WRITE: &str = "write";
const MODE_FLOOD: &str = "flood";
/// `write` の子が書くもの
const FIRST: &[u8] = b"hello";
/// 先頭に戻ってから書くもの（追記だけなので、末尾に付く）
const SECOND: &[u8] = b"X";
/// `flood` の子が 1 回に書く大きさと、止められなかったときに諦めるまでの時間
const FLOOD_CHUNK: usize = 64 * 1024;
const FLOOD_PAUSE: Duration = Duration::from_millis(5);
const FLOOD_FOR: Duration = Duration::from_secs(30);
/// `flood` の親の上限（子はすぐに超える）
const FLOOD_LIMIT: u64 = 1024;

/// 子の中で動く側。継承した書く先のハンドルで、できること・できないことを印つきの行で出す
#[test]
#[ignore = "ほかのテストが子として起動するときだけ動かす"]
fn probe_out_handle() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args
        .iter()
        .find_map(|a| a.strip_prefix(MODE_PREFIX))
        .unwrap()
        .to_owned();
    let value: u64 = args
        .iter()
        .position(|a| a == crate::child::OUT_HANDLE_FLAG)
        .and_then(|at| args.get(at + 1))
        .and_then(|v| v.parse().ok())
        .unwrap();
    let mut out = io::stdout();
    let mut file = match crate::run::take_out_handle(value) {
        Ok(file) => file,
        Err(e) => {
            writeln!(out, "{MARK}take={:?}", e.code()).unwrap();
            return;
        }
    };
    writeln!(out, "{MARK}take=ok").unwrap();
    out.flush().unwrap();
    if mode == MODE_FLOOD {
        let chunk = vec![0u8; FLOOD_CHUNK];
        let started = Instant::now();
        while started.elapsed() < FLOOD_FOR {
            if file.write_all(&chunk).is_err() {
                break;
            }
            std::thread::sleep(FLOOD_PAUSE);
        }
        return;
    }
    let ok = |r: bool| if r { "ok" } else { "denied" };
    let wrote = file.write_all(FIRST).is_ok();
    let mut byte = [0u8; 1];
    let read = file.seek(SeekFrom::Start(0)).is_ok() && file.read(&mut byte).is_ok();
    let truncated = file.set_len(0).is_ok();
    let rewound = file.seek(SeekFrom::Start(0)).is_ok() && file.write_all(SECOND).is_ok();
    writeln!(out, "{MARK}write={}", ok(wrote)).unwrap();
    writeln!(out, "{MARK}read={}", ok(read)).unwrap();
    writeln!(out, "{MARK}truncate={}", ok(truncated)).unwrap();
    writeln!(out, "{MARK}rewind-write={}", ok(rewound)).unwrap();
    if let Some(path) = args.iter().find_map(|a| a.strip_prefix(READ_PREFIX)) {
        writeln!(out, "{MARK}read-path={}", ok(std::fs::read(path).is_ok())).unwrap();
        // `transfer` の自己確認（ADR 0078 §3.2 の 4）を、この子のトークンで
        for (name, expected) in [
            ("low", ChildBox::Low),
            ("appcontainer", ChildBox::AppContainer),
        ] {
            let allowed = crate::run::transfer_allowed_here(expected);
            let verdict = if allowed { "allowed" } else { "refused" };
            writeln!(out, "{MARK}check-{name}={verdict}").unwrap();
        }
    }
    out.flush().unwrap();
}

/// `probe_args` に、パスで開かせてみるファイルを足す
fn probe_args_reading(mode: &str, path: &Path) -> Vec<String> {
    let mut args = probe_args(mode);
    args.push(format!("{READ_PREFIX}{}", path.display()));
    args
}

fn probe_args(mode: &str) -> Vec<String> {
    [
        "--exact",
        PROBE_TEST,
        "--ignored",
        "--nocapture",
        "--test-threads=1",
        // ここから後ろは名前の絞り込み（`spawn_transfer` が末尾に `--out-handle` を足す）
        "--",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .chain(std::iter::once(format!("{MODE_PREFIX}{mode}")))
    .collect()
}

/// 印つきの行だけ
fn probe_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|l| l.find(MARK).map(|at| &l[at + MARK.len()..]))
        .collect()
}

/// 親の `download.bin` と同じ開き方（読み書き、新しく作る）
fn parent_file(dir: &Path) -> (std::path::PathBuf, File) {
    let path = dir.join("download.bin");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    (path, file)
}

#[test]
fn the_child_can_only_append_through_the_handle_it_inherits_with_stdout() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, out) = parent_file(tmp.path());
    let exe = std::env::current_exe().unwrap();
    let (child, mut stdout) =
        spawn_transfer(&exe, &probe_args(MODE_WRITE), &out, Confine::Low).unwrap();
    let mut lines = String::new();
    stdout.read_to_string(&mut lines).unwrap();
    assert_eq!(child.wait().unwrap(), 0, "{lines}");
    assert_eq!(
        probe_lines(&lines),
        [
            "take=ok",
            "write=ok",
            "read=denied",
            "truncate=denied",
            "rewind-write=ok"
        ],
        "{lines}"
    );
    out.sync_all().unwrap();
    // 先頭に戻って書いても、末尾に付く（位置を選べない）
    assert_eq!(std::fs::read(&path).unwrap(), [FIRST, SECOND].concat());
}

#[test]
fn the_parent_supervises_a_child_that_finishes_and_then_copies_what_it_wrote() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, out) = parent_file(tmp.path());
    let exe = std::env::current_exe().unwrap();
    let (child, stdout) =
        spawn_transfer(&exe, &probe_args(MODE_WRITE), &out, Confine::Low).unwrap();
    supervise_transfer(&child, stdout, &out, FLOOD_LIMIT, &mut |_, _| {}).unwrap();
    let expected = [FIRST, SECOND].concat();
    // 親は自分のハンドルで、先頭から読める
    let mut copied = Vec::new();
    (&out).seek(SeekFrom::Start(0)).unwrap();
    (&out).read_to_end(&mut copied).unwrap();
    assert_eq!(copied, expected);
    assert_eq!(
        ikari_resource::sha256::sha256_reader(&expected[..]).unwrap(),
        ikari_resource::sha256::sha256_reader(File::open(&path).unwrap()).unwrap()
    );
}

#[test]
fn the_parent_stops_a_child_that_writes_over_the_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let (_path, out) = parent_file(tmp.path());
    let exe = std::env::current_exe().unwrap();
    let (child, stdout) =
        spawn_transfer(&exe, &probe_args(MODE_FLOOD), &out, Confine::Low).unwrap();
    let started = Instant::now();
    let err = supervise_transfer(&child, stdout, &out, FLOOD_LIMIT, &mut |_, _| {}).unwrap_err();
    assert_eq!(err.code(), FetchErrorCode::TooLarge);
    assert!(started.elapsed() < FLOOD_FOR / 2, "止めずに待った");
}

#[test]
fn the_copy_given_to_the_child_is_append_only_and_not_inheritable() {
    use windows_sys::Win32::Foundation::{GetHandleInformation, HANDLE_FLAG_INHERIT};
    let tmp = tempfile::tempdir().unwrap();
    let (path, out) = parent_file(tmp.path());
    (&out).write_all(b"abc").unwrap();
    let copy = append_only(&out).unwrap();
    let mut flags = 0u32;
    // SAFETY: 開いたハンドルの印を読むだけ
    assert_ne!(
        unsafe { GetHandleInformation(copy.as_raw_handle() as HANDLE, &mut flags) },
        0
    );
    assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
    let mut copy = File::from(copy);
    let mut buf = [0u8; 1];
    assert!(copy.read(&mut buf).is_err(), "読めない");
    assert!(copy.set_len(0).is_err(), "切り詰められない");
    copy.write_all(b"d").unwrap();
    drop(copy);
    assert_eq!(std::fs::read(&path).unwrap(), b"abcd");
}

/// 利用者のファイルの代わり（親が作り、親は読める）
fn user_file(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("user-document.txt");
    std::fs::write(&path, b"not for the child").unwrap();
    path
}

/// GitHub Actions の runner では、AppContainer のプロファイルが作れない（`CreateAppContainerProfile` が一定しない
/// エラーで失敗する。VM と実機では動く。ADR 0072 §12、`lm` と `ikari-sandbox` の段 2 のテストと同じ扱い）。CI
/// （`GITHUB_ACTIONS` が立っている）で作れないときだけ飛ばす。手元・VM・実機では、作れなければ落とす
fn appcontainer_unavailable_here() -> bool {
    match Container::ensure(&TRANSFER_PROFILE, TRANSFER_CAPABILITIES) {
        Ok(_) => false,
        Err(e) if std::env::var_os("GITHUB_ACTIONS").is_some() => {
            eprintln!("skip: CI の runner では AppContainer のプロファイルが作れない（{e}）");
            true
        }
        Err(e) => panic!("AppContainer のプロファイルが作れない: {e}"),
    }
}

#[test]
fn the_low_child_reads_user_files_and_passes_only_the_low_self_check() {
    // 段 1 の限界（低い整合性は読み取りを止めない）を、AppContainer の子と比べるために確かめる
    let tmp = tempfile::tempdir().unwrap();
    let (_path, out) = parent_file(tmp.path());
    let user = user_file(tmp.path());
    let exe = std::env::current_exe().unwrap();
    let (child, mut stdout) = spawn_transfer(
        &exe,
        &probe_args_reading(MODE_WRITE, &user),
        &out,
        Confine::Low,
    )
    .unwrap();
    let mut lines = String::new();
    stdout.read_to_string(&mut lines).unwrap();
    assert_eq!(child.wait().unwrap(), 0, "{lines}");
    let lines = probe_lines(&lines);
    for expected in [
        "read-path=ok",
        "check-low=allowed",
        "check-appcontainer=refused",
    ] {
        assert!(lines.contains(&expected), "{expected}: {lines:?}");
    }
}

#[test]
fn the_appcontainer_child_appends_through_the_handle_and_cannot_open_user_files() {
    if appcontainer_unavailable_here() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let (path, out) = parent_file(tmp.path());
    let user = user_file(tmp.path());
    // 親の側では読める（子が読めないのは、檻のせい）
    assert!(std::fs::read(&user).is_ok());
    let exe = std::env::current_exe().unwrap();
    let (child, mut stdout) = spawn_transfer(
        &exe,
        &probe_args_reading(MODE_WRITE, &user),
        &out,
        Confine::AppContainer(&TRANSFER_PROFILE),
    )
    .unwrap();
    let mut lines = String::new();
    stdout.read_to_string(&mut lines).unwrap();
    assert_eq!(child.wait().unwrap(), 0, "{lines}");
    assert_eq!(
        probe_lines(&lines),
        [
            "take=ok",
            "write=ok",
            "read=denied",
            "truncate=denied",
            "rewind-write=ok",
            "read-path=denied",
            "check-low=allowed",
            "check-appcontainer=allowed",
        ],
        "{lines}"
    );
    out.sync_all().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), [FIRST, SECOND].concat());
    drop(child);
    ikari_sandbox::appcontainer::delete_profile(&TRANSFER_PROFILE).unwrap();
}
