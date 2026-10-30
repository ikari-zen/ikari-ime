//! 設定アプリとの接続の状態（今の Swift の `BridgeServer` の `serve`・`handle`・`apply(UpdateSettings)`・
//! `snapshotFrame`・`notifySettingsChanged`・`sendOpenTab` の判断。L-E で `Shell` につないだ）。
//!
//! ソケットの待ち受け、接続元の確認、接続ごとの読み書きは OS 側に残す。OS は受けた枠を
//! `ShellEvent::BridgeFrame` で渡し、`ShellAction::BridgeReply`・`BridgeClose` を実行するだけにする。

use std::sync::Arc;

use prost::Message;

use crate::bridge::proto::{Frame, UpdateSettings};
use crate::bridge::protocol::{open_tab_event, settings_changed_event, snapshot_frame};
use crate::bridge::server::update_failure;
use crate::bridge::{Handshake, RelayGate, RelayJob, Route, frame_codec, handshake, route};
use crate::settings::{effective_entries, is_acceptable, is_known_key};
use crate::types::{ConnId, SettingsSnapshot, SettingsTab, ShellAction};

/// 枠 1 つを振り分けた結果のうち、`Shell` が続きを行うもの
pub(crate) enum BridgeWork {
    /// 設定を書き換えて、反映後の一式（失敗なら誤りの枠）を返す
    Update {
        conn: ConnId,
        request_id: u64,
        update: UpdateSettings,
    },
    /// エンジンへ中継する（主スレッドの外で）
    Relay { conn: ConnId, job: RelayJob },
}

/// 設定アプリとの接続（同時に 1 つ。2 つ目は OS 側が断る）。
#[derive(Default)]
pub(crate) struct BridgeLink {
    /// `BridgeHello` を交わし終えた接続。殻からの通知はこの接続にだけ送る
    ready: Option<ConnId>,
    gate: Arc<RelayGate>,
}

impl BridgeLink {
    /// 届いた枠（`FrameCodec` の本体）を振り分ける。すぐ返す動作と、`Shell` が続ける仕事を返す。
    pub(crate) fn receive(
        &mut self,
        conn: ConnId,
        bytes: &[u8],
        shell_version: &str,
        settings: &SettingsSnapshot,
    ) -> (Vec<ShellAction>, Option<BridgeWork>) {
        let Ok(frame) = frame_codec::decode_body(bytes) else {
            // 壊れた枠。枠の位置が分からなくなるので閉じる（設定アプリはつなぎ直す）
            return (self.close(conn), None);
        };
        if self.ready != Some(conn) {
            // この接続の最初の枠
            return match handshake(&frame, shell_version) {
                Handshake::Accept(reply) => {
                    self.ready = Some(conn);
                    (vec![reply_action(conn, &reply)], None)
                }
                Handshake::Refuse(reply) => {
                    let mut actions = vec![reply_action(conn, &reply)];
                    actions.extend(self.close(conn));
                    (actions, None)
                }
                Handshake::Close => (self.close(conn), None),
            };
        }
        match route(frame, &self.gate) {
            Route::Reply(reply) => (vec![reply_action(conn, &reply)], None),
            Route::GetSettings { request_id } => {
                (vec![snapshot_action(conn, request_id, settings)], None)
            }
            Route::UpdateSettings { request_id, update } => (
                Vec::new(),
                Some(BridgeWork::Update {
                    conn,
                    request_id,
                    update,
                }),
            ),
            Route::Relay(job) => (Vec::new(), Some(BridgeWork::Relay { conn, job })),
        }
    }

    /// OS 側で接続が切れた。
    pub(crate) fn closed(&mut self, conn: ConnId) {
        if self.ready == Some(conn) {
            self.ready = None;
        }
    }

    /// 中継の結果を返す（その間に接続が替わっていたら捨てる）。
    pub(crate) fn relay_done(&self, conn: ConnId, frame: &Frame) -> Option<ShellAction> {
        (self.ready == Some(conn)).then(|| reply_action(conn, frame))
    }

    /// 設定が変わった（メニュー、設定アプリ、手の編集）。つながっている設定アプリは取り直す
    pub(crate) fn settings_changed(&self) -> Option<ShellAction> {
        self.ready
            .map(|conn| reply_action(conn, &settings_changed_event()))
    }

    /// すでに動いている設定アプリに、開くタブを伝える（起動の引数は動作中のアプリには届かない）
    pub(crate) fn open_tab(&self, tab: SettingsTab) -> Option<ShellAction> {
        self.ready
            .map(|conn| reply_action(conn, &open_tab_event(tab_name(tab))))
    }

    fn close(&mut self, conn: ConnId) -> Vec<ShellAction> {
        self.closed(conn);
        vec![ShellAction::BridgeClose { conn }]
    }
}

/// 書き換える前の検査（今の Swift の `SettingsStore.update` の冒頭）。不正なら誤りの枠のキー名
/// （知らないキーは空。ユーザーの値は入れない）
pub(crate) fn invalid_key(update: &UpdateSettings) -> Option<String> {
    for entry in &update.set {
        if !is_acceptable(&entry.key, &entry.value) {
            return Some(if is_known_key(&entry.key) {
                entry.key.clone()
            } else {
                String::new()
            });
        }
    }
    update
        .reset
        .iter()
        .any(|key| !is_known_key(key))
        .then(String::new)
}

/// 書き換えの結果の枠。`Ok` は反映後の一式、`Err(Some(key))` は不正（キー名だけ）、`Err(None)` は書けなかった
pub(crate) fn update_reply(
    conn: ConnId,
    request_id: u64,
    result: Result<&SettingsSnapshot, Option<&str>>,
) -> ShellAction {
    match result {
        Ok(settings) => snapshot_action(conn, request_id, settings),
        Err(invalid_key) => reply_action(conn, &update_failure(request_id, invalid_key)),
    }
}

fn snapshot_action(conn: ConnId, request_id: u64, settings: &SettingsSnapshot) -> ShellAction {
    let entries = effective_entries(settings);
    let frame = snapshot_frame(
        request_id,
        entries
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    reply_action(conn, &frame)
}

/// 返す枠は protobuf の本体（長さの接頭辞と上限の検査は OS 側の輸送が行う）
fn reply_action(conn: ConnId, frame: &Frame) -> ShellAction {
    ShellAction::BridgeReply {
        conn,
        frame: frame.encode_to_vec(),
    }
}

/// ブリッジの `OpenTab.tab` に入れる名前（設定アプリの `SettingsTab` の `rawValue`）
fn tab_name(tab: SettingsTab) -> &'static str {
    match tab {
        SettingsTab::Input => "input",
        SettingsTab::Conversion => "conversion",
        SettingsTab::UserDictionary => "dictionary",
        SettingsTab::Learning => "learning",
        SettingsTab::About => "about",
    }
}
