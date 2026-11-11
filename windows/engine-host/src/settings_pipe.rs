//! 設定画面が engine-host に話すパイプ（ADR 0040 §2.5、§5 の W4-s の契約。W4-s1 の宣言、W4-s2 の実装）。
//!
//! - 相手: 設定画面は、TIP と同じ engine-host に話す（TIP の DLL はアプリごとにあり、画面が話す相手を 1 つに決められないため）。
//!   ユーザー辞書と学習の管理の要求（ADR 0005 §8.3）は、engine-host の中でエンジンに渡す。
//! - 名前: エンジンのパイプとは別のパイプ（`ikari_pipe::settings_pipe_name`。接頭辞は `ikari_pipe::SETTINGS_NAME_PREFIX`
//!   の 1 か所）。TIP の通り道に設定画面の要求を混ぜず、アクセスの制御（ユーザーの SID だけ）を別々に持てるようにするため。
//!   設定画面の接続が切れても、エンジンのパイプ（`serve`）には何も起きない（別の `Server`、別のスレッド）。
//! - 枠: `ikari_pipe::Frame`（`Request` と `Reply`。通し番号は同じ規則）。本体は shell-core の `bridge` の `Frame`
//!   （`proto/ikari/bridge/v1/bridge.proto`。macOS の設定アプリと殻が話すものと同じ。長さの接頭辞は付けない。枠はパイプの
//!   枠が持つ）。`Hello` → `GetSettings` / `UpdateSettings` / `Engine`（管理の要求の中継）。判断（最初の枠の照合、
//!   中継の許可リストと同時数、設定の検査と書き換え）は shell-core の `bridge::{handshake, route}` と
//!   `settings::SettingsStore` が行い、ここは枠を運んで、エンジンに渡すだけ（macOS の `BridgeServer.swift` に相当する層）。
//! - `ikari.settings.v1` の `AppRequest` / `AppReply`（`ikari_settings_app`。状態を持たない純関数の口）は、このパイプでは
//!   運ばない。答えるのは `ikari_shell_core::settings` の関数で、画面が同じプロセスの中で呼べる（W4-s3 の方式で決める）。
//!   運ぶ必要が出たら、`Frame` の種類を足すのでなく、版を分けた別の枠にする（`SETTINGS_PIPE_PROTOCOL`）。
//! - 上限: パイプの枠は `ikari_pipe::MAX_FRAME_LEN`（16 MiB）。超える要求は、枠の本体を読まずに接続を切る
//!   （枠の位置が分からなくなるため、返事を返せない）。画面は送る前に `settings::import_size` で
//!   見るので（`MAX_IMPORT_BYTES` は 64 MiB。16 MiB を超える分は未解決。PR に書いた）、通常は起きない。
//!   返事が上限を超えるとき（大きな `ExportData`）は、接続を保ったまま `TOO_LARGE` の枠を返す。
//! - TIP が設定の変更を知る方法（ファイルの変更を見るか、engine-host から知らせるか）は W4-s2 では決めない
//!   （この口は `settings.tsv` を書くだけ。`SettingsChanged` の通知は、要求と返事の流れでは送れないので送らない。画面は
//!   `UpdateSettings` の返事で反映後の一式を受け取る）。
//!
//! 判断の置き場所: Rust。判断は shell-core の bridge/settings。engine-host は枠を運ぶだけ。

use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

use ikari_engine::Engine;
use ikari_pipe::frame::KIND_SEQ_BYTES;
use ikari_pipe::{FrameKind, Listener, MAX_FRAME_LEN, Server};
use ikari_shell_core::SettingsEntry;
use ikari_shell_core::bridge::proto::{BridgeErrorCode, Frame, UpdateSettings};
use ikari_shell_core::bridge::protocol::{error_frame, snapshot_frame};
use ikari_shell_core::bridge::server::{relay_reply, update_failure};
use ikari_shell_core::bridge::{Handshake, RelayGate, Route, frame_codec, handshake, route};
use ikari_shell_core::settings::{
    SettingsStore, StoreError, effective_entries, is_acceptable, is_known_key,
};
use prost::Message;

/// 枠の本体のプロトコル。`bridge.proto` の `Frame`（版は `BridgeHello.bridge_version`。接続の最初の `Hello` で照合し、
/// 合わなければ `VERSION_MISMATCH` を返す。判断は shell-core の `handshake`）。
pub const SETTINGS_PIPE_PROTOCOL: &str = "ikari.bridge.v1";

/// 設定画面のパイプで使う枠の種類。`Post`（返事のない通知）は、画面からは送らない（画面の要求はすべて返事を待つ）。
pub const SETTINGS_PIPE_FRAME_KINDS: [FrameKind; 2] = [FrameKind::Request, FrameKind::Reply];

/// 返事の本体の上限（バイト）。パイプの枠の上限から、種類と通し番号の分を引いたもの
const MAX_REPLY_BODY_BYTES: usize = MAX_FRAME_LEN as usize - KIND_SEQ_BYTES;

/// 設定画面の接続が共有するもの。`Engine` はエンジンのパイプと同じ 1 つ。
#[derive(Clone)]
pub struct SettingsHost {
    engine: Arc<Engine>,
    store: Arc<Mutex<SettingsStore>>,
    gate: Arc<RelayGate>,
    host_version: String,
}

impl SettingsHost {
    /// `store` は `settings.tsv` の置き場所を持つ（書き手は、`UpdateSettings` を受けるここだけ）。
    /// `host_version` は `HelloReply.shell_version` に入れる（アプリの版）。
    pub fn new(engine: Arc<Engine>, store: SettingsStore, host_version: &str) -> Self {
        Self {
            engine,
            store: Arc::new(Mutex::new(store)),
            gate: Arc::new(RelayGate::default()),
            host_version: host_version.to_owned(),
        }
    }

    /// 接続 1 本の処理。`Hello` を交わすまでの状態は接続ごとに持つ。
    fn session(&self) -> impl FnMut(&[u8]) -> Option<Vec<u8>> + Send + 'static + use<> {
        let host = self.clone();
        let mut ready = false;
        move |body: &[u8]| host.handle(&mut ready, body)
    }

    /// 枠の本体（`bridge` の `Frame`）1 つに答える。`None` は答えを作れない（パイプの層が `Error` の枠を返す）。
    /// `ready` は、この接続が `Hello` を交わし終えたか。
    fn handle(&self, ready: &mut bool, body: &[u8]) -> Option<Vec<u8>> {
        // 壊れた本体。パイプの枠は正しいので、枠の位置は分かる。接続は保ち、`Error` で知らせる
        let frame = frame_codec::decode_body(body).ok()?;
        if !*ready {
            return match handshake(&frame, &self.host_version) {
                Handshake::Accept(reply) => {
                    *ready = true;
                    Some(encode(reply))
                }
                Handshake::Refuse(reply) => Some(encode(reply)),
                // 最初の枠が `Hello` でない。macOS は何も返さずに閉じる。ここは閉じられないので `Error` にする
                Handshake::Close => None,
            };
        }
        let reply = match route(frame, &self.gate) {
            Route::Reply(reply) => reply,
            Route::GetSettings { request_id } => self.snapshot(request_id),
            Route::UpdateSettings { request_id, update } => self.update(request_id, update),
            Route::Relay(job) => {
                // エンジンは同じプロセスにいる。`EngineLink` の待ち（時間切れ）は要らず、そのまま渡す
                let reply = self.engine.handle(&job.envelope);
                relay_reply(job.request_id, Ok(reply))
            }
        };
        Some(encode(reply))
    }

    fn store(&self) -> MutexGuard<'_, SettingsStore> {
        // 書き込み中の panic で錠が毒になっても、設定の画面を止めない
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 今効いている設定の一式（手の編集があれば読み直してから）
    fn snapshot(&self, request_id: u64) -> Frame {
        let mut store = self.store();
        let _ = store.reload_if_file_changed();
        snapshot_of(request_id, &store)
    }

    fn update(&self, request_id: u64, update: UpdateSettings) -> Frame {
        if let Some(key) = invalid_key(&update) {
            return update_failure(request_id, Some(&key));
        }
        let set = update
            .set
            .into_iter()
            .map(|entry| SettingsEntry {
                key: entry.key,
                value: entry.value,
            })
            .collect();
        let mut store = self.store();
        match store.update(set, update.reset) {
            Ok(_) => snapshot_of(request_id, &store),
            // 書いたあとの警告が増える（キー名は入れない。macOS と同じ）
            Err(StoreError::Rejected { .. }) => update_failure(request_id, Some("")),
            Err(StoreError::WriteFailed) => update_failure(request_id, None),
        }
    }
}

fn snapshot_of(request_id: u64, store: &SettingsStore) -> Frame {
    let entries = effective_entries(store.snapshot());
    snapshot_frame(
        request_id,
        entries.iter().map(|(k, v)| (k.as_str(), v.as_str())),
    )
}

/// 書き換える前の検査。不正なら誤りの枠に入れるキー名（知らないキーは空。ユーザーの値は入れない）。
/// shell-core の `controller::bridge_link::invalid_key`（非公開）と同じ規則。
/// TODO: shell-core で公開して、これを消す（判断が 2 か所にならないように）。
fn invalid_key(update: &UpdateSettings) -> Option<String> {
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

/// 返す枠の本体。パイプの枠に入らない大きさなら、理由だけ返す（接続は保つ）
fn encode(frame: Frame) -> Vec<u8> {
    let bytes = frame.encode_to_vec();
    if bytes.len() <= MAX_REPLY_BODY_BYTES {
        return bytes;
    }
    let request_id = frame.request_id;
    drop(frame);
    error_frame(request_id, BridgeErrorCode::TooLarge, "").encode_to_vec()
}

/// このプロセスのユーザーとセッションの、設定画面のパイプの受け口（`cfg(windows)`。Windows の CI か実機で確かめる）。
/// DACL は自分のユーザーと SYSTEM だけ（`NamedPipeListener::bind`）。
#[cfg(windows)]
pub fn bind_settings_listener() -> io::Result<ikari_pipe::os::windows::NamedPipeListener> {
    use ikari_pipe::os::windows::{current_session_id, current_user_sid};
    let name = ikari_pipe::settings_pipe_name(&current_user_sid()?, current_session_id()?)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "unexpected user SID format"))?;
    ikari_pipe::os::windows::NamedPipeListener::bind(&name)
}

/// 受け口が閉じられるまで、設定画面の接続を受ける。`Server` はエンジンのパイプとは別に持つ。
/// 戻る前に、残っている接続を閉じてスレッドを待つ。
pub fn serve_settings<L: Listener>(listener: L, host: SettingsHost) -> io::Result<()> {
    Server::new(listener).serve(move || host.session())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikari_shell_core::bridge::proto::frame::Body;

    mod envelope {
        use ikari_proto::v1::{Envelope, ExportReply, envelope::Body};

        /// 本体が `bytes` バイトの `ExportData` の返事
        pub fn exported(bytes: usize) -> Envelope {
            Envelope {
                schema_version: 1,
                request_id: 0,
                body: Some(Body::ExportReply(ExportReply {
                    content: vec![b'x'; bytes],
                    ..Default::default()
                })),
            }
        }
    }

    #[test]
    fn settings_pipe_is_a_different_pipe_from_the_engine_pipe() {
        let engine = ikari_pipe::pipe_name("S-1-5-21-1-2-3-500", 1).unwrap();
        let settings = ikari_pipe::settings_pipe_name("S-1-5-21-1-2-3-500", 1).unwrap();
        assert!(!engine.starts_with(ikari_pipe::SETTINGS_NAME_PREFIX));
        assert!(settings.starts_with(ikari_pipe::SETTINGS_NAME_PREFIX));
        assert_ne!(engine, settings);
        assert_eq!(SETTINGS_PIPE_PROTOCOL, "ikari.bridge.v1");
    }

    #[test]
    fn a_reply_over_the_pipe_limit_becomes_too_large() {
        let big = Frame {
            request_id: 7,
            body: Some(Body::Engine(envelope::exported(MAX_REPLY_BODY_BYTES))),
        };
        let bytes = encode(big);
        let reply = Frame::decode(bytes.as_slice()).unwrap();
        assert_eq!(reply.request_id, 7);
        let Some(Body::Error(e)) = reply.body else {
            panic!("expected an error");
        };
        assert_eq!(e.code, BridgeErrorCode::TooLarge as i32);
    }
}
