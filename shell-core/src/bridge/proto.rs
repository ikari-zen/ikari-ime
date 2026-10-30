//! `proto/ikari/bridge/v1/bridge.proto` の Rust の型（手で書いた prost のメッセージ）。
//!
//! `proto/build.rs` は bridge.proto を生成しない（Swift だけが使っていた）。契約のファイルを変えないため、
//! L-B では同じ欄の番号の型をここに手で書き、下のテストで bridge.proto の欄の番号と照らし合わせる。
//! 生成に切り替えるとき（`proto/build.rs` に bridge.proto を足す）は、このファイルを `ikari_proto` の再輸出に替える。

use ikari_proto::v1::Envelope;

#[derive(Clone, PartialEq, prost::Message)]
pub struct Frame {
    /// 設定アプリが付ける。殻からの通知は 0
    #[prost(uint64, tag = "1")]
    pub request_id: u64,
    #[prost(oneof = "frame::Body", tags = "10, 11, 12, 13, 14, 20, 30, 31")]
    pub body: Option<frame::Body>,
}

pub mod frame {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Body {
        #[prost(message, tag = "10")]
        Hello(super::BridgeHello),
        #[prost(message, tag = "11")]
        HelloReply(super::BridgeHelloReply),
        #[prost(message, tag = "12")]
        GetSettings(super::GetSettings),
        #[prost(message, tag = "13")]
        Settings(super::SettingsSnapshot),
        /// 返事は SettingsSnapshot（反映後）
        #[prost(message, tag = "14")]
        UpdateSettings(super::UpdateSettings),
        /// 管理の要求と返事。殻はそのままエンジンへ流し、返事もそのまま返す
        #[prost(message, tag = "20")]
        Engine(super::Envelope),
        /// 殻 → 設定アプリ
        #[prost(message, tag = "30")]
        Event(super::Event),
        #[prost(message, tag = "31")]
        Error(super::BridgeError),
    }
}

#[derive(Clone, PartialEq, Eq, Hash, prost::Message)]
pub struct BridgeHello {
    #[prost(uint32, tag = "1")]
    pub bridge_version: u32,
    #[prost(string, tag = "2")]
    pub app_version: String,
}

#[derive(Clone, PartialEq, Eq, Hash, prost::Message)]
pub struct BridgeHelloReply {
    #[prost(uint32, tag = "1")]
    pub bridge_version: u32,
    #[prost(string, tag = "2")]
    pub shell_version: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, prost::Message)]
pub struct GetSettings {}

/// docs/phase2-contract.md §3 のキーと値
#[derive(Clone, PartialEq, Eq, Hash, prost::Message)]
pub struct SettingEntry {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

/// すべての既知のキーの、今効いている値
#[derive(Clone, PartialEq, Eq, Hash, prost::Message)]
pub struct SettingsSnapshot {
    #[prost(message, repeated, tag = "1")]
    pub effective: Vec<SettingEntry>,
}

/// `reset` = その行を消して既定に戻す
#[derive(Clone, PartialEq, Eq, Hash, prost::Message)]
pub struct UpdateSettings {
    #[prost(message, repeated, tag = "1")]
    pub set: Vec<SettingEntry>,
    #[prost(string, repeated, tag = "2")]
    pub reset: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Hash, prost::Message)]
pub struct Event {
    #[prost(oneof = "event::Kind", tags = "1, 2")]
    pub kind: Option<event::Kind>,
}

pub mod event {
    #[derive(Clone, PartialEq, Eq, Hash, prost::Oneof)]
    pub enum Kind {
        #[prost(message, tag = "1")]
        SettingsChanged(super::SettingsChanged),
        #[prost(message, tag = "2")]
        OpenTab(super::OpenTab),
    }
}

/// メニューや手の編集で変わった。設定アプリは取り直す
#[derive(Clone, Copy, PartialEq, Eq, Hash, prost::Message)]
pub struct SettingsChanged {}

/// `tab` は `SettingsTab` の名前（Swift の rawValue）
#[derive(Clone, PartialEq, Eq, Hash, prost::Message)]
pub struct OpenTab {
    #[prost(string, tag = "1")]
    pub tab: String,
}

/// `key` は不正だった設定のキー名だけ
#[derive(Clone, PartialEq, Eq, Hash, prost::Message)]
pub struct BridgeError {
    #[prost(enumeration = "BridgeErrorCode", tag = "1")]
    pub code: i32,
    #[prost(string, tag = "2")]
    pub key: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum BridgeErrorCode {
    Unspecified = 0,
    VersionMismatch = 1,
    InvalidSetting = 2,
    NotAllowed = 3,
    EngineUnavailable = 4,
    TooLarge = 5,
    WriteFailed = 6,
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    const BRIDGE_PROTO: &str = include_str!("../../../proto/ikari/bridge/v1/bridge.proto");

    /// 手で書いた欄の番号が bridge.proto と同じか（欄の名前と番号の組を文字列で照らす）
    #[test]
    fn field_numbers_match_bridge_proto() {
        let expected = [
            "uint64 request_id = 1;",
            "BridgeHello hello = 10;",
            "BridgeHelloReply hello_reply = 11;",
            "GetSettings get_settings = 12;",
            "SettingsSnapshot settings = 13;",
            "UpdateSettings update_settings = 14;",
            "ikari.engine.v1.Envelope engine = 20;",
            "Event event = 30;",
            "BridgeError error = 31;",
            "message BridgeHello { uint32 bridge_version = 1; string app_version = 2; }",
            "message BridgeHelloReply { uint32 bridge_version = 1; string shell_version = 2; }",
            "message SettingEntry { string key = 1; string value = 2; }",
            "message SettingsSnapshot { repeated SettingEntry effective = 1; }",
            "message UpdateSettings { repeated SettingEntry set = 1; repeated string reset = 2; }",
            "message Event { oneof kind { SettingsChanged settings_changed = 1; OpenTab open_tab = 2; } }",
            "message OpenTab { string tab = 1; }",
            "message BridgeError { BridgeErrorCode code = 1; string key = 2; }",
            "VERSION_MISMATCH = 1; INVALID_SETTING = 2; NOT_ALLOWED = 3;",
            "ENGINE_UNAVAILABLE = 4; TOO_LARGE = 5; WRITE_FAILED = 6;",
        ];
        for line in expected {
            assert!(
                BRIDGE_PROTO.contains(line),
                "bridge.proto に `{line}` がない"
            );
        }
    }

    /// Swift（SwiftProtobuf）と同じバイト列になるか（手で組んだ値と比べる）
    #[test]
    fn wire_bytes_match_the_protobuf_encoding() {
        let frame = Frame {
            request_id: 1,
            body: Some(frame::Body::Hello(BridgeHello {
                bridge_version: 1,
                app_version: String::new(),
            })),
        };
        // 08 01 = request_id 1、52 02 = 欄 10 の長さ 2、08 01 = bridge_version 1
        assert_eq!(frame.encode_to_vec(), [0x08, 0x01, 0x52, 0x02, 0x08, 0x01]);
        let error = Frame {
            request_id: 0,
            body: Some(frame::Body::Error(BridgeError {
                code: BridgeErrorCode::NotAllowed as i32,
                key: String::new(),
            })),
        };
        // fa 01 = 欄 31（長さつき）、02 = 長さ、08 03 = code 3
        assert_eq!(error.encode_to_vec(), [0xFA, 0x01, 0x02, 0x08, 0x03]);
    }
}
