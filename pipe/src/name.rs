//! パイプの名前（ユーザーの SID とセッションを含める。ADR 0040 §2.2）。

/// 名前が作れない（SID に使えない文字がある、など）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipeNameError {
    InvalidSid,
}

/// `\\.\pipe\ikari-ime-engine-v1-<sid>-s<session>`。
///
/// `sid` は `S-1-5-21-…` の形の文字列（取るのは OS 層。W1-b）、`session_id` は Windows のセッション番号。
/// 別のユーザー、別のセッションのエンジンと名前がぶつからないようにする。
pub fn pipe_name(_sid: &str, _session_id: u32) -> Result<String, PipeNameError> {
    todo!()
}
