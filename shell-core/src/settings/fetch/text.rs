//! 取得の文（Windows の設定画面から移した。中身は変えていない。ADR 0079 §3.6）。**OS に依存しない**。
//!
//! 文に入れるのは、理由の語、バイト数、版だけ。URL、ホスト、パスは入れない。

use ikari_resource::app_version::{AppVersion, VersionVerdict, verdict};
use ikari_resource::fetch_report::ChildBox;
pub use ikari_resource::fetch_report::FetchErrorCode;

use super::{Phase, Snapshot};

/// 取得の失敗の文（理由の語から。ADR 0076 §8.2）。URL、ホスト、パスは入れない
pub const fn fetch_error(code: FetchErrorCode) -> &'static str {
    match code {
        FetchErrorCode::Cancelled => "取り消しました",
        FetchErrorCode::Network => {
            "つながりませんでした。ネットワークとプロキシの設定を確かめてください"
        }
        FetchErrorCode::Tls => "安全な接続を確かめられませんでした",
        FetchErrorCode::HttpStatus => "配布元が受け付けませんでした",
        FetchErrorCode::Stalled => "通信が止まりました",
        FetchErrorCode::TooLarge => "ファイルが大きすぎます",
        FetchErrorCode::Checksum => "取ったファイルが期待したものと違いました",
        FetchErrorCode::DiskFull => "ディスクの空きが足りません",
        FetchErrorCode::BadUrl => "この URL は使えません",
        FetchErrorCode::BadReply => "配布元の返事が読めませんでした",
        FetchErrorCode::Tool => "取得の部品を起動できませんでした",
        FetchErrorCode::Busy => "ほかの取得が進んでいます",
        FetchErrorCode::Refused => "この権限では取得できません",
        FetchErrorCode::Internal => "取得に失敗しました",
    }
}

const MEGABYTE: u64 = 1024 * 1024;

/// 取得の子を AppContainer に入れられず、低い整合性だけで動かしたときの印（ADR 0078 §3.2 の 5。2026-11-19 ユーザー決定の
/// 「檻なし」に当たる）。取得の様子の文の後ろに付ける
pub const FETCH_UNBOXED: &str =
    "（隔離なし: 通信する部品を AppContainer に入れられず、低い権限だけで動かしています）";

/// 取得の今の様子の文。取得の子が低い整合性だけで動いたときは、どの段階でも印を付ける
pub fn fetch_snapshot(s: &Snapshot) -> String {
    let text = fetch_phase(s);
    match s.child_box {
        Some(ChildBox::Low) => format!("{text}{FETCH_UNBOXED}"),
        Some(ChildBox::AppContainer) | None => text,
    }
}

fn fetch_phase(s: &Snapshot) -> String {
    match s.phase {
        Phase::Running => match s.total {
            Some(total) if total > 0 => format!(
                "取得しています（{} MB / {} MB）",
                s.received / MEGABYTE,
                total / MEGABYTE
            ),
            _ => format!("取得しています（{} MB）", s.received / MEGABYTE),
        },
        Phase::Done => match &s.latest {
            Some(latest) => version_done(latest),
            None => fetch_done(s),
        },
        Phase::Cancelled => fetch_error(FetchErrorCode::Cancelled).to_owned(),
        Phase::Failed => fetch_error(s.error.unwrap_or(FetchErrorCode::Internal)).to_owned(),
    }
}

/// モデルの取得が終わったときの文（D-4: sha256 なしで取ったときは、数えた sha256 を出す。ADR 0076 §18.1）
fn fetch_done(s: &Snapshot) -> String {
    let _ = s;
    "取得しました。内容を確かめています".to_owned()
}

/// 版の確認が終わったときの文（`latest` の版から。比べ方は `ikari_resource::app_version::verdict`）
fn version_done(latest: &str) -> String {
    version_verdict(latest, env!("CARGO_PKG_VERSION"))
}

const VERSION_NEWER: &str = "新しい版があります";
const VERSION_UP_TO_DATE: &str = "最新です";
const VERSION_AHEAD: &str = "この版のほうが新しい（開発版）";
const VERSION_UNKNOWN: &str = "確かめられませんでした";

/// 版の確認の表示（3 つだけ: 新しい版があります・最新です・この版のほうが新しい（開発版）。どちらかの版が
/// 読めなければ「確かめられませんでした」）。古い版は勧めない
pub fn version_verdict(latest: &str, current: &str) -> String {
    let Some(latest) = AppVersion::parse(latest) else {
        return VERSION_UNKNOWN.to_owned();
    };
    match verdict(current, latest) {
        Some(VersionVerdict::Newer(v)) => format!("{VERSION_NEWER}（{v}）"),
        Some(VersionVerdict::UpToDate) => VERSION_UP_TO_DATE.to_owned(),
        Some(VersionVerdict::Ahead) => VERSION_AHEAD.to_owned(),
        None => VERSION_UNKNOWN.to_owned(),
    }
}

/// 自由指定の URL・ファイルで sha256 を入れなかったときの警告（OS で分けない。「sha256 なしでは、通信の経路だけが
/// 頼り」と書く。ADR 0076 §4.3、§18.1）。契約のスタブ（D-4）: 文はまだ無い
pub fn custom_model_without_sha256_warning() -> &'static str {
    ""
}

/// sha256 なしで取った・取り込んだあとに、数えた sha256 を見せる文（配布元の値と比べてもらう。ADR 0076 §18.1）。
/// 契約のスタブ（D-4）
pub fn counted_sha256(sha256: &str) -> String {
    sha256.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unboxed_mark_is_added_only_for_a_low_child() {
        let mut s = Snapshot {
            phase: Phase::Running,
            received: 0,
            total: None,
            error: None,
            bundle: None,
            sha256: None,
            latest: None,
            child_box: None,
        };
        let plain = fetch_snapshot(&s);
        assert!(!plain.contains(FETCH_UNBOXED));
        s.child_box = Some(ChildBox::AppContainer);
        assert_eq!(fetch_snapshot(&s), plain);
        s.child_box = Some(ChildBox::Low);
        assert_eq!(fetch_snapshot(&s), format!("{plain}{FETCH_UNBOXED}"));
        s.phase = Phase::Failed;
        s.error = Some(FetchErrorCode::Network);
        assert!(
            fetch_snapshot(&s).ends_with(FETCH_UNBOXED),
            "失敗でも印を残す"
        );
    }

    #[test]
    fn version_verdict_has_three_texts_and_never_recommends_older() {
        assert_eq!(
            version_verdict("0.3.2", "0.3.1"),
            "新しい版があります（0.3.2）"
        );
        assert_eq!(version_verdict("0.3.1", "0.3.1"), "最新です");
        assert_eq!(
            version_verdict("0.3.0", "0.3.1"),
            "この版のほうが新しい（開発版）"
        );
        assert_eq!(version_verdict("x", "0.3.1"), "確かめられませんでした");
        assert_eq!(version_verdict("0.3.1", "dev"), "確かめられませんでした");
    }

    #[test]
    fn version_done_uses_the_own_version() {
        let text = version_done("999999999.0.0");
        assert!(text.starts_with("新しい版があります"), "{text}");
    }
}
