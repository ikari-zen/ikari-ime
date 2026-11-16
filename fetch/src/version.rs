//! 版の確認 `ikari-data version-check`（ADR 0076 §6、§18.2）。**契約（第 4 弾）。中身はレーン D-2d**。
//!
//! 決まっていること:
//! - 埋めた固定の URL（[`RELEASES_LATEST_URL`]）に、転送を追わずに 1 回だけ要求する（`NetLimits::VERSION_CHECK`。
//!   本文は `VERSION_CHECK_BODY_LIMIT` まで読んで捨てる）。通信は `transfer.rs` の 1 か所だけ（ADR 0076 §10）。
//! - 応答の `Location` は [`TAG_LOCATION_PREFIX`] で始まるものだけを受け、末尾のタグを
//!   `ikari_resource::app_version::AppVersion::parse_tag` で読む。ほかは `bad-reply`。
//! - 成功は `latest <数.数.数>` の行を出して 0 で終わる。比べるのは設定画面の核。
//! - 取得も導入もしない。自動では確認しない。
//! - Windows では、通信するのは低い整合性の子だけ（ADR 0076 §3.2）。
//!
//! 今のスタブは、今までどおり `error tool` を出して 1 で終わる。

use std::io::Write;

use ikari_resource::app_version::AppVersion;
use ikari_resource::fetch_report::{EXIT_ERROR, FetchErrorCode, Report};

use crate::error::FetchError;
use crate::job;

/// 版の確認の固定の URL（実行ファイルに埋める。応答の中身から URL を作らない）
pub const RELEASES_LATEST_URL: &str = "https://github.com/ikari-zen/ikari-ime/releases/latest";
/// 受ける `Location` の接頭辞（これで始まらなければ `bad-reply`）
pub const TAG_LOCATION_PREFIX: &str = "https://github.com/ikari-zen/ikari-ime/releases/tag/";

/// 応答の `Location`（無ければ None）から版を読む。形が違えば `bad-reply`。
/// 契約のスタブ: 今は常に `bad-reply`（D-2d）
pub fn parse_location(location: Option<&str>) -> Result<AppVersion, FetchError> {
    let _ = location;
    Err(FetchError(FetchErrorCode::BadReply))
}

/// `version-check` の仕事（利用者の側。`run_parent` から呼ぶ）。終了コードを返し、行は `out` へ。
/// 契約のスタブ: 今は今までどおり `error tool`（D-2d）
pub fn run_version_check(out: &mut dyn Write) -> i32 {
    job::emit(out, &Report::Error(FetchErrorCode::Tool));
    EXIT_ERROR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_url_and_the_prefix_point_at_this_repository() {
        assert!(RELEASES_LATEST_URL.starts_with("https://"));
        assert!(TAG_LOCATION_PREFIX.starts_with("https://"));
        assert!(TAG_LOCATION_PREFIX.ends_with("/releases/tag/"));
        let repo = RELEASES_LATEST_URL.trim_end_matches("/releases/latest");
        assert!(TAG_LOCATION_PREFIX.starts_with(repo));
    }

    #[test]
    #[ignore = "D-2d: 接頭辞が違う Location（別のリポジトリ、http、相対）と、Location なしは bad-reply"]
    fn locations_outside_the_prefix_are_bad_replies() {
        unimplemented!("D-2d")
    }

    #[test]
    #[ignore = "D-2d: データのタグ（…/tag/webwords-20261101）は bad-reply（ADR 0076 §16、§18.2）"]
    fn a_data_tag_is_a_bad_reply() {
        unimplemented!("D-2d")
    }

    #[test]
    #[ignore = "D-2d: …/tag/v0.3.1 は latest 0.3.1 の行と終了コード 0"]
    fn an_app_tag_prints_latest() {
        unimplemented!("D-2d")
    }
}
