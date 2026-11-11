//! パイプの名前（ユーザーの SID とセッションを含める。ADR 0040 §2.2）。

/// 名前の先頭（`\\.\pipe\` のあとの固定の部分）。
const NAME_PREFIX: &str = r"\\.\pipe\ikari-ime-engine-v1-";
/// 設定画面のパイプの名前の先頭。エンジンのパイプとは別のパイプにする（ADR 0040 §2.5。アクセスの制御を別々に持つため）。
/// 名前の先頭がエンジンのものと一致しないので、同じ SID とセッションでも衝突しない。
pub const SETTINGS_NAME_PREFIX: &str = r"\\.\pipe\ikari-ime-settings-v1-";
/// SID の長さの上限（文字数）。`S-1-5-21-` と 4 つの部分の SID でも 60 文字ほど。パイプの名前の上限（256）に収める。
const MAX_SID_CHARS: usize = 184;

/// 名前が作れない（SID に使えない文字がある、など）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipeNameError {
    InvalidSid,
}

/// `\\.\pipe\ikari-ime-engine-v1-<sid>-s<session>`。
///
/// `sid` は `S-1-5-21-…` の形の文字列（取るのは OS 層。W1-b）、`session_id` は Windows のセッション番号。
/// 別のユーザー、別のセッションのエンジンと名前がぶつからないようにする。
/// `sid` は `S-` で始まり、英数字と `-` だけで、長さが上限以下のものだけを受ける（`\` などでパイプの名前の
/// 区切りを作らせない）。
pub fn pipe_name(sid: &str, session_id: u32) -> Result<String, PipeNameError> {
    named(NAME_PREFIX, sid, session_id)
}

/// `\\.\pipe\ikari-ime-settings-v1-<sid>-s<session>`。設定画面が engine-host につなぐパイプの名前。
/// `sid` の規則は `pipe_name` と同じ。
pub fn settings_pipe_name(sid: &str, session_id: u32) -> Result<String, PipeNameError> {
    named(SETTINGS_NAME_PREFIX, sid, session_id)
}

fn named(prefix: &str, sid: &str, session_id: u32) -> Result<String, PipeNameError> {
    let valid = sid.starts_with("S-")
        && sid.len() > "S-".len()
        && sid.len() <= MAX_SID_CHARS
        && sid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !valid {
        return Err(PipeNameError::InvalidSid);
    }
    Ok(format!("{prefix}{sid}-s{session_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "S-1-5-21-1004336348-1177238915-682003330-512";

    #[test]
    fn includes_sid_and_session() {
        assert_eq!(
            pipe_name(SID, 2).unwrap(),
            r"\\.\pipe\ikari-ime-engine-v1-S-1-5-21-1004336348-1177238915-682003330-512-s2"
        );
    }

    #[test]
    fn settings_name_has_its_own_prefix_and_never_collides_with_the_engine_name() {
        assert_eq!(
            settings_pipe_name(SID, 2).unwrap(),
            r"\\.\pipe\ikari-ime-settings-v1-S-1-5-21-1004336348-1177238915-682003330-512-s2"
        );
        assert_ne!(settings_pipe_name(SID, 2), pipe_name(SID, 2));
        assert!(SETTINGS_NAME_PREFIX.starts_with(r"\\.\pipe\"));
        assert!(!NAME_PREFIX.starts_with(SETTINGS_NAME_PREFIX));
        assert!(!SETTINGS_NAME_PREFIX.starts_with(NAME_PREFIX));
        assert_ne!(settings_pipe_name(SID, 1), settings_pipe_name(SID, 2));
    }

    #[test]
    fn differs_by_user_and_session() {
        assert_ne!(pipe_name(SID, 1), pipe_name(SID, 2));
        assert_ne!(pipe_name(SID, 1), pipe_name("S-1-5-21-1-2-3-1000", 1));
    }

    #[test]
    fn rejects_sids_that_could_break_the_name() {
        for sid in [
            "",
            "S-",
            "1-5",
            r"S-1\..\x",
            "S-1/5",
            "S-1 5",
            "S-1-5\0",
            "S-1-5-日本",
        ] {
            assert_eq!(pipe_name(sid, 1), Err(PipeNameError::InvalidSid), "{sid:?}");
            assert_eq!(
                settings_pipe_name(sid, 1),
                Err(PipeNameError::InvalidSid),
                "{sid:?}"
            );
        }
        let long = format!("S-{}", "1".repeat(MAX_SID_CHARS));
        assert_eq!(pipe_name(&long, 1), Err(PipeNameError::InvalidSid));
        assert_eq!(settings_pipe_name(&long, 1), Err(PipeNameError::InvalidSid));
    }
}
