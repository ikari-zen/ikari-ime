//! 設定の口（engine-host）とのつながり（ADR 0068 §4。W4-s3 の第 2 段）。
//!
//! - つなぐ（`connect_installed`。`cfg(windows)`）: 高い権限なら何もしない → このユーザーとセッションの設定の口につなぐ →
//!   **何かを送る前に**、相手の PID から実行ファイルのパスを取り、インストール先の engine-host かを照らす（`peer`）→
//!   `Hello` を交わす（`SettingsLink::open`）。
//! - 運ぶのは `bridge.proto` の `Frame`（パイプの枠の本体。長さの接頭辞は付けない）。待ちの上限は shell-core の
//!   `timeouts`（管理は 3 秒、書き出し・取り込みは 30 秒）。
//! - 書き出しと取り込みは、パスの形を送る前にも確かめる（`check_transfer_path`。受け手も確かめる）。
//!
//! 判断の置き場所: Rust。OS から取るもの（SID、セッション、相手の PID とパス、自分の exe のパス）だけが `cfg(windows)`。

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ikari_pipe::{Client, ClientError};
use ikari_proto::v1::{ErrorCode, ExportReply, ImportReply, envelope};
use ikari_shell_core::bridge::proto::frame::Body;
use ikari_shell_core::bridge::proto::{
    BridgeErrorCode, ExportToFile, Frame, GetSettings, ImportFromFile, SettingsSnapshot,
    UpdateSettings,
};
use ikari_shell_core::bridge::protocol::hello;
use ikari_shell_core::bridge::{LinkStatus, check_transfer_path, frame_codec};
use ikari_shell_core::timeouts;
use prost::Message;

/// 送れなかった・期待の返事でなかった理由
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    /// つながりが使えない（状態の欄を変える）
    Status(LinkStatus),
    /// 設定の口が断った（`BridgeError`。`key` は不正だった設定のキー名、`reason` は書き出し・取り込みの理由の語）
    Bridge {
        code: i32,
        key: String,
        reason: String,
    },
    /// エンジンが誤りを返した（開けていない、など）
    Engine(i32),
    /// 送る前に断った（パスの形など。理由の語）
    Refused(&'static str),
    /// 返事の種類が違う
    Unexpected,
}

/// `Hello` を交わしたつながり
pub struct SettingsLink {
    client: Client,
    next_id: AtomicU64,
}

impl SettingsLink {
    /// `Hello` を交わす。版が合わなければ `VersionMismatch`、返事が無ければ `Unavailable`。
    pub fn open(client: Client, app_version: &str) -> Result<SettingsLink, LinkStatus> {
        let link = SettingsLink {
            client,
            next_id: AtomicU64::new(1),
        };
        let id = link.next_request_id();
        match link.send(hello(id, app_version), timeouts::ADMIN) {
            Ok(Frame {
                body: Some(Body::HelloReply(_)),
                ..
            }) => Ok(link),
            Ok(Frame {
                body: Some(Body::Error(e)),
                ..
            }) if e.code == BridgeErrorCode::VersionMismatch as i32 => {
                Err(LinkStatus::VersionMismatch)
            }
            Ok(_) => Err(LinkStatus::Unavailable),
            Err(LinkError::Status(status)) => Err(status),
            Err(_) => Err(LinkStatus::Unavailable),
        }
    }

    fn next_request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn send(&self, frame: Frame, timeout: Duration) -> Result<Frame, LinkError> {
        let reply = self
            .client
            .request(&frame.encode_to_vec(), timeout)
            .map_err(|e| match e {
                ClientError::Timeout | ClientError::Disconnected | ClientError::Rejected => {
                    LinkError::Status(LinkStatus::Unavailable)
                }
                ClientError::TooLarge => LinkError::Unexpected,
            })?;
        let reply = frame_codec::decode_body(&reply).map_err(|_| LinkError::Unexpected)?;
        if reply.request_id != frame.request_id {
            return Err(LinkError::Unexpected);
        }
        if let Some(Body::Error(e)) = &reply.body {
            return Err(LinkError::Bridge {
                code: e.code,
                key: e.key.clone(),
                reason: e.reason.clone(),
            });
        }
        Ok(reply)
    }

    fn call(&self, body: Body, timeout: Duration) -> Result<Frame, LinkError> {
        let request_id = self.next_request_id();
        self.send(
            Frame {
                request_id,
                body: Some(body),
            },
            timeout,
        )
    }

    /// 今効いている設定の一式
    pub fn effective(&self) -> Result<Vec<(String, String)>, LinkError> {
        let reply = self.call(Body::GetSettings(GetSettings {}), timeouts::ADMIN)?;
        snapshot_of(reply)
    }

    /// 書き換えて、反映後の一式を受け取る
    pub fn update(&self, update: UpdateSettings) -> Result<Vec<(String, String)>, LinkError> {
        let reply = self.call(Body::UpdateSettings(update), timeouts::ADMIN)?;
        snapshot_of(reply)
    }

    /// `path` に書き出す（`replace` は画面で上書きの確認を済ませたとき）。返事の中身（全文）は空
    pub fn export_to_file(&self, path: &str, replace: bool) -> Result<ExportReply, LinkError> {
        check_transfer_path(path).map_err(|e| LinkError::Refused(e.as_str()))?;
        let reply = self.call(
            Body::ExportToFile(ExportToFile {
                path: path.to_owned(),
                replace,
            }),
            timeouts::ADMIN_TRANSFER,
        )?;
        match engine_body(reply)? {
            envelope::Body::ExportReply(export) => Ok(export),
            _ => Err(LinkError::Unexpected),
        }
    }

    /// `path` から取り込む（`dry_run` は検証して件数を返すだけ）
    pub fn import_from_file(&self, path: &str, dry_run: bool) -> Result<ImportReply, LinkError> {
        check_transfer_path(path).map_err(|e| LinkError::Refused(e.as_str()))?;
        let reply = self.call(
            Body::ImportFromFile(ImportFromFile {
                path: path.to_owned(),
                dry_run,
            }),
            timeouts::ADMIN_TRANSFER,
        )?;
        match engine_body(reply)? {
            envelope::Body::ImportReply(import) => Ok(import),
            _ => Err(LinkError::Unexpected),
        }
    }
}

fn snapshot_of(reply: Frame) -> Result<Vec<(String, String)>, LinkError> {
    match reply.body {
        Some(Body::Settings(SettingsSnapshot { effective })) => {
            Ok(effective.into_iter().map(|e| (e.key, e.value)).collect())
        }
        _ => Err(LinkError::Unexpected),
    }
}

/// エンジンの返事の本体（エンジンの誤りは `LinkError::Engine`）
fn engine_body(reply: Frame) -> Result<envelope::Body, LinkError> {
    let Some(Body::Engine(envelope)) = reply.body else {
        return Err(LinkError::Unexpected);
    };
    match envelope.body {
        Some(envelope::Body::Error(e)) => Err(LinkError::Engine(e.code)),
        Some(body) => Ok(body),
        None => Err(LinkError::Engine(ErrorCode::Unspecified as i32)),
    }
}

/// このユーザーとセッションの設定の口につなぎ、相手がインストール先の engine-host なら `Hello` まで済ませる。
#[cfg(windows)]
pub fn connect_installed(app_version: &str) -> Result<SettingsLink, LinkStatus> {
    use ikari_pipe::elevation::check_self;
    use ikari_pipe::os::windows::{
        connect_duplex, current_integrity_rid, current_session_id, current_user_sid,
        long_path_name, owner_rejection,
    };

    check_self(current_integrity_rid()).map_err(|_| LinkStatus::Elevated)?;
    let sid = current_user_sid().map_err(|_| LinkStatus::Unavailable)?;
    let session = current_session_id().map_err(|_| LinkStatus::Unavailable)?;
    let name =
        ikari_pipe::settings_pipe_name(&sid, session).map_err(|_| LinkStatus::Unavailable)?;
    // パイプの持ち主（受ける側の利用者とセッション）が自分と違えば、pipe 層が断る（ADR 0068 §2 の 2）
    let (duplex, server) = connect_duplex(&name).map_err(|e| {
        if owner_rejection(&e).is_some() {
            LinkStatus::PeerNotVerified
        } else if e.kind() == std::io::ErrorKind::NotFound {
            LinkStatus::EngineNotRunning
        } else {
            LinkStatus::Unavailable
        }
    })?;
    // 何かを送る前に、相手を照らす（違えば、何も送らずに閉じる）
    let own = std::env::current_exe()
        .ok()
        .and_then(|path| path.to_str().map(long_path_name));
    let verified = match (own, server.image_path.as_deref()) {
        (Some(own), Some(peer)) => crate::peer::verify_engine_path(&own, peer).is_ok(),
        _ => false,
    };
    if !verified {
        drop(duplex);
        return Err(LinkStatus::PeerNotVerified);
    }
    SettingsLink::open(Client::new(duplex, server.pid), app_version)
}

/// Windows 以外では、設定の口は無い
#[cfg(not(windows))]
pub fn connect_installed(_app_version: &str) -> Result<SettingsLink, LinkStatus> {
    Err(LinkStatus::EngineNotRunning)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    use ikari_engine_host::settings_pipe::{SettingsHost, serve_settings};
    use ikari_engine_host::slot::EngineSlot;
    use ikari_pipe::mem;
    use ikari_shell_core::bridge::proto::SettingEntry;
    use ikari_shell_core::settings::{KEY_PAGING_SIZE, SettingsStore};

    /// メモリ上の engine-host の設定の口（エンジンは開かない。設定の読み書きと、パスの検査だけを見る）
    struct Host {
        connector: Option<mem::MemConnector>,
        server: Option<thread::JoinHandle<()>>,
        _dir: tempfile::TempDir,
    }

    impl Host {
        fn new(version: &str) -> Host {
            let dir = tempfile::tempdir().unwrap();
            let (store, _) = SettingsStore::open(dir.path().join("settings.tsv"));
            let engine = Arc::new(EngineSlot::new(
                Box::new(|| None),
                Duration::from_secs(3600),
            ));
            let host = SettingsHost::new(engine, store, version);
            let (listener, connector) = mem::listener();
            let server = thread::spawn(move || serve_settings(listener, host).unwrap());
            Host {
                connector: Some(connector),
                server: Some(server),
                _dir: dir,
            }
        }

        fn client(&self) -> Client {
            Client::new(self.connector.as_ref().unwrap().connect().unwrap(), None)
        }
    }

    impl Drop for Host {
        fn drop(&mut self) {
            self.connector = None;
            if let Some(server) = self.server.take() {
                let _ = server.join();
            }
        }
    }

    #[test]
    fn open_reads_and_writes_the_settings() {
        let host = Host::new("0.0.0-test");
        let link = SettingsLink::open(host.client(), "test-app").unwrap();
        let before = link.effective().unwrap();
        assert!(before.iter().any(|(k, v)| k == KEY_PAGING_SIZE && v == "3"));
        let after = link
            .update(UpdateSettings {
                set: vec![SettingEntry {
                    key: KEY_PAGING_SIZE.to_owned(),
                    value: "5".to_owned(),
                }],
                reset: Vec::new(),
            })
            .unwrap();
        assert!(after.iter().any(|(k, v)| k == KEY_PAGING_SIZE && v == "5"));
        // 不正な値は、キーの名前だけで断られる
        let refused = link.update(UpdateSettings {
            set: vec![SettingEntry {
                key: KEY_PAGING_SIZE.to_owned(),
                value: "many".to_owned(),
            }],
            reset: Vec::new(),
        });
        assert!(matches!(
            refused,
            Err(LinkError::Bridge { code, ref key, .. })
                if code == BridgeErrorCode::InvalidSetting as i32 && key == KEY_PAGING_SIZE
        ));
    }

    #[test]
    fn bad_paths_are_refused_before_sending() {
        let host = Host::new("0.0.0-test");
        let link = SettingsLink::open(host.client(), "test-app").unwrap();
        assert_eq!(
            link.export_to_file(r"\\server\share\x.tsv", false),
            Err(LinkError::Refused("network-or-device"))
        );
        assert_eq!(
            link.import_from_file("relative.tsv", true),
            Err(LinkError::Refused("not-absolute"))
        );
    }

    #[test]
    fn a_closed_host_is_unavailable() {
        let host = Host::new("0.0.0-test");
        let link = SettingsLink::open(host.client(), "test-app").unwrap();
        drop(host);
        assert_eq!(
            link.effective(),
            Err(LinkError::Status(LinkStatus::Unavailable))
        );
    }
}
