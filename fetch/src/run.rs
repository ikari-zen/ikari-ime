//! コマンドの実行（製品の入口）。引数を読み、場所を決め、仕事を流す（ADR 0076 §3、§4.1）。
//!
//! 終了コード: 0（成功）、1（`error` の行を出した）、2（`usage`。行は出さない）。

use std::fs::File;
use std::io::Write;
use std::time::Instant;

use ikari_resource::Created;
#[cfg(windows)]
use ikari_resource::fetch_report::EXIT_OK;
use ikari_resource::fetch_report::{EXIT_ERROR, EXIT_USAGE, FetchErrorCode, Report};

use crate::args::{Command, TransferArgs, TransferCommand, Usage, parse};
use crate::error::FetchError;
use crate::job::{self, Context, DownloadJob, FetchEvent, Fetcher, Throttle};
use crate::transfer::{DownloadRequest, download};
use crate::url::HttpsUrl;
use crate::workspace::Dirs;

/// `args`（実行ファイルの名前を除く）を実行して、終了コードを返す。行は `out` へ
pub fn run(args: &[String], out: &mut dyn Write) -> i32 {
    let command = match parse(args) {
        Ok(command) => command,
        Err(Usage) => return EXIT_USAGE,
    };
    match command {
        Command::Transfer(transfer) => run_transfer(&transfer, out),
        Command::VersionProbe => run_version_probe(out),
        Command::ModelFetch(_) | Command::Cleanup | Command::VersionCheck => {
            run_parent(command, out)
        }
    }
}

/// 利用者の側（中の整合性）の仕事
fn run_parent(command: Command, out: &mut dyn Write) -> i32 {
    if !parent_allowed() {
        job::emit(out, &Report::Error(FetchErrorCode::Refused));
        return EXIT_ERROR;
    }
    // 版の確認（ADR 0076 §6）。作業の場所も錠も使わない
    if command == Command::VersionCheck {
        return crate::version::run_version_check(out);
    }
    let Some(dirs) = Dirs::resolve() else {
        job::emit(out, &Report::Error(FetchErrorCode::Internal));
        return EXIT_ERROR;
    };
    match command {
        Command::Cleanup => job::cleanup(&dirs, out),
        Command::ModelFetch(source) => {
            let Some(created) = now_created() else {
                job::emit(out, &Report::Error(FetchErrorCode::Internal));
                return EXIT_ERROR;
            };
            let ctx = Context {
                dirs: &dirs,
                fetcher: &ProductFetcher,
                created,
            };
            job::model_fetch(&source, &ctx, out)
        }
        Command::VersionCheck | Command::VersionProbe | Command::Transfer(_) => EXIT_USAGE,
    }
}

/// 高い権限で起動されたら何もしない（A-3 と同じ規則）。Windows だけ（macOS は確かめる手段がない）
fn parent_allowed() -> bool {
    #[cfg(windows)]
    {
        use ikari_pipe::elevation::check_self;
        use ikari_pipe::os::windows::current_integrity_rid;
        check_self(current_integrity_rid()).is_ok()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// 今の時刻（UTC）の manifest の日時
fn now_created() -> Option<Created> {
    let text = jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    Created::parse(&text).ok()
}

/// 通信する子の仕事。`url` を、継承した書く先のハンドル（`--out-handle`）に落とし、進み具合を行で出す。
/// Windows では、低い整合性でなければ `refused` で終わる（人が手で打っても、中の整合性では通信しない）。AppContainer で
/// 起こされたはず（`--box appcontainer`）なら、トークンが AppContainer でなければ `refused`
#[cfg(windows)]
fn run_transfer(command: &TransferCommand, out: &mut dyn Write) -> i32 {
    if !transfer_allowed_here(command.child_box) {
        job::emit(out, &Report::Error(FetchErrorCode::Refused));
        return EXIT_ERROR;
    }
    let result = take_out_handle(command.out_handle)
        .and_then(|file| transfer_into(&command.args, file, out));
    match result {
        Ok(()) => EXIT_OK,
        Err(e) => {
            job::emit(out, &Report::Error(e.code()));
            EXIT_ERROR
        }
    }
}

/// `transfer` の自己確認を、自分のトークンで行う（ADR 0078 §3.2 の 4）
#[cfg(windows)]
pub(crate) fn transfer_allowed_here(expected: ikari_resource::fetch_report::ChildBox) -> bool {
    use ikari_pipe::os::windows::current_integrity_rid;
    crate::child::transfer_allowed(
        expected,
        current_integrity_rid(),
        crate::spawn_windows::current_is_appcontainer(),
    )
}

/// macOS は子を作らない（自分で通信する）ので、Windows 以外の `transfer` は使わせない
#[cfg(not(windows))]
fn run_transfer(_command: &TransferCommand, _out: &mut dyn Write) -> i32 {
    EXIT_USAGE
}

/// 継承した書く先のハンドルの値を、持ち主として受け取る（ADR 0078 §4.2）。ディスクのファイルでなければ（開いていない
/// 値、パイプ、コンソールなど）`internal`。そのときは閉じない（自分のものと言えないため）
#[cfg(windows)]
pub(crate) fn take_out_handle(value: u64) -> Result<std::fs::File, FetchError> {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_DISK, GetFileType};

    let internal = || FetchError(FetchErrorCode::Internal);
    let raw = usize::try_from(value)
        .ok()
        .filter(|&v| v != 0)
        .ok_or_else(internal)? as HANDLE;
    // SAFETY: 種類を聞くだけ（開いていない値なら FILE_TYPE_UNKNOWN が返る）
    if unsafe { GetFileType(raw) } != FILE_TYPE_DISK {
        return Err(internal());
    }
    // SAFETY: 親が、この値のハンドルを子にだけ継承させた（ADR 0078 §3.1 の 3）。ほかに持ち主はいない
    Ok(unsafe { std::fs::File::from_raw_handle(raw) })
}

/// `url` を `file` の末尾に落とす。進み具合の行は間引いて出し、最後の 1 行は必ず出す。`sync_all` は呼ばない（追記だけの
/// 権利では失敗する見込み。ディスクに出すのは、子が終わったあとの親。ADR 0078 §3.1 の 7）
#[cfg_attr(not(windows), allow(dead_code))]
fn transfer_into(
    args: &TransferArgs,
    mut file: impl Write,
    out: &mut dyn Write,
) -> Result<(), FetchError> {
    let url = HttpsUrl::parse(&args.url)?;
    let request = DownloadRequest {
        url: &url,
        max_size: args.max_size,
        expect_size: args.expect_size,
    };
    let mut throttle = Throttle::standard();
    let mut last = None;
    let received = download(&request, &mut file, &mut |received, total| {
        last = Some((received, total));
        if throttle.ready(Instant::now()) {
            job::emit(out, &Report::Progress { received, total });
        }
    })?;
    file.flush()
        .map_err(|_| FetchError(FetchErrorCode::Internal))?;
    let total = last.and_then(|(_, total)| total);
    job::emit(
        out,
        &Report::Progress {
            received,
            total: total.or(Some(received)),
        },
    );
    Ok(())
}

/// 版の確認の通信をする子の仕事。`transfer` と同じく、Windows では低い整合性でなければ `refused`
#[cfg(windows)]
fn run_version_probe(out: &mut dyn Write) -> i32 {
    use ikari_pipe::os::windows::current_integrity_rid;
    if !crate::child::is_low_integrity(current_integrity_rid()) {
        job::emit(out, &Report::Error(FetchErrorCode::Refused));
        return EXIT_ERROR;
    }
    crate::version::run_probe(out)
}

/// macOS は子を作らない（`version-check` が自分で通信する）ので、Windows 以外の `version-probe` は使わせない
#[cfg(not(windows))]
fn run_version_probe(_out: &mut dyn Write) -> i32 {
    EXIT_USAGE
}

/// 製品の取得の部品。Windows は AppContainer の子（起こせなければ低い整合性の子。ADR 0078 §3.2 の 5）、macOS は自分で
/// 通信する
pub struct ProductFetcher;

impl Fetcher for ProductFetcher {
    #[cfg(windows)]
    fn fetch(
        &self,
        job: &DownloadJob<'_>,
        out: &File,
        events: &mut dyn FnMut(FetchEvent),
    ) -> Result<(), FetchError> {
        use crate::child::{spawn_preferring_container, transfer_args};
        use crate::spawn_windows::{Confine, TRANSFER_PROFILE, spawn_transfer, supervise_transfer};

        let tool = || FetchError(FetchErrorCode::Tool);
        let exe = std::env::current_exe().map_err(|_| tool())?;
        let args = transfer_args(&TransferArgs {
            url: job.url.as_str().to_owned(),
            max_size: job.max_size,
            expect_size: job.expect_size,
        });
        // AppContainer で起こせなければ（プロファイルが作れない、起動が失敗する）、低い整合性に落として続け、`box low` の
        // 行で知らせる（設定画面に印が出る）
        let ((child, stdout), child_box) = spawn_preferring_container(
            || spawn_transfer(&exe, &args, out, Confine::AppContainer(&TRANSFER_PROFILE)),
            || spawn_transfer(&exe, &args, out, Confine::Low),
        )
        .map_err(|_| tool())?;
        events(FetchEvent::Box(child_box));
        supervise_transfer(&child, stdout, out, job.max_size, &mut |received, total| {
            events(FetchEvent::Progress { received, total });
        })
    }

    #[cfg(not(windows))]
    fn fetch(
        &self,
        job: &DownloadJob<'_>,
        out: &File,
        events: &mut dyn FnMut(FetchEvent),
    ) -> Result<(), FetchError> {
        let request = DownloadRequest {
            url: job.url,
            max_size: job.max_size,
            expect_size: job.expect_size,
        };
        let mut writer = out;
        download(&request, &mut writer, &mut |received, total| {
            events(FetchEvent::Progress { received, total });
        })?;
        out.sync_all()
            .map_err(|_| FetchError(FetchErrorCode::Internal))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_str(line: &str) -> (i32, String) {
        let args: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        let mut out = Vec::new();
        let code = run(&args, &mut out);
        (code, String::from_utf8(out).unwrap())
    }

    #[test]
    fn bad_arguments_exit_with_usage_and_print_nothing() {
        for line in [
            "",
            "model",
            "model fetch --data-dir /x --preset zenz-small",
            "bogus",
        ] {
            assert_eq!(run_str(line), (EXIT_USAGE, String::new()), "{line:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn version_probe_refuses_unless_low_integrity() {
        // Windows のテストは中の整合性で動くので、通信の前に `refused` で終わる
        assert_eq!(
            run_str("version-probe"),
            (EXIT_ERROR, "error refused\n".to_owned())
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn version_probe_is_not_for_people_outside_windows() {
        assert_eq!(run_str("version-probe"), (EXIT_USAGE, String::new()));
    }

    #[cfg(not(windows))]
    #[test]
    fn transfer_is_not_a_public_command_outside_windows() {
        assert_eq!(
            run_str("transfer --url https://example.com/ --max-size 1 --out-handle 4"),
            (EXIT_USAGE, String::new())
        );
    }

    #[test]
    fn transfer_into_refuses_bad_urls_before_writing() {
        let args = TransferArgs {
            url: "http://example.com/x".into(),
            max_size: 10,
            expect_size: None,
        };
        let mut written = Vec::new();
        let mut sink = Vec::new();
        assert_eq!(
            transfer_into(&args, &mut written, &mut sink)
                .unwrap_err()
                .code(),
            FetchErrorCode::BadUrl
        );
        assert!(written.is_empty(), "URL が悪ければ、何も書かない");
        assert!(sink.is_empty());
    }

    #[test]
    fn transfer_refuses_a_path_for_the_output() {
        // 書く先は場所で受けない（ADR 0078 §3.1 の 4）。どの OS でも、行を出さずに使い方の誤り
        assert_eq!(
            run_str("transfer --url https://example.com/ --out /tmp/x --max-size 1"),
            (EXIT_USAGE, String::new())
        );
    }

    #[cfg(windows)]
    #[test]
    fn transfer_refuses_unless_low_integrity_even_with_a_handle() {
        // Windows のテストは中の整合性で動くので、ハンドルを受け取る前に `refused` で終わる
        assert_eq!(
            run_str("transfer --url https://example.com/ --max-size 1 --out-handle 4"),
            (EXIT_ERROR, "error refused\n".to_owned())
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_handle_that_is_not_a_disk_file_is_internal() {
        use std::os::windows::io::AsRawHandle;
        // パイプ（ディスクのファイルでない）。受け取らないので、閉じられない
        let (_reader, writer) = std::io::pipe().unwrap();
        let value = writer.as_raw_handle() as usize as u64;
        assert_eq!(
            take_out_handle(value).unwrap_err().code(),
            FetchErrorCode::Internal
        );
        assert_eq!(
            take_out_handle(0).unwrap_err().code(),
            FetchErrorCode::Internal
        );
        // 開いていない値
        assert_eq!(
            take_out_handle(0x7fff_fff0).unwrap_err().code(),
            FetchErrorCode::Internal
        );
        drop(writer);
    }
}
