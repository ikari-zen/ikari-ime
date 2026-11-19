//! 書く先のハンドル渡しのテスト（Windows。ADR 0078 §6 の段 1 の (a)〜(d)）。テストの実行ファイル自身を、`transfer` と
//! 同じ起こし方（`spawn_transfer`。低い整合性、Job、追記だけの写し）で子にし、子の中と親の両方から確かめる。
//! cmd.exe は引用符つきの引数を読めないので使わない。

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
    out.flush().unwrap();
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
