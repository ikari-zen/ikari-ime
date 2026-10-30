//! エンジンとの通信（ADR 0041 §4.3。レーン L-B）。
//!
//! `EngineLink` は、今の Swift の `XPCEngineClient` の中身（`Envelope` の組み立てと読み取り、要求番号、誤りの 4 種への写し、
//! 返事の検証、再接続の世代、落ちた回数の休止、診断の片道、管理の要求の素通し）を持つ。OS 側はバイト列を運ぶだけ。
//!
//! **契約**: 公開の関数のシグネチャ（下の `impl EngineLink`）は M1-a で固めた。L-B は中身だけを書き、シグネチャは変えない。
//! 要求と返事は `ikari_proto::v1` の型をそのまま使う（`shell-core` は proto に依存する。Swift のような写しの型は作らない）。
//!
//! スレッド: `EngineLink` は `Send + Sync`。状態機械からは 1 つのスレッドで呼ぶ。`notify_down` だけは、どのスレッドからでも呼べる。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ikari_proto::v1;

use crate::host::{Clock, DownKind, EngineTransport, TransportHandle};
use crate::types::EngineError;

/// エンジンへの口。
pub struct EngineLink {
    // M1-a の stub では使わない。L-B が再接続と休止の状態とともに使う
    #[allow(dead_code)]
    transport: Mutex<Box<dyn EngineTransport>>,
    #[allow(dead_code)]
    clock: Arc<dyn Clock>,
}

impl EngineLink {
    /// まだつながない（最初の要求のときにつなぐ）。
    pub fn new(transport: Box<dyn EngineTransport>, clock: Arc<dyn Clock>) -> Self {
        Self {
            transport: Mutex::new(transport),
            clock,
        }
    }

    /// 版の照合とエンジンの起動の前倒し（今の `hello`）。
    pub fn hello(
        &self,
        _shell_version: &str,
        _timeout: Duration,
    ) -> Result<v1::HelloReply, EngineError> {
        Err(EngineError::Unavailable)
    }

    pub fn begin_session(&self, _session: v1::BeginSession) {}

    pub fn end_session(&self, _session_id: u64) {}

    /// 読みが変わるたびに呼ぶ。返事を待たない。
    pub fn prefetch(&self, _request: v1::Prefetch) {}

    /// 同期で待つ（上限は `timeout`。今の `EngineTimeouts.convertTimeout`）。返事は検証してから返す。
    pub fn convert(
        &self,
        _request: v1::Convert,
        _timeout: Duration,
    ) -> Result<v1::ConvertReply, EngineError> {
        Err(EngineError::Unavailable)
    }

    pub fn page(
        &self,
        _conversion_id: u64,
        _index: u32,
        _timeout: Duration,
    ) -> Result<v1::ConvertReply, EngineError> {
        Err(EngineError::Unavailable)
    }

    /// `candidate_index` は生成済みの列の中の通し番号（ページをまたぐ）。
    pub fn commit(&self, _conversion_id: u64, _candidate_index: u32) {}

    /// 変換せずに確定した文字列（確定履歴のため）。
    pub fn commit_text(&self, _request: v1::CommitText) {}

    pub fn cancel(&self, _conversion_id: u64) {}

    /// 文節の候補の 1 ページ（ADR 0006 §5.2）。
    pub fn segment_page(
        &self,
        _request: v1::SegmentPageRequest,
        _timeout: Duration,
    ) -> Result<v1::SegmentPageReply, EngineError> {
        Err(EngineError::Unavailable)
    }

    /// 区切り直し。返事は新しい区切り（`layout_rev` が上がる）。
    pub fn resize(
        &self,
        _request: v1::ResizeSegment,
        _timeout: Duration,
    ) -> Result<v1::SegmentLayout, EngineError> {
        Err(EngineError::Unavailable)
    }

    /// 文節ごとの選択で確定する（返事を待たない）。
    pub fn commit_segments(&self, _request: v1::CommitSegments) {}

    pub fn reconvert(
        &self,
        _request: v1::Reconvert,
        _timeout: Duration,
    ) -> Result<v1::SegmentLayout, EngineError> {
        Err(EngineError::Unavailable)
    }

    /// 保存して終了するよう頼む。終了は落ちた回数に数えない。
    pub fn shutdown(&self, _reason: v1::ShutdownReason) {}

    /// 管理の要求（ADR 0005 §8.3）。`Envelope` のバイト列をそのまま送り、返事のバイト列をそのまま返す。
    /// `request_id` は付け直す。許可リストでの判定は呼ぶ側（`bridge`）が行う。
    pub fn relay(&self, _envelope: &[u8], _timeout: Duration) -> Result<Vec<u8>, EngineError> {
        Err(EngineError::Unavailable)
    }

    /// 絵文字のショートコードの表（ADR 0027 §9.2）。
    pub fn emoji_table(&self, _timeout: Duration) -> Result<v1::EmojiTableReply, EngineError> {
        Err(EngineError::Unavailable)
    }

    /// 診断ログの事象（ADR 0034 §2.2）。返事を待たない。secure のセッションでは呼ばない。
    pub fn diag(&self, _event: v1::DiagEvent) {}

    /// 英語のモードの郵便番号（ADR 0032 §9.1）。`code` は ASCII 数字 7 桁。
    pub fn postal_lookup(
        &self,
        _code: &str,
        _timeout: Duration,
    ) -> Result<Vec<String>, EngineError> {
        Err(EngineError::Unavailable)
    }

    /// 今の接続の相手のプロセスの番号。
    pub fn pid(&self) -> Option<i32> {
        None
    }

    /// 接続が落ちた（**どのスレッドからでも呼べる**。ADR 0041 §4.3）。
    /// `connection` が今の接続でなければ（古い世代の通知なら）捨てる。
    pub fn notify_down(&self, _connection: TransportHandle, _kind: DownKind) {}
}
