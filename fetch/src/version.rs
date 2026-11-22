//! 版の確認 `ikari-data version-check`（ADR 0076 §6、§18.2）。
//!
//! 決まっていること:
//! - 埋めた固定の URL（[`RELEASES_LATEST_URL`]）に、転送を追わずに 1 回だけ要求する（`NetLimits::VERSION_CHECK`。
//!   本文は `VERSION_CHECK_BODY_LIMIT` まで読んで捨てる）。通信は `transfer.rs` の 1 か所だけ（ADR 0076 §10）。
//! - 応答の `Location` は [`TAG_LOCATION_PREFIX`] で始まるものだけを受け、末尾のタグを
//!   `ikari_resource::app_version::AppVersion::parse_tag` で読む。ほかは `bad-reply`。
//! - 成功は `latest <数.数.数>` の行を出して 0 で終わる。比べるのは設定画面の核。
//! - 取得も導入もしない。自動では確認しない。
//! - Windows では、通信するのは絞った子だけ（ADR 0076 §3.2、ADR 0078 §3.2）。
//!
//! Windows: 通信するのは子（`ikari-data version-probe`）だけ。起こし方はモデルの取得の `transfer` と同じで、
//! AppContainer で起こし、起こせなければ低い整合性に落として続ける（ADR 0078 §3.2 の 5）。どちらで起こしたかは
//! `box` の行で知らせる（`low` なら設定画面に印が出る）。利用者の側（`run_version_check`）は、子の行を信じず、
//! `latest` の行の版を読み直してから出す。macOS は子なし（自分で通信する）。

use std::io::Write;

use ikari_resource::app_version::AppVersion;
use ikari_resource::fetch_report::{EXIT_ERROR, EXIT_OK, FetchErrorCode, Report};

use crate::error::FetchError;
use crate::job;
use crate::limits::VERSION_CHECK_BODY_LIMIT;
#[cfg(windows)]
use crate::limits::VERSION_CHECK_DEADLINE;
use crate::transfer::redirect_location;
use crate::url::HttpsUrl;

/// 版の確認の固定の URL（実行ファイルに埋める。応答の中身から URL を作らない）
pub const RELEASES_LATEST_URL: &str = "https://github.com/ikari-zen/ikari-ime/releases/latest";
/// 受ける `Location` の接頭辞（これで始まらなければ `bad-reply`）
pub const TAG_LOCATION_PREFIX: &str = "https://github.com/ikari-zen/ikari-ime/releases/tag/";

/// 応答の `Location`（無ければ None）から版を読む。接頭辞が違う・タグの形が違う・`Location` がなければ `bad-reply`。
pub fn parse_location(location: Option<&str>) -> Result<AppVersion, FetchError> {
    let bad = || FetchError(FetchErrorCode::BadReply);
    let tag = location
        .and_then(|l| l.strip_prefix(TAG_LOCATION_PREFIX))
        .ok_or_else(bad)?;
    AppVersion::parse_tag(tag).ok_or_else(bad)
}

/// 通信して版を読む。`locate` は転送を追わずに `Location` を返す部品（製品は `redirect_location`）
fn check(
    locate: impl FnOnce(&HttpsUrl, u64) -> Result<Option<String>, FetchError>,
) -> Result<AppVersion, FetchError> {
    let url = HttpsUrl::parse(RELEASES_LATEST_URL)?;
    let location = locate(&url, VERSION_CHECK_BODY_LIMIT)?;
    parse_location(location.as_deref())
}

/// 通信して `latest <版>` か `error <語>` の行を出す（通信する側。Windows では低い整合性の子、macOS では自分）
pub fn run_probe(out: &mut dyn Write) -> i32 {
    report(out, check(redirect_location))
}

fn report(out: &mut dyn Write, result: Result<AppVersion, FetchError>) -> i32 {
    match result {
        Ok(version) => {
            job::emit(
                out,
                &Report::Latest {
                    version: version.to_string(),
                },
            );
            EXIT_OK
        }
        Err(e) => {
            job::emit(out, &Report::Error(e.code()));
            EXIT_ERROR
        }
    }
}

/// `version-check` の仕事（利用者の側。`run_parent` から呼ぶ）。終了コードを返し、行は `out` へ。
#[cfg(windows)]
pub fn run_version_check(out: &mut dyn Write) -> i32 {
    let result = check_in_child(&mut |child_box| {
        job::emit(out, &Report::Box(child_box));
    });
    report(out, result)
}

/// macOS は子なし。自分で通信する
#[cfg(not(windows))]
pub fn run_version_check(out: &mut dyn Write) -> i32 {
    run_probe(out)
}

/// 子に通信させ、`latest` の行の版を読み直して返す（子の出力は信じない）。子を起こしたら、どの檻で起こしたかを
/// `boxed` に 1 回だけ渡す
#[cfg(windows)]
fn check_in_child(
    boxed: &mut dyn FnMut(ikari_resource::fetch_report::ChildBox),
) -> Result<AppVersion, FetchError> {
    use std::io::BufReader;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::thread;

    use ikari_resource::fetch_report::{Line, ReportReader, read_line_capped};

    use crate::child::spawn_preferring_container;
    use crate::spawn_windows::{Confine, TRANSFER_PROFILE, spawn_version_probe};

    let tool = || FetchError(FetchErrorCode::Tool);
    let exe = std::env::current_exe().map_err(|_| tool())?;
    // モデルの取得と同じ。AppContainer で起こせなければ低い整合性に落として続け、`box low` の行で知らせる
    let ((child, stdout), child_box) = spawn_preferring_container(
        || spawn_version_probe(&exe, Confine::AppContainer(&TRANSFER_PROFILE)),
        || spawn_version_probe(&exe, Confine::Low),
    )
    .map_err(|_| tool())?;
    boxed(child_box);
    // 親の全体の締め切り（FV-1）。子が出力を閉じなくても、過ぎたら Job を閉じて（子を止めて）`network` にする
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let timed_out = AtomicBool::new(false);
    let (latest, failure) = thread::scope(|scope| {
        let (child, timed_out) = (&child, &timed_out);
        scope.spawn(move || {
            if let Err(mpsc::RecvTimeoutError::Timeout) =
                done_rx.recv_timeout(VERSION_CHECK_DEADLINE)
            {
                timed_out.store(true, Ordering::SeqCst);
                child.terminate();
            }
        });
        let mut reader = BufReader::new(stdout);
        let mut reports = ReportReader::new();
        let mut latest = None;
        let mut failure = None;
        loop {
            match read_line_capped(&mut reader) {
                Ok(Line::Text(line)) => match reports.feed(&line) {
                    Some(Report::Latest { version }) => latest = AppVersion::parse(&version),
                    Some(Report::Error(code)) => failure = Some(code),
                    Some(_) | None => {}
                },
                Ok(Line::Dropped) => {}
                Ok(Line::Eof) | Err(_) => break,
            }
        }
        // 読み終えた。見張りを止める
        let _ = done_tx.send(());
        (latest, failure)
    });
    if timed_out.load(Ordering::SeqCst) {
        let _ = child.wait();
        return Err(FetchError(FetchErrorCode::Network));
    }
    let exit = child.wait().map_err(|_| tool())?;
    match (failure, latest, exit) {
        (Some(code), _, _) => Err(FetchError(code)),
        // 子の終了コードは u32（成功は 0）
        (None, Some(version), 0) => Ok(version),
        (None, _, _) => Err(FetchError(FetchErrorCode::Internal)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(t: &str) -> String {
        format!("{TAG_LOCATION_PREFIX}{t}")
    }

    fn code(r: Result<AppVersion, FetchError>) -> FetchErrorCode {
        r.unwrap_err().code()
    }

    #[test]
    fn the_url_and_the_prefix_point_at_this_repository() {
        assert!(RELEASES_LATEST_URL.starts_with("https://"));
        assert!(TAG_LOCATION_PREFIX.starts_with("https://"));
        assert!(TAG_LOCATION_PREFIX.ends_with("/releases/tag/"));
        let repo = RELEASES_LATEST_URL.trim_end_matches("/releases/latest");
        assert!(TAG_LOCATION_PREFIX.starts_with(repo));
        assert!(HttpsUrl::parse(RELEASES_LATEST_URL).is_ok());
    }

    #[test]
    fn locations_outside_the_prefix_are_bad_replies() {
        for bad in [
            None,
            Some(""),
            Some("v0.3.1"),
            Some("/ikari-zen/ikari-ime/releases/tag/v0.3.1"),
            Some("http://github.com/ikari-zen/ikari-ime/releases/tag/v0.3.1"),
            Some("https://github.com/other/ikari-ime/releases/tag/v0.3.1"),
            Some("https://github.com.evil.example/ikari-zen/ikari-ime/releases/tag/v0.3.1"),
            Some("https://evil.example/https://github.com/ikari-zen/ikari-ime/releases/tag/v0.3.1"),
            Some("https://github.com/ikari-zen/ikari-ime/releases/tag"),
            Some("https://github.com/ikari-zen/ikari-ime/releases/tag/"),
            Some("https://github.com/ikari-zen/ikari-ime/releases/latest"),
        ] {
            assert_eq!(
                code(parse_location(bad)),
                FetchErrorCode::BadReply,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_tag_with_extra_path_or_query_is_a_bad_reply() {
        for t in ["v0.3.1/", "v0.3.1/x", "v0.3.1?a=b", "v0.3.1#x", "v0.3.1%20"] {
            assert_eq!(
                code(parse_location(Some(&tag(t)))),
                FetchErrorCode::BadReply,
                "{t}"
            );
        }
    }

    #[test]
    fn a_data_tag_is_a_bad_reply() {
        for t in ["webwords-20261101", "0.3.1", "v0.3.1-rc1", "v0.3"] {
            assert_eq!(
                code(parse_location(Some(&tag(t)))),
                FetchErrorCode::BadReply,
                "{t}"
            );
        }
    }

    fn run_with(location: Result<Option<String>, FetchError>) -> (i32, String) {
        let mut out = Vec::new();
        let result = check(|url, limit| {
            assert_eq!(url.as_str(), RELEASES_LATEST_URL);
            assert_eq!(limit, VERSION_CHECK_BODY_LIMIT);
            location
        });
        let code = report(&mut out, result);
        (code, String::from_utf8(out).unwrap())
    }

    #[test]
    fn an_app_tag_prints_latest() {
        assert_eq!(
            run_with(Ok(Some(tag("v0.3.1")))),
            (EXIT_OK, "latest 0.3.1\n".to_owned())
        );
    }

    #[test]
    fn failures_print_one_error_word() {
        assert_eq!(
            run_with(Ok(Some(tag("webwords-20261101")))),
            (EXIT_ERROR, "error bad-reply\n".to_owned())
        );
        assert_eq!(
            run_with(Ok(None)),
            (EXIT_ERROR, "error bad-reply\n".to_owned())
        );
        assert_eq!(
            run_with(Err(FetchError(FetchErrorCode::Network))),
            (EXIT_ERROR, "error network\n".to_owned())
        );
    }
}
