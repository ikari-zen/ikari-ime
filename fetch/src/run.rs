//! コマンドの実行（製品の入口）。引数を読み、場所を決め、仕事を流す（ADR 0076 §3、§4.1）。
//!
//! 終了コード: 0（成功）、1（`error` の行を出した）、2（`usage`。行は出さない）。

use std::fs::OpenOptions;
use std::io::Write;
use std::time::Instant;

use ikari_resource::Created;
#[cfg(windows)]
use ikari_resource::fetch_report::EXIT_OK;
use ikari_resource::fetch_report::{EXIT_ERROR, EXIT_USAGE, FetchErrorCode, Report};

use crate::args::{Command, TransferArgs, Usage, parse};
use crate::error::FetchError;
use crate::job::{self, Context, DownloadJob, Fetcher, Throttle};
use crate::nofollow;
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
    // 版の確認は次の段（D-2d。ADR 0076 §6、§13）。ここでは動かない
    if command == Command::VersionCheck {
        job::emit(out, &Report::Error(FetchErrorCode::Tool));
        return EXIT_ERROR;
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
        Command::VersionCheck | Command::Transfer(_) => EXIT_USAGE,
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

/// 通信する子の仕事。`url` を `out` のファイルに落とし、進み具合を行で出す。
/// Windows では、低い整合性でなければ `refused` で終わる（人が手で打っても、中の整合性では通信しない）
#[cfg(windows)]
fn run_transfer(args: &TransferArgs, out: &mut dyn Write) -> i32 {
    use ikari_pipe::os::windows::current_integrity_rid;
    if !crate::child::is_low_integrity(current_integrity_rid()) {
        job::emit(out, &Report::Error(FetchErrorCode::Refused));
        return EXIT_ERROR;
    }
    match transfer_to_file(args, out) {
        Ok(()) => EXIT_OK,
        Err(e) => {
            job::emit(out, &Report::Error(e.code()));
            EXIT_ERROR
        }
    }
}

/// macOS は子を作らない（自分で通信する）ので、Windows 以外の `transfer` は使わせない
#[cfg(not(windows))]
fn run_transfer(_args: &TransferArgs, _out: &mut dyn Write) -> i32 {
    EXIT_USAGE
}

/// `url` を `args.out` に落とす（新しく作る。すでにあれば失敗）。進み具合の行は間引いて出し、最後の 1 行は必ず出す
#[cfg_attr(not(windows), allow(dead_code))]
fn transfer_to_file(args: &TransferArgs, out: &mut dyn Write) -> Result<(), FetchError> {
    let url = HttpsUrl::parse(&args.url)?;
    let mut file =
        nofollow::open_plain_file(OpenOptions::new().write(true).create_new(true), &args.out)
            .map_err(|_| FetchError(FetchErrorCode::Internal))?;
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
    file.sync_all()
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

/// 製品の取得の部品。Windows は低い整合性の子、macOS は自分で通信する
pub struct ProductFetcher;

impl Fetcher for ProductFetcher {
    #[cfg(windows)]
    fn fetch(
        &self,
        job: &DownloadJob<'_>,
        dirs: &Dirs,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<(), FetchError> {
        use std::io::BufReader;

        use ikari_resource::fetch_report::{Line, ReportReader, read_line_capped};

        use crate::child::transfer_args;
        use crate::spawn_windows::spawn_low;

        let tool = || FetchError(FetchErrorCode::Tool);
        let exe = std::env::current_exe().map_err(|_| tool())?;
        let args = transfer_args(&TransferArgs {
            url: job.url.as_str().to_owned(),
            out: dirs.download_path(),
            max_size: job.max_size,
            expect_size: job.expect_size,
        });
        let (child, stdout) = spawn_low(&exe, &args).map_err(|_| tool())?;

        // 子の行は信じない。進み具合は表示にだけ使い、受けた大きさは写すときに数え直す
        let mut reader = BufReader::new(stdout);
        let mut reports = ReportReader::new();
        let mut failure = None;
        loop {
            match read_line_capped(&mut reader) {
                Ok(Line::Text(line)) => match reports.feed(&line) {
                    Some(Report::Progress { received, total }) => progress(received, total),
                    Some(Report::Error(code)) => failure = Some(code),
                    Some(_) | None => {}
                },
                Ok(Line::Dropped) => {}
                Ok(Line::Eof) | Err(_) => break,
            }
        }
        let exit = child.wait().map_err(|_| tool())?;
        match (failure, exit) {
            (Some(code), _) => Err(FetchError(code)),
            (None, 0) => Ok(()),
            (None, _) => Err(FetchError(FetchErrorCode::Internal)),
        }
    }

    #[cfg(not(windows))]
    fn fetch(
        &self,
        job: &DownloadJob<'_>,
        dirs: &Dirs,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<(), FetchError> {
        let mut file = nofollow::open_plain_file(
            OpenOptions::new().write(true).create_new(true),
            &dirs.download_path(),
        )
        .map_err(|_| FetchError(FetchErrorCode::Internal))?;
        let request = DownloadRequest {
            url: job.url,
            max_size: job.max_size,
            expect_size: job.expect_size,
        };
        download(&request, &mut file, progress)?;
        file.sync_all()
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

    #[test]
    fn version_check_is_not_implemented_yet() {
        let (code, out) = run_str("version-check");
        // Windows のテストは中の整合性で動くので、実装の前に `refused` で終わる
        let expected = if cfg!(windows) {
            "error refused\n"
        } else {
            "error tool\n"
        };
        assert_eq!((code, out.as_str()), (EXIT_ERROR, expected));
    }

    #[cfg(not(windows))]
    #[test]
    fn transfer_is_not_a_public_command_outside_windows() {
        assert_eq!(
            run_str("transfer --url https://example.com/ --out /tmp/x --max-size 1"),
            (EXIT_USAGE, String::new())
        );
    }

    #[test]
    fn transfer_to_file_refuses_bad_urls_and_existing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let out_path = tmp.path().join("d.bin");
        let mut args = TransferArgs {
            url: "http://example.com/x".into(),
            out: out_path.clone(),
            max_size: 10,
            expect_size: None,
        };
        let mut sink = Vec::new();
        assert_eq!(
            transfer_to_file(&args, &mut sink).unwrap_err().code(),
            FetchErrorCode::BadUrl
        );
        assert!(!out_path.exists(), "URL が悪ければ、ファイルも作らない");
        std::fs::write(&out_path, b"x").unwrap();
        args.url = "https://example.com/x".into();
        assert_eq!(
            transfer_to_file(&args, &mut sink).unwrap_err().code(),
            FetchErrorCode::Internal
        );
        assert_eq!(
            std::fs::read(&out_path).unwrap(),
            b"x",
            "既存のファイルを上書きしない"
        );
    }
}
