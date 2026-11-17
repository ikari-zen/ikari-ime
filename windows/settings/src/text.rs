//! 設定画面の結果の欄に出す文（Windows。macOS の `Messages.swift` と同じ言い方にそろえる）。**OS に依存しない**。
//!
//! 文に入れるのは、キーの名前、件数、理由の種類だけ。パスやファイルの中身は入れない。

use ikari_proto::v1::{DataCounts, DataIssue, DataIssueKind, ErrorCode, ExportReply, ImportReply};
use ikari_resource::app_version::{AppVersion, VersionVerdict, verdict};
use ikari_resource::fetch_report::FetchErrorCode;
use ikari_shell_core::bridge::proto::BridgeErrorCode;

use crate::edit::EditError;
use crate::link::LinkError;

pub fn edit_error(e: EditError) -> &'static str {
    match e {
        EditError::UnknownKey => "知らない設定です",
        EditError::KindMismatch | EditError::NotAccepted => "設定の値が正しくありません",
        EditError::NotEditable => "この設定は、この画面ではまだ変えられません",
    }
}

/// 書き出し・取り込みの理由の語（engine-host の `TransferError` とパスの検査の語）
fn transfer_reason(reason: &str) -> &'static str {
    match reason {
        "exists" => "同じ名前のファイルがあります",
        "not-found" => "ファイルが見つかりません",
        "not-a-file" => "ふつうのファイルではありません",
        "too-large" => "ファイルが大きすぎます",
        "empty" => "ファイルが空です",
        "engine" => "変換エンジンにつながりません",
        "io" => "ファイルの読み書きに失敗しました",
        "network-or-device" => "ネットワークの場所やデバイスは使えません",
        "length" | "not-absolute" | "bad-name" | "reserved-name" => {
            "このファイルの場所は使えません"
        }
        _ => "ファイルを読み書きできませんでした",
    }
}

/// 取得の失敗の文（理由の語から。ADR 0076 §8.2）。URL、ホスト、パスは入れない
pub fn fetch_error(code: FetchErrorCode) -> &'static str {
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

/// 取得の今の様子の文
pub fn fetch_snapshot(s: &crate::fetch::Snapshot) -> String {
    use crate::fetch::Phase;
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
fn fetch_done(s: &crate::fetch::Snapshot) -> String {
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

pub fn link_error(e: &LinkError) -> String {
    match e {
        LinkError::Status(status) => status.text().to_owned(),
        LinkError::Refused(reason) => transfer_reason(reason).to_owned(),
        LinkError::Bridge { code, key, reason } => {
            match BridgeErrorCode::try_from(*code)
                .unwrap_or(BridgeErrorCode::BridgeErrorUnspecified)
            {
                BridgeErrorCode::InvalidSetting if !key.is_empty() => {
                    format!("設定「{key}」の値が正しくありません")
                }
                BridgeErrorCode::InvalidSetting => "設定の値が正しくありません".to_owned(),
                BridgeErrorCode::NotAllowed => "この操作は許可されていません".to_owned(),
                BridgeErrorCode::EngineUnavailable => "変換エンジンにつながりません".to_owned(),
                BridgeErrorCode::TooLarge => "データが大きすぎます".to_owned(),
                BridgeErrorCode::WriteFailed => "設定ファイルを書き込めませんでした".to_owned(),
                BridgeErrorCode::TransferFailed => transfer_reason(reason).to_owned(),
                BridgeErrorCode::VersionMismatch => "エンジンと設定画面の版が合いません".to_owned(),
                BridgeErrorCode::BridgeErrorUnspecified => {
                    "予期しないエラーが起きました".to_owned()
                }
            }
        }
        LinkError::Engine(code) if *code == ErrorCode::EngineNotReady as i32 => {
            "変換エンジンを準備中です。しばらくしてからもう一度お試しください".to_owned()
        }
        LinkError::Engine(_) => "変換エンジンがエラーを返しました".to_owned(),
        LinkError::Unexpected => "予期しないエラーが起きました".to_owned(),
    }
}

fn counts(c: Option<&DataCounts>) -> String {
    let c = c.cloned().unwrap_or_default();
    format!(
        "辞書 {} 語、学習 {} 行、分野 {}、取り込み元 {}",
        c.user_dict_entries, c.learning_rows, c.profile_domains, c.ingest_sources
    )
}

fn issue(issue: &DataIssue) -> String {
    let section = if issue.section.is_empty() {
        String::new()
    } else {
        format!("（{}）", issue.section)
    };
    match DataIssueKind::try_from(issue.kind).unwrap_or(DataIssueKind::DataIssueNone) {
        DataIssueKind::NotIkariExport => "ikari の書き出しファイルではありません".to_owned(),
        DataIssueKind::SchemaUnsupported => format!(
            "このファイルの形式の版 {}{section}には対応していません。対応しているのは版 {} です",
            issue.found_schema, issue.supported_schema
        ),
        DataIssueKind::Parse if issue.line > 0 => {
            format!("{} 行目{section}を読めません", issue.line)
        }
        DataIssueKind::Parse => format!("ファイル{section}を読めません"),
        DataIssueKind::InvalidEntry => "登録できない項目があります".to_owned(),
        DataIssueKind::NotFound => "該当する項目がありません".to_owned(),
        DataIssueKind::TooLarge => "データが大きすぎます".to_owned(),
        DataIssueKind::Io => "ファイルの読み書きに失敗しました".to_owned(),
        DataIssueKind::DataIssueNone => "不明な問題が起きました".to_owned(),
    }
}

pub fn exported(reply: &ExportReply) -> String {
    format!("書き出しました（{}）", counts(reply.counts.as_ref()))
}

/// 取り込みの結果。`dry_run` は確かめただけ（何も変えていない）
pub fn imported(reply: &ImportReply, dry_run: bool) -> String {
    if let Some(i) = reply.issue.as_ref().filter(|i| i.kind != 0) {
        return format!("{}。何も変更していません", issue(i));
    }
    if dry_run {
        format!("取り込めます（{}）", counts(reply.counts.as_ref()))
    } else if reply.applied {
        format!("取り込みました（{}）", counts(reply.counts.as_ref()))
    } else {
        "取り込めませんでした。何も変更していません".to_owned()
    }
}

/// 書き換えのあとの文。エンジンを起こし直す設定なら、そう添える
pub fn updated(engine_restart: bool) -> &'static str {
    if engine_restart {
        "変更しました。変換エンジンを起こし直します（数秒は辞書だけで変換します）"
    } else {
        "変更しました"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_failures_never_echo_the_path() {
        let text = link_error(&LinkError::Bridge {
            code: BridgeErrorCode::TransferFailed as i32,
            key: String::new(),
            reason: "exists".to_owned(),
        });
        assert_eq!(text, "同じ名前のファイルがあります");
        assert_eq!(
            link_error(&LinkError::Refused("reserved-name")),
            "このファイルの場所は使えません"
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

    #[test]
    fn import_texts() {
        let ok = ImportReply {
            applied: false,
            counts: Some(DataCounts {
                user_dict_entries: 2,
                learning_rows: 5,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            imported(&ok, true),
            "取り込めます（辞書 2 語、学習 5 行、分野 0、取り込み元 0）"
        );
        let bad = ImportReply {
            issue: Some(DataIssue {
                kind: DataIssueKind::Parse as i32,
                line: 3,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            imported(&bad, true),
            "3 行目を読めません。何も変更していません"
        );
    }
}
