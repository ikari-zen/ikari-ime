//! 状態機械から見たエンジン（今の `EngineClient` の、状態機械が使う部分）。
//!
//! 本番は `EngineLink`（L-B）。テストは替え玉を使う。要求と返事は `ikari_proto::v1` の型をそのまま使う。
//! `convert`・`page`・文節・再変換は同期で待つ（ADR 0004 §7.5）。それ以外は返事を待たない。

use std::time::Duration;

use ikari_proto::v1;

use crate::diag::DiagEvent;
use crate::engine::{EngineLink, diag_message};
use crate::types::EngineError;

/// 状態機械が呼ぶエンジンの口。
pub trait MachineEngine: Send + Sync {
    fn begin_session(&self, session: v1::BeginSession);
    fn end_session(&self, session_id: u64);
    fn prefetch(&self, request: v1::Prefetch);
    fn convert(
        &self,
        request: v1::Convert,
        timeout: Duration,
    ) -> Result<v1::ConvertReply, EngineError>;
    fn page(
        &self,
        conversion_id: u64,
        index: u32,
        timeout: Duration,
    ) -> Result<v1::ConvertReply, EngineError>;
    fn commit(&self, conversion_id: u64, candidate_index: u32);
    fn commit_text(&self, request: v1::CommitText);
    fn cancel(&self, conversion_id: u64);
    fn segment_page(
        &self,
        request: v1::SegmentPageRequest,
        timeout: Duration,
    ) -> Result<v1::SegmentPageReply, EngineError>;
    fn resize(
        &self,
        request: v1::ResizeSegment,
        timeout: Duration,
    ) -> Result<v1::SegmentLayout, EngineError>;
    /// 変換中の文節を取り除く（ADR 0050 §5.3）。返事は新しい区切り
    fn remove_segments(
        &self,
        request: v1::RemoveSegments,
        timeout: Duration,
    ) -> Result<v1::SegmentLayout, EngineError>;
    fn commit_segments(&self, request: v1::CommitSegments);
    fn reconvert(
        &self,
        request: v1::Reconvert,
        timeout: Duration,
    ) -> Result<v1::SegmentLayout, EngineError>;
    /// 診断ログの事象（返事を待たない。secure のセッションでは呼ばない）
    fn diag(&self, event: &DiagEvent);
}

impl MachineEngine for EngineLink {
    fn begin_session(&self, session: v1::BeginSession) {
        EngineLink::begin_session(self, session);
    }

    fn end_session(&self, session_id: u64) {
        EngineLink::end_session(self, session_id);
    }

    fn prefetch(&self, request: v1::Prefetch) {
        EngineLink::prefetch(self, request);
    }

    fn convert(
        &self,
        request: v1::Convert,
        timeout: Duration,
    ) -> Result<v1::ConvertReply, EngineError> {
        EngineLink::convert(self, request, timeout)
    }

    fn page(
        &self,
        conversion_id: u64,
        index: u32,
        timeout: Duration,
    ) -> Result<v1::ConvertReply, EngineError> {
        EngineLink::page(self, conversion_id, index, timeout)
    }

    fn commit(&self, conversion_id: u64, candidate_index: u32) {
        EngineLink::commit(self, conversion_id, candidate_index);
    }

    fn commit_text(&self, request: v1::CommitText) {
        EngineLink::commit_text(self, request);
    }

    fn cancel(&self, conversion_id: u64) {
        EngineLink::cancel(self, conversion_id);
    }

    fn segment_page(
        &self,
        request: v1::SegmentPageRequest,
        timeout: Duration,
    ) -> Result<v1::SegmentPageReply, EngineError> {
        EngineLink::segment_page(self, request, timeout)
    }

    fn resize(
        &self,
        request: v1::ResizeSegment,
        timeout: Duration,
    ) -> Result<v1::SegmentLayout, EngineError> {
        EngineLink::resize(self, request, timeout)
    }

    fn remove_segments(
        &self,
        request: v1::RemoveSegments,
        timeout: Duration,
    ) -> Result<v1::SegmentLayout, EngineError> {
        EngineLink::remove_segments(self, request, timeout)
    }

    fn commit_segments(&self, request: v1::CommitSegments) {
        EngineLink::commit_segments(self, request);
    }

    fn reconvert(
        &self,
        request: v1::Reconvert,
        timeout: Duration,
    ) -> Result<v1::SegmentLayout, EngineError> {
        EngineLink::reconvert(self, request, timeout)
    }

    fn diag(&self, event: &DiagEvent) {
        EngineLink::diag(self, diag_message(event));
    }
}
