//! 設定アプリと殻のブリッジの約束（ADR 0005 §6.3。今の Swift の `BridgeProtocol`、`BridgeLimits`、`Frame.error`、
//! `BridgeHandshake`）。

use super::proto::Frame;
use super::proto::{
    BridgeError, BridgeErrorCode, BridgeHello, BridgeHelloReply, Event, OpenTab, SettingEntry,
    SettingsChanged, SettingsSnapshot, event, frame::Body,
};

/// `BridgeHello.bridge_version`。互換を壊す変更のときに上げる
pub const BRIDGE_VERSION: u32 = 1;
/// 殻が待つソケットの、Application Support の下の相対パス（docs/phase2-contract.md §5.1）
pub const SOCKET_RELATIVE_PATH: &str = "run/bridge.sock";
/// 長さの接頭辞の大きさ（バイト。ビッグエンディアンの u32）
pub const LENGTH_PREFIX_BYTES: usize = 4;
/// 1 つの枠（protobuf の本体）の上限。エンジンの `MAX_ADMIN_REQUEST_BYTES` と同じ（ADR 0005 §8.4）
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// `BridgeError` の枠。`key` は不正だった設定のキー名だけを入れる（ユーザーの値を入れない）
pub fn error_frame(request_id: u64, code: BridgeErrorCode, key: &str) -> Frame {
    Frame {
        request_id,
        body: Some(Body::Error(BridgeError {
            code: code as i32,
            key: key.to_owned(),
        })),
    }
}

/// 殻側: 設定アプリの `BridgeHello` に返事を作る。版が合わなければ `VERSION_MISMATCH` の `BridgeError`
pub fn hello_reply(hello: &BridgeHello, request_id: u64, shell_version: &str) -> Frame {
    if hello.bridge_version != BRIDGE_VERSION {
        return error_frame(request_id, BridgeErrorCode::VersionMismatch, "");
    }
    Frame {
        request_id,
        body: Some(Body::HelloReply(BridgeHelloReply {
            bridge_version: BRIDGE_VERSION,
            shell_version: shell_version.to_owned(),
        })),
    }
}

/// 設定アプリ側: 最初に送る枠
pub fn hello(request_id: u64, app_version: &str) -> Frame {
    Frame {
        request_id,
        body: Some(Body::Hello(BridgeHello {
            bridge_version: BRIDGE_VERSION,
            app_version: app_version.to_owned(),
        })),
    }
}

/// 接続の最初の枠の扱い（今の `BridgeServer.serve` の冒頭）
#[derive(Debug, Clone, PartialEq)]
pub enum Handshake {
    /// 返事を送り、以後の枠を受け付ける
    Accept(Frame),
    /// 返事（版の不一致）を送ってから閉じる
    Refuse(Frame),
    /// 最初の枠が `BridgeHello` でない。何も送らずに閉じる
    Close,
}

pub fn handshake(first: &Frame, shell_version: &str) -> Handshake {
    let Some(Body::Hello(hello)) = &first.body else {
        return Handshake::Close;
    };
    let reply = hello_reply(hello, first.request_id, shell_version);
    if matches!(reply.body, Some(Body::Error(_))) {
        Handshake::Refuse(reply)
    } else {
        Handshake::Accept(reply)
    }
}

/// 設定が変わった（メニュー、設定アプリ、手の編集）の通知。つながっている設定アプリは取り直す
pub fn settings_changed_event() -> Frame {
    event_frame(event::Kind::SettingsChanged(SettingsChanged {}))
}

/// すでに動いている設定アプリに、開くタブを伝える通知（`tab` は `SettingsTab` の名前）
pub fn open_tab_event(tab: &str) -> Frame {
    event_frame(event::Kind::OpenTab(OpenTab {
        tab: tab.to_owned(),
    }))
}

fn event_frame(kind: event::Kind) -> Frame {
    Frame {
        request_id: 0,
        body: Some(Body::Event(Event { kind: Some(kind) })),
    }
}

/// 今効いている設定の一式の枠（今の `BridgeServer.snapshotFrame`）
pub fn snapshot_frame<'a>(
    request_id: u64,
    effective: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Frame {
    Frame {
        request_id,
        body: Some(Body::Settings(SettingsSnapshot {
            effective: effective
                .into_iter()
                .map(|(key, value)| SettingEntry {
                    key: key.to_owned(),
                    value: value.to_owned(),
                })
                .collect(),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_a_matching_version_with_the_shell_version() {
        let first = hello(1, "0.3.0");
        let Some(Body::Hello(message)) = &first.body else {
            panic!("expected a hello");
        };
        assert_eq!(message.bridge_version, BRIDGE_VERSION);
        let reply = hello_reply(message, first.request_id, "0.3.0");
        assert_eq!(reply.request_id, 1);
        let Some(Body::HelloReply(body)) = &reply.body else {
            panic!("expected a hello reply, got {reply:?}");
        };
        assert_eq!(body.bridge_version, BRIDGE_VERSION);
        assert_eq!(body.shell_version, "0.3.0");
        assert_eq!(handshake(&first, "0.3.0"), Handshake::Accept(reply));
    }

    #[test]
    fn answers_a_version_mismatch_with_an_error() {
        let message = BridgeHello {
            bridge_version: BRIDGE_VERSION + 1,
            app_version: String::new(),
        };
        let expected = error_frame(2, BridgeErrorCode::VersionMismatch, "");
        assert_eq!(hello_reply(&message, 2, "0.3.0"), expected);
        let first = Frame {
            request_id: 2,
            body: Some(Body::Hello(message)),
        };
        assert_eq!(handshake(&first, "0.3.0"), Handshake::Refuse(expected));
    }

    #[test]
    fn a_first_frame_that_is_not_a_hello_closes_without_a_reply() {
        let first = snapshot_frame(1, []);
        assert_eq!(handshake(&first, "0.3.0"), Handshake::Close);
        assert_eq!(handshake(&Frame::default(), "0.3.0"), Handshake::Close);
    }

    #[test]
    fn error_frames_carry_only_the_key_name() {
        let frame = error_frame(3, BridgeErrorCode::InvalidSetting, "paging.size");
        let Some(Body::Error(error)) = frame.body else {
            panic!("expected an error");
        };
        assert_eq!(error.code, BridgeErrorCode::InvalidSetting as i32);
        assert_eq!(error.key, "paging.size");
    }

    #[test]
    fn events_are_notifications_with_request_id_zero() {
        let changed = settings_changed_event();
        assert_eq!(changed.request_id, 0);
        let open = open_tab_event("dictionary");
        let Some(Body::Event(Event {
            kind: Some(event::Kind::OpenTab(tab)),
        })) = open.body
        else {
            panic!("expected open_tab");
        };
        assert_eq!(tab.tab, "dictionary");
    }

    #[test]
    fn snapshot_keeps_the_order_of_the_entries() {
        let frame = snapshot_frame(4, [("a", "1"), ("b", "2")]);
        let Some(Body::Settings(snapshot)) = frame.body else {
            panic!("expected settings");
        };
        let keys: Vec<_> = snapshot.effective.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["a", "b"]);
        assert_eq!(frame.request_id, 4);
    }
}
